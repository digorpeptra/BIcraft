//! A Tabular-Object-Model-like view of a [`SemanticModel`]: object handles, rule scopes, the
//! DAX dependency graph in both directions, and the names each object kind goes by.

use std::collections::HashMap;

use bicraft_dax::deps::{self, DependencyGraph, ObjectRef};
use bicraft_model::{Column, SemanticModel, Table};
use serde_json::Value;

/// A handle to one model object (indexes into the model's collections).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Obj {
    Model,
    Database,
    Table(usize),
    Column(usize, usize),
    Measure(usize, usize),
    /// The KPI of a measure (table, measure).
    Kpi(usize, usize),
    Relationship(usize),
    Partition(usize, usize),
    Expression(usize),
    Role(usize),
    /// Role, index in its table permissions.
    TablePermission(usize, usize),
    Hierarchy(usize, usize),
    /// Table, hierarchy, level.
    Level(usize, usize, usize),
    CalculationItem(usize, usize),
    /// Index in the model's `dataSources`.
    DataSource(usize),
    /// Index in the model's `perspectives`.
    Perspective(usize),
    /// Index in the model's `cultures`.
    Culture(usize),
}

/// Rule scopes (the BPA `Scope` names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Model,
    Table,
    CalculatedTable,
    CalculationGroup,
    DataColumn,
    CalculatedColumn,
    CalculatedTableColumn,
    Measure,
    Kpi,
    Hierarchy,
    Level,
    Relationship,
    Partition,
    NamedExpression,
    ModelRole,
    TablePermission,
    CalculationItem,
    ProviderDataSource,
    StructuredDataSource,
    Perspective,
    Culture,
}

impl Scope {
    pub(crate) fn parse(name: &str) -> Option<Scope> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "model" => Scope::Model,
            "table" => Scope::Table,
            "calculatedtable" => Scope::CalculatedTable,
            "calculationgroup" => Scope::CalculationGroup,
            "datacolumn" => Scope::DataColumn,
            "calculatedcolumn" => Scope::CalculatedColumn,
            "calculatedtablecolumn" => Scope::CalculatedTableColumn,
            "measure" => Scope::Measure,
            "kpi" => Scope::Kpi,
            "hierarchy" => Scope::Hierarchy,
            "level" => Scope::Level,
            "relationship" | "singlecolumnrelationship" => Scope::Relationship,
            "partition" => Scope::Partition,
            "namedexpression" => Scope::NamedExpression,
            "modelrole" => Scope::ModelRole,
            "tablepermission" => Scope::TablePermission,
            "calculationitem" => Scope::CalculationItem,
            "providerdatasource" => Scope::ProviderDataSource,
            "structureddatasource" => Scope::StructuredDataSource,
            "perspective" => Scope::Perspective,
            "culture" => Scope::Culture,
            _ => return None,
        })
    }
}

/// What a column is (TOM `ColumnType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnKind {
    Data,
    Calculated,
    CalculatedTableColumn,
    RowNumber,
}

pub(crate) struct ModelView<'m> {
    pub(crate) m: &'m SemanticModel,
    graph: DependencyGraph,
    /// `dependents[i]`: graph nodes that depend on node `i`.
    dependents: Vec<Vec<usize>>,
    node_obj: Vec<Option<Obj>>,
    obj_node: HashMap<Obj, usize>,
}

fn find_index<T>(items: &[T], name: &str, get: impl Fn(&T) -> &str) -> Option<usize> {
    items.iter().position(|x| get(x).eq_ignore_ascii_case(name))
}

fn json_list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The table's `hierarchies` collection (kept in `extra`).
pub(crate) fn hierarchies(t: &Table) -> &[Value] {
    json_list(t.extra.get("hierarchies"))
}

/// A hierarchy's `levels`.
pub(crate) fn levels(h: &Value) -> &[Value] {
    json_list(h.get("levels"))
}

/// The calculation items of a calculation-group table.
pub(crate) fn calculation_items(t: &Table) -> &[Value] {
    json_list(
        t.extra
            .get("calculationGroup")
            .and_then(|g| g.get("calculationItems")),
    )
}

/// The model's `dataSources`.
pub(crate) fn data_sources(m: &SemanticModel) -> &[Value] {
    json_list(m.extra.get("dataSources"))
}

