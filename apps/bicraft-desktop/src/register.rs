//! `bicraft --register` / `--unregister`: per-user Windows integration, under
//! `HKEY_CURRENT_USER` only (no administrator rights, nothing machine-wide):
//!
//! - a Start menu shortcut `BIcraft Desktop.lnk` with the AppUserModelID `BIcraft.Desktop`;
//! - `Software\Classes\Applications\bicraft.exe`: the open command, the friendly name,
//!   the icon and `SupportedTypes` (.pbix, .pbit, .pbip, .bim), so the app is offered in
//!   "Open with";
//! - `Software\Classes\.<ext>\OpenWithList\bicraft.exe` for each type.
//!
//! The user's default application for these types is never changed: no `ProgId` is written as
//! the default of `.pbix`, no `UserChoice` is touched. `--unregister` removes exactly these keys
//! and the shortcut.

use std::path::{Path, PathBuf};

/// The executable's name in the registry.
pub const EXE: &str = "bicraft.exe";
/// The AppUserModelID of the shortcut and the window (Windows).
pub const APP_ID: &str = "BIcraft.Desktop";
/// The freedesktop application id (Linux): the window's Wayland app id and X11 class, the name
/// of the `.desktop` file, the icons and the metainfo file the Linux release archive installs
/// (`cargo xtask dist`, docs/releasing.md).
pub const DESKTOP_ID: &str = "io.github.digorpeptra.bicraft";
/// The Start menu shortcut's file name.
pub const SHORTCUT: &str = "BIcraft Desktop.lnk";

/// The registry entries of `--register`: `(key under HKCU, value name, value)`; an empty value
/// name is the key's default value.
pub fn entries(exe: &Path) -> Vec<(String, String, String)> {
    let exe = exe.display().to_string();
    let app = format!(r"Software\Classes\Applications\{EXE}");
    let mut e = vec![
        (app.clone(), "FriendlyAppName".into(), "BIcraft".into()),
        (
            format!(r"{app}\DefaultIcon"),
            String::new(),
            format!("\"{exe}\",0"),
        ),
        (
            format!(r"{app}\shell\open\command"),
            String::new(),
            format!("\"{exe}\" \"%1\""),
        ),
    ];
    for ext in crate::EXTENSIONS {
        e.push((format!(r"{app}\SupportedTypes"), format!(".{ext}"), String::new()));
        e.push((
            format!(r"Software\Classes\.{ext}\OpenWithList\{EXE}"),
            String::new(),
            String::new(),
        ));
    }
    e
}

/// The keys `--unregister` deletes (with their subkeys).
pub fn owned_keys() -> Vec<String> {
    let mut k = vec![format!(r"Software\Classes\Applications\{EXE}")];
    for ext in crate::EXTENSIONS {
        k.push(format!(r"Software\Classes\.{ext}\OpenWithList\{EXE}"));
    }
    k
}

/// The per-user Start menu programs folder.
pub fn start_menu() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| {
        PathBuf::from(a)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    })
}

/// The shortcut to `exe`.
pub fn shortcut(exe: &Path) -> crate::lnk::Shortcut {
    let dir = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
    crate::lnk::Shortcut {
        target: exe.display().to_string(),
        working_dir: dir,
        description: "BIcraft".into(),
        icon: exe.display().to_string(),
        app_user_model_id: APP_ID.into(),
    }
}

/// `--register`: returns what was done, one line per item.
#[cfg(windows)]
pub fn register(exe: &Path) -> Result<Vec<String>, String> {
    let hkcu = windows_registry::CURRENT_USER;
    let mut done = Vec::new();
    for (key, name, value) in entries(exe) {
        let k = hkcu.create(&key).map_err(|e| format!(r"HKCU\{key}: {e}"))?;
        k.set_string(&name, &value)
            .map_err(|e| format!(r"HKCU\{key} {name}: {e}"))?;
    }
    done.push(format!(
        r"HKCU\Software\Classes\Applications\{EXE} (Open with: .pbix .pbit .pbip .bim)"
    ));
    let dir = start_menu().ok_or("no APPDATA folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lnk = dir.join(SHORTCUT);
    std::fs::write(&lnk, crate::lnk::shortcut_bytes(&shortcut(exe)))
        .map_err(|e| format!("{}: {e}", lnk.display()))?;
    done.push(format!("{} (AppUserModelID {APP_ID})", lnk.display()));
    Ok(done)
}

/// `--unregister`: returns what was removed.
#[cfg(windows)]
pub fn unregister() -> Result<Vec<String>, String> {
    let hkcu = windows_registry::CURRENT_USER;
    let mut done = Vec::new();
    for key in owned_keys() {
        // a missing key is already unregistered
        if hkcu.remove_tree(&key).is_ok() {
            done.push(format!(r"HKCU\{key}"));
        }
    }
    if let Some(lnk) = start_menu().map(|d| d.join(SHORTCUT))
        && lnk.exists()
    {
        std::fs::remove_file(&lnk).map_err(|e| format!("{}: {e}", lnk.display()))?;
        done.push(lnk.display().to_string());
    }
    Ok(done)
}

#[cfg(not(windows))]
pub fn register(_exe: &Path) -> Result<Vec<String>, String> {
    Err("--register is for Windows; on Linux use a .desktop file (docs/desktop.md)".into())
}

#[cfg(not(windows))]
pub fn unregister() -> Result<Vec<String>, String> {
    Err("--unregister is for Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_stay_per_user_and_never_take_the_default() {
        let exe = Path::new(r"C:\x\bicraft.exe");
        let e = entries(exe);
        let cmd = e
            .iter()
            .find(|(k, _, _)| k.ends_with(r"shell\open\command"))
            .unwrap();
        assert_eq!(cmd.2, r#""C:\x\bicraft.exe" "%1""#);
        for ext in crate::EXTENSIONS {
            assert!(
                e.iter()
                    .any(|(k, n, _)| k.ends_with("SupportedTypes") && n == &format!(".{ext}"))
            );
        }
        // under Software\Classes, never a type's own default value or a UserChoice
        for (k, n, _) in &e {
            assert!(k.starts_with(r"Software\Classes\"), "{k}");
            assert!(!k.contains("UserChoice"));
            let is_type_key = crate::EXTENSIONS
                .iter()
                .any(|x| k == &format!(r"Software\Classes\.{x}"));
            assert!(
                !(is_type_key && n.is_empty()),
                "the default app of a type stays the user's"
            );
        }
        // what --unregister removes is what --register creates, nothing above it
        for k in owned_keys() {
            assert!(k.contains(EXE));
        }
        assert_eq!(shortcut(exe).app_user_model_id, APP_ID);
    }
}
