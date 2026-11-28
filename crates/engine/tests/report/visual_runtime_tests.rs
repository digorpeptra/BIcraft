//! BIcraft's visual runtime (ROADMAP M12.2, docs/community-visuals.md "Runtime"): the
//! capabilities of the Spec visual's package against the MIT `powerbi-visuals-api` 5.3.0
//! capabilities schema (vendored under `powerbi-visuals-api/`), and the runtime's TypeScript,
//! compiled by BIcraft's bundler and run with Node when Node is on the PATH (a development check:
//! it prints a note and passes without Node): formatting, keys, selection rules, tooltips, landing
//! text, high contrast, the format pane model, and the interactions wired to a stand-in host.

use std::path::Path;

use bicraft_engine::spec_visual::{SPEC_GUID, package_bytes};
use bicraft_engine::visual_runtime::helpers_script;
use serde_json::{Value, json};

use crate::tempdir::TempDir;

fn package_capabilities() -> Value {
    let read = bicraft_pbix::pbiviz::read_pbiviz(&package_bytes().unwrap()).unwrap();
    let name = format!("resources/{SPEC_GUID}.pbiviz.json");
    let (_, body) = read.files.iter().find(|(n, _)| *n == name).unwrap_or_else(|| {
        panic!(
            "no {name} in {:?}",
            read.files.iter().map(|f| &f.0).collect::<Vec<_>>()
        )
    });
    let v: Value = serde_json::from_slice(body).unwrap();
    v["capabilities"].clone()
}

#[test]
fn the_spec_visual_capabilities_follow_the_api_schema() {
    let caps = package_capabilities();
    let schema_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/report/powerbi-visuals-api/schema.capabilities.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    let issues = bicraft_pbir::JsonSchema::new(schema.clone()).validate(&caps);
    assert!(issues.is_empty(), "{issues:?}\n{caps}");
    // what the runtime needs, and nothing that reaches outside the sandbox
    assert_eq!(caps["privileges"], json!([]));
    assert_eq!(caps["supportsLandingPage"], json!(true));
    assert_eq!(caps["supportsEmptyDataView"], json!(true));
    assert_eq!(caps["supportsKeyboardFocus"], json!(true));
    assert_eq!(
        caps["tooltips"]["supportedTypes"],
        json!({"default": true, "canvas": true})
    );
    // another visual's selection highlights it (G114)
    assert_eq!(caps["supportsHighlight"], json!(true), "{caps}");
    // the validator does refuse a wrong capabilities document
    let bad = json!({"privileges": [], "supportsKeyboardFocus": "yes", "colour": 1});
    assert!(!bicraft_pbir::JsonSchema::new(schema).validate(&bad).is_empty());
}

