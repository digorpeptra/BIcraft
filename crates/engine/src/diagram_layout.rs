//! The model diagram: where the Model view's table cards sit (the PBIX/PBIT `DiagramLayout` part,
//! the PBIP `<Name>.SemanticModel/diagramLayout.json`; docs/formats/diagram-layout.md).
//!
//! The file's layout is kept as it is until a command edits it (`diagram.move`, the layout tabs'
//! `diagram.add`, `diagram.rename`, `diagram.remove`, and `table.rename` for its nodes); it then keeps the
//! edited text in [`Session::diagram_layout`], which the undo history snapshots with the model
//! (`history.rs`), and `file.save` / `file.save_as` write it back (UTF-16LE in a package, UTF-8 in
//! a project). Every other member of the layout (other diagrams, zoom, flags, unknown members)
//! keeps its value and its place.
//!
//! [`snapshot`] gives the Model view the selected diagram's node positions.

use serde_json::{Map, Value, json};

use crate::commands::model;
use crate::params::{allow, opt_str, str_param};
use crate::{Session, Source};

/// The package part.
pub(crate) const PART: &str = "DiagramLayout";
/// Largest layout read or edited (a Desktop layout is a few kilobytes per table).
pub(crate) const MAX_LAYOUT_BYTES: usize = 16 * 1024 * 1024;
/// Most nodes the snapshot lists, and a diagram may hold.
pub(crate) const MAX_NODES: usize = 10_000;
/// Largest coordinate accepted (Desktop's canvas is far smaller).
const MAX_COORD: f64 = 1_000_000.0;
/// The width Desktop gives a node (observed in Desktop-saved projects), and the height a new node
/// gets (unchecked: Desktop sizes the card itself when it opens the file).
const NODE_WIDTH: u64 = 234;
const NODE_HEIGHT: u64 = 300;
/// The diagram a new layout starts with (the PBIT writer's empty layout).
const NEW_DIAGRAM: &str = "All tables";