/// The model's `perspectives`.
pub(crate) fn perspectives(m: &SemanticModel) -> &[Value] {
    json_list(m.extra.get("perspectives"))
}

/// The model's `cultures` (each with its `translations`).
pub(crate) fn cultures(m: &SemanticModel) -> &[Value] {
    json_list(m.extra.get("cultures"))
}

/// Whether a data source is a structured (Power Query) one.
pub(crate) fn is_structured(ds: &Value) -> bool {
    ds.get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("structured"))
}

pub(crate) fn is_calculation_group(t: &Table) -> bool {
    t.extra.get("calculationGroup").is_some()
}

pub(crate) fn is_calculated_table(t: &Table) -> bool {
    deps::calculated_table_expression(t).is_some()
}

/// A table with a DirectQuery partition.
pub(crate) fn is_direct_query(t: &Table) -> bool {
    t.partitions.iter().any(|p| {
        p.mode
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("directQuery"))
    })
}

pub(crate) fn column_kind(t: &Table, c: &Column) -> ColumnKind {
    if c.is_row_number() {
        return ColumnKind::RowNumber;
    }
    match c.extra.get("type").and_then(Value::as_str) {
        Some(k) if k.eq_ignore_ascii_case("calculated") => ColumnKind::Calculated,
        Some(k) if k.eq_ignore_ascii_case("calculatedTableColumn") => ColumnKind::CalculatedTableColumn,
        Some(k) if k.eq_ignore_ascii_case("data") => ColumnKind::Data,
        _ if c.expression.is_some() => ColumnKind::Calculated,
        _ if is_calculated_table(t) => ColumnKind::CalculatedTableColumn,
        _ => ColumnKind::Data,
    }
}

