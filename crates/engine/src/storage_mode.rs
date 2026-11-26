//! `table.set_storage_mode`: a table's storage mode (Import, DirectQuery, Dual), written as the
//! `mode` of each of its partitions (docs/formats/model-objects.md, "Storage mode"). The rules
//! come from Microsoft Learn ("Table storage mode in Power BI semantic models", "Use composite
//! models in Power BI Desktop", "Model relationships in Power BI Desktop"):
//!
//! - a calculated table (and a calculation group or field parameter, which are calculated
//!   tables) is computed from the model and is always Import;
//! - DirectQuery and Dual need a source that can be queried live: a Power Query partition that
//!   reaches a connector (an Enter data or `#table` table has none);
//! - a relationship between an Import table and a DirectQuery table is limited (the two are in
//!   different source groups); a Dual table belongs to both, so it keeps them regular;
//! - Dual is meant for dimension tables, the one side of relationships;
//! - Desktop changes only a DirectQuery table, to Import or Dual, and asks before switching to
//!   Import, offering to set the DirectQuery dimension tables it reaches (one side only) to Dual
//!   (`dual_related`).
//!
//! BIcraft evaluates imported data only: a DirectQuery table shows "not evaluated: DirectQuery"
//! in visuals (`preview.rs`), and a table with an incremental refresh policy keeps its partitions.

use std::collections::HashSet;

use bicraft_model::{SemanticModel, Table};
use serde_json::{Value, json};

use crate::Session;
use crate::edit::{Edit, apply, dax_column, dax_table, table_index};
use crate::mquery::is_m;
use crate::params::{allow, opt_bool, str_param};

/// The modes `table.set_storage_mode` writes, as TMSL spells them.
pub(crate) const MODES: &[&str] = &["import", "directQuery", "dual"];
/// Most tables the Dual propagation visits.
const MAX_VISITED: usize = 10_000;

/// The model's default partition mode (`defaultMode`; Import when absent).
fn model_default(m: &SemanticModel) -> &str {
    m.extra
        .get("defaultMode")
        .and_then(Value::as_str)
        .filter(|v| !v.eq_ignore_ascii_case("default"))
        .unwrap_or("import")
}

/// A partition mode in TMSL's spelling (`None`: one BIcraft does not name).
fn canonical(mode: &str) -> Option<&'static str> {
    ["import", "directQuery", "dual", "directLake", "push", "default"]
        .into_iter()
        .find(|x| x.eq_ignore_ascii_case(mode))
}

/// A table's storage mode: its partitions' mode (`default` or none is the model's default),
/// `hybrid` when they differ (an incremental refresh policy with real time), `import` for a
/// table without partitions.
pub(crate) fn table_mode(m: &SemanticModel, t: &Table) -> &'static str {
    let dflt = canonical(model_default(m)).unwrap_or("import");
    let mut seen: Option<&'static str> = None;
    for p in &t.partitions {
        let mode = match p.mode.as_deref().and_then(canonical) {
            None | Some("default") => dflt,
            Some(x) => x,
        };
        match seen {
            None => seen = Some(mode),
            Some(x) if x == mode => {}
            Some(_) => return "hybrid",
        }
    }
    seen.unwrap_or("import")
}

/// Whether a table holds no imported rows: every partition is DirectQuery.
pub(crate) fn is_direct_query(m: &SemanticModel, t: &Table) -> bool {
    !t.partitions.is_empty() && table_mode(m, t) == "directQuery"
}

/// The reason a visual gives for a DirectQuery table.
pub(crate) fn direct_query_reason(table: &str) -> String {
    format!(
        "not evaluated: DirectQuery (table '{table}' is queried from its source live; BIcraft \
         evaluates imported data only)"
    )
}

/// The source groups a mode belongs to: Import, DirectQuery, or both (Dual).
fn groups(mode: &str) -> (bool, bool) {
    match mode {
        "directQuery" => (false, true),
        "dual" | "hybrid" => (true, true),
        _ => (true, false),
    }
}

