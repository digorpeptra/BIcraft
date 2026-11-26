//! Tests of the model translation commands (M10.4): `culture.*`, `translation.*`, renames and
//! removals, reads and the viewer snapshot in a culture, and format round trips.

use serde_json::{Value, json};

use crate::tempdir::TempDir;
use bicraft_engine::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn fails(s: &mut Session, id: &str, params: Value) -> String {
    let before = s.model.clone();
    let err = s
        .execute(id, &params)
        .expect_err(&format!("{id} {params} should fail"));
    assert_eq!(s.model, before, "{id} changed the model although it failed");
    err
}

/// Sales(Amount, Region, City) with [Total] and the hierarchy Geo (Region, City), Product(Name),
/// the shared expression Source, the role Reader and a fr-FR culture.
fn session() -> Session {
    let mut s = Session::new();
    for (id, p) in [
        ("model.new", json!({"name": "Shop"})),
        ("table.add", json!({"name": "Sales"})),
        ("table.add", json!({"name": "Product"})),
        (
            "column.add",
            json!({"table": "Sales", "name": "Amount", "data_type": "double"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "Region", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Sales", "name": "City", "data_type": "string"}),
        ),
        (
            "column.add",
            json!({"table": "Product", "name": "Name", "data_type": "string"}),
        ),
        (
            "measure.add",
            json!({"table": "Sales", "name": "Total", "expression": "SUM(Sales[Amount])"}),
        ),
        (
            "hierarchy.add",
            json!({"table": "Sales", "name": "Geo", "levels": ["Region", "City"]}),
        ),
        ("expression.add", json!({"name": "Source", "expression": "\"x\""})),
        ("role.add", json!({"name": "Reader"})),
        ("culture.add", json!({"name": "fr-FR"})),
    ] {
        run(&mut s, id, p);
    }
    s
}

fn set(s: &mut Session, object: Value, property: &str, value: Value) -> Value {
    run(
        s,
        "translation.set",
        json!({"culture": "fr-FR", "object": object, "property": property, "value": value}),
    )
}

fn culture(s: &Session, name: &str) -> Value {
    s.model.as_ref().unwrap().extra["cultures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .cloned()
        .unwrap_or(Value::Null)
}

fn fr_tables(s: &Session) -> Value {
    culture(s, "fr-FR")["translations"]["model"]["tables"].clone()
}

/// Translations of every kind of object, set out of model order.
fn translated() -> Session {
    let mut s = session();
    set(
        &mut s,
        json!({"table": "Sales", "measure": "Total"}),
        "caption",
        json!("Total FR"),
    );
    set(
        &mut s,
        json!({"table": "Sales", "column": "City"}),
        "caption",
        json!("Ville"),
    );
    set(
        &mut s,
        json!({"table": "Sales", "column": "Amount"}),
        "display_folder",
        json!("Dossier"),
    );
    set(
        &mut s,
        json!({"table": "Sales", "column": "Amount"}),
        "caption",
        json!("Montant"),
    );
    set(&mut s, json!({"table": "Product"}), "caption", json!("Produit"));
    set(
        &mut s,
        json!({"table": "Sales"}),
        "description",
        json!("Table des ventes"),
    );
    set(&mut s, json!({"table": "Sales"}), "caption", json!("Ventes"));
    set(
        &mut s,
        json!({"table": "Sales", "hierarchy": "Geo"}),
        "caption",
        json!("Geo FR"),
    );
    set(
        &mut s,
        json!({"table": "Sales", "hierarchy": "Geo", "level": "City"}),
        "caption",
        json!("Ville (niveau)"),
    );
    set(&mut s, json!({}), "caption", json!("Modele"));
    set(&mut s, json!({"role": "Reader"}), "caption", json!("Lecteur"));
    set(
        &mut s,
        json!({"expression": "Source"}),
        "description",
        json!("Requete"),
    );
    s
}

#[test]
fn translations_are_kept_in_model_order_like_the_readers() {
    let s = translated();
    assert_eq!(
        culture(&s, "fr-FR"),
        json!({"name": "fr-FR", "translations": {"model": {
            "name": "Model", "translatedCaption": "Modele",
            "tables": [
                {"name": "Sales", "translatedCaption": "Ventes", "translatedDescription": "Table des ventes",
                 "columns": [
                    {"name": "Amount", "translatedCaption": "Montant", "translatedDisplayFolder": "Dossier"},
                    {"name": "City", "translatedCaption": "Ville"}],
                 "measures": [{"name": "Total", "translatedCaption": "Total FR"}],
                 "hierarchies": [{"name": "Geo", "translatedCaption": "Geo FR",
                    "levels": [{"name": "City", "translatedCaption": "Ville (niveau)"}]}]},
                {"name": "Product", "translatedCaption": "Produit"}],
            "roles": [{"name": "Reader", "translatedCaption": "Lecteur"}],
            "expressions": [{"name": "Source", "translatedDescription": "Requete"}]
        }}})
    );
}

#[test]
fn translation_list_flattens_every_translation() {
    let mut s = translated();
    let v = run(&mut s, "translation.list", json!({}));
    assert_eq!(v["count"], 12);
    assert_eq!(
        v["cultures"],
        json!([{"name": "fr-FR", "model_culture": false, "translations": 12, "linguistic_metadata": false}])
    );
    let list = v["translations"].as_array().unwrap();
    assert!(
        list.contains(&json!({"culture": "fr-FR", "object": {}, "property": "caption", "value": "Modele"}))
    );
    assert!(list.contains(&json!({"culture": "fr-FR",
        "object": {"table": "Sales", "hierarchy": "Geo", "level": "City"},
        "property": "caption", "value": "Ville (niveau)"})));
    assert!(list.contains(
        &json!({"culture": "fr-FR", "object": {"table": "Sales", "column": "Amount"},
        "property": "display_folder", "value": "Dossier"})
    ));
    // Every listed object is accepted back by translation.set.
    for t in list {
        let r = run(
            &mut s,
            "translation.set",
            json!({"culture": t["culture"], "object": t["object"], "property": t["property"], "value": t["value"]}),
        );
        assert_eq!(r["changed"], false, "{t}");
    }
    run(&mut s, "culture.add", json!({"name": "de-DE"}));
    let only = run(&mut s, "translation.list", json!({"culture": "de-de"}));
    assert_eq!(only["count"], 0);
    assert!(fails(&mut s, "translation.list", json!({"culture": "es-ES"})).contains("no culture `es-ES`"));
}

#[test]
fn removing_a_translation_leaves_nothing_empty_behind() {
    let mut s = session();
    set(
        &mut s,
        json!({"table": "Sales", "column": "Amount"}),
        "caption",
        json!("Montant"),
    );
    set(&mut s, json!({"measure": "Total"}), "caption", json!("Total FR"));
    let v = set(
        &mut s,
        json!({"table": "Sales", "column": "Amount"}),
        "caption",
        Value::Null,
    );
    assert_eq!(v["changed"], true);
    assert_eq!(
        fr_tables(&s),
        json!([{"name": "Sales", "measures": [{"name": "Total", "translatedCaption": "Total FR"}]}])
    );
    let v = set(
        &mut s,
        json!({"table": "Sales", "measure": "Total"}),
        "caption",
        json!(""),
    );
    assert_eq!(v["changed"], true);
    assert_eq!(culture(&s, "fr-FR"), json!({"name": "fr-FR"}));
    // Removing what is not there changes nothing.
    let v = set(&mut s, json!({"table": "Sales"}), "caption", Value::Null);
    assert_eq!(v["changed"], false);
    assert_eq!(culture(&s, "fr-FR"), json!({"name": "fr-FR"}));
}

#[test]
fn bad_translation_requests_change_nothing() {
    let mut s = session();
    let set =
        |obj: Value, prop: &str| json!({"culture": "fr-FR", "object": obj, "property": prop, "value": "x"});
    for (params, wanted) in [
        (
            json!({"culture": "es-ES", "object": {}, "property": "caption", "value": "x"}),
            "no culture `es-ES`",
        ),
        (set(json!({"table": "Nope"}), "caption"), "no table named `Nope`"),
        (
            set(json!({"table": "Sales", "column": "Nope"}), "caption"),
            "no column 'Sales'[Nope]",
        ),
        (
            set(
                json!({"table": "Sales", "column": "Amount", "measure": "Total"}),
                "caption",
            ),
            "one of column",
        ),
        (
            set(json!({"level": "City"}), "caption"),
            "needs `object.hierarchy`",
        ),
        (set(json!({"column": "Amount"}), "caption"), "needs `table`"),
        (
            set(json!({"table": "Sales"}), "display_folder"),
            "a table has no display folder",
        ),
        (set(json!({"table": "Sales"}), "title"), "`property` must be"),
        (set(json!({"tabel": "Sales"}), "caption"), "unknown key `tabel`"),
        (
            set(json!({"role": "Reader", "table": "Sales"}), "caption"),
            "cannot be combined",
        ),
        (
            set(json!({"perspective": "None"}), "caption"),
            "no perspective named",
        ),
    ] {
        let e = fails(&mut s, "translation.set", params.clone());
        assert!(e.contains(wanted), "{params}: {e}");
    }
    assert!(
        fails(&mut s, "culture.add", json!({"name": "FR-fr"})).contains("already has the culture `fr-FR`")
    );
    assert!(fails(&mut s, "culture.add", json!({"name": "french"})).contains("not a culture name"));
    assert!(fails(&mut s, "culture.remove", json!({"name": "de-DE"})).contains("no culture"));
}

#[test]
fn renames_keep_translations_attached() {
    let mut s = translated();
    run(
        &mut s,
        "table.rename",
        json!({"name": "Sales", "new_name": "Orders"}),
    );
    run(
        &mut s,
        "column.rename",
        json!({"table": "Orders", "name": "Amount", "new_name": "Value"}),
    );
    run(
        &mut s,
        "measure.rename",
        json!({"name": "Total", "new_name": "Revenue"}),
    );
    let v = run(
        &mut s,
        "hierarchy.set",
        json!({"table": "Orders", "name": "Geo", "new_name": "Place", "rename_levels": {"City": "Town"}}),
    );
    assert!(v["metadata_updated"].as_array().unwrap().len() >= 2, "{v}");
    run(
        &mut s,
        "expression.rename",
        json!({"name": "Source", "new_name": "Origin"}),
    );
    let list = run(&mut s, "translation.list", json!({}));
    assert_eq!(list["count"], 12, "{list}");
    let t = &fr_tables(&s)[0];
    assert_eq!(t["name"], "Orders");
    assert_eq!(t["columns"][0]["name"], "Value");
    assert_eq!(t["measures"][0]["name"], "Revenue");
    assert_eq!(t["hierarchies"][0]["name"], "Place");
    assert_eq!(t["hierarchies"][0]["levels"][0]["name"], "Town");
    assert_eq!(
        culture(&s, "fr-FR")["translations"]["model"]["expressions"][0]["name"],
        "Origin"
    );
    let caption = run(
        &mut s,
        "column.get",
        json!({"table": "Orders", "name": "Value", "culture": "fr-FR"}),
    );
    assert_eq!(caption["caption"], "Montant");
}

#[test]
fn removals_drop_their_translations() {
    let mut s = translated();
    let v = run(
        &mut s,
        "column.remove",
        json!({"table": "Sales", "name": "Amount", "force": true}),
    );
    assert_eq!(
        v["translations_dropped"],
        json!([{"culture": "fr-FR", "objects": ["column of `Sales` `Amount`"]}])
    );
    let v = run(
        &mut s,
        "hierarchy.set",
        json!({"table": "Sales", "name": "Geo", "levels": ["Region"]}),
    );
    assert_eq!(
        v["translations_dropped"][0]["objects"],
        json!(["level of `Sales`[Geo] `City`"])
    );
    run(&mut s, "measure.remove", json!({"name": "Total", "force": true}));
    run(
        &mut s,
        "hierarchy.remove",
        json!({"table": "Sales", "name": "Geo"}),
    );
    run(&mut s, "role.remove", json!({"name": "Reader"}));
    let v = run(&mut s, "table.remove", json!({"name": "Product"}));
    assert_eq!(
        v["translations_dropped"][0]["objects"],
        json!(["table `Product`"])
    );
    assert_eq!(
        fr_tables(&s),
        json!([{"name": "Sales", "translatedCaption": "Ventes", "translatedDescription": "Table des ventes",
                "columns": [{"name": "City", "translatedCaption": "Ville"}]}])
    );
    assert!(
        culture(&s, "fr-FR")["translations"]["model"]
            .get("roles")
            .is_none()
    );
    // An edit that removes nothing reports nothing.
    let v = run(
        &mut s,
        "measure.add",
        json!({"table": "Sales", "name": "N", "expression": "1"}),
    );
    assert!(v.get("translations_dropped").is_none(), "{v}");
}

#[test]
fn culture_commands_undo_and_redo() {
    let mut s = session();
    let before = s.model.clone();
    set(&mut s, json!({"table": "Sales"}), "caption", json!("Ventes"));
    let with = s.model.clone();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.model, with);
    let v = run(&mut s, "culture.remove", json!({"name": "fr-fr"}));
    assert_eq!(v["removed"], "fr-FR");
    assert_eq!(v["translations_removed"], 1);
    assert!(s.model.as_ref().unwrap().extra.get("cultures").is_none());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.model, with);
}

#[test]
fn reads_apply_a_culture() {
    let mut s = translated();
    let m = run(&mut s, "model.get", json!({"detail": "full", "culture": "fr-FR"}));
    assert_eq!(m["translation_culture"], "fr-FR");
    assert_eq!(m["caption"], "Modele");
    let sales = &m["tables"][0];
    assert_eq!(sales["caption"], "Ventes");
    assert_eq!(sales["description"], "Table des ventes");
    assert_eq!(
        sales["translation"],
        json!({"caption": "Ventes", "description": "Table des ventes", "display_folder": null})
    );
    let amount = sales["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Amount")
        .unwrap();
    assert_eq!(amount["caption"], "Montant");
    assert_eq!(amount["display_folder"], "Dossier");
    let region = sales["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Region")
        .unwrap();
    assert_eq!(
        region["caption"], "Region",
        "untranslated objects show their name"
    );
    assert_eq!(sales["hierarchies"][0]["levels"][1]["caption"], "Ville (niveau)");
    assert_eq!(m["roles"][0]["caption"], "Lecteur");
    assert_eq!(m["expressions"][0]["description"], "Requete");

    let brief = run(&mut s, "model.get", json!({"culture": "fr-FR"}));
    assert_eq!(brief["tables"][1]["caption"], "Produit");
    let t = run(&mut s, "table.get", json!({"name": "Sales", "culture": "fr-FR"}));
    assert_eq!(t["measures"][0]["caption"], "Total FR");
    let x = run(
        &mut s,
        "measure.get",
        json!({"name": "Total", "culture": "fr-FR"}),
    );
    assert_eq!(x["caption"], "Total FR");
    assert_eq!(x["name"], "Total");
    // Without `culture` nothing is added; the model's own culture shows the names.
    let plain = run(&mut s, "measure.get", json!({"name": "Total"}));
    assert!(plain.get("caption").is_none());
    let own = run(
        &mut s,
        "measure.get",
        json!({"name": "Total", "culture": "en-US"}),
    );
    assert_eq!(own["caption"], "Total");
    assert!(fails(&mut s, "model.get", json!({"culture": "de-DE"})).contains("no culture `de-DE`"));
}

#[test]
fn the_snapshot_and_render_show_translated_captions() {
    let mut s = translated();
    run(&mut s, "report.new", json!({}));
    let page = s.report.as_ref().unwrap().pages[0].name.clone();
    let v = run(
        &mut s,
        "visual.add",
        json!({"page": page, "visual_type": "tableEx", "x": 0, "y": 0, "width": 300, "height": 200,
               "fields": [{"role": "Values", "field": "Sales[City]"}, {"role": "Values", "field": "[Total]"},
                          {"role": "Values", "field": "Sales[Region]"}]}),
    );
    let fields = |snap: &Value| -> Vec<Value> {
        let visuals = snap["report"]["pages"][0]["visuals"].as_array().unwrap();
        let vis = visuals.iter().find(|x| x["name"] == v["visual"]).unwrap();
        vis["fields"].as_array().unwrap().clone()
    };
    let plain = s.snapshot();
    assert_eq!(plain["display_culture"], Value::Null);
    assert!(fields(&plain).iter().all(|f| f.get("caption").is_none()));

    let r = run(&mut s, "view.set_culture", json!({"culture": "fr-FR"}));
    assert_eq!(r["display_culture"], "fr-FR");
    let snap = s.snapshot();
    assert_eq!(snap["display_culture"], "fr-FR");
    let f = fields(&snap);
    assert_eq!(f[0]["caption"], "Ville");
    assert_eq!(f[0]["name"], "City", "names stay for data lookups");
    assert_eq!(f[1]["caption"], "Total FR");
    assert!(
        f[2].get("caption").is_none(),
        "untranslated: the renderer falls back to the name"
    );
    assert_eq!(f[0]["label"], "Ville");
    assert_eq!(f[2]["label"], "Region");
    let visuals = snap["report"]["pages"][0]["visuals"].as_array().unwrap();
    let vis = visuals.iter().find(|x| x["name"] == v["visual"]).unwrap();
    let title = vis["auto_title"].as_str().unwrap_or_default();
    assert!(title.contains("Total FR") || title.contains("Ville"), "{vis}");
    let sales = &snap["model"]["tables"][0];
    assert_eq!(sales["caption"], "Ventes");
    assert_eq!(sales["measures"][0]["caption"], "Total FR");

    assert!(fails(&mut s, "view.set_culture", json!({"culture": "de-DE"})).contains("no culture"));
    run(&mut s, "view.set_culture", json!({"culture": null}));
    assert_eq!(s.display_culture, None);
    // report.render takes the culture for one call. A well-formed culture without translations
    // (or the model's own) renders the base captions with a warning naming the right options;
    // only a malformed name is refused, with the same pointer.
    let text_of = |r: &Value| r["pages"][0]["texts"].to_string();
    let base = run(
        &mut s,
        "report.render",
        json!({"system_fonts": false, "text_dump": true}),
    );
    assert!(base.get("warnings").is_none(), "{base}");
    let fr = run(
        &mut s,
        "report.render",
        json!({"culture": "fr-FR", "system_fonts": false, "text_dump": true}),
    );
    assert!(fr.get("warnings").is_none(), "{fr}");
    assert!(text_of(&fr).contains("Ville"), "{fr}");
    let de = run(
        &mut s,
        "report.render",
        json!({"culture": "de-DE", "system_fonts": false, "text_dump": true}),
    );
    let w = de["warnings"][0].as_str().unwrap_or_default();
    assert!(
        w.contains("`de-DE` is not a translation culture")
            && w.contains("fr-FR")
            && w.contains("format_culture")
            && w.contains("ui_language"),
        "{de}"
    );
    assert_eq!(text_of(&de), text_of(&base), "base captions");
    assert_eq!(s.display_culture, None);
    s.model.as_mut().unwrap().culture = Some("en-US".into());
    // the session's display culture is not used while the requested one has no translations
    run(&mut s, "view.set_culture", json!({"culture": "fr-FR"}));
    let en = run(
        &mut s,
        "report.render",
        json!({"culture": "en-us", "system_fonts": false, "text_dump": true}),
    );
    let w = en["warnings"][0].as_str().unwrap_or_default();
    assert!(
        w.contains("model's own culture") && w.contains("format_culture"),
        "{en}"
    );
    assert_eq!(text_of(&en), text_of(&base));
    assert_eq!(s.display_culture.as_deref(), Some("fr-FR"), "put back");
    run(&mut s, "view.set_culture", json!({"culture": null}));
    for bad in ["not a culture", "x", "en-US!", "123"] {
        let e = s.execute("report.render", &json!({"culture": bad})).unwrap_err();
        assert!(
            e.contains("is not a culture name") && e.contains("format_culture"),
            "{bad}: {e}"
        );
    }
}

/// TMDL (cultures/<name>.tmdl) and TMSL (model.bim) keep translations exactly.
#[test]
fn projects_and_bim_round_trip_translations() {
    let mut s = translated();
    let want = run(&mut s, "translation.list", json!({}));
    let tmp = TempDir::new("engine-translations");
    for (format, path) in [
        ("pbip-tmdl", tmp.path().join("Tmdl.pbip")),
        ("pbip-tmsl", tmp.path().join("Tmsl.pbip")),
        ("bim", tmp.path().join("model.bim")),
    ] {
        run(&mut s, "file.save_as", json!({"path": path, "format": format}));
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(run(&mut back, "translation.list", json!({})), want, "{format}");
        assert_eq!(
            culture(&back, "fr-FR"),
            culture(&s, "fr-FR"),
            "{format}: the culture comes back as written"
        );
    }
    assert!(
        tmp.path()
            .join("Tmdl.SemanticModel/definition/cultures/fr-FR.tmdl")
            .is_file()
    );
}

/// A PBIX (engine-written metadata fixture with a translated fr-FR culture): new, changed and
/// removed translations and a new culture are written to metadata.sqlitedb and read back.
#[test]
fn pbix_writes_translations_into_the_metadata() {
    let tmp = TempDir::new("engine-translations-pbix");
    let path = tmp.path().join("Objects.pbix");
    let sqlitedb = include_bytes!("../../../metadata/tests/engine/objects.sqlitedb");
    let pkg = bicraft_pbix::synthetic::pbix(sqlitedb).unwrap();
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": path}));
    let before = run(&mut s, "translation.list", json!({"culture": "fr-FR"}));
    assert!(before["count"].as_u64().unwrap() >= 5, "{before}");
    set(&mut s, json!({"table": "Sales"}), "caption", json!("Ventes 2"));
    set(
        &mut s,
        json!({"table": "Sales", "column": "City"}),
        "caption",
        json!("Ville"),
    );
    set(
        &mut s,
        json!({"table": "Sales", "measure": "Total"}),
        "caption",
        Value::Null,
    );
    run(&mut s, "culture.add", json!({"name": "de-DE"}));
    run(
        &mut s,
        "translation.set",
        json!({"culture": "de-DE", "object": {"table": "Sales", "column": "Amount"}, "property": "caption", "value": "Betrag"}),
    );
    let want = run(&mut s, "translation.list", json!({}));
    run(&mut s, "file.save", json!({"backup": false}));
    let mut back = Session::new();
    run(&mut back, "file.open", json!({"path": path}));
    assert_eq!(back.model_error, None);
    assert_eq!(run(&mut back, "translation.list", json!({})), want);
    let m = run(
        &mut back,
        "measure.get",
        json!({"name": "Total", "culture": "fr-FR"}),
    );
    assert_eq!(m["translation"]["caption"], Value::Null);
}