/// The open file's own layout text (`None`: it has none).
pub(crate) fn source_text(s: &Session) -> Result<Option<String>, String> {
    let text = match &s.source {
        Some(Source::Package { package, .. }) => {
            let Some(part) = package.get(PART) else {
                return Ok(None);
            };
            if part.data.len() > MAX_LAYOUT_BYTES {
                return Err(format!(
                    "the model diagram is larger than {} MiB",
                    MAX_LAYOUT_BYTES / 1024 / 1024
                ));
            }
            bicraft_opc::decode_text(&part.name, &part.data)
                .map_err(|e| e.to_string())?
                .0
        }
        Some(Source::Project { project, .. }) => {
            match bicraft_pbip::read_diagram_layout(project).map_err(|e| e.to_string())? {
                Some(t) => t,
                None => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    if text.len() > MAX_LAYOUT_BYTES {
        return Err(format!(
            "the model diagram is larger than {} MiB",
            MAX_LAYOUT_BYTES / 1024 / 1024
        ));
    }
    Ok(Some(text))
}

/// The layout in force: the session's (read from the file at open, then edited). `None` with a
/// file whose layout cannot be read is that error, so that a move never replaces it.
pub(crate) fn current_text(s: &Session) -> Result<Option<String>, String> {
    match &s.diagram_layout {
        Some(t) => Ok(Some(t.clone())),
        None => source_text(s).map(|_| None),
    }
}

/// The layout `file.open` puts in the session: the file's, when it can be read.
pub(crate) fn at_open(s: &Session) -> Option<String> {
    source_text(s).ok().flatten()
}

/// What a save over the open file writes as its layout: the session's when it differs from the
/// file's, an empty layout when the session has none but the file has one (a move undone past a
/// save), else nothing (the file keeps its own bytes).
pub(crate) fn to_write(s: &Session) -> Option<String> {
    let file = source_text(s).ok().flatten();
    match (&s.diagram_layout, file) {
        (Some(t), Some(f)) if *t == f => None,
        (Some(t), _) => Some(t.clone()),
        (None, Some(_)) => serde_json::to_string(&Value::Object(new_layout())).ok(),
        (None, None) => None,
    }
}

/// The index of the diagram named `name`, else of `selectedDiagram`, `defaultDiagram`, the first.
fn diagram_index(layout: &Map<String, Value>, name: Option<&str>) -> Option<usize> {
    let diagrams = layout.get("diagrams")?.as_array()?;
    let named = |n: &str| {
        diagrams
            .iter()
            .position(|d| d.get("name").and_then(Value::as_str) == Some(n))
    };
    if let Some(n) = name {
        return named(n);
    }
    let pick = |k: &str| layout.get(k).and_then(Value::as_str).and_then(named);
    pick("selectedDiagram")
        .or_else(|| pick("defaultDiagram"))
        .or_else(|| (!diagrams.is_empty()).then_some(0))
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}

/// The snapshot's `model.diagram`: `{name, diagrams: [names], nodes: [{table, x, y}], layouts:
/// [{name, all, nodes}]}`: the selected diagram's name and nodes, then every layout tab (`all`:
/// the one that shows every table); `None` when the file has no usable layout.
pub(crate) fn snapshot(s: &Session) -> Option<Value> {
    let text = current_text(s).ok()??;
    let layout = serde_json::from_str::<Value>(&text).ok()?;
    let layout = layout.as_object()?;
    let names: Vec<&str> = layout
        .get("diagrams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| d.get("name").and_then(Value::as_str))
        .take(100)
        .collect();
    let i = diagram_index(layout, None)?;
    let diagrams = layout.get("diagrams")?.as_array()?;
    let d = diagrams.get(i)?;
    let node_list = |d: &Value, budget: usize| -> Vec<Value> {
        d.get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(budget)
            .filter_map(|n| {
                let table = n.get("nodeIndex")?.as_str()?;
                let loc = n.get("location")?;
                let (x, y) = (num(loc.get("x"))?, num(loc.get("y"))?);
                Some(json!({"table": table, "x": x, "y": y}))
            })
            .collect()
    };
    let nodes = node_list(d, MAX_NODES);
    // every layout tab with its nodes (the Model view's tabs), MAX_NODES in all
    let all = all_tables_index(diagrams);
    let mut budget = MAX_NODES;
    let layouts: Vec<Value> = diagrams
        .iter()
        .take(MAX_DIAGRAMS)
        .enumerate()
        .filter(|(_, d)| d.get("name").is_some_and(Value::is_string))
        .map(|(k, d)| {
            let nodes = node_list(d, budget);
            budget = budget.saturating_sub(nodes.len());
            json!({"name": diagram_name(d), "all": k == all, "nodes": nodes})
        })
        .collect();
    Some(json!({
        "name": d.get("name").and_then(Value::as_str).unwrap_or(""),
        "diagrams": names,
        "nodes": nodes,
        "layouts": layouts,
    }))
}

/// A new layout: one empty diagram, as the PBIT writer's.
fn new_layout() -> Map<String, Value> {
    let v = json!({
        "version": "1.1.0",
        "diagrams": [{
            "ordinal": 0,
            "scrollPosition": {"x": 0, "y": 0},
            "nodes": [],
            "name": NEW_DIAGRAM,
            "zoomValue": 100,
            "pinKeyFieldsToTop": false,
            "showExtraHeaderInfo": false,
            "hideKeyFieldsWhenCollapsed": false,
            "tablesLocked": false,
        }],
        "selectedDiagram": NEW_DIAGRAM,
        "defaultDiagram": NEW_DIAGRAM,
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn coord(p: &Value, k: &str) -> Result<f64, String> {
    let v = p.get(k).ok_or_else(|| format!("`{k}` is required (a number)"))?;
    let x = v
        .as_f64()
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("`{k}` must be a number"))?;
    if x.abs() > MAX_COORD {
        return Err(format!("`{k}` must be between -{MAX_COORD} and {MAX_COORD}"));
    }
    Ok(x)
}

/// A coordinate as Desktop writes it: an integer when whole, else the number.
fn coord_json(x: f64) -> Value {
    let r = x.round();
    if (x - r).abs() < 1e-9 && r.abs() <= MAX_COORD {
        json!(r as i64)
    } else {
        json!(x)
    }
}

/// One card to place: the table, its corner, its lineage tag.
struct Place {
    table: String,
    x: f64,
    y: f64,
    lineage: Option<String>,
}

/// The cards of `diagram.move`: `{table, x, y}` or `tables: [{table, x, y}]`.
fn places(s: &Session, p: &Value) -> Result<Vec<Place>, String> {
    let m = model(s)?;
    let one = |p: &Value| -> Result<Place, String> {
        let table = str_param(p, "table")?;
        let (x, y) = (coord(p, "x")?, coord(p, "y")?);
        let t = m
            .tables
            .iter()
            .find(|t| t.name == table)
            .ok_or_else(|| format!("no table `{table}` in the model"))?;
        Ok(Place {
            table: table.to_string(),
            x,
            y,
            lineage: t.lineage_tag.clone(),
        })
    };
    match p.get("tables") {
        None => {
            allow(p, &["table", "x", "y", "diagram"])?;
            Ok(vec![one(p)?])
        }
        Some(list) => {
            allow(p, &["tables", "diagram"])?;
            let list = list
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or("`tables` must be a non-empty list of {table, x, y}")?;
            if list.len() > MAX_NODES {
                return Err(format!("`tables` lists more than {MAX_NODES} tables"));
            }
            list.iter()
                .map(|item| {
                    allow(item, &["table", "x", "y"])?;
                    one(item)
                })
                .collect()
        }
    }
}

/// `diagram.move {table, x, y, diagram?}` or `{tables: [{table, x, y}], diagram?}`: puts each
/// table's card at (`x`, `y`) in the diagram (the selected one by default), adding its node when
/// the diagram has none for it. One undo step.
pub(crate) fn diagram_move(s: &mut Session, p: &Value) -> Result<Value, String> {
    if !p.is_object() {
        return Err("diagram.move takes {table, x, y} or {tables: [{table, x, y}]}".into());
    }
    let places = places(s, p)?;
    let want = opt_str(p, "diagram")?;
    let mut layout = load(s)?;
    let i = diagram_index(&layout, want).ok_or_else(|| match want {
        Some(n) => format!("no diagram named `{n}`"),
        None => "the model diagram has no diagram".into(),
    })?;
    let diagram = layout
        .get_mut("diagrams")
        .and_then(Value::as_array_mut)
        .and_then(|a| a.get_mut(i))
        .and_then(Value::as_object_mut)
        .ok_or("the model diagram's diagram is not a JSON object")?;
    let name = diagram
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let nodes = diagram.entry("nodes").or_insert_with(|| Value::Array(Vec::new()));
    if !nodes.is_array() {
        *nodes = Value::Array(Vec::new());
    }
    let nodes = nodes.as_array_mut().ok_or("the diagram's nodes are not a list")?;
    for place in &places {
        let location = json!({"x": coord_json(place.x), "y": coord_json(place.y)});
        match nodes
            .iter_mut()
            .find(|n| n.get("nodeIndex").and_then(Value::as_str) == Some(place.table.as_str()))
            .and_then(Value::as_object_mut)
        {
            Some(node) => {
                node.insert("location".into(), location);
            }
            None => {
                if nodes.len() >= MAX_NODES {
                    return Err(format!("the diagram already has {MAX_NODES} tables"));
                }
                let mut node = Map::new();
                node.insert("location".into(), location);
                node.insert("nodeIndex".into(), json!(place.table));
                if let Some(tag) = &place.lineage {
                    node.insert("nodeLineageTag".into(), json!(tag));
                }
                node.insert("size".into(), json!({"height": NODE_HEIGHT, "width": NODE_WIDTH}));
                node.insert("zIndex".into(), json!(0));
                nodes.push(Value::Object(node));
            }
        }
    }
    store(s, layout)?;
    let moved: Vec<Value> = places
        .iter()
        .map(|p| json!({"table": p.table, "x": p.x, "y": p.y}))
        .collect();
    Ok(json!({"moved": moved, "diagram": name}))
}

/// The layout a command edits: the session's, else a new one; a layout without diagrams gets the
/// new layout's one. A layout that is not a JSON object is refused (left as it is).
fn load(s: &Session) -> Result<Map<String, Value>, String> {
    let mut layout = match current_text(s)? {
        None => new_layout(),
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => {
                return Err("the file's model diagram is not a JSON object; it is left as it is".into());
            }
        },
    };
    if layout
        .get("diagrams")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        let fresh = new_layout();
        for k in ["diagrams", "selectedDiagram", "defaultDiagram"] {
            if let Some(v) = fresh.get(k) {
                layout.insert(k.into(), v.clone());
            }
        }
    }
    Ok(layout)
}

/// Keeps the edited layout in the session (an unsaved edit, one undo step).
fn store(s: &mut Session, layout: Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string(&Value::Object(layout)).map_err(|e| e.to_string())?;
    if text.len() > MAX_LAYOUT_BYTES {
        return Err(format!(
            "the model diagram would be larger than {} MiB",
            MAX_LAYOUT_BYTES / 1024 / 1024
        ));
    }
    s.diagram_layout = Some(text);
    s.dirty = true;
    Ok(())
}

/// Most layout tabs a model diagram may hold (the snapshot lists as many).
const MAX_DIAGRAMS: usize = 100;
/// Longest layout name.
const MAX_NAME: usize = 200;
/// Cards per row when `diagram.add` places tables, and the gap between them.
const GRID_COLUMNS: usize = 4;
const GRID_GAP: u64 = 60;

fn diagram_name(d: &Value) -> &str {
    d.get("name").and_then(Value::as_str).unwrap_or("")
}

/// The layout that shows every table: the diagram with ordinal 0, else the first (Desktop's
/// "All tables"; docs/formats/diagram-layout.md).
fn all_tables_index(diagrams: &[Value]) -> usize {
    diagrams
        .iter()
        .position(|d| d.get("ordinal").and_then(Value::as_i64) == Some(0))
        .unwrap_or(0)
}

fn diagrams_mut(layout: &mut Map<String, Value>) -> Result<&mut Vec<Value>, String> {
    layout
        .get_mut("diagrams")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "the model diagram has no list of diagrams".into())
}

/// The index of the diagram named `name` (exactly, else ignoring case).
fn named_index(diagrams: &[Value], name: &str) -> Result<usize, String> {
    diagrams
        .iter()
        .position(|d| diagram_name(d) == name)
        .or_else(|| {
            diagrams
                .iter()
                .position(|d| diagram_name(d).eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| format!("no layout named `{name}` in the model diagram"))
}

/// A new layout name: not empty, not too long, not another layout's (ignoring case).
fn check_new_name(diagrams: &[Value], name: &str, except: Option<usize>) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("a layout's name must not be empty".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("a layout's name has at most {MAX_NAME} characters"));
    }
    if let Some((_, d)) = diagrams
        .iter()
        .enumerate()
        .find(|(j, d)| Some(*j) != except && diagram_name(d).eq_ignore_ascii_case(name))
    {
        return Err(format!("a layout named `{}` already exists", diagram_name(d)));
    }
    Ok(())
}

/// `diagram.add {name, tables?}`: a new layout tab, after the others, showing `tables` placed in
/// rows of four (none by default); it becomes the selected one. One undo step.
pub(crate) fn diagram_add(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "tables"])?;
    let name = str_param(p, "name")?.to_string();
    let tables: Vec<(String, Option<String>)> = match p.get("tables") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => {
            if list.len() > MAX_NODES {
                return Err(format!("`tables` lists more than {MAX_NODES} tables"));
            }
            let m = model(s)?;
            let mut out: Vec<(String, Option<String>)> = Vec::new();
            for v in list {
                let t = v.as_str().ok_or("`tables` must be a list of table names")?;
                let table = m
                    .tables
                    .iter()
                    .find(|x| x.name == t)
                    .ok_or_else(|| format!("no table `{t}` in the model"))?;
                if !out.iter().any(|(n, _)| *n == table.name) {
                    out.push((table.name.clone(), table.lineage_tag.clone()));
                }
            }
            out
        }
        Some(_) => return Err("`tables` must be a list of table names".into()),
    };
    model(s)?;
    let mut layout = load(s)?;
    let diagrams = diagrams_mut(&mut layout)?;
    if diagrams.len() >= MAX_DIAGRAMS {
        return Err(format!("the model diagram already has {MAX_DIAGRAMS} layouts"));
    }
    check_new_name(diagrams, &name, None)?;
    let ordinal = diagrams
        .iter()
        .filter_map(|d| d.get("ordinal").and_then(Value::as_i64))
        .max()
        .map_or(0, |o| o.saturating_add(1))
        .max(i64::try_from(diagrams.len()).unwrap_or(i64::MAX));
    let nodes: Vec<Value> = tables
        .iter()
        .enumerate()
        .map(|(k, (table, lineage))| {
            let (col, row) = ((k % GRID_COLUMNS) as u64, (k / GRID_COLUMNS) as u64);
            let mut node = Map::new();
            node.insert(
                "location".into(),
                json!({
                    "x": GRID_GAP + col * (NODE_WIDTH + GRID_GAP),
                    "y": GRID_GAP + row * (NODE_HEIGHT + GRID_GAP),
                }),
            );
            node.insert("nodeIndex".into(), json!(table));
            if let Some(tag) = lineage {
                node.insert("nodeLineageTag".into(), json!(tag));
            }
            node.insert("size".into(), json!({"height": NODE_HEIGHT, "width": NODE_WIDTH}));
            node.insert("zIndex".into(), json!(0));
            Value::Object(node)
        })
        .collect();
    diagrams.push(json!({
        "ordinal": ordinal,
        "scrollPosition": {"x": 0, "y": 0},
        "nodes": nodes,
        "name": name,
        "zoomValue": 100,
        "pinKeyFieldsToTop": false,
        "showExtraHeaderInfo": false,
        "hideKeyFieldsWhenCollapsed": false,
        "tablesLocked": false,
    }));
    layout.insert("selectedDiagram".into(), json!(name));
    store(s, layout)?;
    Ok(json!({"diagram": name, "tables": tables.len()}))
}

