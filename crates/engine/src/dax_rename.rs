//! Keeping DAX consistent through renames and removals (G15).
//!
//! Every DAX expression the model stores is a [`DaxSite`]: measures, calculated columns,
//! calculated tables (field parameters included: `NAMEOF('T'[C])` is an ordinary reference), row
//! filters, KPI target/status/trend expressions, dynamic format strings, detail rows expressions,
//! calculation items (and their format strings), a calculation group's selection expressions and
//! the bodies of the model's user-defined functions (`functions`). A rename rewrites the
//! references in all of them ([`rename_in_model`]), a measure moved to another table gets its
//! `Table[Measure]` qualifiers rewritten ([`DaxChange::MoveMeasure`]); a removal lists the ones
//! that reference the removed object ([`dependants`]).
//!
//! Not DAX, so never touched: M (Power Query) partitions and shared expressions (Desktop does not
//! rewrite them either), refresh policies, annotations, and strings inside DAX.

use bicraft_dax::deps::ObjectRef;
use bicraft_dax::rename::{Renamer, Rewrite, Site};
use bicraft_model::SemanticModel;
use serde_json::{Map, Value, json};

/// Most expressions listed in an error message.
const MAX_IN_MESSAGE: usize = 20;

/// One step into a JSON property.
#[derive(Debug, Clone)]
enum Seg {
    Key(&'static str),
    Index(usize),
}

/// Where an expression is stored.
#[derive(Debug, Clone)]
enum Loc {
    Measure(usize, usize),
    Column(usize, usize),
    Partition(usize, usize),
    RowFilter(usize, usize),
    /// Inside a measure's unmodelled properties (`kpi`, `formatStringDefinition`, …).
    MeasureExtra(usize, usize, Vec<Seg>),
    /// Inside a table's unmodelled properties (`calculationGroup`, `defaultDetailRowsDefinition`).
    TableExtra(usize, Vec<Seg>),
    /// The expression of the model's user-defined function `functions[i]`.
    Function(usize),
}

/// One DAX expression of the model.
#[derive(Debug, Clone)]
pub(crate) struct DaxSite {
    loc: Loc,
    /// `measure`, `calculated column`, `kpi target`, …
    kind: &'static str,
    /// The table the object belongs to (the table of a row filter).
    table: String,
    /// The object's name: measure, column, table, role, calculation item.
    name: String,
    /// The owning measure of a measure property (`kpi`, format string, detail rows).
    measure: Option<String>,
    /// Home table for name resolution (`None` for calculated tables).
    home: Option<String>,
    row_context: bool,
    text: String,
}

/// What a [`DaxSite`] belongs to, for the usage graph (`usage.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SiteOwner<'a> {
    Measure {
        table: &'a str,
        name: &'a str,
    },
    Column {
        table: &'a str,
        name: &'a str,
    },
    /// A calculated table's definition or the table's default detail rows expression.
    Table(&'a str),
    /// A calculation item, its format string or a selection expression of a calculation group.
    CalculationGroup(&'a str),
    RowFilter {
        role: &'a str,
        table: &'a str,
    },
    Function(&'a str),
}

impl DaxSite {
    pub(crate) fn owner(&self) -> SiteOwner<'_> {
        match &self.loc {
            Loc::Measure(..) | Loc::MeasureExtra(..) => SiteOwner::Measure {
                table: &self.table,
                name: self.measure.as_deref().unwrap_or(&self.name),
            },
            Loc::Column(..) => SiteOwner::Column {
                table: &self.table,
                name: &self.name,
            },
            Loc::Partition(..) => SiteOwner::Table(&self.table),
            Loc::TableExtra(..) if self.kind == "default detail rows expression" => {
                SiteOwner::Table(&self.table)
            }
            Loc::TableExtra(..) => SiteOwner::CalculationGroup(&self.table),
            Loc::RowFilter(..) => SiteOwner::RowFilter {
                role: &self.name,
                table: &self.table,
            },
            Loc::Function(..) => SiteOwner::Function(&self.name),
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn site(&self) -> Site<'_> {
        Site {
            home: self.home.as_deref(),
            row_context: self.row_context,
            udf: matches!(self.loc, Loc::Function(_)),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("kind".into(), json!(self.kind));
        o.insert("table".into(), json!(self.table));
        o.insert("name".into(), json!(self.name));
        if let Some(m) = &self.measure {
            o.insert("measure".into(), json!(m));
        }
        Value::Object(o)
    }

    pub(crate) fn label(&self) -> String {
        match &self.measure {
            Some(m) => format!("{} of measure [{m}]", self.kind),
            None if self.kind == "row filter" => {
                format!("row filter of role `{}` on `{}`", self.name, self.table)
            }
            None if self.kind == "calculated table" => format!("calculated table `{}`", self.table),
            None if self.kind == "function" => format!("function `{}`", self.name),
            None => format!("{} `{}`[{}]", self.kind, self.table, self.name),
        }
    }
}

/// The text of a TMSL expression property: a string, or an array of lines.
fn expression_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(lines) => lines
            .iter()
            .map(|l| l.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(|l| l.join("\n")),
        _ => None,
    }
}

fn get_path<'v>(root: &'v Map<String, Value>, path: &[Seg]) -> Option<&'v Value> {
    let (first, rest) = path.split_first()?;
    let Seg::Key(k) = first else { return None };
    let mut cur = root.get(*k)?;
    for seg in rest {
        cur = match seg {
            Seg::Key(k) => cur.get(*k)?,
            Seg::Index(i) => cur.get(*i)?,
        };
    }
    Some(cur)
}

fn get_path_mut<'v>(root: &'v mut Map<String, Value>, path: &[Seg]) -> Option<&'v mut Value> {
    let (first, rest) = path.split_first()?;
    let Seg::Key(k) = first else { return None };
    let mut cur = root.get_mut(*k)?;
    for seg in rest {
        cur = match seg {
            Seg::Key(k) => cur.get_mut(*k)?,
            Seg::Index(i) => cur.get_mut(*i)?,
        };
    }
    Some(cur)
}

