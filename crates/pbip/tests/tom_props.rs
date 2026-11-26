//! TMSL -> TMDL conversion of the TOM oracle's "every property set" model: bicraft must write
//! the TMDL that TOM writes for the same model (docs/formats/tmdl.md "Property order"), although
//! TOM's JSON lists the properties in another order (`sourceColumn` before `formatString`,
//! `sortByColumn` in the middle, `expressionContext` before `stringIndexingBehavior`, ...).

use std::path::Path;

fn tmdl_fixture() -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                let text = std::fs::read_to_string(&path).unwrap();
                out.push((relative, text.replace("\r\n", "\n").replace('\n', "\r\n")));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tmdl/tests/fixtures/tom-props");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

#[test]
fn tom_json_converts_to_tom_tmdl() {
    let bim = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tmsl/tests/fixtures/tom-props.bim"),
    )
    .unwrap();
    let model = bicraft_tmsl::read(&bim).unwrap();
    let want = tmdl_fixture();
    let got = bicraft_tmdl::write_documents(&model);
    let paths = |d: &[(String, String)]| d.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&want), paths(&got));
    for ((path, w), (_, g)) in want.iter().zip(&got) {
        if w != g {
            let line = w.lines().zip(g.lines()).position(|(a, b)| a != b).unwrap_or(0);
            panic!(
                "{path}: first difference at line {}\n  want: {:?}\n  got:  {:?}\n{g}",
                line + 1,
                w.lines().nth(line),
                g.lines().nth(line)
            );
        }
    }
}