/// `'Table'` with quotes doubled.
pub(crate) fn quote_table(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

/// `[Name]` with closing brackets doubled.
pub(crate) fn bracket(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

impl<'m> ModelView<'m> {
    pub(crate) fn new(m: &'m SemanticModel) -> Self {
        let graph = deps::dependencies(m);
        let mut dependents = vec![Vec::new(); graph.nodes.len()];
        for (i, e) in graph.edges.iter().enumerate() {
            for &j in e {
                if let Some(d) = dependents.get_mut(j) {
                    d.push(i);
                }
            }
        }
        let mut view = ModelView {
            m,
            graph,
            dependents,
            node_obj: Vec::new(),
            obj_node: HashMap::new(),
        };
        let node_obj: Vec<Option<Obj>> = view.graph.nodes.iter().map(|n| view.obj_of_ref(n)).collect();
        for (i, o) in node_obj.iter().enumerate() {
            if let Some(o) = o {
                view.obj_node.entry(*o).or_insert(i);
            }
        }
        view.node_obj = node_obj;
        view
    }

    pub(crate) fn table_index(&self, name: &str) -> Option<usize> {
        find_index(&self.m.tables, name, |t| &t.name)
    }

    fn obj_of_ref(&self, r: &ObjectRef) -> Option<Obj> {
        match r {
            ObjectRef::Table { name } => self.table_index(name).map(Obj::Table),
            ObjectRef::Column { table, name } => self.column_obj(table, name),
            ObjectRef::Measure { table, name } => {
                let ti = self.table_index(table)?;
                let mi = find_index(&self.m.tables.get(ti)?.measures, name, |m| &m.name)?;
                Some(Obj::Measure(ti, mi))
            }
            ObjectRef::RowFilter { role, table } => {
                let ri = find_index(&self.m.roles, role, |r| &r.name)?;
                let pi = find_index(&self.m.roles.get(ri)?.table_permissions, table, |p| &p.table)?;
                Some(Obj::TablePermission(ri, pi))
            }
        }
    }

    /// Objects whose DAX references `o` (`referenced_by`), or that `o`'s DAX references.
    pub(crate) fn direct(&self, o: Obj, referenced_by: bool) -> Vec<Obj> {
        let Some(&n) = self.obj_node.get(&o) else {
            return Vec::new();
        };
        let list = if referenced_by {
            self.dependents.get(n)
        } else {
            self.graph.edges.get(n)
        };
        list.map(|l| {
            l.iter()
                .filter_map(|&j| self.node_obj.get(j).copied().flatten())
                .collect()
        })
        .unwrap_or_default()
    }

    /// [`ModelView::direct`], followed transitively (each object once, `o` excluded).
    pub(crate) fn transitive(&self, o: Obj, referenced_by: bool) -> Vec<Obj> {
        let Some(&start) = self.obj_node.get(&o) else {
            return Vec::new();
        };
        let mut seen = vec![false; self.graph.nodes.len()];
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        let mut stack = vec![start];
        let mut out = Vec::new();
        while let Some(n) = stack.pop() {
            let next = if referenced_by {
                self.dependents.get(n)
            } else {
                self.graph.edges.get(n)
            };
            for &j in next.map_or(&[][..], Vec::as_slice) {
                if let Some(s) = seen.get_mut(j)
                    && !*s
                {
                    *s = true;
                    stack.push(j);
                    if let Some(Some(o)) = self.node_obj.get(j) {
                        out.push(*o);
                    }
                }
            }
        }
        out
    }

    pub(crate) fn table(&self, ti: usize) -> Option<&'m Table> {
        self.m.tables.get(ti)
    }

    pub(crate) fn column(&self, ti: usize, ci: usize) -> Option<(&'m Table, &'m Column)> {
        let t = self.m.tables.get(ti)?;
        Some((t, t.columns.get(ci)?))
    }

    /// The column `table[name]` as an object (row-number columns excluded).
    pub(crate) fn column_obj(&self, table: &str, name: &str) -> Option<Obj> {
        let ti = self.table_index(table)?;
        let ci = self
            .m
            .tables
            .get(ti)?
            .columns
            .iter()
            .position(|c| !c.is_row_number() && c.name.eq_ignore_ascii_case(name))?;
        Some(Obj::Column(ti, ci))
    }

    /// The measure named `name` anywhere in the model.
    pub(crate) fn measure_obj(&self, name: &str) -> Option<Obj> {
        self.m
            .tables
            .iter()
            .enumerate()
            .find_map(|(ti, t)| find_index(&t.measures, name, |m| &m.name).map(|mi| Obj::Measure(ti, mi)))
    }

    pub(crate) fn data_source_obj(&self, name: &str) -> Option<Obj> {
        data_sources(self.m)
            .iter()
            .position(|d| {
                d.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .map(Obj::DataSource)
    }

    /// Every object a rule with `scope` runs on.
    pub(crate) fn objects(&self, scope: Scope) -> Vec<Obj> {
        let m = self.m;
        let mut out = Vec::new();
        match scope {
            Scope::Model => out.push(Obj::Model),
            Scope::Table | Scope::CalculatedTable | Scope::CalculationGroup => {
                for (ti, t) in m.tables.iter().enumerate() {
                    let kind = if is_calculation_group(t) {
                        Scope::CalculationGroup
                    } else if is_calculated_table(t) {
                        Scope::CalculatedTable
                    } else {
                        Scope::Table
                    };
                    if kind == scope {
                        out.push(Obj::Table(ti));
                    }
                }
            }
            Scope::DataColumn | Scope::CalculatedColumn | Scope::CalculatedTableColumn => {
                let want = match scope {
                    Scope::DataColumn => ColumnKind::Data,
                    Scope::CalculatedColumn => ColumnKind::Calculated,
                    _ => ColumnKind::CalculatedTableColumn,
                };
                for (ti, t) in m.tables.iter().enumerate() {
                    for (ci, c) in t.columns.iter().enumerate() {
                        if column_kind(t, c) == want {
                            out.push(Obj::Column(ti, ci));
                        }
                    }
                }
            }
            Scope::Measure | Scope::Kpi => {
                for (ti, t) in m.tables.iter().enumerate() {
                    for (mi, x) in t.measures.iter().enumerate() {
                        if scope == Scope::Measure {
                            out.push(Obj::Measure(ti, mi));
                        } else if x.extra.get("kpi").is_some_and(Value::is_object) {
                            out.push(Obj::Kpi(ti, mi));
                        }
                    }
                }
            }
            Scope::Hierarchy => {
                for (ti, t) in m.tables.iter().enumerate() {
                    out.extend((0..hierarchies(t).len()).map(|hi| Obj::Hierarchy(ti, hi)));
                }
            }
            Scope::Level => {
                for (ti, t) in m.tables.iter().enumerate() {
                    for (hi, h) in hierarchies(t).iter().enumerate() {
                        out.extend((0..levels(h).len()).map(|li| Obj::Level(ti, hi, li)));
                    }
                }
            }
            Scope::Relationship => out.extend((0..m.relationships.len()).map(Obj::Relationship)),
            Scope::Partition => {
                for (ti, t) in m.tables.iter().enumerate() {
                    out.extend((0..t.partitions.len()).map(|pi| Obj::Partition(ti, pi)));
                }
            }
            Scope::NamedExpression => out.extend((0..m.expressions.len()).map(Obj::Expression)),
            Scope::ModelRole => out.extend((0..m.roles.len()).map(Obj::Role)),
            Scope::TablePermission => {
                for (ri, r) in m.roles.iter().enumerate() {
                    out.extend((0..r.table_permissions.len()).map(|pi| Obj::TablePermission(ri, pi)));
                }
            }
            Scope::CalculationItem => {
                for (ti, t) in m.tables.iter().enumerate() {
                    out.extend((0..calculation_items(t).len()).map(|i| Obj::CalculationItem(ti, i)));
                }
            }
            Scope::ProviderDataSource | Scope::StructuredDataSource => {
                let structured = scope == Scope::StructuredDataSource;
                for (i, d) in data_sources(m).iter().enumerate() {
                    if is_structured(d) == structured {
                        out.push(Obj::DataSource(i));
                    }
                }
            }
            Scope::Perspective => out.extend((0..perspectives(m).len()).map(Obj::Perspective)),
            Scope::Culture => out.extend((0..cultures(m).len()).map(Obj::Culture)),
        }
        out
    }

    /// The `ObjectTypeName` of an object.
    pub(crate) fn type_name(&self, o: Obj) -> &'static str {
        match o {
            Obj::Model => "Model",
            Obj::Database => "Database",
            Obj::Table(ti) => match self.table(ti) {
                Some(t) if is_calculation_group(t) => "Calculation Group Table",
                Some(t) if is_calculated_table(t) => "Calculated Table",
                Some(t) if is_direct_query(t) => "Table (DirectQuery)",
                _ => "Table",
            },
            Obj::Column(ti, ci) => match self.column(ti, ci).map(|(t, c)| column_kind(t, c)) {
                Some(ColumnKind::Calculated) => "Calculated Column",
                Some(ColumnKind::CalculatedTableColumn) => "Calculated Table Column",
                _ => "Data Column",
            },
            Obj::Measure(..) => "Measure",
            Obj::Kpi(..) => "KPI",
            Obj::Relationship(_) => "Relationship",
            Obj::Partition(..) => "Partition",
            Obj::Expression(_) => "Shared Expression",
            Obj::Role(_) => "Role",
            Obj::TablePermission(..) => "Table Permission",
            Obj::Hierarchy(..) => "Hierarchy",
            Obj::Level(..) => "Level",
            Obj::CalculationItem(..) => "Calculation Item",
            Obj::DataSource(i) => match data_sources(self.m).get(i) {
                Some(d) if is_structured(d) => "Structured Data Source",
                _ => "Provider Data Source",
            },
            Obj::Perspective(_) => "Perspective",
            Obj::Culture(_) => "Culture",
        }
    }

    /// The JSON object behind objects bicraft keeps as JSON (hierarchies, levels, calculation
    /// items, KPIs, data sources, perspectives, cultures).
    pub(crate) fn json(&self, o: Obj) -> Option<&'m Value> {
        let m = self.m;
        match o {
            Obj::Hierarchy(ti, hi) => hierarchies(self.table(ti)?).get(hi),
            Obj::Level(ti, hi, li) => levels(hierarchies(self.table(ti)?).get(hi)?).get(li),
            Obj::CalculationItem(ti, i) => calculation_items(self.table(ti)?).get(i),
            Obj::Kpi(ti, mi) => self.table(ti)?.measures.get(mi)?.extra.get("kpi"),
            Obj::DataSource(i) => data_sources(m).get(i),
            Obj::Perspective(i) => perspectives(m).get(i),
            Obj::Culture(i) => cultures(m).get(i),
            _ => None,
        }
    }

    /// The object's name.
    pub(crate) fn name(&self, o: Obj) -> String {
        let m = self.m;
        let json_name = |o: Obj| {
            self.json(o)
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match o {
            Obj::Model | Obj::Database => m.name.clone(),
            Obj::Table(ti) => self.table(ti).map(|t| t.name.clone()).unwrap_or_default(),
            Obj::Column(ti, ci) => self
                .column(ti, ci)
                .map(|(_, c)| c.name.clone())
                .unwrap_or_default(),
            Obj::Measure(ti, mi) | Obj::Kpi(ti, mi) => self
                .table(ti)
                .and_then(|t| t.measures.get(mi))
                .map(|x| x.name.clone())
                .unwrap_or_default(),
            Obj::Relationship(ri) => m
                .relationships
                .get(ri)
                .map(|r| r.name.clone())
                .unwrap_or_default(),
            Obj::Partition(ti, pi) => self
                .table(ti)
                .and_then(|t| t.partitions.get(pi))
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            Obj::Expression(ei) => m.expressions.get(ei).map(|e| e.name.clone()).unwrap_or_default(),
            Obj::Role(ri) => m.roles.get(ri).map(|r| r.name.clone()).unwrap_or_default(),
            Obj::TablePermission(ri, pi) => m
                .roles
                .get(ri)
                .and_then(|r| r.table_permissions.get(pi))
                .map(|p| p.table.clone())
                .unwrap_or_default(),
            Obj::Hierarchy(..)
            | Obj::Level(..)
            | Obj::CalculationItem(..)
            | Obj::DataSource(_)
            | Obj::Perspective(_)
            | Obj::Culture(_) => json_name(o),
        }
    }

    /// The table an object belongs to.
    pub(crate) fn table_of(&self, o: Obj) -> Option<usize> {
        match o {
            Obj::Table(ti)
            | Obj::Column(ti, _)
            | Obj::Measure(ti, _)
            | Obj::Kpi(ti, _)
            | Obj::Partition(ti, _)
            | Obj::Hierarchy(ti, _)
            | Obj::Level(ti, _, _)
            | Obj::CalculationItem(ti, _) => Some(ti),
            Obj::TablePermission(ri, pi) => {
                let p = self.m.roles.get(ri)?.table_permissions.get(pi)?;
                self.table_index(&p.table)
            }
            _ => None,
        }
    }

    /// How a finding names the object: DAX syntax where there is one.
    pub(crate) fn display(&self, o: Obj) -> String {
        let table = || {
            self.table_of(o)
                .and_then(|ti| self.table(ti))
                .map(|t| quote_table(&t.name))
                .unwrap_or_default()
        };
        match o {
            Obj::Model | Obj::Database => format!("model {}", self.m.name),
            Obj::Table(_) => table(),
            Obj::Column(..) => format!("{}{}", table(), bracket(&self.name(o))),
            Obj::Measure(..) => bracket(&self.name(o)),
            Obj::Kpi(..) => format!("KPI of {}", bracket(&self.name(o))),
            Obj::Relationship(ri) => match self.m.relationships.get(ri) {
                Some(r) => format!(
                    "{}{} -> {}{}",
                    quote_table(&r.from_table),
                    bracket(&r.from_column),
                    quote_table(&r.to_table),
                    bracket(&r.to_column)
                ),
                None => String::new(),
            },
            Obj::Partition(..) => format!("partition {} of {}", self.name(o), table()),
            Obj::Expression(_) => format!("expression {}", self.name(o)),
            Obj::Role(_) => format!("role {}", self.name(o)),
            Obj::TablePermission(ri, _) => format!(
                "row filter of role {} on {}",
                self.m.roles.get(ri).map(|r| r.name.as_str()).unwrap_or_default(),
                table()
            ),
            Obj::Hierarchy(..) => format!("hierarchy {} of {}", self.name(o), table()),
            Obj::Level(ti, hi, _) => format!(
                "level {} of hierarchy {} on {}",
                self.name(o),
                self.name(Obj::Hierarchy(ti, hi)),
                table()
            ),
            Obj::CalculationItem(..) => format!("calculation item {} of {}", self.name(o), table()),
            Obj::DataSource(_) => format!("data source {}", self.name(o)),
            Obj::Perspective(_) => format!("perspective {}", self.name(o)),
            Obj::Culture(_) => format!("culture {}", self.name(o)),
        }
    }
}
