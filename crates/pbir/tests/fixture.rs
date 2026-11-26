//! Round trips, edits, `write_dir` and hostile input on the synthetic fixture
//! `tests/fixtures/sales.Report` (our own content).

use std::path::{Path, PathBuf};

use bicraft_pbir::{read_dir, read_documents, validate_documents, write_dir, write_documents};
use bicraft_report::{FieldBinding, FieldRef, NamedDocument, Page, Position, Report, Resource, Visual};
use serde_json::{Value, json};

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

type Docs = Vec<(String, Vec<u8>)>;

const PAGE_A: &str = "a1b2c3d4e5f6a7b8c9d0";
const PAGE_B: &str = "f0e1d2c3b4a596877869";

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sales.Report")
}

fn walk(root: &Path, dir: &Path, out: &mut Docs) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, std::fs::read(&path).unwrap()));
        }
    }
}

/// The fixture's report documents (without `definition.pbir`), sorted.
fn fixture_docs() -> Docs {
    let root = fixture_dir();
    let mut docs = Vec::new();
    for top in ["definition", "StaticResources"] {
        walk(&root, &root.join(top), &mut docs);
    }
    docs.sort();
    docs
}

fn sorted(mut docs: Docs) -> Docs {
    docs.sort();
    docs
}

fn assert_valid(docs: &Docs) {
    let issues = validate_documents(docs);
    assert!(issues.is_empty(), "{issues:#?}");
}

/// A guarded temp folder and a path inside it that does not exist yet.
fn tmp_dir(tag: &str) -> (TempDir, PathBuf) {
    let tmp = TempDir::new(&format!("pbir-{tag}"));
    let dir = tmp.join("out");
    (tmp, dir)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

fn col(table: &str, column: &str) -> FieldRef {
    FieldRef::Column {
        table: table.into(),
        column: column.into(),
    }
}

fn measure(table: &str, m: &str) -> FieldRef {
    FieldRef::Measure {
        table: table.into(),
        measure: m.into(),
    }
}

fn binding(role: &str, field: FieldRef) -> FieldBinding {
    FieldBinding {
        display_name: None,
        show_as: None,
        role: role.into(),
        field,
    }
}

#[test]
fn fixture_is_valid() {
    assert_valid(&fixture_docs());
}

#[test]
fn read_fills_the_typed_model() {
    let report = read_dir(&fixture_dir()).unwrap();
    assert_eq!(
        report.pages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        [PAGE_A, PAGE_B]
    );
    assert_eq!(report.active_page.as_deref(), Some(PAGE_A));
    assert!(report.theme_collection.is_some() && report.filter_config.is_some());
    assert!(report.extra.contains_key("resourcePackages"));
    assert_eq!(report.version_extra.get("version"), Some(&json!("2.0.0")));
    for kind in [
        "version",
        "report",
        "pages",
        "page",
        "visual",
        "bookmark",
        "bookmarks",
    ] {
        assert!(report.schemas.contains_key(kind), "{kind}");
    }

    let a = report.page("Overview").unwrap();
    assert_eq!((a.width, a.height), (1280.0, 720.0));
    assert_eq!(a.display_option.as_deref(), Some("FitToPage"));
    assert!(a.filter_config.is_some());
    assert!(
        !a.extra.contains_key("$schema"),
        "a common $schema is kept in report.schemas only"
    );
    let chart = a.visual("c0ffee00000000000002").unwrap();
    assert_eq!(chart.visual_type.as_deref(), Some("clusteredColumnChart"));
    assert_eq!(chart.position.width, 480.5);
    assert_eq!(chart.position.tab_order, Some(1000));
    assert!(chart.filter_config.is_some());
    assert!(chart.visual_extra.contains_key("query"), "the raw query is kept");
    assert_eq!(
        chart.fields,
        [
            binding("Category", col("Date", "Year")),
            binding(
                "Y",
                FieldRef::Aggregation {
                    table: "Sales".into(),
                    column: "Amount".into(),
                    function: "Sum".into()
                }
            )
        ]
    );
    let slicer = a.visual("c0ffee00000000000004").unwrap();
    assert_eq!(slicer.parent_group.as_deref(), Some("c0ffee00000000000005"));
    let group = a.visual("c0ffee00000000000005").unwrap();
    assert!(group.visual_type.is_none() && group.extra.contains_key("visualGroup"));

    let b = report.page("details").unwrap();
    assert_eq!(b.visibility.as_deref(), Some("HiddenInViewMode"));
    let line = b.visual("0123456789abcdef0001").unwrap();
    assert!(matches!(line.fields[0].field, FieldRef::Other { .. }));
    assert_eq!(line.fields[1].field, measure("Sales", "Total Sales"));
    assert!(b.visual("0123456789abcdef0002").unwrap().is_hidden);

    let names: Vec<&str> = report.bookmarks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Bookmark0000000000001", "Bookmark0000000000002"]);
    let paths: Vec<&str> = report.resources.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "StaticResources/RegisteredResources/bicraft-logo.png",
            "StaticResources/RegisteredResources/bicraft-test-theme.json",
            "definition/bookmarks/bookmarks.json",
            &format!("definition/pages/{PAGE_A}/visuals/c0ffee00000000000001/mobile.json"),
            "definition/reportExtensions.json",
        ]
    );
}

