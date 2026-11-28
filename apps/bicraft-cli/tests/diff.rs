//! End-to-end tests of `bicraft-cli diff`: the forms, exit codes, git's textconv and external-diff
//! calls, and `--git` on a throwaway repository (skipped when git is not installed).

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

#[path = "../../../testsupport/tempdir.rs"]
mod tempdir;
use tempdir::TempDir;

const BIM_A: &str = r#"{"name":"M","model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double"}],
    "measures":[{"name":"Total","expression":"SUM(Sales[Amount])","lineageTag":"m1"}]}]}}"#;
const BIM_B: &str = r#"{"name":"M","model":{"tables":[{"name":"Sales",
    "columns":[{"name":"Amount","dataType":"double"}],
    "measures":[{"name":"Revenue","expression":"SUM ( Sales[Amount] ) // total","lineageTag":"m1"},
                {"name":"Double","expression":"[Revenue] * 2"}]}]}}"#;

fn bicraft(dir: Option<&Path>, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_bicraft-cli"));
    if let Some(d) = dir {
        c.current_dir(d);
    }
    c.args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

#[test]
fn forms_and_exit_codes() {
    let dir = TempDir::new("cli-diff-forms");
    let (a, b) = (dir.join("a.bim"), dir.join("b.bim"));
    std::fs::write(&a, BIM_A).unwrap();
    std::fs::write(&b, BIM_B).unwrap();
    let (sa, sb) = (a.to_str().unwrap(), b.to_str().unwrap());

    let o = bicraft(None, &["diff", sa, sb]);
    assert_eq!(o.status.code(), Some(0));
    let text = stdout(&o);
    assert!(
        text.contains("> measure 'Sales'[Revenue] (renamed from measure 'Sales'[Total], same lineage tag)"),
        "{text}"
    );
    assert!(
        text.contains("expression: whitespace, comments or case only"),
        "{text}"
    );
    assert!(text.contains("+ measure 'Sales'[Double]"), "{text}");
    assert!(!text.contains('\r'));

    assert_eq!(
        bicraft(None, &["diff", sa, sb, "--exit-code"]).status.code(),
        Some(1)
    );
    assert_eq!(
        bicraft(None, &["diff", sa, sa, "--exit-code"]).status.code(),
        Some(0)
    );
    let quiet = stdout(&bicraft(None, &["diff", sa, sb, "--ignore-whitespace"]));
    assert!(!quiet.contains("whitespace, comments"), "{quiet}");

    let j: Value = serde_json::from_str(&stdout(&bicraft(None, &["diff", sa, sb, "--json"]))).unwrap();
    assert_eq!(j["summary"]["renamed"], 1);
    assert_eq!(j["summary"]["added"], 1);
    assert_eq!(j["a"]["kind"], "bim");

    let md = stdout(&bicraft(None, &["diff", sa, sb, "--markdown"]));
    assert!(md.starts_with("### Power BI diff"), "{md}");

    // textconv: one file's canonical text
    let t = stdout(&bicraft(None, &["diff", "--textconv", sb]));
    assert!(
        t.contains("measure 'Sales'[Double]\n    expression:\n        [Revenue] * 2"),
        "{t}"
    );

    // git's external diff call; a deleted file is /dev/null; differences do not fail git
    let o = bicraft(
        None,
        &[
            "diff",
            "m.bim",
            sa,
            "0000",
            "100644",
            "/dev/null",
            "0000",
            "100644",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    let t = stdout(&o);
    assert!(
        t.starts_with("a: a/m.bim\nb: b/m.bim\n") && t.contains("- table 'Sales'"),
        "{t}"
    );

    // usage and unreadable files
    assert_eq!(bicraft(None, &["diff", sa]).status.code(), Some(2));
    assert_eq!(bicraft(None, &["diff", sa, sb, "--bogus"]).status.code(), Some(2));
    let missing = dir.join("missing.pbix");
    assert_eq!(
        bicraft(None, &["diff", sa, missing.to_str().unwrap()])
            .status
            .code(),
        Some(2)
    );
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .current_dir(repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

#[test]
fn git_revisions_of_a_project() {
    let dir = TempDir::new("cli-diff-git");
    if git(&dir, &["init", "-q"]).is_none() {
        eprintln!("note: git is not available, skipped");
        return;
    }
    let bim = dir.join("source.bim");
    std::fs::write(&bim, BIM_A).unwrap();
    let o = bicraft(
        None,
        &[
            "new-project",
            dir.join("proj").to_str().unwrap(),
            "Shop",
            "--open",
            bim.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    // a local cache must never be copied out
    std::fs::create_dir_all(dir.join("proj/.pbi")).unwrap();
    std::fs::write(dir.join("proj/.pbi/cache.abf"), b"not a backup").unwrap();
    git(&dir, &["add", "-A"]).unwrap();
    git(&dir, &["commit", "-q", "-m", "one"]).unwrap();
    let project = dir.join("proj/Shop.pbip");
    let o = bicraft(
        None,
        &[
            "run",
            "--open",
            project.to_str().unwrap(),
            "--cmd",
            r#"measure.set={"name":"Total","expression":"SUM(Sales[Amount]) * 2"}"#,
            "--save",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    git(&dir, &["commit", "-q", "-a", "-m", "two"]).unwrap();
    let status = git(&dir, &["status", "--porcelain"]).unwrap();

    // from the repository's sub-folder, a path relative to it
    let sub = dir.join("proj");
    let o = bicraft(
        Some(&sub),
        &["diff", "--git", "HEAD~1", "HEAD", "Shop.pbip", "--json"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let j: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(j["a"]["path"], "HEAD~1:proj/Shop.pbip", "{j}");
    let changes = j["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "{j}");
    assert_eq!(changes[0]["id"], "measure 'Sales'[Total]");

    // against the working tree: same as HEAD
    let o = bicraft(Some(&dir), &["diff", "--git", "HEAD", "proj", "--exit-code"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}{}",
        stdout(&o),
        String::from_utf8_lossy(&o.stderr)
    );
    // a project folder absent at a revision: everything added
    let o = bicraft(Some(&dir), &["diff", "--git", "HEAD", "HEAD", "nowhere"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("No differences."));
    // bad revisions are refused, options never pass through as revisions
    assert_eq!(
        bicraft(Some(&dir), &["diff", "--git", "nope", "HEAD", "proj"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        bicraft(Some(&dir), &["diff", "--git", "--", "-x", "HEAD", "proj"])
            .status
            .code(),
        Some(2)
    );
    // the repository is untouched
    assert_eq!(git(&dir, &["status", "--porcelain"]).unwrap(), status);
}
