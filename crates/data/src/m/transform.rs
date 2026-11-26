//! The M step each transform of Power Query's editor writes: `Table.SelectRows(#"Prev", each …)`
//! and the like, with the step name the editor gives it ("Filtered Rows", "Changed Type"…).
//!
//! The function shapes are the public Power Query M function reference's (Microsoft Learn, "Power
//! Query M function reference": `Table.SelectRows`, `Table.RemoveColumns`, `Table.SelectColumns`,
//! `Table.RenameColumns`, `Table.TransformColumnTypes`, `Table.PromoteHeaders`, `Table.Sort`,
//! `Table.Group`, `Table.NestedJoin`, `Table.ExpandTableColumn`, `Table.Combine`,
//! `Table.Distinct`, `Table.ReplaceValue`, `Table.AddColumn`, `Table.SplitColumn`,
//! `Table.Unpivot`, `Table.UnpivotOtherColumns`, `Table.Pivot`), written the way queries saved by
//! Power BI Desktop read (black-box observation of saved queries). The English step names follow
//! those queries; a Desktop in another language names its steps in that language (unchecked
//! here: BIcraft always writes the English names).

use super::catalog::identifier;

/// `name` as an M identifier: as written when regular, else `#"…"` with the text escapes
/// (`""`, `#(#)`, `#(lf)`), which quoted identifiers share with text literals.
pub fn ident(name: &str) -> String {
    let id = identifier(name);
    if id.starts_with("#\"") {
        format!("#{}", text(name))
    } else {
        id
    }
}

/// A literal value of a transform (a filter's value, a replaced value).
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Logical(bool),
    Number(f64),
    Text(String),
    /// `#date(year, month, day)`.
    Date(i32, u32, u32),
    /// `#datetime(year, month, day, hour, minute, second)`.
    DateTime(i32, u32, u32, u32, u32, u32),
}

impl Literal {
    /// The value as M source text.
    pub fn m(&self) -> String {
        match self {
            Literal::Null => "null".into(),
            Literal::Logical(b) => if *b { "true" } else { "false" }.into(),
            Literal::Number(n) if n.is_finite() => format!("{n}"),
            Literal::Number(_) => "null".into(),
            Literal::Text(t) => text(t),
            Literal::Date(y, m, d) => format!("#date({y}, {m}, {d})"),
            Literal::DateTime(y, m, d, h, mi, sec) => {
                format!("#datetime({y}, {m}, {d}, {h}, {mi}, {sec})")
            }
        }
    }
}

/// A text literal: `"…"` with `"` doubled and control characters escaped (`#(lf)`).
pub fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\"\""),
            '\r' => out.push_str("#(cr)"),
            '\n' => out.push_str("#(lf)"),
            '\t' => out.push_str("#(tab)"),
            '#' if chars.peek() == Some(&'(') => out.push_str("#(#)"),
            c if c.is_control() => out.push_str(&format!("#({:04X})", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A list of texts: `{"A", "B"}`.
fn texts(items: &[String]) -> String {
    format!(
        "{{{}}}",
        items.iter().map(|s| text(s)).collect::<Vec<_>>().join(", ")
    )
}

/// A field access inside `each`: `[Name]`, `[#"odd]name"]`.
pub fn field(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '_' || c == '.')
        && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
        && !name.contains("..");
    if plain {
        format!("[{name}]")
    } else {
        format!("[{}]", identifier_quoted(name))
    }
}

fn identifier_quoted(name: &str) -> String {
    format!("#{}", text(name))
}

/// A filter's comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Equals,
    NotEquals,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
    IsNull,
    IsNotNull,
}