/// The runtime's checks under Node: each case's result, compared below with what it must be.
const HARNESS: &str = r##"
var out = {};
function fakeId(k) { return { k: k, equals: function (o) { return !!o && o.k === k; } }; }
var dv = {
  metadata: { columns: [{ roles: { category: true } }, { roles: { values: true } }], objects: { spec: { json: "x" }, c: { fill: { solid: { color: "#112233" } } } } },
  categorical: {
    categories: [{ source: { displayName: "K", roles: { category: true } }, values: ["a", "b", "c"] }],
    values: [{ source: { displayName: "Total", format: "#,0.0", roles: { values: true } }, values: [1000, null, 2.25] }]
  }
};
out.numbers = [
  R.formatNumber(1234567.891, "#,0"), R.formatNumber(1234.5, "#,0.00"), R.formatNumber(0.1234, "0.0%"),
  R.formatNumber(-1234, "$#,0;($#,0)"), R.formatNumber(0, "$#,0;($#,0);\"zero\""), R.formatNumber(1.005, "0.00"),
  R.formatNumber(-0.001, "0.00"), R.formatNumber(1234567, "#,0,\"K\""), R.formatNumber(12345.678, "0.00E+00"),
  R.formatNumber(1234.5, "#,0.00", "fr-FR"), R.formatNumber(1234.5, ""), R.formatNumber(1 / 3, ""),
  R.formatNumber(0.5, "Percent"), R.formatNumber(5, "00"), R.formatNumber(-7, "0"), R.formatNumber(2.5, "\\#0"),
  R.formatNumber(1234567, "#,##0.0,\"K\""), R.formatNumber(-7433915, "$#,0.00,,\"M\";($#,0.00,,\"M\")"),
  R.formatNumber(1234567, "#,0.0\",\"")
];
out.values = [
  R.formatValue(null), R.formatValue(undefined, "", "fr-FR"), R.formatValue(true, "", "fr"), R.formatValue("x", "0.00"),
  R.formatValue(new Date(2024, 0, 31), "dd/MM/yyyy"), R.formatValue(new Date(2024, 0, 31), "Long Date"),
  R.formatValue(new Date(2024, 0, 31), "Long Date", "fr-FR"), R.formatValue(new Date(2024, 0, 31), ""),
  R.formatValue(new Date(2024, 0, 31, 13, 5, 9), "yyyy-MM-dd HH:mm"), R.formatValue(new Date(2024, 0, 31, 13, 5, 9), "h:mm AM/PM"),
  R.formatValue(new Date(2024, 0, 31), "MMM d"), R.formatValue(new Date(NaN), "yyyy")
];
out.round = [R.roundDigits(2.5, 0), R.roundDigits(0.125, 2), R.roundDigits(1e20, 2)[0]];
out.sections = R.sections("0;\"a;b\";\\;x");
out.keys = [
  R.keyAction("Enter", false, false), R.keyAction(" ", true, false), R.keyAction("Escape", false, false),
  R.keyAction("ArrowRight", false, false), R.keyAction("Up", false, false), R.keyAction("Tab", false, false),
  R.keyAction("F10", false, true), R.keyAction("Home", false, false)
];
out.focus = [R.moveFocus(0, "previous", 3), R.moveFocus(1, "next", 3), R.moveFocus(2, "next", 3), R.moveFocus(-1, "next", 3), R.moveFocus(0, "last", 3), R.moveFocus(0, "next", 0)];
out.plans = [R.clickPlan([2], 2, false), R.clickPlan([2], 2, true), R.clickPlan([], 1, false), R.clickPlan([1, 2], 2, false)];
out.rows = R.rowsOf([fakeId(2), fakeId(0), fakeId(9)], [fakeId(0), fakeId(1), fakeId(2)]);
out.groupRows = R.rowsOf([fakeId(1), fakeId(2)], [[fakeId(1), fakeId(2)], [fakeId(1), fakeId(3)], [], fakeId(2), null]);
out.dim = [R.dimmed([1], 0), R.dimmed([1], 1), R.dimmed([], 0)];
function ks(x) { return Array.isArray(x) ? x.map(function (i) { return i.k; }) : x.k; }
out.adds = [ks(R.multiIds([fakeId(1)], [fakeId(1), fakeId(2)])), ks(R.multiIds([], fakeId(3))), ks(R.multiIds([fakeId(1), fakeId(2)], [fakeId(1), fakeId(2)])), ks(R.multiIds([fakeId(1)], fakeId(1)))];
var built = [];
var idHost = { createSelectionIdBuilder: function () { var b = { withCategory: function (c, i) { b.i = i; b.c = c; return b; }, createSelectionId: function () { built.push(b.i); return fakeId(b.i); } }; return b; } };
out.ids = [R.categoryIds(idHost, {}, 3).length, built, R.categoryIds(idHost, null, 3).length, R.categoryIds(undefined, {}, 3).length];
var table = R.tableOf(dv);
out.table = [table.rows, table.category.name, table.values.length, table.values[0].format];
out.tips = [R.tooltipItems(table, 0, "en"), R.tooltipItems(table, 1, "fr-FR"), R.tooltipItems(table, 3, "en"), R.tooltipText(R.tooltipItems(table, 2, "en"))];
out.roles = [R.boundRoles(dv), R.missingRoles(dv, ["category", "values", "x"]), R.boundRoles(undefined)];
out.objects = [R.objectValue(dv, "spec", "json", ""), R.objectValue(dv, "spec", "nope", "d"), R.objectValue(dv, "c", "fill", ""), R.objectValue(dv, "spec", "json", 3)];
var hc = R.contrastOf({ colorPalette: { isHighContrast: true, foreground: { value: "#FFFF00" }, background: { value: "#000000" }, foregroundSelected: { value: "#00FFFF" } } });
out.contrast = [hc, R.contrastOf({ colorPalette: { isHighContrast: false } }), R.contrastOf(undefined),
  R.contrastPrim({ t: "rect", fill: "#1F6F8B", opacity: 0.5 }, hc), R.contrastPrim({ t: "poly", fill: "#1F6F8B", opacity: 0.35 }, hc), R.contrastPrim({ t: "text", fill: "#555555" }, hc),
  R.contrastPrim({ t: "rect", fill: "#1F6F8B", opacity: 1, rx: 4, grad: "#4A8BA2", stroke: "#123456", sw: 1, shadow: 6 }, hc)];
