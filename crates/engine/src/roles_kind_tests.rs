//! G14 leftovers of the role checks: the `kind` inside conditions (scatter chart, maps), the
//! "hierarchically related" fields of slicers, and the scorecard. Expected verdicts are those of
//! Power BI Desktop on generated reports (`docs/formats/visual-roles-bb.md`, test set `kinds`).

use bicraft_report::{FieldBinding, FieldRef};
use serde_json::{Value, json};

use super::check_roles;
use crate::Session;

fn col(role: &str, c: &str) -> FieldBinding {
    FieldBinding {
        display_name: None,
        show_as: None,
        role: role.into(),
        field: FieldRef::Column {
            table: "T".into(),
            column: c.into(),
        },
    }
}

fn sum(role: &str, c: &str) -> FieldBinding {
    FieldBinding {
        display_name: None,
        show_as: None,
        role: role.into(),
        field: FieldRef::Aggregation {
            table: "T".into(),
            column: c.into(),
            function: "Sum".into(),
        },
    }
}

fn mea(role: &str, m: &str) -> FieldBinding {
    FieldBinding {
        display_name: None,
        show_as: None,
        role: role.into(),
        field: FieldRef::Measure {
            table: "T".into(),
            measure: m.into(),
        },
    }
}

fn errors(t: &str, f: &[FieldBinding]) -> Vec<String> {
    check_roles(Some(t), f)
        .into_iter()
        .filter(|i| i.error)
        .map(|i| i.message)
        .collect()
}

#[test]
fn scatter_kinds_follow_desktop() {
    // Desktop draws these.
    for ok in [
        vec![col("Category", "R"), sum("X", "A"), sum("Y", "Q")],
        vec![sum("X", "A"), sum("Y", "Q")],
        vec![col("X", "A"), col("Y", "Q")],
        vec![col("X", "A"), sum("Y", "Q")],
        vec![sum("X", "A"), col("Y", "Q")],
        vec![sum("X", "A"), sum("Y", "Q"), sum("Size", "N")],
        vec![
            col("Category", "R"),
            mea("X", "Total"),
            sum("Y", "Q"),
            sum("Size", "N"),
        ],
        // A grouped X axis (several columns) with a measure on Y.
        vec![col("X", "A"), col("X", "B"), mea("Y", "Total")],
    ] {
        assert!(
            errors("scatterChart", &ok).is_empty(),
            "{ok:?}: {:?}",
            errors("scatterChart", &ok)
        );
    }
    // Desktop: "remove Values to show x/y pairs, or summarize the x and y axes".
    let e = errors(
        "scatterChart",
        &[col("Category", "R"), col("X", "A"), sum("Y", "Q")],
    );
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(
        e[0].contains("`X` needs a measure or a summarized column"),
        "{e:?}"
    );
    assert!(e[0].contains("T[A] is a column that is not summarized"), "{e:?}");
    assert!(e[0].contains("X≤1 summarized"), "{e:?}");
    let e = errors(
        "scatterChart",
        &[col("Category", "R"), mea("X", "Total"), col("Y", "Q")],
    );
    assert!(
        e[0].contains("`Y` needs a measure or a summarized column"),
        "{e:?}"
    );
    // Without details, columns that are not summarized cannot go with a size.
    let e = errors("scatterChart", &[col("X", "A"), col("Y", "Q"), sum("Size", "N")]);
    assert!(e.is_empty(), "{e:?}");
    // X is a drill role: two columns are two levels, counted as one. Desktop shows an error here
    // (kinds-s12: several X columns with a raw Y column) that the counts cannot express: not
    // reported (permissive).
    assert!(errors("scatterChart", &[col("X", "A"), col("X", "B"), col("Y", "Q")]).is_empty());
}

#[test]
fn map_conditions_are_unknown_so_not_checked() {
    // Desktop could not draw any map in the black-box tests (map visuals switched off in its
    // security options, Azure Maps needs a sign-in: kinds-m01..m04, f01, f02, a01..a04), so no
    // kind or field-count combination is known for maps: raw or summarized coordinates pass.
    for t in ["map", "azureMap", "filledMap"] {
        assert!(errors(t, &[col("Category", "City"), sum("Y", "Lat"), sum("X", "Lon")]).is_empty());
        assert!(errors(t, &[col("Y", "Lat"), col("X", "Lon")]).is_empty());
        assert!(errors(t, &[col("Category", "City"), col("Y", "Lat"), col("X", "Lon")]).is_empty());
        // A location takes columns only (wells-B refused a measure).
        let e = errors(t, &[mea("Category", "Total")]);
        assert!(e[0].contains("takes columns only"), "{t}: {e:?}");
    }
}
#[test]
fn scorecard_takes_no_fields() {
    let e = errors("scorecard", &[mea("Values", "Total")]);
    assert!(e[0].contains("takes no data fields"), "{e:?}");
    assert!(check_roles(Some("scorecard"), &[]).is_empty());
}

#[test]
fn qna_and_narratives_ignore_fields() {
    // Desktop 2.158 draws both with or without fields: warnings only.
    let all = |t: &str, f: &[FieldBinding]| -> Vec<(bool, String)> {
        check_roles(Some(t), f)
            .into_iter()
            .map(|i| (i.error, i.message))
            .collect()
    };
    let q = all("qnaVisual", &[]);
    assert_eq!(q.len(), 1, "{q:?}");
    assert!(!q[0].0 && q[0].1.contains("retired in February 2027"), "{q:?}");
    let q = all("qnaVisual", &[col("Values", "R"), mea("Values", "Total")]);
    assert_eq!(q.len(), 2, "{q:?}");
    assert!(q.iter().all(|(e, _)| !e), "{q:?}");
    assert!(q[1].1.contains("ignores its 2 fields"), "{q:?}");
    assert!(all("aiNarratives", &[]).is_empty());
    let n = all("aiNarratives", &[mea("Values", "Total")]);
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(!n[0].0 && n[0].1.contains("ignores its 1 field"), "{n:?}");
}