/// `diagram.rename {name, new_name}`: renames a layout tab; `selectedDiagram` and
/// `defaultDiagram` follow. One undo step.
pub(crate) fn diagram_rename(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name", "new_name"])?;
    let name = str_param(p, "name")?;
    let new_name = str_param(p, "new_name")?.to_string();
    model(s)?;
    let mut layout = load(s)?;
    let diagrams = diagrams_mut(&mut layout)?;
    let i = named_index(diagrams, name)?;
    check_new_name(diagrams, &new_name, Some(i))?;
    let d = diagrams
        .get_mut(i)
        .and_then(Value::as_object_mut)
        .ok_or("the layout is not a JSON object")?;
    let old = d.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    d.insert("name".into(), json!(new_name));
    for k in ["selectedDiagram", "defaultDiagram"] {
        if layout.get(k).and_then(Value::as_str) == Some(old.as_str()) {
            layout.insert(k.into(), json!(new_name));
        }
    }
    store(s, layout)?;
    Ok(json!({"renamed": old, "to": new_name}))
}

/// `diagram.remove {name}`: deletes a layout tab; the layout that shows every table stays.
/// `selectedDiagram` and `defaultDiagram` that named it go to that one. One undo step.
pub(crate) fn diagram_remove(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["name"])?;
    let name = str_param(p, "name")?;
    model(s)?;
    let mut layout = load(s)?;
    let diagrams = diagrams_mut(&mut layout)?;
    let i = named_index(diagrams, name)?;
    let all = all_tables_index(diagrams);
    if i == all || diagrams.len() < 2 {
        return Err(format!(
            "`{}` is the layout of all tables; it cannot be deleted",
            diagram_name(diagrams.get(i).unwrap_or(&Value::Null))
        ));
    }
    let removed = diagrams.remove(i);
    let old = diagram_name(&removed).to_string();
    let all = all_tables_index(diagrams);
    let fallback = diagrams
        .get(all)
        .map(|d| diagram_name(d).to_string())
        .unwrap_or_default();
    for k in ["selectedDiagram", "defaultDiagram"] {
        if layout.get(k).and_then(Value::as_str) == Some(old.as_str()) {
            layout.insert(k.into(), json!(fallback));
        }
    }
    store(s, layout)?;
    Ok(json!({"removed": old}))
}