out.model = [
  R.model([R.card("c", "C", [R.group("g", "G", [R.toggle("o", "show", "Show", true), R.number("o", "n", "N", 3, 0, 10)])])]),
  R.card("s", "S", [R.group("g", "G", [R.textArea("spec", "json", "Spec (JSON)", "{}")])], false).revertToDefaultDescriptors,
  R.color("o", "fill", "Fill", "#112233").control
];
out.strings = [R.t("fr-FR", "blank"), R.t("de-DE", "blank"), R.t("fr", "points", "X", 3), R.t("en", "nokey"),
  R.landingText("en", ["Category", "Values"]), R.landingText("fr-FR", ["Catégorie", "Valeurs"]),
  R.joinNames("en", ["A", "B", "C"]), R.langOf("FR_ca"), R.pick("fr", { en: "e", fr: "f" })];

// the interactions, wired to a stand-in host and stand-in elements
function el(row) {
  var e = { attrs: {}, setAttribute: function (k, v) { e.attrs[k] = String(v); }, getAttribute: function (k) { return k in e.attrs ? e.attrs[k] : null; },
    removeAttribute: function (k) { delete e.attrs[k]; }, closest: function () { return "data-row" in e.attrs ? e : null; },
    getBoundingClientRect: function () { return { left: 10 * row, top: 5, width: 4, height: 2 }; }, focus: function () { focused = row; } };
  if (row === 0) { e.attrs["fill-opacity"] = "0.8"; }
  return e;
}
var focused = -1;
var log = [];
var sel = [];
var onSelect = null;
var manager = {
  select: function (id, multi) { if (multi) { var i = sel.findIndex(function (s) { return s.equals(id); }); if (i >= 0) { sel.splice(i, 1); } else { sel.push(id); } } else { sel = [id]; } return Promise.resolve(sel.slice()); },
  clear: function () { sel = []; log.push("clear"); return Promise.resolve({}); },
  getSelectionIds: function () { return sel.slice(); },
  registerOnSelectCallback: function (cb) { onSelect = cb; },
  showContextMenu: function (id, pos) { log.push(["menu", id.k === undefined ? null : id.k, pos.x, pos.y]); return Promise.resolve({}); }
};
var host = { locale: "en-US", createSelectionManager: function () { return manager; },
  tooltipService: { enabled: function () { return true; }, show: function (o) { log.push(["show", o.coordinates, o.dataItems.length, o.identities[0].k]); },
    move: function (o) { log.push(["move", o.coordinates]); }, hide: function (o) { log.push(["hide", o.immediately]); } } };
var listeners = {};
var root = { addEventListener: function (type, fn) { listeners[type] = fn; } };
var it = new R.Interactions(host, root);
var container = el(-1);
var els = [el(0), el(1), el(2)];
var hitEl = el(1);
it.wire({ container: container, targets: [{ row: 0, el: els[0] }, { row: 1, el: els[1] }, { row: 2, el: els[2] }, { row: 1, el: hitEl, hit: true }],
  ids: [fakeId(0), fakeId(1), fakeId(2)], tooltip: function (r) { return R.tooltipItems(table, r, "en"); }, label: function (r) { return "row " + r; }, contrast: null });