/// Every DAX expression of the model, in a fixed order.
pub(crate) fn model_sites(m: &SemanticModel) -> Vec<DaxSite> {
    let mut out = Vec::new();
    for (ti, t) in m.tables.iter().enumerate() {
        let site =
            |loc, kind, name: &str, measure: Option<&str>, home: bool, row: bool, text: String| DaxSite {
                loc,
                kind,
                table: t.name.clone(),
                name: name.to_string(),
                measure: measure.map(str::to_string),
                home: home.then(|| t.name.clone()),
                row_context: row,
                text,
            };
        for (mi, x) in t.measures.iter().enumerate() {
            out.push(site(
                Loc::Measure(ti, mi),
                "measure",
                &x.name,
                None,
                true,
                false,
                x.expression.clone(),
            ));
            let props: [(&'static str, &'static str, &'static str); 5] = [
                ("kpi", "targetExpression", "kpi target"),
                ("kpi", "statusExpression", "kpi status"),
                ("kpi", "trendExpression", "kpi trend"),
                ("formatStringDefinition", "expression", "format string expression"),
                ("detailRowsDefinition", "expression", "detail rows expression"),
            ];
            for (key, sub, kind) in props {
                let path = vec![Seg::Key(key), Seg::Key(sub)];
                if let Some(text) = get_path(&x.extra, &path).and_then(expression_text) {
                    out.push(site(
                        Loc::MeasureExtra(ti, mi, path),
                        kind,
                        &x.name,
                        Some(&x.name),
                        true,
                        false,
                        text,
                    ));
                }
            }
        }
        for (ci, c) in t.columns.iter().enumerate() {
            if let Some(e) = &c.expression {
                out.push(site(
                    Loc::Column(ti, ci),
                    "calculated column",
                    &c.name,
                    None,
                    true,
                    true,
                    e.clone(),
                ));
            }
        }
        for (pi, p) in t.partitions.iter().enumerate() {
            if p.source_type.eq_ignore_ascii_case("calculated")
                && let Some(e) = &p.expression
            {
                out.push(site(
                    Loc::Partition(ti, pi),
                    "calculated table",
                    &t.name,
                    None,
                    false,
                    false,
                    e.clone(),
                ));
            }
        }
        let path = vec![Seg::Key("defaultDetailRowsDefinition"), Seg::Key("expression")];
        if let Some(text) = get_path(&t.extra, &path).and_then(expression_text) {
            out.push(site(
                Loc::TableExtra(ti, path),
                "default detail rows expression",
                &t.name,
                None,
                true,
                false,
                text,
            ));
        }
        for (key, kind) in [
            (
                "multipleOrEmptySelectionExpression",
                "multiple or empty selection expression",
            ),
            ("noSelectionExpression", "no selection expression"),
        ] {
            let path = vec![
                Seg::Key("calculationGroup"),
                Seg::Key(key),
                Seg::Key("expression"),
            ];
            if let Some(text) = get_path(&t.extra, &path).and_then(expression_text) {
                out.push(site(
                    Loc::TableExtra(ti, path),
                    kind,
                    &t.name,
                    None,
                    true,
                    false,
                    text,
                ));
            }
        }
        let items = t
            .extra
            .get("calculationGroup")
            .and_then(|g| g.get("calculationItems"))
            .and_then(Value::as_array);
        for (ii, item) in items.into_iter().flatten().enumerate() {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            for (path, kind) in [
                (
                    vec![
                        Seg::Key("calculationGroup"),
                        Seg::Key("calculationItems"),
                        Seg::Index(ii),
                        Seg::Key("expression"),
                    ],
                    "calculation item",
                ),
                (
                    vec![
                        Seg::Key("calculationGroup"),
                        Seg::Key("calculationItems"),
                        Seg::Index(ii),
                        Seg::Key("formatStringDefinition"),
                        Seg::Key("expression"),
                    ],
                    "calculation item format string",
                ),
            ] {
                if let Some(text) = get_path(&t.extra, &path).and_then(expression_text) {
                    out.push(site(
                        Loc::TableExtra(ti, path),
                        kind,
                        name,
                        None,
                        true,
                        false,
                        text,
                    ));
                }
            }
        }
    }
    let functions = m.extra.get("functions").and_then(Value::as_array);
    for (fi, f) in functions.into_iter().flatten().enumerate() {
        if let Some(text) = f.get("expression").and_then(expression_text) {
            out.push(DaxSite {
                loc: Loc::Function(fi),
                kind: "function",
                table: String::new(),
                name: f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                measure: None,
                home: None,
                row_context: false,
                text,
            });
        }
    }
    for (ri, r) in m.roles.iter().enumerate() {
        for (pi, tp) in r.table_permissions.iter().enumerate() {
            if let Some(e) = &tp.filter_expression {
                out.push(DaxSite {
                    loc: Loc::RowFilter(ri, pi),
                    kind: "row filter",
                    table: tp.table.clone(),
                    name: r.name.clone(),
                    measure: None,
                    home: Some(tp.table.clone()),
                    row_context: true,
                    text: e.clone(),
                });
            }
        }
    }
    out
}

/// Writes `text` where `loc` points. A JSON expression stored as an array of lines stays one.
fn set_text(m: &mut SemanticModel, loc: &Loc, text: String) -> bool {
    let set_json = |v: Option<&mut Value>, text: String| match v {
        Some(v @ Value::Array(_)) => {
            *v = Value::Array(text.split('\n').map(|l| Value::String(l.to_string())).collect());
            true
        }
        Some(v) => {
            *v = Value::String(text);
            true
        }
        None => false,
    };
    match loc {
        Loc::Measure(ti, mi) => m
            .tables
            .get_mut(*ti)
            .and_then(|t| t.measures.get_mut(*mi))
            .map(|x| x.expression = text)
            .is_some(),
        Loc::Column(ti, ci) => m
            .tables
            .get_mut(*ti)
            .and_then(|t| t.columns.get_mut(*ci))
            .map(|c| c.expression = Some(text))
            .is_some(),
        Loc::Partition(ti, pi) => m
            .tables
            .get_mut(*ti)
            .and_then(|t| t.partitions.get_mut(*pi))
            .map(|p| p.expression = Some(text))
            .is_some(),
        Loc::RowFilter(ri, pi) => m
            .roles
            .get_mut(*ri)
            .and_then(|r| r.table_permissions.get_mut(*pi))
            .map(|tp| tp.filter_expression = Some(text))
            .is_some(),
        Loc::MeasureExtra(ti, mi, path) => set_json(
            m.tables
                .get_mut(*ti)
                .and_then(|t| t.measures.get_mut(*mi))
                .and_then(|x| get_path_mut(&mut x.extra, path)),
            text,
        ),
        Loc::TableExtra(ti, path) => set_json(
            m.tables
                .get_mut(*ti)
                .and_then(|t| get_path_mut(&mut t.extra, path)),
            text,
        ),
        Loc::Function(fi) => set_json(
            m.extra
                .get_mut("functions")
                .and_then(|f| f.get_mut(*fi))
                .and_then(|f| f.get_mut("expression")),
            text,
        ),
    }
}

/// What happens to a model object, for its DAX references.
#[derive(Debug, Clone)]
pub(crate) enum DaxChange {
    /// The object gets this new name.
    Rename(String),
    /// The measure moves to this table: `Table[Measure]` qualifiers follow.
    MoveMeasure(String),
}

impl DaxChange {
    pub(crate) fn rewrite(
        &self,
        renamer: &Renamer<'_>,
        text: &str,
        site: Site<'_>,
        target: &ObjectRef,
    ) -> Rewrite {
        match self {
            DaxChange::Rename(new) => renamer.rewrite(text, site, target, new),
            DaxChange::MoveMeasure(to) => renamer.move_measure(text, site, target, to),
        }
    }
}

/// What a rename did to the model's DAX.
#[derive(Debug, Default)]
pub(crate) struct RenameOutcome {
    /// The rewritten expressions, `{kind, table, name, measure?, references}`.
    pub rewritten: Vec<Value>,
    /// Expressions left (partly) as they are, with a `reason`: they mention the old name but do
    /// not parse, or a variable would capture the new name.
    pub unparsable: Vec<Value>,
}

impl RenameOutcome {
    pub(crate) fn into_result(self, out: &mut Map<String, Value>) {
        out.insert("dax_rewritten".into(), Value::Array(self.rewritten));
        if !self.unparsable.is_empty() {
            out.insert("dax_not_rewritten".into(), Value::Array(self.unparsable));
        }
    }
}

/// The name `target` is known by, for the "mentions the old name" test.
fn old_name(target: &ObjectRef) -> &str {
    match target {
        ObjectRef::Table { name } | ObjectRef::Column { name, .. } | ObjectRef::Measure { name, .. } => name,
        ObjectRef::RowFilter { table, .. } => table,
    }
}

fn mentions(text: &str, name: &str) -> bool {
    !name.is_empty() && text.to_lowercase().contains(&name.to_lowercase())
}

/// Rewrites every DAX expression of `m` that references `target` (named as it is before the
/// change) for `change`. Call it before renaming or moving the object itself: references are
/// resolved against the model as it is.
pub(crate) fn rename_in_model(
    m: &mut SemanticModel,
    target: &ObjectRef,
    change: &DaxChange,
) -> RenameOutcome {
    let new_name = match change {
        DaxChange::Rename(n) => n.as_str(),
        DaxChange::MoveMeasure(_) => old_name(target),
    };
    let sites = model_sites(m);
    let mut outcome = RenameOutcome::default();
    let mut changes: Vec<(usize, String)> = Vec::new();
    {
        let renamer = Renamer::new(m);
        for (i, s) in sites.iter().enumerate() {
            match change.rewrite(&renamer, &s.text, s.site(), target) {
                Rewrite::Unchanged => {}
                Rewrite::Rewritten {
                    text,
                    references,
                    captured,
                } => {
                    let mut entry = s.to_json();
                    if let Some(o) = entry.as_object_mut()
                        && let ObjectRef::Table { name } = target
                        && matches!(change, DaxChange::Rename(_))
                        && !s.table.is_empty()
                        && s.table.eq_ignore_ascii_case(name)
                    {
                        // Objects are listed under their table's name after the edit.
                        o.insert("table".into(), json!(new_name));
                        if s.name == s.table {
                            o.insert("name".into(), json!(new_name));
                        }
                    }
                    if captured > 0 {
                        let mut skipped = entry.clone();
                        if let Some(o) = skipped.as_object_mut() {
                            o.insert(
                                "reason".into(),
                                json!(format!(
                                    "{captured} reference(s) not rewritten: a variable named `{new_name}` is in \
                                     scope there and would capture the new name"
                                )),
                            );
                        }
                        outcome.unparsable.push(skipped);
                    }
                    if references > 0 {
                        if let Some(o) = entry.as_object_mut() {
                            o.insert("references".into(), json!(references));
                        }
                        outcome.rewritten.push(entry);
                        changes.push((i, text));
                    }
                }
                Rewrite::Unparsable => {
                    if mentions(&s.text, old_name(target)) {
                        let mut entry = s.to_json();
                        if let Some(o) = entry.as_object_mut() {
                            o.insert("reason".into(), json!("the expression does not parse"));
                        }
                        outcome.unparsable.push(entry);
                    }
                }
            }
        }
    }
    for (i, text) in changes {
        if let Some(s) = sites.get(i) {
            set_text(m, &s.loc, text);
        }
    }
    outcome
}

/// Whether reference `r` points at `target` or (for a table) at anything inside it.
pub(crate) fn refers_to(r: &ObjectRef, target: &ObjectRef) -> bool {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match (target, r) {
        (
            ObjectRef::Table { name },
            ObjectRef::Table { name: t }
            | ObjectRef::Column { table: t, .. }
            | ObjectRef::Measure { table: t, .. },
        ) => eq(name, t),
        (ObjectRef::Column { table, name }, ObjectRef::Column { table: t, name: n })
        | (ObjectRef::Measure { table, name }, ObjectRef::Measure { table: t, name: n }) => {
            eq(table, t) && eq(name, n)
        }
        _ => false,
    }
}

/// Whether `s` goes away with `target` (it belongs to the removed object).
fn removed_with(s: &DaxSite, target: &ObjectRef) -> bool {
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    match target {
        ObjectRef::Table { name } => eq(&s.table, name),
        ObjectRef::Column { table, name } => {
            matches!(s.loc, Loc::Column(..)) && eq(&s.table, table) && eq(&s.name, name)
        }
        ObjectRef::Measure { table, name } => {
            eq(&s.table, table)
                && (matches!(s.loc, Loc::Measure(..)) && eq(&s.name, name)
                    || s.measure.as_deref().is_some_and(|m| eq(m, name)))
        }
        ObjectRef::RowFilter { .. } => false,
    }
}

/// The model expressions that reference `target` (or, for a table, anything in it), except those
/// removed with it: what a removal would break.
pub(crate) fn dependants(m: &SemanticModel, target: &ObjectRef) -> Vec<(Value, String)> {
    let renamer = Renamer::new(m);
    model_sites(m)
        .iter()
        .filter(|s| !removed_with(s, target))
        .filter(|s| {
            renamer
                .references(&s.text, s.site())
                .is_some_and(|refs| refs.iter().any(|r| refers_to(r, target)))
        })
        .map(|s| (s.to_json(), s.label()))
        .collect()
}

/// The refusal message of a removal that would break `labels`.
pub(crate) fn refusal(what: &str, labels: &[String]) -> String {
    let mut listed: Vec<&str> = labels.iter().take(MAX_IN_MESSAGE).map(String::as_str).collect();
    let more = labels.len().saturating_sub(MAX_IN_MESSAGE);
    let tail = if more > 0 {
        format!(", and {more} more")
    } else {
        String::new()
    };
    listed.dedup();
    format!(
        "{what} is used by {} DAX expression(s): {}{tail}. Nothing was removed; pass \"force\": true \
         to remove it anyway (those expressions are left as they are and no longer validate)",
        labels.len(),
        listed.join("; ")
    )
}

/// Rewrites the DAX expressions of `m` that `f` returns a new text for (`objects.import` renames
/// user-defined function calls with it). Returns how many were rewritten.
pub(crate) fn rewrite_sites(m: &mut SemanticModel, f: impl Fn(&DaxSite) -> Option<String>) -> usize {
    let changes: Vec<(Loc, String)> = model_sites(m)
        .into_iter()
        .filter_map(|s| f(&s).map(|text| (s.loc, text)))
        .collect();
    let mut n = 0usize;
    for (loc, text) in changes {
        if set_text(m, &loc, text) {
            n = n.saturating_add(1);
        }
    }
    n
}