/// `diagram.remove_tables {tables, diagram?}`: takes the tables' cards out of a layout tab (the
/// selected one by default) other than the layout of all tables; the tables stay in the model.
/// One undo step.
pub(crate) fn diagram_remove_tables(s: &mut Session, p: &Value) -> Result<Value, String> {
    allow(p, &["tables", "diagram"])?;
    let tables: Vec<&str> = p
        .get("tables")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.len() <= MAX_NODES)
        .ok_or_else(|| format!("`tables` must be a list of 1 to {MAX_NODES} table names"))?
        .iter()
        .map(|v| v.as_str().ok_or("`tables` must be a list of table names"))
        .collect::<Result<_, _>>()?;
    let want = opt_str(p, "diagram")?;
    model(s)?;
    let mut layout = load(s)?;
    let i = match want {
        Some(n) => named_index(diagrams_mut(&mut layout)?, n)?,
        None => diagram_index(&layout, None).ok_or("the model diagram has no diagram")?,
    };
    let diagrams = diagrams_mut(&mut layout)?;
    if i == all_tables_index(diagrams) {
        return Err(format!(
            "`{}` is the layout of all tables; it shows every table",
            diagram_name(diagrams.get(i).unwrap_or(&Value::Null))
        ));
    }
    let d = diagrams
        .get_mut(i)
        .and_then(Value::as_object_mut)
        .ok_or("the layout is not a JSON object")?;
    let name = d.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let mut removed: Vec<String> = Vec::new();
    if let Some(nodes) = d.get_mut("nodes").and_then(Value::as_array_mut) {
        nodes.retain(|n| match n.get("nodeIndex").and_then(Value::as_str) {
            Some(t) if tables.contains(&t) => {
                if !removed.iter().any(|r| r == t) {
                    removed.push(t.to_string());
                }
                false
            }
            _ => true,
        });
    }
    if removed.is_empty() {
        return Err(format!("the layout `{name}` shows none of these tables"));
    }
    store(s, layout)?;
    Ok(json!({"removed": removed, "diagram": name}))
}