function opac() { return els.map(function (e) { return e.attrs.opacity || ""; }); }
function ev(target, extra) { var e = { target: target, clientX: 7, clientY: 9, ctrlKey: false, metaKey: false, shiftKey: false, prevented: false, preventDefault: function () { e.prevented = true; }, stopPropagation: function () {} }; for (var k in extra) { e[k] = extra[k]; } return e; }
function tick() { return new Promise(function (r) { setTimeout(r, 0); }); }
async function interactions() {
  var steps = {};
  steps.wired = [container.attrs.role, container.attrs["aria-multiselectable"], els.map(function (e) { return e.attrs.tabindex; }), hitEl.attrs.tabindex === undefined, els[1].attrs["aria-label"], els[0].attrs["data-fo"]];
  listeners.click(ev(els[1])); await tick();
  steps.click = [opac(), els.map(function (e) { return e.attrs["aria-selected"]; }), hitEl.attrs.stroke || ""];
  listeners.keydown(ev(els[2], { key: "Enter", ctrlKey: true })); await tick();
  steps.ctrlEnter = [opac(), it.selection.selected];
  listeners.click(ev(els[2], { ctrlKey: true })); await tick();
  steps.ctrlClickAgain = [opac(), it.selection.selected];
  listeners.click(ev(els[1])); await tick();
  steps.clickOnlySelected = [opac(), log[log.length - 1]];
  onSelect([fakeId(0)]);
  steps.bookmark = opac();
  listeners.keydown(ev(els[0], { key: "Escape" }));
  steps.escape = opac();
  var e = ev(els[0], { key: "ArrowRight" }); listeners.keydown(e);
  listeners.keydown(ev(els[1], { key: "End" }));
  steps.focus = [focused, e.prevented];
  var classes = function () { return [container.attrs["class"] || "", els.map(function (e) { return e.attrs["class"] || ""; })]; };
  listeners.mousemove(ev(els[0])); steps.hover = classes(); listeners.mousemove(ev(els[0])); listeners.mouseleave(ev(null));
  steps.left = classes();
  listeners.mousemove(ev({ closest: function () { return null; } }));
  var m = ev(els[2]); listeners.contextmenu(m);
  var bg = ev({ closest: function () { return null; } }); listeners.contextmenu(bg);
  steps.log = log.slice(1).concat([m.prevented]);
  listeners.click(ev(els[0])); await tick();
  listeners.click(ev({ closest: function () { return null; } })); await tick();
  steps.background = [opac(), it.selection.selected];
  // the host redraws after a selection (Power BI Desktop 2.159.1231.0): the focused point is
  // replaced, and the new point of its row takes the focus back while the frame has it
  var doc = { body: {}, hasFocus: function () { return true; } };
  doc.activeElement = doc.body;
  root.ownerDocument = doc;
  function rewire(list) {
    it.wire({ container: el(-1), targets: list.map(function (x, i) { return { row: i, el: x }; }), ids: [fakeId(0), fakeId(1), fakeId(2)],
      tooltip: function () { return []; }, label: function (r) { return "row " + r; }, contrast: null });
  }
  listeners.focusin(ev(els[1]));
  els[1].isConnected = false; focused = -1;
  var els2 = [el(0), el(1), el(2)];
  rewire(els2);
  var back = focused;
  // the user left the visual (the frame has no focus): nothing is focused
  listeners.focusin(ev(els2[2]));
  els2[2].isConnected = false; focused = -1;
  doc.hasFocus = function () { return false; };
  rewire([el(0), el(1), el(2)]);
  steps.refocus = [back, focused];
  // a host without selection or tooltips: nothing breaks, nothing is selectable
  var bare = new R.Interactions({}, root);
  bare.wire({ container: el(-1), targets: [{ row: 0, el: el(0) }], ids: [], tooltip: function () { return []; }, label: function () { return ""; }, contrast: null });
  listeners.click(ev(el(0))); listeners.mousemove(ev(el(0)));
  steps.bare = [bare.selection.enabled(), bare.selection.menu(0, 1, 1)];
  return steps;
}
interactions().then(function (s) { out.interactions = s; require("fs").writeFileSync(process.argv[2], JSON.stringify(out)); },
  function (e) { console.error(e && e.stack ? e.stack : e); process.exit(1); });