#[test]
fn read_write_is_byte_identical() {
    let docs = fixture_docs();
    let report = read_documents(&docs).unwrap();
    let written = sorted(write_documents(&report));
    assert_eq!(written.len(), docs.len());
    for ((p1, b1), (p2, b2)) in docs.iter().zip(&written) {
        assert_eq!(p1, p2);
        assert!(b1 == b2, "{p1} differs:\n{}", String::from_utf8_lossy(b2));
    }
    // Stable: a second pass gives the same model and bytes.
    let again = read_documents(&written).unwrap();
    assert_eq!(again, report);
    assert_eq!(sorted(write_documents(&again)), written);
}

#[test]
fn backslash_paths_and_ignored_files() {
    let mut docs: Docs = fixture_docs()
        .into_iter()
        .map(|(p, b)| (p.replace('/', "\\"), b))
        .collect();
    docs.push(("definition.pbir".into(), b"{}".to_vec()));
    docs.push((".platform".into(), b"{}".to_vec()));
    docs.push((".pbi/localSettings.json".into(), b"not json".to_vec()));
    let report = read_documents(&docs).unwrap();
    assert_eq!(report, read_dir(&fixture_dir()).unwrap());
}

#[test]
fn edited_fields_are_written_and_read_back() {
    let mut report = read_dir(&fixture_dir()).unwrap();
    let page = report.page_mut(PAGE_A).unwrap();
    // Chart: the aggregation becomes a measure; the category stays (and keeps `active`).
    let chart = page.visual_mut("c0ffee00000000000002").unwrap();
    chart.fields[1].field = measure("Sales", "Total Sales");
    // Table: drop the measure, add a column.
    let table = page.visual_mut("c0ffee00000000000003").unwrap();
    table.fields.remove(1);
    table.fields.push(binding("Values", col("Product", "Color")));
    // Card: no field any more.
    page.visual_mut("c0ffee00000000000001").unwrap().fields.clear();
    // A new visual with fields and no raw query.
    page.visuals.push(Visual {
        name: "c0ffee00000000000007".into(),
        position: Position {
            x: 10.0,
            y: 10.0,
            z: 5000.0,
            width: 200.0,
            height: 100.0,
            tab_order: Some(5000),
        },
        visual_type: Some("barChart".into()),
        fields: vec![
            binding("Category", col("Product", "Category")),
            binding(
                "Y",
                FieldRef::Aggregation {
                    table: "Sales".into(),
                    column: "Amount".into(),
                    function: "Average".into(),
                },
            ),
        ],
        ..Default::default()
    });
    // A new page without schema or display option.
    report.pages.push(Page {
        name: "0000000000000000new1".into(),
        display_name: "New".into(),
        width: 1280.0,
        height: 720.0,
        ..Default::default()
    });

    let written = write_documents(&report);
    assert_valid(&written);
    let back = read_documents(&written).unwrap();
    let page = back.page(PAGE_A).unwrap();
    let chart = page.visual("c0ffee00000000000002").unwrap();
    assert_eq!(chart.fields[1].field, measure("Sales", "Total Sales"));
    let raw = &chart.visual_extra["query"];
    assert_eq!(raw["queryState"]["Category"]["projections"][0]["active"], true);
    assert_eq!(
        raw["queryState"]["Y"]["projections"][0]["queryRef"],
        "Sales.Total Sales"
    );
    assert!(raw.get("sortDefinition").is_some(), "other query keys are kept");
    assert_eq!(
        page.visual("c0ffee00000000000003").unwrap().fields,
        [
            binding("Values", col("Product", "Category")),
            binding(
                "Values",
                FieldRef::Aggregation {
                    table: "Sales".into(),
                    column: "Quantity".into(),
                    function: "CountNonNull".into()
                }
            ),
            binding("Values", col("Product", "Color")),
        ]
    );
    let card = page.visual("c0ffee00000000000001").unwrap();
    assert!(card.fields.is_empty() && !card.visual_extra.contains_key("query"));
    let new = page.visual("c0ffee00000000000007").unwrap();
    assert_eq!(
        new.fields[1].field,
        FieldRef::Aggregation {
            table: "Sales".into(),
            column: "Amount".into(),
            function: "Avg".into()
        }
    );
    let new_page = back.page("New").unwrap();
    assert_eq!(new_page.display_option.as_deref(), Some("FitToPage"));
    assert_eq!(back.pages.len(), 3);
    // Untouched visuals are byte-identical.
    let original = fixture_docs();
    let slicer = format!("definition/pages/{PAGE_A}/visuals/c0ffee00000000000004/visual.json");
    let find = |docs: &Docs| docs.iter().find(|(p, _)| *p == slicer).map(|(_, b)| b.clone());
    assert_eq!(find(&original), find(&written));
}