impl FilterOp {
    pub const ALL: [FilterOp; 12] = [
        FilterOp::Equals,
        FilterOp::NotEquals,
        FilterOp::Greater,
        FilterOp::GreaterOrEqual,
        FilterOp::Less,
        FilterOp::LessOrEqual,
        FilterOp::Contains,
        FilterOp::NotContains,
        FilterOp::BeginsWith,
        FilterOp::EndsWith,
        FilterOp::IsNull,
        FilterOp::IsNotNull,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FilterOp::Equals => "equals",
            FilterOp::NotEquals => "not_equals",
            FilterOp::Greater => "greater",
            FilterOp::GreaterOrEqual => "greater_or_equal",
            FilterOp::Less => "less",
            FilterOp::LessOrEqual => "less_or_equal",
            FilterOp::Contains => "contains",
            FilterOp::NotContains => "not_contains",
            FilterOp::BeginsWith => "begins_with",
            FilterOp::EndsWith => "ends_with",
            FilterOp::IsNull => "is_null",
            FilterOp::IsNotNull => "is_not_null",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.name().eq_ignore_ascii_case(s))
    }

    /// Whether the comparison takes a value.
    pub fn takes_value(self) -> bool {
        !matches!(self, FilterOp::IsNull | FilterOp::IsNotNull)
    }

    pub(crate) fn condition(self, column: &str, v: &Literal) -> String {
        let f = field(column);
        let x = v.m();
        match self {
            FilterOp::Equals => format!("{f} = {x}"),
            FilterOp::NotEquals => format!("{f} <> {x}"),
            FilterOp::Greater => format!("{f} > {x}"),
            FilterOp::GreaterOrEqual => format!("{f} >= {x}"),
            FilterOp::Less => format!("{f} < {x}"),
            FilterOp::LessOrEqual => format!("{f} <= {x}"),
            FilterOp::Contains => format!("Text.Contains({f}, {x})"),
            FilterOp::NotContains => format!("not Text.Contains({f}, {x})"),
            FilterOp::BeginsWith => format!("Text.StartsWith({f}, {x})"),
            FilterOp::EndsWith => format!("Text.EndsWith({f}, {x})"),
            FilterOp::IsNull => format!("{f} = null"),
            FilterOp::IsNotNull => format!("{f} <> null"),
        }
    }
}

/// A column type of Change type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Text,
    Whole,
    Decimal,
    Currency,
    Percentage,
    Date,
    DateTime,
    DateTimeZone,
    Time,
    Duration,
    Logical,
    Any,
}

impl ColumnType {
    pub const ALL: [ColumnType; 12] = [
        ColumnType::Text,
        ColumnType::Whole,
        ColumnType::Decimal,
        ColumnType::Currency,
        ColumnType::Percentage,
        ColumnType::Date,
        ColumnType::DateTime,
        ColumnType::DateTimeZone,
        ColumnType::Time,
        ColumnType::Duration,
        ColumnType::Logical,
        ColumnType::Any,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ColumnType::Text => "text",
            ColumnType::Whole => "whole",
            ColumnType::Decimal => "decimal",
            ColumnType::Currency => "currency",
            ColumnType::Percentage => "percentage",
            ColumnType::Date => "date",
            ColumnType::DateTime => "datetime",
            ColumnType::DateTimeZone => "datetimezone",
            ColumnType::Time => "time",
            ColumnType::Duration => "duration",
            ColumnType::Logical => "logical",
            ColumnType::Any => "any",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.name().eq_ignore_ascii_case(s))
    }

    /// The type as `Table.TransformColumnTypes` writes it.
    pub fn m(self) -> &'static str {
        match self {
            ColumnType::Text => "type text",
            ColumnType::Whole => "Int64.Type",
            ColumnType::Decimal => "type number",
            ColumnType::Currency => "Currency.Type",
            ColumnType::Percentage => "Percentage.Type",
            ColumnType::Date => "type date",
            ColumnType::DateTime => "type datetime",
            ColumnType::DateTimeZone => "type datetimezone",
            ColumnType::Time => "type time",
            ColumnType::Duration => "type duration",
            ColumnType::Logical => "type logical",
            ColumnType::Any => "type any",
        }
    }
}

/// An aggregation of Group by (and Pivot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aggregate {
    Count,
    CountDistinct,
    Sum,
    Average,
    Median,
    Min,
    Max,
    AllRows,
}

