use std::collections::BTreeMap;

use super::*;

fn files(list: &[(&str, &str)]) -> BTreeMap<String, String> {
    list.iter().map(|(p, t)| (p.to_string(), t.to_string())).collect()
}

const VISUAL: &str = r#"
import powerbi from "powerbi-visuals-api";
import VisualConstructorOptions = powerbi.extensibility.visual.VisualConstructorOptions;
import { label } from "./util";

interface Point { x: number; y: number }

export class Visual {
    private target: HTMLElement;
    private count: number = 0;
    constructor(options: VisualConstructorOptions) {
        this.target = options.element;
    }
    public update(p: Point): string {
        this.count += 1;
        return label(p.x as number) + this.count;
    }
}
"#;

#[test]
fn strips_types_and_bundles_relative_imports_into_one_script() {
    let src = files(&[
        ("src/visual.ts", VISUAL),
        (
            "src/util.ts",
            "export function label(n: number): string { return `n=${n}`; }",
        ),
        (
            "plugin.js",
            "import { Visual } from \"./src/visual\";\nwindow.made = new Visual({element: null});",
        ),
    ]);
    let out = bundle(&src, "plugin.js").unwrap();
    let js = &out.js;
    assert!(js.contains("class Visual"), "{js}");
    assert!(js.contains("function label"), "{js}");
    assert!(js.contains("window.made"), "{js}");
    // no types, no module syntax, no import of the API package
    for gone in [
        "interface",
        ": number",
        "import ",
        "export ",
        "powerbi-visuals-api",
        "as number",
    ] {
        assert!(!js.contains(gone), "`{gone}` left in:\n{js}");
    }
    assert_eq!(out.modules, ["plugin.js", "src/util.ts", "src/visual.ts"]);
}

#[test]
fn errors_name_the_file_and_line() {
    let src = files(&[("src/visual.ts", "let a: number = 1;\nlet b = ;\n")]);
    let e = bundle(&src, "src/visual.ts").unwrap_err();
    assert!(e.starts_with("src/visual.ts:2:"), "{e}");
}

#[test]
fn packages_missing_files_and_bad_paths_are_refused() {
    let e = bundle(
        &files(&[("a.ts", "import * as d3 from \"d3\";\nconsole.log(d3);")]),
        "a.ts",
    )
    .unwrap_err();
    assert!(e.contains("`d3`, a package"), "{e}");
    let e = bundle(
        &files(&[("a.ts", "import { x } from \"./nope\";\nconsole.log(x);")]),
        "a.ts",
    )
    .unwrap_err();
    assert!(e.contains("`./nope`"), "{e}");
    // an import cannot climb out of the project
    let e = bundle(
        &files(&[("a.ts", "import { x } from \"../../etc/x\";\nconsole.log(x);")]),
        "a.ts",
    )
    .unwrap_err();
    assert!(e.contains("../../etc/x"), "{e}");
    assert!(bundle(&files(&[("../a.ts", "")]), "../a.ts").is_err());
    assert!(bundle(&files(&[("a.ts", "")]), "b.ts").is_err());
    let big = "x".repeat(MAX_FILE_BYTES + 1);
    assert!(bundle(&files(&[("a.ts", big.as_str())]), "a.ts").is_err());
}

#[test]
fn hostile_sources_give_errors_not_panics() {
    for text in [
        "",
        "export {}",
        "(((((((((((",
        "class { }",
        "import x from './a.ts'; export default x;",
        "\u{feff}let x = 1;",
        "enum E { A, B } console.log(E.A);",
        "namespace N { export const a = 1; } console.log(N.a);",
        "@dec class C {}",
        "import './a';\nconsole.log(1);",
        "export * from './a';",
        "const x = import('./b'); export default x;",
        "let a = <any>1;",
        "declare module 'x' { }",
    ] {
        let _ = bundle(&files(&[("a.ts", text)]), "a.ts");
    }
    // import cycles (the bundler would recurse without end) are refused, named
    let e = bundle(
        &files(&[("a.ts", "import x from './a.ts'; export default x;")]),
        "a.ts",
    )
    .unwrap_err();
    assert!(e.contains("import cycle a.ts → a.ts"), "{e}");
    let e = bundle(
        &files(&[
            ("a.ts", "import { b } from './b';\nexport const a = b;"),
            (
                "b.ts",
                "import { a } from './a';\nexport const b = 1; console.log(a);",
            ),
        ]),
        "a.ts",
    )
    .unwrap_err();
    assert!(e.contains("a.ts → b.ts → a.ts"), "{e}");
    // a module imported twice (a diamond) is no cycle
    let out = bundle(
        &files(&[
            (
                "a.ts",
                "import { b } from './b';\nimport { c } from './c';\nconsole.log(b, c);",
            ),
            ("b.ts", "import { d } from './d';\nexport const b = d + 1;"),
            ("c.ts", "import { d } from './d';\nexport const c = d + 2;"),
            ("d.ts", "export const d: number = 1;"),
        ]),
        "a.ts",
    );
    assert!(out.is_ok(), "{out:?}");
    // deep nesting is refused before the parser recurses; what passes the cap compiles
    let deep = format!("let x = {}1{};", "(".repeat(5000), ")".repeat(5000));
    let e = bundle(&files(&[("a.ts", deep.as_str())]), "a.ts").unwrap_err();
    assert!(e.contains("nested deeper"), "{e}");
    let ok = format!(
        "let x = {}1{};\nconsole.log(x);",
        "(".repeat(MAX_NESTING),
        ")".repeat(MAX_NESTING)
    );
    assert!(bundle(&files(&[("a.ts", ok.as_str())]), "a.ts").is_ok());
}
