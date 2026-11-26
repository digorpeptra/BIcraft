//! The desktop's settings file, the native [`PrefsStore`]: recent files and servers (never a
//! token) and the zoom ([`bicraft_ui::prefs::Prefs`]). `%APPDATA%\BIcraft\desktop.json` on
//! Windows, `$XDG_CONFIG_HOME/bicraft/desktop.json` (else `~/.config/bicraft/desktop.json`)
//! elsewhere. A missing or damaged file is an empty one.

use std::path::{Path, PathBuf};

use bicraft_ui::shell::PrefsStore;
use serde_json::Value;

/// Largest settings file read.
const MAX_BYTES: u64 = 1 << 20;

/// The settings file's path.
pub fn default_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("BIcraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("bicraft"))
    };
    dir.map(|d| d.join("desktop.json"))
}

/// A settings file.
pub struct FileStore(pub PathBuf);

impl PrefsStore for FileStore {
    fn load(&self) -> Value {
        let path = &self.0;
        let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_BYTES);
        if !small {
            return Value::Null;
        }
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null)
    }

    /// Writes the file (through a temporary file, so a crash never leaves half of it).
    fn save(&self, v: &Value) -> Result<(), String> {
        save(&self.0, v).map_err(|e| format!("{}: {e}", self.0.display()))
    }
}

fn save(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bicraft_ui::prefs::Prefs;

    #[test]
    fn round_trip_and_damaged_files() {
        let mut p = Prefs::default();
        p.add_recent(Path::new("f5.pbix"), 1);
        p.add_server("https://h", "a");
        p.show_gridlines = true;
        let dir = std::env::temp_dir().join(format!("bicraft-desktop-prefs-{}", std::process::id()));
        let store = FileStore(dir.join("desktop.json"));
        store.save(&p.to_json()).unwrap();
        assert_eq!(Prefs::from_json(&store.load()), p);
        std::fs::write(&store.0, "{not json").unwrap();
        assert_eq!(Prefs::from_json(&store.load()), Prefs::default());
        assert_eq!(FileStore(dir.join("missing.json")).load(), Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