"##;

#[test]
fn the_runtime_behaves_under_node() {
    let node = std::process::Command::new("node").arg("--version").output();
    if !node.is_ok_and(|o| o.status.success()) {
        eprintln!(
            "note: node is not on the PATH; the visual runtime's TypeScript is not run (development check)"
        );
        return;
    }
    let dir = TempDir::new("visual-runtime");
    let script = dir.join("runtime.js");
    std::fs::write(
        &script,
        format!("var R = {}\n{HARNESS}", helpers_script().unwrap()),
    )
    .unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&script)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let got: Value = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    let nb = "\u{a0}";
    let want: Vec<(&str, Value)> = vec![
        (
            "numbers",
            json!([
                "1,234,568",
                "1,234.50",
                "12.3%",
                "($1,234)",
                "zero",
                "1.01",
                "0.00",
                "1,235K",
                "1.23E+04",
                format!("1{nb}234,50"),
                "1234.5",
                "0.333333333333333",
                "50.00%",
                "05",
                "-7",
                "#3",
                // commas right after the last decimal scale too; a quoted comma does not
                "1,234.6K",
                "($7.43M)",
                "1,234,567.0,"
            ]),
        ),
        (
            "values",
            json!([
                "(Blank)",
                "(Vide)",
                "Vrai",
                "x",
                "31/01/2024",
                "Wednesday, January 31, 2024",
                "mercredi 31 janvier 2024",
                "2024-01-31",
                "2024-01-31 13:05",
                "1:05 PM",
                "Jan 31",
                ""
            ]),
        ),
        ("round", json!([["3", ""], ["0", "13"], "100000000000000000000"])),
        ("sections", json!(["0", "\"a;b\"", "\\;x"])),
        (
            "keys",
            json!([
                "select",
                "multiSelect",
                "clear",
                "next",
                "previous",
                null,
                "menu",
                "first"
            ]),
        ),
        ("focus", json!([0, 2, 2, 1, 2, -1])),
        ("plans", json!(["clear", "select", "select", "select"])),
        ("rows", json!([0, 2])),
        // a row of several ids (a histogram bin) is selected when all of them are
        ("groupRows", json!([0, 3])),
        ("dim", json!([true, false, false])),
        // Ctrl+click on a row of several ids adds the ones not yet selected (the host toggles each id:
        // a Sankey node sharing a link with a selected node lost that link, Desktop 2.159, 2026-11-24);
        // on a selected row it passes them all, which removes it
        ("adds", json!([[2], 3, [1, 2], 1])),
        ("ids", json!([3, [0, 1, 2], 0, 0])),
        ("table", json!([3, "K", 1, "#,0.0"])),
        (
            "tips",
            json!([
                [{"displayName": "K", "value": "a"}, {"displayName": "Total", "value": "1,000.0"}],
                [{"displayName": "K", "value": "b"}, {"displayName": "Total", "value": "(Vide)"}],
                [],
                "K: c, Total: 2.3"
            ]),
        ),
        ("roles", json!([["category", "values"], ["x"], []])),
        ("objects", json!(["x", "d", "#112233", 3])),
        (
            "contrast",
            json!([
                {"foreground": "#FFFF00", "background": "#000000", "selected": "#00FFFF"}, null, null,
                {"t": "rect", "fill": "#FFFF00", "opacity": 1},
                {"t": "poly", "fill": "#000000", "opacity": 1, "stroke": "#FFFF00"},
                {"t": "text", "fill": "#FFFF00"},
                {"t": "rect", "fill": "#FFFF00", "opacity": 1, "rx": 4, "stroke": "#000000", "sw": 1}
            ]),
        ),
        (
            "model",
            json!([
                {"cards": [{"uid": "c", "displayName": "C", "groups": [{"uid": "g", "displayName": "G", "slices": [
                    {"uid": "o_show", "displayName": "Show", "control": {"type": "ToggleSwitch", "properties": {"descriptor": {"objectName": "o", "propertyName": "show"}, "value": true}}},
                    {"uid": "o_n", "displayName": "N", "control": {"type": "NumUpDown", "properties": {"descriptor": {"objectName": "o", "propertyName": "n"}, "value": 3,
                        "options": {"minValue": {"type": 0, "value": 0}, "maxValue": {"type": 1, "value": 10}}}}}
                ]}], "revertToDefaultDescriptors": [{"objectName": "o", "propertyName": "show"}, {"objectName": "o", "propertyName": "n"}]}]},
                [],
                {"type": "ColorPicker", "properties": {"descriptor": {"objectName": "o", "propertyName": "fill"}, "value": {"value": "#112233"}}}
            ]),
        ),
        (
            "strings",
            json!([
                "(Vide)",
                "(Blank)",
                "X : 3 points de données",
                "nokey",
                "Add fields to Category and Values to draw this visual.",
                "Ajoutez des champs dans Catégorie et Valeurs pour dessiner ce visuel.",
                "A, B and C",
                "fr",
                "f"
            ]),
        ),
        (
            "interactions",
            json!({
                "wired": ["listbox", "true", ["0", "0", "0"], true, "row 1", "0.8"],
                "click": [["0.4", "", "0.4"], ["false", "true", "false"], "#252423"],
                "ctrlEnter": [["0.4", "", ""], [1, 2]],
                "ctrlClickAgain": [["0.4", "", "0.4"], [1]],
                "clickOnlySelected": [["", "", ""], "clear"],
                "bookmark": ["", "0.4", "0.4"],
                "escape": ["", "", ""],
                "focus": [2, true],
                "hover": ["bicraft-hovering", ["bicraft-point bicraft-hot", "bicraft-point", "bicraft-point"]],
                "left": ["", ["bicraft-point", "bicraft-point", "bicraft-point"]],
                "log": [
                    "clear",
                    ["show", [7, 9], 2, 0],
                    ["move", [7, 9]],
                    ["hide", false],
                    ["menu", 2, 7, 9],
                    ["menu", null, 7, 9],
                    true
                ],
                "background": [["", "", ""], []],
                "refocus": [1, -1],
                "bare": [false, false]
            }),
        ),
    ];
    let mut errors = Vec::new();
    for (k, v) in &want {
        if &got[*k] != v {
            errors.push(format!("{k}:\n  got:  {}\n  want: {v}", got[*k]));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// Milliseconds since 1970-01-01 of a date and time (proleptic Gregorian).
fn ms(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64) -> f64 {
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000) as f64
}

/// The date labels of BIcraft's visuals (G105, G109, G110): the runtime's TypeScript
/// (`dateLabel`) writes every date of a corpus with every format string as the renderer's Rust
/// does (`bicraft_render::date_label`: the DAX `FORMAT` rules and the visuals' named formats), in
/// English and French; a few are checked against what they must read.
#[test]
fn date_labels_match_the_renderer() {
    let label = |d: f64, f: &str, loc: &str| bicraft_render::date_label(d, f, loc);
    let jan31 = ms(2024, 1, 31, 0, 0, 0);
    let pm = ms(2024, 3, 5, 14, 7, 9);
    assert_eq!(label(jan31, "dd/mm/yyyy", "fr-FR").as_deref(), Some("31/01/2024"));
    assert_eq!(label(jan31, "mmm yyyy", "fr-FR").as_deref(), Some("janv 2024"));
    assert_eq!(
        label(jan31, "Long Date", "en-US").as_deref(),
        Some("Wednesday, January 31, 2024")
    );
    assert_eq!(
        label(jan31, "Long Date", "fr").as_deref(),
        Some("mercredi 31 janvier 2024")
    );
    assert_eq!(
        label(jan31, "General Date", "fr-FR").as_deref(),
        Some("31/01/2024 00:00:00")
    );
    assert_eq!(label(pm, "h:mm AM/PM", "en-US").as_deref(), Some("2:07 PM"));
    assert_eq!(label(pm, "hh:nn", "en-US").as_deref(), Some("14:07"));
    assert_eq!(label(jan31, "0.00", "en-US"), None);
    assert_eq!(label(jan31, "", "en-US"), None);
    assert_eq!(label(ms(50, 1, 1, 0, 0, 0), "yyyy", "en-US"), None);
    let dates = [
        jan31,
        pm,
        ms(2024, 11, 25, 9, 5, 0),
        ms(2024, 12, 30, 0, 0, 0),
        ms(2021, 1, 3, 23, 59, 59),
        ms(1899, 12, 30, 13, 5, 0),
        ms(1999, 7, 1, 12, 0, 0),
        ms(50, 1, 1, 0, 0, 0),
    ];
    let formats = [
        "dd/mm/yyyy",
        "mm/dd/yyyy",
        "m/d/yyyy",
        "yyyy-mm-dd",
        "mmm yyyy",
        "mmmm yyyy",
        "mmm yy",
        "dddd d mmmm yyyy",
        "ddd dd mmm",
        "d mmm",
        "dd/MM/yyyy HH:mm",
        "MMM d",
        "yyyy-MM-dd HH:mm:ss",
        "d",
        "D",
        "G",
        "T",
        "t",
        " Short Date ",
        "Long Date",
        "General Date",
        "Medium Date",
        "Long Time",
        "Medium Time",
        "Short Time",
        "hh:nn:ss",
        "h:mm AM/PM",
        "h:mm am/pm",
        "hh:mm a/p",
        "hh:mm ampm",
        "h:mm tt",
        "yyyy \"Q\"q",
        "\"week \"ww",
        "w",
        "y",
        "yyy",
        "c",
        "dddddd",
        "ddddd ttttt",
        "mm:ss",
        "\\d\\a\\y d",
        "0.00",
        "",
        "#,0",
        "\"open",
        "General Number",
        "%d",
    ];
    let mut cases = Vec::new();
    let mut rust = Vec::new();
    for loc in ["en-US", "fr-FR"] {
        for f in formats {
            for d in dates {
                cases.push(json!([d, f, loc]));
                rust.push(json!(label(d, f, loc)));
            }
        }
    }
    let node = std::process::Command::new("node").arg("--version").output();
    if !node.is_ok_and(|o| o.status.success()) {
        eprintln!(
            "note: node is not on the PATH; the TypeScript date labels are not compared (development check)"
        );
        return;
    }
    let dir = TempDir::new("visual-runtime-dates");
    let script = dir.join("dates.js");
    std::fs::write(
        &script,
        format!(
            "var R = {}\nvar fs = require('fs');\nvar c = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
             fs.writeFileSync(process.argv[3], JSON.stringify(c.map(function (x) {{ return R.dateLabel(x[0], x[1], x[2]); }})));\n",
            helpers_script().unwrap()
        ),
    )
    .unwrap();
    let cases_path = dir.join("cases.json");
    std::fs::write(&cases_path, Value::Array(cases.clone()).to_string()).unwrap();
    let out_path = dir.join("out.json");
    let o = std::process::Command::new("node")
        .arg(&script)
        .arg(&cases_path)
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let ts: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&out_path).unwrap()).unwrap();
    assert_eq!(ts.len(), rust.len());
    let errors: Vec<String> = cases
        .iter()
        .zip(ts.iter().zip(&rust))
        .filter(|(_, (t, r))| t != r)
        .map(|(c, (t, r))| format!("{c}: ts {t} != rust {r}"))
        .collect();
    assert!(
        errors.is_empty(),
        "{} differ:\n{}",
        errors.len(),
        errors.join("\n")
    );
}