impl Aggregate {
    pub const ALL: [Aggregate; 8] = [
        Aggregate::Count,
        Aggregate::CountDistinct,
        Aggregate::Sum,
        Aggregate::Average,
        Aggregate::Median,
        Aggregate::Min,
        Aggregate::Max,
        Aggregate::AllRows,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Aggregate::Count => "count",
            Aggregate::CountDistinct => "count_distinct",
            Aggregate::Sum => "sum",
            Aggregate::Average => "average",
            Aggregate::Median => "median",
            Aggregate::Min => "min",
            Aggregate::Max => "max",
            Aggregate::AllRows => "all_rows",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.name().eq_ignore_ascii_case(s))
    }

    /// Whether it reads a column (Count and All rows count or keep the rows).
    pub fn takes_column(self) -> bool {
        !matches!(
            self,
            Aggregate::Count | Aggregate::CountDistinct | Aggregate::AllRows
        )
    }

    fn list_function(self) -> Option<&'static str> {
        Some(match self {
            Aggregate::Sum => "List.Sum",
            Aggregate::Average => "List.Average",
            Aggregate::Median => "List.Median",
            Aggregate::Min => "List.Min",
            Aggregate::Max => "List.Max",
            Aggregate::Count | Aggregate::CountDistinct | Aggregate::AllRows => return None,
        })
    }

    /// `{"Name", each …, type}` of `Table.Group`.
    fn group_item(self, name: &str, column: Option<&str>) -> Result<String, String> {
        let n = text(name);
        Ok(match self {
            Aggregate::Count => format!("{{{n}, each Table.RowCount(_), Int64.Type}}"),
            Aggregate::CountDistinct => {
                format!("{{{n}, each Table.RowCount(Table.Distinct(_)), Int64.Type}}")
            }
            Aggregate::AllRows => format!("{{{n}, each _, type table}}"),
            _ => {
                let c = column.ok_or_else(|| format!("aggregation `{}` needs a column", self.name()))?;
                let f = self.list_function().unwrap_or("List.Sum");
                let ty = match self {
                    Aggregate::Sum | Aggregate::Average | Aggregate::Median => ", type nullable number",
                    _ => "",
                };
                format!("{{{n}, each {f}({}){ty}}}", field(c))
            }
        })
    }
}

/// A join of Merge queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    LeftOuter,
    RightOuter,
    FullOuter,
    Inner,
    LeftAnti,
    RightAnti,
}

impl JoinKind {
    pub const ALL: [JoinKind; 6] = [
        JoinKind::LeftOuter,
        JoinKind::RightOuter,
        JoinKind::FullOuter,
        JoinKind::Inner,
        JoinKind::LeftAnti,
        JoinKind::RightAnti,
    ];

    pub fn name(self) -> &'static str {
        match self {
            JoinKind::LeftOuter => "left_outer",
            JoinKind::RightOuter => "right_outer",
            JoinKind::FullOuter => "full_outer",
            JoinKind::Inner => "inner",
            JoinKind::LeftAnti => "left_anti",
            JoinKind::RightAnti => "right_anti",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.name().eq_ignore_ascii_case(s))
    }

    fn m(self) -> &'static str {
        match self {
            JoinKind::LeftOuter => "JoinKind.LeftOuter",
            JoinKind::RightOuter => "JoinKind.RightOuter",
            JoinKind::FullOuter => "JoinKind.FullOuter",
            JoinKind::Inner => "JoinKind.Inner",
            JoinKind::LeftAnti => "JoinKind.LeftAnti",
            JoinKind::RightAnti => "JoinKind.RightAnti",
        }
    }
}

/// One transform of the editor's ribbon.
#[derive(Debug, Clone, PartialEq)]
pub enum Transform {
    /// Keep the rows where `column op value` holds.
    FilterRows {
        column: String,
        op: FilterOp,
        value: Literal,
    },
    /// The value list of a column's filter menu: keep the rows whose `column` is one of `values`
    /// (`exclude`: is none of them), as `each ([C] = a or [C] = b)` / `each ([C] <> a and …)`.
    FilterValues {
        column: String,
        values: Vec<Literal>,
        exclude: bool,
    },
    RemoveColumns(Vec<String>),
    /// Remove other columns: keep these, in this order.
    KeepColumns(Vec<String>),
    /// (old, new) names.
    RenameColumns(Vec<(String, String)>),
    ChangeType(Vec<(String, ColumnType)>),
    PromoteHeaders,
    /// (column, descending).
    Sort(Vec<(String, bool)>),
    /// Rows unique over these columns (none: whole rows).
    RemoveDuplicates(Vec<String>),
    ReplaceValues {
        columns: Vec<String>,
        find: Literal,
        replace: Literal,
    },
    GroupBy {
        keys: Vec<String>,
        /// (new column, aggregation, column read).
        aggregations: Vec<(String, Aggregate, Option<String>)>,
    },
    /// Merge queries: a nested join with query `other`, in new column `column`.
    Merge {
        other: String,
        keys: Vec<String>,
        other_keys: Vec<String>,
        kind: JoinKind,
        column: String,
    },
    /// Expand a table column (a merge's) into these columns, named `column.name` when `prefix`.
    Expand {
        column: String,
        columns: Vec<String>,
        prefix: bool,
    },
    /// Append queries: this table then the `others`.
    Append(Vec<String>),
    /// Add a column computed by M `formula` for each row (`each` is added).
    AddColumn {
        name: String,
        formula: String,
    },
    /// Split a text column by a delimiter into `parts` columns `column.1`…
    SplitColumn {
        column: String,
        delimiter: String,
        parts: usize,
    },
    /// Unpivot these columns (`other`: every column but these).
    Unpivot {
        columns: Vec<String>,
        other: bool,
    },
    /// Pivot `column`'s values into columns holding `values`, aggregated (none: Don't aggregate).
    Pivot {
        column: String,
        values: String,
        aggregate: Option<Aggregate>,
    },
    /// Insert step after: the step before, as is (`= #"Prev"`), to edit in the formula bar.
    Custom,
    /// The row, column, text, number and date transforms of [`super::transform_more`].
    More(super::transform_more::More),
}

