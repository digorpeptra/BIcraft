//! Report-derived aggregations, first half (`docs/storage-engine.md`, "Aggregations"): what
//! each visual of the report needs from the model, read deterministically from the report and
//! the model alone (no data).
//!
//! For each visual: its grouping columns (every bound column), the columns of the report's, the
//! page's and its own filters and of the page's slicers, and the measures it shows with their
//! dependency closure, read from their DAX: value columns (under an aggregation function or in a
//! row iterator's body over its own table), facts (tables aggregated, counted or iterated) and
//! grain columns (anything else read, except columns only removed from the filter context). The
//! engine half (`preview_eval_aggs.rs`) maps those columns onto each fact and chooses the
//! aggregations.

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use bicraft_dax::ast::Expr;
use bicraft_model::SemanticModel;
use bicraft_report::{FieldRef, Report};
use serde_json::{Value, json};

use crate::refs::{SiteKind, find_refs};

/// Deepest measure reference / expression nesting followed.
const MAX_DEPTH: usize = 32;
/// Most visuals read from one report.
const MAX_VISUALS: usize = 20_000;

/// A model column: (table, column) as the model names them.
pub(crate) type ColName = (String, String);

/// What one visual needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct Requirement {
    pub page: String,
    pub visual: String,
    /// Columns grouped or filtered on (any table).
    pub grain: BTreeSet<ColName>,
    /// Columns aggregated.
    pub values: BTreeSet<ColName>,
    /// Tables aggregated, counted or iterated.
    pub facts: BTreeSet<String>,
}

impl Requirement {
    pub fn to_json(&self) -> Value {
        let cols =
            |s: &BTreeSet<ColName>| -> Vec<String> { s.iter().map(|(t, c)| format!("'{t}'[{c}]")).collect() };
        json!({
            "page": self.page,
            "visual": self.visual,
            "grain": cols(&self.grain),
            "values": cols(&self.values),
            "facts": self.facts,
        })
    }
}

/// What the report needs, visual by visual, and a hash of it (the aggregations' key).
#[derive(Debug, Clone, Default)]
pub(crate) struct Derivation {
    pub visuals: Vec<Requirement>,
    pub hash: u64,
}

/// The requirements of every visual of `report` over `model`.
pub(crate) fn derive(model: &SemanticModel, report: &Report) -> Derivation {
    let r = Reader::new(model, report);
    let mut visuals = Vec::new();
    let report_filters = report.filter_config.as_ref();
    for page in &report.pages {
        // the page's slicers filter every visual of the page
        let mut page_grain = Requirement::default();
        for v in &page.visuals {
            if crate::preview::filters::is_slicer(v) {
                for b in &v.fields {
                    r.field(&b.field, &mut page_grain);
                }
            }
        }
        for v in &page.visuals {
            if visuals.len() >= MAX_VISUALS {
                break;
            }
            let mut req = Requirement {
                page: page.name.clone(),
                visual: v.name.clone(),
                ..Requirement::default()
            };
            for b in &v.fields {
                r.field(&b.field, &mut req);
            }
            for f in [
                report_filters,
                page.filter_config.as_ref(),
                v.filter_config.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                r.raw(f, &mut req);
            }
            req.grain.extend(page_grain.grain.iter().cloned());
            req.values.extend(page_grain.values.iter().cloned());
            req.facts.extend(page_grain.facts.iter().cloned());
            if req.facts.is_empty() && req.values.is_empty() {
                continue;
            }
            visuals.push(req);
        }
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for v in &visuals {
        (&v.grain, &v.values, &v.facts).hash(&mut h);
    }
    Derivation {
        visuals,
        hash: h.finish(),
    }
}

/// What a column read in DAX contributes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role<'t> {
    /// Grouped or filtered on.
    Grain,
    /// Aggregated.
    Value,
    /// In the body of a row iterator over table `t`: its own columns are values, others grain.
    Body(&'t str),
    /// Only removed from the filter context.
    Removed,
}

struct Reader<'a> {
    model: &'a SemanticModel,
    /// Lower-cased measure name → (home table, expression), model measures then report measures.
    measures: HashMap<String, (String, String)>,
    /// Lower-cased table name → lower-cased column names.
    columns: HashMap<String, (String, Vec<String>)>,
    /// Needs of the measures already read, by lower-cased name.
    memo: std::cell::RefCell<HashMap<String, Requirement>>,
}