/// Whether a relationship between tables of these modes is limited (no source group in common).
fn limited(a: &str, b: &str) -> bool {
    let (ai, ad) = groups(a);
    let (bi, bd) = groups(b);
    !((ai && bi) || (ad && bd))
}

/// Whether the relationship has `table` on its one side (`toCardinality` defaults to one).
fn one_side_is(r: &bicraft_model::Relationship, table: &str) -> Option<String> {
    let many_to_one = r
        .from_cardinality
        .as_deref()
        .is_none_or(|c| c.eq_ignore_ascii_case("many"))
        && r.to_cardinality
            .as_deref()
            .is_none_or(|c| c.eq_ignore_ascii_case("one"));
    if !many_to_one {
        return None;
    }
    r.from_table
        .eq_ignore_ascii_case(table)
        .then(|| r.to_table.clone())
}

/// The DirectQuery tables Desktop sets to Dual when `table` turns to Import: the tables reached
/// from it through the one side of one-to-many relationships, in visiting order.
pub(crate) fn dual_candidates(m: &SemanticModel, table: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(table.to_lowercase());
    let mut todo = vec![table.to_string()];
    while let Some(cur) = todo.pop() {
        if seen.len() > MAX_VISITED {
            break;
        }
        for r in &m.relationships {
            let Some(one) = one_side_is(r, &cur) else {
                continue;
            };
            if !seen.insert(one.to_lowercase()) {
                continue;
            }
            if let Some(t) = m.tables.iter().find(|t| t.name.eq_ignore_ascii_case(&one))
                && table_mode(m, t) == "directQuery"
            {
                out.push(t.name.clone());
            }
            todo.push(one);
        }
    }
    out
}

fn parse_mode(p: &Value) -> Result<&'static str, String> {
    let m = str_param(p, "mode")?;
    MODES
        .iter()
        .copied()
        .find(|x| x.eq_ignore_ascii_case(m))
        .ok_or_else(|| format!("unknown mode `{m}` (expected import, directQuery or dual)"))
}

/// Why `t` cannot take `mode` (`None`: it can).
fn refusal(m: &SemanticModel, t: &Table, mode: &str, pbix: bool) -> Option<String> {
    let name = dax_table(&t.name);
    let calculated = bicraft_dax::deps::calculated_table_expression(t).is_some()
        || t.extra.contains_key("calculationGroup")
        || t.partitions
            .iter()
            .any(|p| p.source_type.eq_ignore_ascii_case("calculationGroup"));
    if calculated && mode != "import" {
        return Some(format!(
            "{name} is a calculated table: it is computed from the model's data, so it is always \
             Import"
        ));
    }
    if t.partitions.is_empty() {
        return Some(format!("{name} has no partition to set a storage mode on"));
    }
    if t.extra.contains_key("refreshPolicy") {
        return Some(format!(
            "{name} has an incremental refresh policy, which manages its partitions (an Import \
             table, with a DirectQuery partition for real time): remove the policy first \
             (table.remove_refresh_policy)"
        ));
    }
    if mode == "import" {
        return None;
    }
    if let Some(p) = t
        .partitions
        .iter()
        .find(|p| !is_m(&p.source_type) && !p.source_type.eq_ignore_ascii_case("query"))
    {
        return Some(format!(
            "{name} has a `{}` partition: only Power Query (M) and query partitions can be \
             DirectQuery or Dual",
            p.source_type
        ));
    }
    let live = t.partitions.iter().any(|p| {
        !is_m(&p.source_type)
            || p.expression
                .as_deref()
                .is_some_and(|e| !crate::partition::connectors(m, e).is_empty())
    });
    if !live {
        return Some(format!(
            "{name} has no data source to query live: its rows are entered or computed in Power \
             Query (Enter data, #table), which only Import supports"
        ));
    }
    if pbix && table_mode(m, t) == "import" {
        return Some(format!(
            "{name} is an Import table whose rows this PBIX stores; Power BI Desktop changes only \
             a DirectQuery table's mode: {}",
            crate::partition::PBIX_ADVICE
        ));
    }
    None
}