/// Most columns one transform names.
pub const MAX_COLUMNS: usize = 4_096;
/// Most parts Split column makes.
pub const MAX_PARTS: usize = 1_000;

fn nonempty(what: &str, items: &[String]) -> Result<(), String> {
    if items.is_empty() {
        return Err(format!("{what}: name at least one column"));
    }
    if items.len() > MAX_COLUMNS {
        return Err(format!("{what}: at most {MAX_COLUMNS} columns"));
    }
    if items.iter().any(|c| c.is_empty()) {
        return Err(format!("{what}: a column name cannot be empty"));
    }
    Ok(())
}

impl Transform {
    /// The kind's name (`filter_rows`…).
    pub fn kind(&self) -> &'static str {
        match self {
            Transform::FilterRows { .. } => "filter_rows",
            Transform::FilterValues { .. } => "filter_values",
            Transform::RemoveColumns(_) => "remove_columns",
            Transform::KeepColumns(_) => "keep_columns",
            Transform::RenameColumns(_) => "rename_columns",
            Transform::ChangeType(_) => "change_type",
            Transform::PromoteHeaders => "promote_headers",
            Transform::Sort(_) => "sort",
            Transform::RemoveDuplicates(_) => "remove_duplicates",
            Transform::ReplaceValues { .. } => "replace_values",
            Transform::GroupBy { .. } => "group_by",
            Transform::Merge { .. } => "merge",
            Transform::Expand { .. } => "expand",
            Transform::Append(_) => "append",
            Transform::AddColumn { .. } => "add_column",
            Transform::SplitColumn { .. } => "split_column",
            Transform::Unpivot { .. } => "unpivot",
            Transform::Pivot { .. } => "pivot",
            Transform::Custom => "custom",
            Transform::More(x) => x.kind(),
        }
    }

    /// The step name the editor gives the step.
    pub fn step_name(&self) -> String {
        match self {
            Transform::FilterRows { .. } | Transform::FilterValues { .. } => "Filtered Rows".into(),
            Transform::RemoveColumns(_) => "Removed Columns".into(),
            Transform::KeepColumns(_) => "Removed Other Columns".into(),
            Transform::RenameColumns(_) => "Renamed Columns".into(),
            Transform::ChangeType(_) => "Changed Type".into(),
            Transform::PromoteHeaders => "Promoted Headers".into(),
            Transform::Sort(_) => "Sorted Rows".into(),
            Transform::RemoveDuplicates(_) => "Removed Duplicates".into(),
            Transform::ReplaceValues { .. } => "Replaced Value".into(),
            Transform::GroupBy { .. } => "Grouped Rows".into(),
            Transform::Merge { .. } => "Merged Queries".into(),
            Transform::Expand { column, .. } => format!("Expanded {column}"),
            Transform::Append(_) => "Appended Query".into(),
            Transform::AddColumn { .. } => "Added Custom".into(),
            Transform::SplitColumn { .. } => "Split Column by Delimiter".into(),
            Transform::Unpivot { other: true, .. } => "Unpivoted Other Columns".into(),
            Transform::Unpivot { other: false, .. } => "Unpivoted Columns".into(),
            Transform::Pivot { .. } => "Pivoted Column".into(),
            Transform::Custom => "Custom".into(),
            Transform::More(x) => x.step_name(),
        }
    }

    /// The step's M, reading step `prev` (its name, unquoted).
    pub fn m(&self, prev: &str) -> Result<String, String> {
        let p = ident(prev);
        Ok(match self {
            Transform::FilterRows { column, op, value } => {
                if column.is_empty() {
                    return Err("filter_rows: name the column".into());
                }
                format!("Table.SelectRows({p}, each {})", op.condition(column, value))
            }
            Transform::FilterValues {
                column,
                values,
                exclude,
            } => {
                if column.is_empty() {
                    return Err("filter_values: name the column".into());
                }
                if values.is_empty() || values.len() > MAX_COLUMNS {
                    return Err(format!("filter_values: 1 to {MAX_COLUMNS} values"));
                }
                let f = field(column);
                let (op, join) = if *exclude { ("<>", " and ") } else { ("=", " or ") };
                let items: Vec<String> = values.iter().map(|v| format!("{f} {op} {}", v.m())).collect();
                format!("Table.SelectRows({p}, each ({}))", items.join(join))
            }
            Transform::RemoveColumns(c) => {
                nonempty("remove_columns", c)?;
                format!("Table.RemoveColumns({p},{})", texts(c))
            }
            Transform::KeepColumns(c) => {
                nonempty("keep_columns", c)?;
                format!("Table.SelectColumns({p},{})", texts(c))
            }
            Transform::RenameColumns(r) => {
                if r.is_empty() || r.len() > MAX_COLUMNS {
                    return Err(format!("rename_columns: 1 to {MAX_COLUMNS} renames"));
                }
                if r.iter().any(|(a, b)| a.is_empty() || b.trim().is_empty()) {
                    return Err("rename_columns: a column name cannot be empty".into());
                }
                let items: Vec<String> = r
                    .iter()
                    .map(|(a, b)| format!("{{{}, {}}}", text(a), text(b)))
                    .collect();
                format!("Table.RenameColumns({p},{{{}}})", items.join(", "))
            }
            Transform::ChangeType(c) => {
                if c.is_empty() || c.len() > MAX_COLUMNS {
                    return Err(format!("change_type: 1 to {MAX_COLUMNS} columns"));
                }
                let items: Vec<String> = c
                    .iter()
                    .map(|(n, t)| format!("{{{}, {}}}", text(n), t.m()))
                    .collect();
                format!("Table.TransformColumnTypes({p},{{{}}})", items.join(", "))
            }
            Transform::PromoteHeaders => format!("Table.PromoteHeaders({p}, [PromoteAllScalars=true])"),
            Transform::Sort(keys) => {
                if keys.is_empty() || keys.len() > MAX_COLUMNS {
                    return Err(format!("sort: 1 to {MAX_COLUMNS} columns"));
                }
                let items: Vec<String> = keys
                    .iter()
                    .map(|(c, desc)| {
                        format!(
                            "{{{}, {}}}",
                            text(c),
                            if *desc {
                                "Order.Descending"
                            } else {
                                "Order.Ascending"
                            }
                        )
                    })
                    .collect();
                format!("Table.Sort({p},{{{}}})", items.join(", "))
            }
            Transform::RemoveDuplicates(c) => {
                if c.len() > MAX_COLUMNS {
                    return Err(format!("remove_duplicates: at most {MAX_COLUMNS} columns"));
                }
                if c.is_empty() {
                    format!("Table.Distinct({p})")
                } else {
                    format!("Table.Distinct({p}, {})", texts(c))
                }
            }
            Transform::ReplaceValues {
                columns,
                find,
                replace,
            } => {
                nonempty("replace_values", columns)?;
                let replacer = match (find, replace) {
                    (Literal::Text(_), Literal::Text(_)) => "Replacer.ReplaceText",
                    _ => "Replacer.ReplaceValue",
                };
                format!(
                    "Table.ReplaceValue({p},{},{},{replacer},{})",
                    find.m(),
                    replace.m(),
                    texts(columns)
                )
            }
            Transform::GroupBy { keys, aggregations } => {
                if keys.len() > MAX_COLUMNS || aggregations.len() > MAX_COLUMNS {
                    return Err(format!("group_by: at most {MAX_COLUMNS} keys and aggregations"));
                }
                if aggregations.is_empty() {
                    return Err("group_by: add at least one aggregation".into());
                }
                let items = aggregations
                    .iter()
                    .map(|(n, a, c)| {
                        if n.trim().is_empty() {
                            return Err("group_by: an aggregation needs a column name".to_string());
                        }
                        a.group_item(n, c.as_deref())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                format!("Table.Group({p}, {}, {{{}}})", texts(keys), items.join(", "))
            }
            Transform::Merge {
                other,
                keys,
                other_keys,
                kind,
                column,
            } => {
                nonempty("merge", keys)?;
                if keys.len() != other_keys.len() {
                    return Err("merge: name as many columns of the other query as of this one".into());
                }
                if other.trim().is_empty() || column.trim().is_empty() {
                    return Err("merge: name the other query and the new column".into());
                }
                format!(
                    "Table.NestedJoin({p}, {}, {}, {}, {}, {})",
                    texts(keys),
                    ident(other),
                    texts(other_keys),
                    text(column),
                    kind.m()
                )
            }
            Transform::Expand {
                column,
                columns,
                prefix,
            } => {
                nonempty("expand", columns)?;
                if column.is_empty() {
                    return Err("expand: name the column to expand".into());
                }
                let names: Vec<String> = columns
                    .iter()
                    .map(|c| {
                        if *prefix {
                            format!("{column}.{c}")
                        } else {
                            c.clone()
                        }
                    })
                    .collect();
                format!(
                    "Table.ExpandTableColumn({p}, {}, {}, {})",
                    text(column),
                    texts(columns),
                    texts(&names)
                )
            }
            Transform::Append(others) => {
                if others.is_empty() || others.len() > MAX_COLUMNS {
                    return Err(format!("append: name 1 to {MAX_COLUMNS} queries"));
                }
                if others.iter().any(|o| o.trim().is_empty()) {
                    return Err("append: a query name cannot be empty".into());
                }
                let mut items = vec![p];
                items.extend(others.iter().map(|o| ident(o)));
                format!("Table.Combine({{{}}})", items.join(", "))
            }
            Transform::AddColumn { name, formula } => {
                if name.trim().is_empty() {
                    return Err("add_column: name the new column".into());
                }
                let f = formula.trim();
                let f = f.strip_prefix('=').map_or(f, str::trim_start);
                if f.is_empty() {
                    return Err("add_column: write the column's formula".into());
                }
                format!("Table.AddColumn({p}, {}, each {f})", text(name))
            }
            Transform::SplitColumn {
                column,
                delimiter,
                parts,
            } => {
                if column.is_empty() || delimiter.is_empty() {
                    return Err("split_column: name the column and the delimiter".into());
                }
                if *parts < 1 || *parts > MAX_PARTS {
                    return Err(format!("split_column: 1 to {MAX_PARTS} parts"));
                }
                let names: Vec<String> = (1..=*parts).map(|i| format!("{column}.{i}")).collect();
                format!(
                    "Table.SplitColumn({p}, {}, Splitter.SplitTextByDelimiter({}, QuoteStyle.Csv), {})",
                    text(column),
                    text(delimiter),
                    texts(&names)
                )
            }
            Transform::Unpivot { columns, other } => {
                nonempty("unpivot", columns)?;
                let f = if *other {
                    "Table.UnpivotOtherColumns"
                } else {
                    "Table.Unpivot"
                };
                format!("{f}({p}, {}, \"Attribute\", \"Value\")", texts(columns))
            }
            Transform::Pivot {
                column,
                values,
                aggregate,
            } => {
                if column.is_empty() || values.is_empty() {
                    return Err("pivot: name the column to pivot and the values column".into());
                }
                let agg = match aggregate {
                    None => String::new(),
                    Some(Aggregate::Count) => ", List.Count".into(),
                    Some(a) => match a.list_function() {
                        Some(f) => format!(", {f}"),
                        None => return Err(format!("pivot: `{}` cannot aggregate values", a.name())),
                    },
                };
                format!(
                    "Table.Pivot({p}, List.Distinct({p}{}), {}, {}{agg})",
                    field(column),
                    text(column),
                    text(values)
                )
            }
            Transform::Custom => p,
            Transform::More(x) => return x.m(prev),
        })
    }
}