/// Aggregation functions over one column.
const AGGS: &[&str] = &[
    "SUM",
    "AVERAGE",
    "MIN",
    "MAX",
    "COUNT",
    "COUNTA",
    "COUNTBLANK",
    "PRODUCT",
    "MEDIAN",
    "STDEV.S",
    "STDEV.P",
    "VAR.S",
    "VAR.P",
];
/// Row iterators: (name, whether its second argument is a row predicate).
const ITERATORS: &[(&str, bool)] = &[
    ("SUMX", false),
    ("AVERAGEX", false),
    ("MINX", false),
    ("MAXX", false),
    ("COUNTX", false),
    ("COUNTAX", false),
    ("PRODUCTX", false),
    ("MEDIANX", false),
    ("CONCATENATEX", false),
    ("RANKX", false),
    ("FILTER", true),
];
/// `CALCULATE` modifiers that only remove filters (or change relationships).
const REMOVERS: &[&str] = &[
    "ALL",
    "REMOVEFILTERS",
    "ALLEXCEPT",
    "ALLSELECTED",
    "ALLNOBLANKROW",
    "USERELATIONSHIP",
    "CROSSFILTER",
];

impl<'a> Reader<'a> {
    fn new(model: &'a SemanticModel, report: &Report) -> Self {
        let mut measures = HashMap::new();
        let mut columns = HashMap::new();
        for t in &model.tables {
            columns.insert(
                t.name.to_lowercase(),
                (
                    t.name.clone(),
                    t.columns.iter().map(|c| c.name.to_lowercase()).collect(),
                ),
            );
            for m in &t.measures {
                measures
                    .entry(m.name.to_lowercase())
                    .or_insert_with(|| (t.name.clone(), m.expression.clone()));
            }
        }
        for m in crate::report_measures::report_measures(report) {
            measures
                .entry(m.name.to_lowercase())
                .or_insert((m.entity, m.expression));
        }
        Reader {
            model,
            measures,
            columns,
            memo: Default::default(),
        }
    }

    /// The model's names of column `table[name]`, when it is one.
    fn column(&self, table: &str, name: &str) -> Option<ColName> {
        let (t, cols) = self.columns.get(&table.to_lowercase())?;
        let lower = name.to_lowercase();
        let i = cols.iter().position(|c| *c == lower)?;
        let tab = self.model.tables.iter().find(|x| x.name == *t)?;
        Some((t.clone(), tab.columns.get(i)?.name.clone()))
    }

    fn table(&self, name: &str) -> Option<String> {
        self.columns.get(&name.to_lowercase()).map(|(t, _)| t.clone())
    }

    fn field(&self, f: &FieldRef, out: &mut Requirement) {
        match f {
            FieldRef::Column { table, column } => {
                if let Some(c) = self.column(table, column) {
                    out.grain.insert(c);
                }
            }
            FieldRef::Measure { table, measure } => self.measure(Some(table), measure, out, 0),
            FieldRef::Aggregation {
                table,
                column,
                function,
            } => {
                if let Some(c) = self.column(table, column) {
                    out.facts.insert(c.0.clone());
                    // PBIR `Count` is a distinct count: only derivable from the grain
                    if function.eq_ignore_ascii_case("Count") {
                        out.grain.insert(c.clone());
                    }
                    out.values.insert(c);
                }
            }
            FieldRef::Other { raw } => self.raw(raw, out),
        }
    }

    /// Columns and measures referenced in raw report JSON (filters, hierarchy levels, report
    /// measures' bindings).
    fn raw(&self, v: &Value, out: &mut Requirement) {
        let mut sites: Vec<(SiteKind, String, String)> = Vec::new();
        find_refs(v, &mut |s| {
            if matches!(s.kind, SiteKind::Column | SiteKind::Measure) {
                sites.push((s.kind, s.entity.to_string(), s.property.to_string()));
            }
        });
        for (kind, entity, property) in sites {
            match kind {
                SiteKind::Column => {
                    if let Some(c) = self.column(&entity, &property) {
                        out.grain.insert(c);
                    }
                }
                _ => self.measure(Some(&entity), &property, out, 0),
            }
        }
    }

    /// A measure's needs, read once per measure (a measure met again while it is being read, a
    /// cycle, adds nothing).
    fn measure(&self, _table: Option<&str>, name: &str, out: &mut Requirement, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let key = name.to_lowercase();
        let Some((_, text)) = self.measures.get(&key) else {
            return;
        };
        let known = self.memo.borrow().get(&key).cloned();
        let got = match known {
            Some(r) => r,
            None => {
                self.memo.borrow_mut().insert(key.clone(), Requirement::default());
                let mut r = Requirement::default();
                let (parsed, _) = bicraft_dax::parse_expression(text);
                if let Some(e) = parsed {
                    self.expr(&e, Role::Grain, &mut r, depth.saturating_add(1));
                }
                self.memo.borrow_mut().insert(key, r.clone());
                r
            }
        };
        out.grain.extend(got.grain);
        out.values.extend(got.values);
        out.facts.extend(got.facts);
    }