/// Geo 1—* Sales *—1 Prod, Prod *—* Tag, Geo 1—1 Geo2, Iso2 *—1 Geo (inactive), Iso alone.
fn session() -> Session {
    let mut s = Session::new();
    let mut run = |id: &str, p: Value| {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    run("model.new", json!({"name": "Shop"}));
    for (t, cols) in [
        ("Geo", &["Region", "Country"][..]),
        ("Prod", &["Category", "Family"][..]),
        ("Sales", &["Region", "Category"][..]),
        ("Tag", &["Category", "Tag"][..]),
        ("Geo2", &["Region", "Zone"][..]),
        ("Iso2", &["Region", "Name"][..]),
        ("Iso", &["Name"][..]),
    ] {
        run("table.add", json!({"name": t}));
        for c in cols {
            run(
                "column.add",
                json!({"table": t, "name": c, "data_type": "string"}),
            );
        }
    }
    run(
        "relationship.add",
        json!({"from": "Sales[Region]", "to": "Geo[Region]"}),
    );
    run(
        "relationship.add",
        json!({"from": "Sales[Category]", "to": "Prod[Category]"}),
    );
    run(
        "relationship.add",
        json!({"from": "Tag[Category]", "to": "Prod[Category]", "from_cardinality": "many", "to_cardinality": "many"}),
    );
    run(
        "relationship.add",
        json!({"from": "Geo2[Region]", "to": "Geo[Region]", "from_cardinality": "one", "to_cardinality": "one"}),
    );
    run(
        "relationship.add",
        json!({"from": "Iso2[Region]", "to": "Geo[Region]", "active": false}),
    );
    run("report.new", json!({}));
    s
}

fn add(s: &mut Session, t: &str, fields: &[&str], force: bool) -> Result<Value, String> {
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| json!({"role": "Values", "field": f}))
        .collect();
    s.execute(
        "visual.add",
        &json!({"page": "Page 1", "visual_type": t, "x": 0, "y": 0, "width": 300, "height": 200,
                "fields": fields, "force": force}),
    )
}

#[test]
fn slicer_fields_must_be_hierarchically_related() {
    let mut s = session();
    // Desktop 2.158 draws these slicers.
    for ok in [
        &["Geo[Region]", "Sales[Category]"][..],
        &["Sales[Category]", "Geo[Region]"][..],
        &["Geo[Country]", "Geo[Region]"][..],
        &["Geo[Region]", "Geo2[Zone]"][..],
        &["Geo2[Zone]", "Geo[Region]"][..],
        &["Geo[Region]"][..],
    ] {
        add(&mut s, "slicer", ok, false).unwrap_or_else(|e| panic!("{ok:?}: {e}"));
    }
    add(&mut s, "listSlicer", &["Geo[Region]", "Sales[Category]"], false).unwrap();
    // And shows "fields that don't have a supported set of relationships" for these: two
    // dimensions of one fact, unrelated tables, an inactive or a many-to-many relationship.
    for bad in [
        &["Geo[Region]", "Prod[Family]"][..],
        &["Geo[Region]", "Iso[Name]"][..],
        &["Geo[Region]", "Iso2[Name]"][..],
        &["Prod[Category]", "Tag[Tag]"][..],
        &["Geo[Region]", "Sales[Category]", "Prod[Family]"][..],
        &["Sales[Category]", "Geo[Region]", "Prod[Family]"][..],
    ] {
        let e = add(&mut s, "slicer", bad, false).unwrap_err();
        assert!(e.contains("no chain of active relationships"), "{bad:?}: {e}");
    }
    let e = add(&mut s, "listSlicer", &["Geo[Region]", "Prod[Family]"], false).unwrap_err();
    assert!(e.contains("supported set of relationships"), "{e}");
    // Forced, the visual is written and report.validate reports it.
    add(&mut s, "slicer", &["Geo[Region]", "Prod[Family]"], true).unwrap();
    let v = s.execute("report.validate", &json!({})).unwrap();
    let found: Vec<&Value> = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| {
            i["message"]
                .as_str()
                .unwrap_or("")
                .contains("no chain of active relationships")
        })
        .collect();
    assert_eq!(found.len(), 1, "{v:#}");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["role"], "Values");
    // Without a model nothing is said about relationships.
    assert!(
        super::check_bindings(None, Some("slicer"), &[col("Values", "A"), col("Values", "B")]).is_empty()
    );
}

#[test]
fn relationship_search_is_bounded() {
    // A dense model: every table related to the next ones (one-to-one, both directions).
    let mut s = Session::new();
    s.execute("model.new", &json!({"name": "Dense"})).unwrap();
    let n = 40;
    for i in 0..n {
        s.execute("table.add", &json!({"name": format!("T{i}")})).unwrap();
        s.execute(
            "column.add",
            &json!({"table": format!("T{i}"), "name": "K", "data_type": "string"}),
        )
        .unwrap();
    }
    for i in 0..n {
        for j in (i + 1)..n.min(i + 4) {
            s.execute(
                "relationship.add",
                &json!({"from": format!("T{i}[K]"), "to": format!("T{j}[K]"),
                        "from_cardinality": "one", "to_cardinality": "one"}),
            )
            .unwrap();
        }
    }
    s.execute("report.new", &json!({})).unwrap();
    let fields: Vec<String> = (0..n).step_by(5).map(|i| format!("T{i}[K]")).collect();
    let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
    // Either verdict is fine; it must come back.
    let _ = add(&mut s, "slicer", &refs, false);
}
