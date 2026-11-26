//! The desktop open tests that need the engine's private items (`openable`); the others are the `files`
//! integration test (`tests/files/desktop_open_tests.rs`), under the same module path.

use super::*;
use crate::tempdir::TempDir;

#[test]
fn only_power_bi_files_are_handed_to_the_os() {
    let dir = TempDir::new("engine-desktop-open-ext");
    for name in ["run.exe", "x.bat", "notes.txt", "noext"] {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        // the default opener refuses before launching anything
        let e = open_with_default_app(&p).unwrap_err();
        assert!(e.contains("only .pbix"), "{e}");
    }
    let e = open_with_default_app(&dir.join("missing.pbix")).unwrap_err();
    assert!(e.contains("does not exist"), "{e}");
    assert!(openable(dir.path()).is_err());
}