/// After `table.rename`: the nodes of every layout that named the table (`nodeIndex`) name the
/// new one. A layout that cannot be read is left alone.
pub(crate) fn rename_table(s: &mut Session, old: &str, new: &str) {
    let Ok(Some(text)) = current_text(s) else {
        return;
    };
    let Ok(Value::Object(mut layout)) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let mut changed = false;
    for d in layout
        .get_mut("diagrams")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for n in d
            .get_mut("nodes")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object_mut)
        {
            if n.get("nodeIndex").and_then(Value::as_str) == Some(old) {
                n.insert("nodeIndex".into(), json!(new));
                changed = true;
            }
        }
    }
    if changed {
        // a rename keeps the layout's size; a failure leaves the old names (shown unplaced)
        let _ = store(s, layout);
    }
}

/// The edited layout as a package part's bytes (UTF-16LE without BOM, as Desktop writes it).
pub(crate) fn part_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Writes the layout a save writes (`to_write`) into `package`, built from the open file.
pub(crate) fn put_in_package(s: &Session, package: &mut bicraft_opc::Package) {
    if let Some(text) = &to_write(s) {
        // the part keeps its name's case and its compression
        let (name, stored) = package
            .get(PART)
            .map_or((PART.to_string(), false), |p| (p.name.clone(), p.stored));
        package.put(&name, part_bytes(text), stored);
    }
}