#[test]
fn new_report_gets_current_schema_versions() {
    let report = Report {
        pages: vec![Page {
            name: "p1".into(),
            display_name: "Page 1".into(),
            width: 1280.0,
            height: 720.0,
            visuals: vec![Visual {
                name: "v1".into(),
                visual_type: Some("card".into()),
                fields: vec![binding("Values", measure("Sales", "Total"))],
                ..Default::default()
            }],
            ..Default::default()
        }],
        active_page: Some("p1".into()),
        bookmarks: vec![NamedDocument {
            name: "b1".into(),
            document: json!({"displayName": "B", "name": "b1",
                "explorationState": {"version": "1.3", "activeSection": "p1", "sections": {"p1": {"visualContainers": {}}}}}),
        }],
        ..Default::default()
    };
    let written = write_documents(&report);
    assert_valid(&written);
    let get = |path: &str| -> Value {
        let bytes = &written.iter().find(|(p, _)| p == path).unwrap().1;
        serde_json::from_slice(bytes).unwrap()
    };
    let schema = |path: &str| get(path)["$schema"].as_str().unwrap().to_string();
    assert!(schema("definition/report.json").contains("/report/3.3.0/"));
    assert!(schema("definition/version.json").contains("/versionMetadata/1.0.0/"));
    assert_eq!(get("definition/version.json")["version"], "2.0.0");
    assert!(schema("definition/pages/pages.json").contains("/pagesMetadata/1.1.0/"));
    assert!(schema("definition/pages/p1/page.json").contains("/page/2.1.0/"));
    assert!(schema("definition/pages/p1/visuals/v1/visual.json").contains("/visualContainer/2.12.0/"));
    assert!(schema("definition/bookmarks/b1.bookmark.json").contains("/bookmark/2.1.0/"));
    assert!(schema("definition/bookmarks/bookmarks.json").contains("/bookmarksMetadata/1.0.0/"));
    assert_eq!(
        get("definition/bookmarks/bookmarks.json")["items"],
        json!([{"name": "b1"}])
    );
    let bytes = &written[0].1;
    assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]) && !bytes.contains(&b'\r') && !bytes.ends_with(b"\n"));
}