    fn col_ref(&self, table: Option<&str>, name: &str, role: Role<'_>, out: &mut Requirement, depth: usize) {
        let table = match (table, role) {
            (Some(t), _) => Some(t.to_string()),
            (None, Role::Body(t)) => Some(t.to_string()),
            (None, _) => None,
        };
        let col = table.as_deref().and_then(|t| self.column(t, name));
        let Some(col) = col else {
            // `[Name]` or `T[Name]` that is not a column: a measure
            self.measure(table.as_deref(), name, out, depth);
            return;
        };
        match role {
            Role::Grain => {
                out.grain.insert(col);
            }
            Role::Value => {
                out.facts.insert(col.0.clone());
                out.values.insert(col);
            }
            Role::Body(t) => {
                if col.0.eq_ignore_ascii_case(t) {
                    out.facts.insert(col.0.clone());
                    out.values.insert(col);
                } else {
                    out.grain.insert(col);
                }
            }
            Role::Removed => {}
        }
    }

    fn expr(&self, e: &Expr, role: Role<'_>, out: &mut Requirement, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let d = depth.saturating_add(1);
        match e {
            Expr::Column { table, name, .. } => self.col_ref(table.as_deref(), name, role, out, depth),
            Expr::Call { name, args, .. } => self.call(name, args, role, out, d),
            Expr::Unary { operand, .. } => self.expr(operand, role, out, d),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs, role, out, d);
                self.expr(rhs, role, out, d);
            }
            Expr::Var { bindings, body, .. } => {
                for b in bindings {
                    self.expr(&b.value, role, out, d);
                }
                self.expr(body, role, out, d);
            }
            Expr::TableCtor { rows, .. } => rows.iter().for_each(|x| self.expr(x, role, out, d)),
            Expr::Tuple { items, .. } => items.iter().for_each(|x| self.expr(x, role, out, d)),
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } | Expr::Name { .. } => {}
        }
    }

    fn rest(&self, args: &[Option<Expr>], from: usize, role: Role<'_>, out: &mut Requirement, d: usize) {
        for a in args.iter().skip(from).flatten() {
            self.expr(a, role, out, d);
        }
    }

    fn call(&self, name: &str, args: &[Option<Expr>], role: Role<'_>, out: &mut Requirement, d: usize) {
        let up = name.to_ascii_uppercase();
        let arg = |i: usize| args.get(i).and_then(Option::as_ref);
        let one_column = args.len() == 1 && matches!(arg(0), Some(Expr::Column { .. }));
        if AGGS.contains(&up.as_str()) && one_column {
            self.rest(args, 0, Role::Value, out, d);
            return;
        }
        if (up == "DISTINCTCOUNT" || up == "DISTINCTCOUNTNOBLANK") && one_column {
            self.rest(args, 0, Role::Value, out, d);
            self.rest(args, 0, Role::Grain, out, d);
            return;
        }
        if up == "COUNTROWS" {
            if let Some(Expr::Name { name, .. }) = arg(0)
                && let Some(t) = self.table(name)
            {
                out.facts.insert(t);
                return;
            }
            self.rest(args, 0, role, out, d);
            return;
        }
        if let Some((_, predicate)) = ITERATORS.iter().find(|(n, _)| *n == up) {
            match arg(0) {
                Some(Expr::Name { name, .. }) => match self.table(name) {
                    Some(t) => {
                        out.facts.insert(t.clone());
                        let body = if *predicate { Role::Grain } else { Role::Body(&t) };
                        self.rest(args, 1, body, out, d);
                    }
                    None => self.rest(args, 1, role, out, d),
                },
                Some(table) => {
                    self.expr(table, Role::Grain, out, d);
                    self.rest(args, 1, role, out, d);
                }
                None => self.rest(args, 1, role, out, d),
            }
            return;
        }
        match up.as_str() {
            "CALCULATE" | "CALCULATETABLE" => {
                self.rest(args.get(..args.len().min(1)).unwrap_or_default(), 0, role, out, d);
                for a in args.iter().skip(1).flatten() {
                    let removes = matches!(a, Expr::Call { name, .. }
                        if REMOVERS.contains(&name.to_ascii_uppercase().as_str()));
                    self.expr(a, if removes { Role::Removed } else { Role::Grain }, out, d);
                }
            }
            "KEEPFILTERS" | "RELATED" | "TREATAS" | "VALUES" | "DISTINCT" => {
                self.rest(args, 0, Role::Grain, out, d);
            }
            "TOTALYTD" | "TOTALQTD" | "TOTALMTD" => {
                self.rest(args.get(..args.len().min(1)).unwrap_or_default(), 0, role, out, d);
                self.rest(args, 1, Role::Grain, out, d);
            }
            _ if REMOVERS.contains(&up.as_str()) => {
                let r = if role == Role::Removed {
                    Role::Removed
                } else {
                    Role::Grain
                };
                self.rest(args, 0, r, out, d);
            }
            _ => self.rest(args, 0, role, out, d),
        }
    }
}