/// `table.set_storage_mode`.
pub(crate) fn table_set_storage_mode(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["table", "mode", "dual_related"])?;
    let table = str_param(p, "table")?;
    let mode = parse_mode(p)?;
    let dual_related = opt_bool(p, "dual_related")?.unwrap_or(false);
    let pbix = crate::partition::pbix_with_data(s);
    apply(s, p, |m| {
        let ti = table_index(m, table)?;
        let t = m.tables.get(ti).ok_or("internal: table index out of range")?;
        let name = t.name.clone();
        let before = table_mode(m, t);
        if let Some(why) = refusal(m, t, mode, pbix) {
            return Err(why);
        }
        let mut notes = Vec::new();
        let duals = if mode == "import" && dual_related {
            dual_candidates(m, &name)
        } else {
            Vec::new()
        };
        if before == "import" && mode != "import" {
            notes.push(format!(
                "Power BI Desktop changes only a DirectQuery table's mode: it cannot turn the Import \
                 table {} into {mode} itself; BIcraft writes the mode, and Desktop queries the \
                 source when it opens the model",
                dax_table(&name)
            ));
        }
        if before == "directQuery" && mode != "directQuery" {
            notes.push(format!(
                "{} now keeps a copy of its rows: refresh it to load them (Power BI Desktop does not \
                 switch it back to DirectQuery)",
                dax_table(&name)
            ));
        }
        for d in std::iter::once(&name).chain(duals.iter()) {
            let i = table_index(m, d)?;
            let to = if *d == name { mode } else { "dual" };
            if let Some(t) = m.tables.get_mut(i) {
                for part in &mut t.partitions {
                    part.mode = Some(to.to_string());
                }
            }
        }
        // what the change means for relationships and for Dual
        let mode_of = |n: &str| {
            m.tables
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(n))
                .map_or("import", |t| table_mode(m, t))
        };
        let mut limited_rels = Vec::new();
        let mut many_side = 0usize;
        for r in &m.relationships {
            let touches = r.from_table.eq_ignore_ascii_case(&name) || r.to_table.eq_ignore_ascii_case(&name);
            if !touches {
                continue;
            }
            if r.from_table.eq_ignore_ascii_case(&name) {
                many_side = many_side.saturating_add(1);
            }
            if limited(mode_of(&r.from_table), mode_of(&r.to_table)) {
                let text = format!(
                    "{} → {}",
                    dax_column(&r.from_table, &r.from_column),
                    dax_column(&r.to_table, &r.to_column)
                );
                notes.push(format!(
                    "the relationship {text} is limited: Import and DirectQuery tables are in \
                     different source groups (set the one side to Dual to keep it regular)"
                ));
                limited_rels.push(json!(text));
            }
        }
        if mode == "dual" && many_side > 0 {
            notes.push(format!(
                "Dual is meant for dimension tables (the one side of relationships): {} is on the \
                 many side of {many_side} relationship{}",
                dax_table(&name),
                if many_side == 1 { "" } else { "s" }
            ));
        }
        let suggested: Vec<String> = if mode == "import" && !dual_related {
            dual_candidates(m, &name)
        } else {
            Vec::new()
        };
        if !suggested.is_empty() {
            notes.push(format!(
                "these DirectQuery dimension tables could be Dual to keep their relationships \
                 regular: {} (pass dual_related: true)",
                suggested.join(", ")
            ));
        }
        let mut edit = Edit::new(json!({
            "table": name,
            "mode": mode,
            "previous_mode": before,
            "set_to_dual": duals,
            "dual_candidates": suggested,
            "limited_relationships": limited_rels,
        }));
        edit.notes = notes;
        Ok(edit)
    })
}