#[test]
fn write_dir_removes_stale_files_only() {
    let (_tmp, dir) = tmp_dir("write");
    copy_dir(&fixture_dir(), &dir);
    std::fs::create_dir_all(dir.join(".pbi")).unwrap();
    std::fs::write(dir.join(".pbi/localSettings.json"), b"{}").unwrap();
    std::fs::write(dir.join(".platform"), b"{}").unwrap();
    std::fs::write(
        dir.join("definition/pages").join(PAGE_B).join("notes.txt"),
        b"mine",
    )
    .unwrap();
    let pbir_before = std::fs::read(dir.join("definition.pbir")).unwrap();

    let mut report = read_dir(&dir).unwrap();
    // Unchanged: nothing to do, same files.
    write_dir(&report, &dir).unwrap();
    assert_eq!(read_dir(&dir).unwrap(), report);

    report
        .page_mut(PAGE_A)
        .unwrap()
        .visuals
        .retain(|v| v.name != "c0ffee00000000000001");
    report.page_mut(PAGE_B).unwrap().visuals.clear();
    report.bookmarks.retain(|b| b.name != "Bookmark0000000000002");
    report.resources.retain(|r| !r.path.ends_with("bicraft-logo.png"));
    if let Some(Value::Array(packages)) = report.extra.get_mut("resourcePackages")
        && let Some(Value::Array(items)) = packages[0].get_mut("items")
    {
        items.retain(|i| i["path"] != "bicraft-logo.png");
    }
    report.resources.push(Resource {
        path: "StaticResources/RegisteredResources/new.png".into(),
        bytes: vec![1, 2, 3],
    });
    write_dir(&report, &dir).unwrap();

    let visuals_a = dir.join("definition/pages").join(PAGE_A).join("visuals");
    assert!(
        !visuals_a.join("c0ffee00000000000001").exists(),
        "visual and mobile.json removed"
    );
    assert!(visuals_a.join("c0ffee00000000000002/visual.json").exists());
    assert!(!dir.join("definition/pages").join(PAGE_B).join("visuals").exists());
    assert!(
        dir.join("definition/pages")
            .join(PAGE_B)
            .join("notes.txt")
            .exists(),
        "foreign files stay"
    );
    assert!(
        !dir.join("definition/bookmarks/Bookmark0000000000002.bookmark.json")
            .exists()
    );
    assert!(
        !dir.join("StaticResources/RegisteredResources/bicraft-logo.png")
            .exists()
    );
    assert!(dir.join("StaticResources/RegisteredResources/new.png").exists());
    assert_eq!(std::fs::read(dir.join("definition.pbir")).unwrap(), pbir_before);
    assert!(dir.join(".pbi/localSettings.json").exists() && dir.join(".platform").exists());
    let back = read_dir(&dir).unwrap();
    // The removed visual's mobile.json went with it.
    let mut expected = report.clone().tap_bookmarks_json();
    expected.resources.retain(|r| !r.path.ends_with("mobile.json"));
    expected.schemas.remove("mobile");
    assert!(
        back == expected,
        "write_dir result does not read back as the edited report"
    );
    std::fs::remove_dir_all(&dir).unwrap();

    // Removing a whole page removes its folder.
    let (_tmp, dir) = tmp_dir("write2");
    copy_dir(&fixture_dir(), &dir);
    let mut report = read_dir(&dir).unwrap();
    report.pages.retain(|p| p.name != PAGE_B);
    write_dir(&report, &dir).unwrap();
    assert!(!dir.join("definition/pages").join(PAGE_B).exists());
    let docs = {
        let mut d = Vec::new();
        walk(&dir, &dir.join("definition"), &mut d);
        walk(&dir, &dir.join("StaticResources"), &mut d);
        d
    };
    let issues = validate_documents(&docs);
    // The bookmark that pointed at the removed page is the caller's business; files are consistent.
    assert!(issues.is_empty(), "{issues:#?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `bookmarks.json` is regenerated when bookmarks change; the model compares its bytes too.
trait TapBookmarks {
    fn tap_bookmarks_json(self) -> Self;
}

impl TapBookmarks for Report {
    fn tap_bookmarks_json(mut self) -> Self {
        let written = write_documents(&self);
        let meta = written
            .iter()
            .find(|(p, _)| p == "definition/bookmarks/bookmarks.json")
            .unwrap();
        for r in &mut self.resources {
            if r.path == "definition/bookmarks/bookmarks.json" {
                r.bytes = meta.1.clone();
            }
        }
        self.resources.sort_by(|a, b| a.path.cmp(&b.path));
        self
    }
}

#[test]
fn write_dir_rejects_unsafe_names() {
    let (_tmp, dir) = tmp_dir("unsafe");
    for name in ["../escape", "a/b", "CON", "x:y", "", "dot."] {
        let report = Report {
            pages: vec![Page {
                name: name.into(),
                display_name: "x".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(write_dir(&report, &dir).is_err(), "{name:?}");
        // write_documents never emits such a path.
        assert!(
            write_documents(&report)
                .iter()
                .all(|(p, _)| !p.contains("..") && !p.contains("//"))
        );
    }
    let report = Report {
        resources: vec![Resource {
            path: "StaticResources/../../outside.txt".into(),
            bytes: vec![],
        }],
        ..Default::default()
    };
    assert!(write_dir(&report, &dir).is_err());
    let report = Report {
        resources: vec![Resource {
            path: "definition.pbir".into(),
            bytes: vec![],
        }],
        ..Default::default()
    };
    assert!(write_dir(&report, &dir).is_err());
    assert!(!dir.exists());
}

#[test]
fn hostile_documents_are_errors_not_panics() {
    let base = fixture_docs();
    let page = format!("definition/pages/{PAGE_A}/page.json");
    let visual = format!("definition/pages/{PAGE_A}/visuals/c0ffee00000000000002/visual.json");
    let replace = |path: &str, bytes: &[u8]| -> Docs {
        base.iter()
            .map(|(p, b)| (p.clone(), if p == path { bytes.to_vec() } else { b.clone() }))
            .collect()
    };
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    let cases: Vec<(&str, Docs)> = vec![
        ("not json", replace(&page, b"{ nope")),
        ("bom", replace(&page, b"\xEF\xBB\xBF{}")),
        ("utf16", replace(&page, b"\xFF\xFE{\x00}\x00")),
        ("invalid utf8", replace(&page, b"{\"name\": \"\xC3\x28\"}")),
        ("array", replace(&page, b"[]")),
        ("deep", replace(&page, deep.as_bytes())),
        ("name type", replace(&page, b"{\"name\": 3}")),
        ("name traversal", replace(&page, b"{\"name\": \"../x\"}")),
        (
            "width type",
            replace(&page, b"{\"name\": \"p\", \"width\": \"wide\"}"),
        ),
        (
            "visual type",
            replace(&visual, b"{\"name\": \"v\", \"visual\": []}"),
        ),
        (
            "visualType type",
            replace(&visual, b"{\"name\": \"v\", \"visual\": {\"visualType\": 1}}"),
        ),
        (
            "duplicate visual name",
            replace(&visual, b"{\"name\": \"c0ffee00000000000001\"}"),
        ),
        (
            "traversal path",
            vec![("definition/pages/../../x/page.json".into(), b"{}".to_vec())],
        ),
        (
            "empty segment",
            vec![("definition//report.json".into(), b"{}".to_vec())],
        ),
        ("bad char", vec![("StaticResources/a:b.png".into(), vec![])]),
        (
            "duplicate path",
            vec![
                ("definition/report.json".into(), b"{}".to_vec()),
                ("definition\\report.json".into(), b"{}".to_vec()),
            ],
        ),
        (
            "orphan visual",
            vec![("definition/pages/p/visuals/v/visual.json".into(), b"{}".to_vec())],
        ),
        (
            "pageOrder type",
            vec![(
                "definition/pages/pages.json".into(),
                b"{\"pageOrder\": {}}".to_vec(),
            )],
        ),
    ];
    for (label, docs) in cases {
        assert!(read_documents(&docs).is_err(), "{label}");
        // The validator reports, never panics.
        let _ = validate_documents(&docs);
    }
    // Lenient cases still read: unknown keys, odd but valid shapes.
    let ok = replace(
        &visual,
        b"{\"name\": \"c0ffee00000000000002\", \"position\": {\"x\": 1, \"angle\": 45}, \"zz\": [1]}",
    );
    let report = read_documents(&ok).unwrap();
    let v = report
        .page(PAGE_A)
        .unwrap()
        .visual("c0ffee00000000000002")
        .unwrap();
    assert_eq!(v.position.x, 1.0);
    let out = write_documents(&report);
    let bytes = &out.iter().find(|(p, _)| *p == visual).unwrap().1;
    let back: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(back["position"]["angle"], 45);
    assert_eq!(back["zz"], json!([1]));
    assert!(
        !validate_documents(&ok).is_empty(),
        "missing visual / required props are reported"
    );
}

#[test]
fn validator_reports_consistency_problems() {
    let base = fixture_docs();
    let edit = |path: &str, f: &dyn Fn(&mut Value)| -> Docs {
        base.iter()
            .map(|(p, b)| {
                if p == path {
                    let mut v: Value = serde_json::from_slice(b).unwrap();
                    f(&mut v);
                    (p.clone(), serde_json::to_vec(&v).unwrap())
                } else {
                    (p.clone(), b.clone())
                }
            })
            .collect()
    };
    let pointers = |docs: &Docs| -> Vec<(String, String)> {
        validate_documents(docs)
            .into_iter()
            .map(|i| (i.path, i.pointer))
            .collect()
    };
    let pages = "definition/pages/pages.json";
    let docs = edit(pages, &|v| {
        v["pageOrder"] = json!([PAGE_A, "missing"]);
        v["activePageName"] = json!("gone");
    });
    let found = pointers(&docs);
    assert!(
        found.contains(&(pages.into(), "/pageOrder/1".into())),
        "{found:?}"
    );
    // The page left out of `pageOrder` is not an issue (Desktop appends it).
    assert!(!found.contains(&(pages.into(), "/pageOrder".into())), "{found:?}");
    assert!(
        found.contains(&(pages.into(), "/activePageName".into())),
        "{found:?}"
    );

    // Schema errors deep inside a visual.
    let chart = format!("definition/pages/{PAGE_A}/visuals/c0ffee00000000000002/visual.json");
    let docs = edit(&chart, &|v| {
        v["position"]["bogus"] = json!(1);
        v["visual"]["query"]["queryState"]["Y"]["projections"][0]["field"] = json!({"Nope": {}});
        v["visual"]["query"]["queryState"]["Y"]["projections"][0]["queryRef"] = json!(7);
        v["filterConfig"]["filters"][0]["filter"]["Version"] = json!(3);
    });
    let found = pointers(&docs);
    for p in [
        "/position/bogus",
        "/visual/query/queryState/Y/projections/0/field/Nope",
        "/visual/query/queryState/Y/projections/0/queryRef",
        "/filterConfig/filters/0/filter/Version",
    ] {
        assert!(found.contains(&(chart.clone(), p.into())), "{p}: {found:?}");
    }

    let slicer = format!("definition/pages/{PAGE_A}/visuals/c0ffee00000000000004/visual.json");
    let docs = edit(&slicer, &|v| v["parentGroupName"] = json!("c0ffee00000000000003"));
    assert!(pointers(&docs).contains(&(slicer.clone(), "/parentGroupName".into())));
    let docs = edit(&slicer, &|v| v["name"] = json!("c0ffee00000000000001"));
    assert!(
        pointers(&docs)
            .iter()
            .any(|(p, ptr)| ptr == "/name" && p.contains("visuals"))
    );

    let report = "definition/report.json";
    let docs: Docs = base
        .iter()
        .filter(|(p, _)| !p.ends_with("bicraft-logo.png"))
        .cloned()
        .collect();
    assert!(pointers(&docs).contains(&(report.into(), "/resourcePackages/0/items/1/path".into())));

    let docs = edit(report, &|v| {
        v["themeCollection"]["customTheme"]["type"] = json!("Elsewhere")
    });
    assert!(pointers(&docs).contains(&(report.into(), "/themeCollection/customTheme/type".into())));
    let docs = edit(report, &|v| {
        v["$schema"] = json!(
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/9.0.0/schema.json"
        )
    });
    assert!(pointers(&docs).contains(&(report.into(), "/$schema".into())));
    // A newer minor validates against the newest known one.
    let docs = edit(report, &|v| {
        v["$schema"] = json!(
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.99.0/schema.json"
        )
    });
    assert_eq!(pointers(&docs), Vec::<(String, String)>::new());

    let docs = edit("definition/bookmarks/bookmarks.json", &|v| {
        v["items"][0]["name"] = json!("nope")
    });
    assert!(pointers(&docs).contains(&(
        "definition/bookmarks/bookmarks.json".into(),
        "/items/0/name".into()
    )));

    let mut docs = base.clone();
    docs.push(("definition/stray.json".into(), b"{}".to_vec()));
    assert!(pointers(&docs).contains(&("definition/stray.json".into(), String::new())));
}

/// A custom visual package's items resolve under `CustomVisuals/<name>/resources/` (Desktop
/// 2.158 loads the visual only then; pbix-package.md §2, "Custom visuals").
#[test]
fn custom_visual_items_resolve_under_resources() {
    let guid = "bicraftHello7C3E9A41B2D54F60";
    let mut docs = fixture_docs();
    let report = docs
        .iter_mut()
        .find(|(p, _)| p == "definition/report.json")
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&report.1).unwrap();
    let packages = value
        .as_object_mut()
        .unwrap()
        .entry("resourcePackages")
        .or_insert_with(|| serde_json::json!([]));
    packages.as_array_mut().unwrap().push(serde_json::json!({
        "name": guid, "type": "CustomVisual",
        "items": [{"name": guid, "path": format!("{guid}.pbiviz.json"), "type": "CustomVisualMetadata"}]
    }));
    report.1 = serde_json::to_vec(&value).unwrap();
    let missing = validate_documents(&docs);
    assert!(
        missing
            .iter()
            .any(|i| format!("{i:?}").contains(&format!("CustomVisuals/{guid}/resources/{guid}.pbiviz.json"))),
        "{missing:#?}"
    );
    docs.push((format!("CustomVisuals/{guid}/package.json"), b"{}".to_vec()));
    docs.push((
        format!("CustomVisuals/{guid}/resources/{guid}.pbiviz.json"),
        b"{}".to_vec(),
    ));
    assert_valid(&docs);
}

/// G13: Desktop writes no page size for the deprecated dynamic display option, nor a zero size
/// (docs/formats/legacy-layout-bb.md).
#[test]
fn dynamic_and_zero_page_sizes_are_not_written() {
    let page = |name: &str, option: &str, w: f64| Page {
        name: name.into(),
        display_name: name.into(),
        display_option: Some(option.into()),
        width: w,
        height: w,
        ..Page::default()
    };
    let report = Report {
        pages: vec![
            page("dyn", "DeprecatedDynamic", 960.0),
            page("zero", "FitToPage", 0.0),
            page("fit", "FitToPage", 720.0),
        ],
        ..Report::default()
    };
    let docs = write_documents(&report);
    let page_json = |name: &str| -> Value {
        let path = format!("definition/pages/{name}/page.json");
        let (_, bytes) = docs.iter().find(|(p, _)| *p == path).unwrap();
        serde_json::from_slice(bytes).unwrap()
    };
    for name in ["dyn", "zero"] {
        let p = page_json(name);
        assert!(
            p.get("width").is_none() && p.get("height").is_none(),
            "{name}: {p}"
        );
    }
    assert_eq!(page_json("fit")["width"].as_f64(), Some(720.0));
}

/// Regression (docs/formats/packaging-verification.md, P-NAME): Power BI Desktop only loads
/// pages and visuals whose names are letters, digits, underscores or hyphens; it ignores the
/// others. The writer refuses such names and the validator reports them.
#[test]
fn object_names_must_be_word_characters_or_hyphens() {
    let (_tmp, dir) = tmp_dir("object-names");
    for name in ["pg.1", "pg 1", "p+q"] {
        let report = Report {
            pages: vec![Page {
                name: name.into(),
                display_name: "x".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(write_dir(&report, &dir).is_err(), "{name:?}");
    }
    let base = fixture_docs();
    let page = format!("definition/pages/{PAGE_A}/page.json");
    let docs: Docs = base
        .iter()
        .map(|(p, b)| {
            if *p == page {
                let mut v: Value = serde_json::from_slice(b).unwrap();
                v["name"] = json!("bad name");
                (p.clone(), serde_json::to_vec(&v).unwrap())
            } else {
                (p.clone(), b.clone())
            }
        })
        .collect();
    let issues = validate_documents(&docs);
    assert!(
        issues
            .iter()
            .any(|i| i.path == page && i.pointer == "/name" && i.message.contains("Desktop ignores")),
        "{issues:?}"
    );
}
