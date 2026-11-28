//! The app's preferences: recent files (with the date they were last opened, pinned or not) and
//! servers (URL and workspace; never a token), and the UI preferences (light or dark,
//! ribbon collapsed, the tab shown). Where they are kept is the shell's
//! ([`crate::shell::PrefsStore`]): the desktop keeps a settings file
//! (`apps/bicraft-desktop/src/prefs.rs`), the browser keeps the UI preferences only ([`ui_only`],
//! in `localStorage`; docs/webui.md, "Browser storage"), its recent list holding workspace ids and
//! workspace-relative file names only, never a path of the server's disk or a URL. A missing or
//! damaged store is an empty one.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::remote::{safe_relative, valid_workspace};
use crate::theme::Brightness;

/// Recent files kept.
pub const MAX_RECENT: usize = 50;
/// Recent servers kept.
pub const MAX_SERVERS: usize = 10;

/// The keys of the UI preferences: the only ones a browser keeps (`recent` there holds
/// workspace-relative names only, see [`ui_only`]).
pub const UI_KEYS: [&str; 12] = [
    "theme",
    "ribbon",
    "tab",
    "recent",
    "commands",
    "autosave",
    "panes",
    "snap",
    "gridlines",
    "lock",
    "guides",
    "phone",
];

/// Recent commands kept (the command palette lists them first).
pub const MAX_RECENT_COMMANDS: usize = 12;

/// Bounds and defaults of the autosave delay, in seconds.
pub const AUTOSAVE_MIN_SECS: u32 = 5;
pub const AUTOSAVE_MAX_SECS: u32 = 3600;
pub const AUTOSAVE_EVERY_SECS: u32 = 300;
pub const AUTOSAVE_IDLE_SECS: u32 = 30;

/// "Save automatically" (docs/recovery.md, "Autosave"): off (the default), every N seconds while
/// there are unsaved edits, or N seconds after the last edit. PBIP projects only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Autosave {
    #[default]
    Off,
    Every(u32),
    Idle(u32),
}

impl Autosave {
    fn from_json(v: Option<&Value>) -> Autosave {
        let Some(v) = v else { return Autosave::Off };
        let secs = v
            .get("secs")
            .and_then(Value::as_u64)
            .and_then(|s| u32::try_from(s).ok())
            .map(|s| s.clamp(AUTOSAVE_MIN_SECS, AUTOSAVE_MAX_SECS));
        match v.get("mode").and_then(Value::as_str) {
            Some("interval") => Autosave::Every(secs.unwrap_or(AUTOSAVE_EVERY_SECS)),
            Some("idle") => Autosave::Idle(secs.unwrap_or(AUTOSAVE_IDLE_SECS)),
            _ => Autosave::Off,
        }
    }

    fn to_json(self) -> Value {
        match self {
            Autosave::Off => json!({"mode": "off"}),
            Autosave::Every(s) => json!({"mode": "interval", "secs": s}),
            Autosave::Idle(s) => json!({"mode": "idle", "secs": s}),
        }
    }
}

/// The tabs a preference may name.
pub const TABS: [&str; 3] = ["report", "model", "validation"];

/// A recent file: a path on the desktop, or a workspace-relative name with its workspace in the
/// browser (or on a server the desktop connects to).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recent {
    /// The file's path as text (nothing is read from disk to list it).
    pub path: String,
    /// The server workspace the file belongs to, when it is a workspace file.
    pub ws: Option<String>,
    /// When it was last opened, in seconds since 1970 (0: unknown).
    pub opened: u64,
    pub pinned: bool,
}

impl Recent {
    fn from_json(v: &Value) -> Option<Recent> {
        // the format before 2026-11-04: a bare path
        if let Some(s) = v.as_str() {
            return (!s.is_empty() && s.len() < 4096).then(|| Recent {
                path: s.to_string(),
                ..Recent::default()
            });
        }
        let path = v.get("path")?.as_str()?;
        if path.is_empty() || path.len() >= 4096 {
            return None;
        }
        let ws = match v.get("ws").and_then(Value::as_str) {
            Some(w) if valid_workspace(w) && safe_relative(path) => Some(w.to_string()),
            Some(_) => return None,
            None => None,
        };
        Some(Recent {
            path: path.to_string(),
            ws,
            opened: v.get("opened").and_then(Value::as_u64).unwrap_or(0),
            pinned: v.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    fn to_json(&self) -> Value {
        let mut v = json!({"path": self.path, "opened": self.opened, "pinned": self.pinned});
        if let (Some(ws), Some(o)) = (&self.ws, v.as_object_mut()) {
            o.insert("ws".into(), json!(ws));
        }
        v
    }

    fn same(&self, o: &Recent) -> bool {
        self.path == o.path && self.ws == o.ws
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prefs {
    /// Pinned files first, then the others, most recently opened first.
    pub recent: Vec<Recent>,
    /// (URL, workspace)
    pub servers: Vec<(String, String)>,
    /// Light (the default), dark or high contrast.
    pub theme: Brightness,
    /// The report pages are drawn with the high-contrast palette (View › High-contrast
    /// colours): a view option, the files are never changed.
    pub report_contrast: bool,
    /// The ribbon card is hidden (the tab row stays).
    pub ribbon_collapsed: bool,
    /// The tab shown at start (`report`, `model`, `validation`; empty: the Report tab).
    pub tab: String,
    /// The commands run last from the palette or a form, most recent first (command ids).
    pub commands: Vec<String>,
    /// Save PBIP projects automatically (off by default).
    pub autosave: Autosave,
    /// The Report view's task panes: open or collapsed, and their widths.
    pub panes: PanePrefs,
    /// The canvas shows its grid (View › Gridlines, `crate::canvas`).
    pub show_gridlines: bool,
    /// Moved and resized visuals snap to the canvas grid (View › Snap to grid).
    pub snap_grid: bool,
    /// Smart guides are off (View › Smart guides): moved and resized visuals no longer snap to
    /// the other visuals' and the page's edges and centres, and no guide is drawn.
    pub hide_guides: bool,
    /// Visuals cannot be moved or resized on the canvas (View › Lock objects); selecting them,
    /// the menus and the panes still work.
    pub lock_objects: bool,
    /// A page with a mobile layout is read on its phone canvas in a narrow window (View ›
    /// Phone layout when narrow, `crate::mobile`).
    pub mobile_narrow: bool,
}

/// The Report view's right panes (Build and Data, `crate::panes`): open or collapsed, and how
/// wide (points, kept within [`PANE_WIDTHS`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanePrefs {
    pub data: bool,
    pub data_width: f32,
    pub build: bool,
    pub build_width: f32,
    /// The Filters pane (left of Build).
    pub filters: bool,
    pub filters_width: f32,
    /// The Selection pane (right of every other pane; closed until asked for).
    pub selection: bool,
    pub selection_width: f32,
    /// The Bookmarks pane (next to the Selection pane; closed until asked for).
    pub bookmarks: bool,
    pub bookmarks_width: f32,
    /// The Sync slicers pane (left of the Bookmarks pane; closed until asked for).
    pub sync: bool,
    pub sync_width: f32,
}

/// The widths a pane may have.
pub const PANE_WIDTHS: std::ops::RangeInclusive<f32> = 180.0..=520.0;

impl Default for PanePrefs {
    fn default() -> Self {
        PanePrefs {
            data: true,
            data_width: 250.0,
            build: true,
            build_width: 250.0,
            filters: true,
            filters_width: 230.0,
            selection: false,
            selection_width: 230.0,
            bookmarks: false,
            bookmarks_width: 230.0,
            sync: false,
            sync_width: 230.0,
        }
    }
}

impl PanePrefs {
    fn from_json(v: Option<&Value>) -> PanePrefs {
        let d = PanePrefs::default();
        let Some(v) = v.filter(|v| v.is_object()) else {
            return d;
        };
        let open = |k: &str, d: bool| match v.get(k).and_then(Value::as_str) {
            Some("open") => true,
            Some("collapsed") => false,
            _ => d,
        };
        let width = |k: &str, d: f32| {
            v.get(k)
                .and_then(Value::as_f64)
                .map(|w| w as f32)
                .filter(|w| w.is_finite())
                .map_or(d, |w| w.clamp(*PANE_WIDTHS.start(), *PANE_WIDTHS.end()))
        };
        PanePrefs {
            data: open("data", d.data),
            data_width: width("data_width", d.data_width),
            build: open("build", d.build),
            build_width: width("build_width", d.build_width),
            filters: open("filters", d.filters),
            filters_width: width("filters_width", d.filters_width),
            selection: open("selection", d.selection),
            selection_width: width("selection_width", d.selection_width),
            bookmarks: open("bookmarks", d.bookmarks),
            bookmarks_width: width("bookmarks_width", d.bookmarks_width),
            sync: open("sync", d.sync),
            sync_width: width("sync_width", d.sync_width),
        }
    }

    fn to_json(self) -> Value {
        let open = |b: bool| if b { "open" } else { "collapsed" };
        json!({
            "data": open(self.data),
            "data_width": self.data_width.round(),
            "build": open(self.build),
            "build_width": self.build_width.round(),
            "filters": open(self.filters),
            "filters_width": self.filters_width.round(),
            "selection": open(self.selection),
            "selection_width": self.selection_width.round(),
            "bookmarks": open(self.bookmarks),
            "bookmarks_width": self.bookmarks_width.round(),
            "sync": open(self.sync),
            "sync_width": self.sync_width.round(),
        })
    }
}

/// The UI preferences of `v` only (theme, ribbon, tab, canvas options, and the recent workspace files),
/// with valid values: what a browser may keep. Paths of a disk, servers, tokens and anything else
/// are dropped.
pub fn ui_only(v: &Value) -> Value {
    let p = Prefs::from_json(v);
    let recent: Vec<Value> = p
        .recent
        .iter()
        .filter(|r| r.ws.is_some())
        .map(Recent::to_json)
        .collect();
    json!({
        "theme": p.theme.name(),
        "ribbon": if p.ribbon_collapsed { "collapsed" } else { "open" },
        "tab": p.tab,
        "recent": recent,
        "commands": p.commands,
        "autosave": p.autosave.to_json(),
        "panes": p.panes.to_json(),
        "snap": if p.snap_grid { "grid" } else { "guides" },
        "gridlines": p.show_gridlines,
        "lock": p.lock_objects,
        "guides": if p.hide_guides { "off" } else { "on" },
        "phone": if p.mobile_narrow { "narrow" } else { "off" },
    })
}

impl Prefs {
    pub fn from_json(v: &Value) -> Prefs {
        let recent = v
            .get("recent")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(MAX_RECENT * 4)
            .filter_map(Recent::from_json)
            .take(MAX_RECENT)
            .collect();
        let servers = v
            .get("servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let url = s.get("url")?.as_str()?;
                let ws = s.get("workspace")?.as_str()?;
                (url.len() < 2048 && ws.len() < 256).then(|| (url.to_string(), ws.to_string()))
            })
            .take(MAX_SERVERS)
            .collect();
        Prefs {
            recent,
            servers,
            theme: Brightness::from_name(v.get("theme").and_then(Value::as_str).unwrap_or("")),
            report_contrast: v.get("report_contrast").and_then(Value::as_bool) == Some(true),
            ribbon_collapsed: v.get("ribbon").and_then(Value::as_str) == Some("collapsed"),
            tab: v
                .get("tab")
                .and_then(Value::as_str)
                .filter(|t| TABS.contains(t))
                .unwrap_or("")
                .to_string(),
            commands: v
                .get("commands")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| command_id(id))
                .take(MAX_RECENT_COMMANDS)
                .map(str::to_string)
                .collect(),
            autosave: Autosave::from_json(v.get("autosave")),
            panes: PanePrefs::from_json(v.get("panes")),
            show_gridlines: v.get("gridlines").and_then(Value::as_bool) == Some(true),
            snap_grid: v.get("snap").and_then(Value::as_str) == Some("grid"),
            lock_objects: v.get("lock").and_then(Value::as_bool) == Some(true),
            hide_guides: v.get("guides").and_then(Value::as_str) == Some("off"),
            mobile_narrow: v.get("phone").and_then(Value::as_str) == Some("narrow"),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "recent": self.recent.iter().map(Recent::to_json).collect::<Vec<_>>(),
            "servers": self.servers.iter().map(|(u, w)| json!({"url": u, "workspace": w})).collect::<Vec<_>>(),
            "theme": self.theme.name(),
            "report_contrast": self.report_contrast,
            "ribbon": if self.ribbon_collapsed { "collapsed" } else { "open" },
            "tab": self.tab,
            "commands": self.commands,
            "autosave": self.autosave.to_json(),
            "panes": self.panes.to_json(),
            "snap": if self.snap_grid { "grid" } else { "guides" },
            "gridlines": self.show_gridlines,
            "lock": self.lock_objects,
            "guides": if self.hide_guides { "off" } else { "on" },
            "phone": if self.mobile_narrow { "narrow" } else { "off" },
        })
    }

    /// Puts command `id` first in the recent commands.
    pub fn add_command(&mut self, id: &str) {
        if !command_id(id) {
            return;
        }
        self.commands.retain(|c| c != id);
        self.commands.insert(0, id.to_string());
        self.commands.truncate(MAX_RECENT_COMMANDS);
    }

    /// Puts a file first in the recent files (after the pinned ones; a pinned file stays pinned
    /// and goes first).
    fn touch(&mut self, entry: Recent) {
        let pinned = self.recent.iter().any(|r| r.same(&entry) && r.pinned);
        self.recent.retain(|r| !r.same(&entry));
        let at = if pinned {
            0
        } else {
            self.recent.iter().take_while(|r| r.pinned).count()
        };
        let at = at.min(self.recent.len());
        self.recent.insert(at, Recent { pinned, ..entry });
        self.recent.truncate(MAX_RECENT);
    }

    /// Puts `path` first in the recent files, opened at `now` (seconds since 1970).
    pub fn add_recent(&mut self, path: &Path, now: u64) {
        self.touch(Recent {
            path: path.display().to_string(),
            ws: None,
            opened: now,
            pinned: false,
        });
    }

    /// Puts the workspace file `rel` of workspace `ws` first in the recent files (ignored unless
    /// both are valid: a browser keeps nothing else).
    pub fn add_ws_recent(&mut self, ws: &str, rel: &str, now: u64) {
        if valid_workspace(ws) && safe_relative(rel) {
            self.touch(Recent {
                path: rel.to_string(),
                ws: Some(ws.to_string()),
                opened: now,
                pinned: false,
            });
        }
    }

    /// Removes a recent file ("Remove from Recent"); the file itself is not touched.
    pub fn remove_recent(&mut self, r: &Recent) {
        self.recent.retain(|x| !x.same(r));
    }

    /// Pins or unpins a recent file: pinned files come first.
    pub fn toggle_pin(&mut self, r: &Recent) {
        if let Some(x) = self.recent.iter_mut().find(|x| x.same(r)) {
            x.pinned = !x.pinned;
        }
        // stable: pinned first, each group keeps its order
        let (mut a, b): (Vec<Recent>, Vec<Recent>) = self.recent.drain(..).partition(|x| x.pinned);
        a.extend(b);
        self.recent = a;
    }

    /// The recent desktop paths (not workspace files).
    pub fn recent_paths(&self) -> Vec<PathBuf> {
        self.recent
            .iter()
            .filter(|r| r.ws.is_none())
            .map(|r| PathBuf::from(&r.path))
            .collect()
    }

    /// Puts a server first in the recent servers.
    pub fn add_server(&mut self, url: &str, workspace: &str) {
        self.servers.retain(|(u, w)| !(u == url && w == workspace));
        self.servers.insert(0, (url.to_string(), workspace.to_string()));
        self.servers.truncate(MAX_SERVERS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_damaged_values() {
        let mut p = Prefs::default();
        for i in 0..60 {
            p.add_recent(Path::new(&format!("f{i}.pbix")), i);
        }
        p.add_recent(Path::new("f15.pbix"), 99);
        assert_eq!(p.recent.len(), MAX_RECENT);
        assert_eq!((p.recent[0].path.as_str(), p.recent[0].opened), ("f15.pbix", 99));
        let second = p.recent[1].clone();
        p.toggle_pin(&second);
        assert!(p.recent[0].pinned && p.recent[0].path == second.path);
        p.add_recent(Path::new("new.pbix"), 100);
        assert_eq!(p.recent[1].path, "new.pbix", "after the pinned one");
        p.add_recent(Path::new(&second.path), 101);
        assert!(p.recent[0].pinned && p.recent[0].opened == 101, "stays pinned");
        p.remove_recent(&second);
        assert!(!p.recent.iter().any(|r| r.path == second.path));
        p.add_server("https://h", "a");
        p.show_gridlines = true;
        p.snap_grid = true;
        p.lock_objects = true;
        p.hide_guides = true;
        p.mobile_narrow = true;
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        assert!(!p.to_json().to_string().contains("token"));
        assert_eq!(
            Prefs::from_json(&json!({"recent": 3, "servers": [{"url": 1}]})),
            Prefs::default()
        );
        // the format before 2026-11-04: bare paths
        let old = Prefs::from_json(&json!({"recent": ["a.pbix", "", 5, {"path": 3}]}));
        assert_eq!(old.recent_paths(), vec![PathBuf::from("a.pbix")]);
        // workspace entries with an unsafe name or workspace are dropped
        let ws = Prefs::from_json(
            &json!({"recent": [{"path": "../x", "ws": "a"}, {"path": "x.pbix", "ws": "../a"}]}),
        );
        assert!(ws.recent.is_empty());
        let mut w = Prefs::default();
        w.add_ws_recent("sales", "Sales.pbip", 5);
        w.add_ws_recent("sales", "C:/x.pbix", 5);
        assert_eq!(w.recent.len(), 1);
        assert_eq!(Prefs::from_json(&w.to_json()), w);
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    #[test]
    fn ui_preferences_round_trip_and_default_to_light() {
        assert_eq!(Prefs::default().theme, Brightness::Light);
        let p = Prefs {
            theme: Brightness::Dark,
            ribbon_collapsed: true,
            tab: "model".into(),
            show_gridlines: true,
            ..Prefs::default()
        };
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        for bad in [
            json!({"theme": "purple", "tab": "x", "ribbon": 3}),
            json!(null),
            json!([1]),
        ] {
            let q = Prefs::from_json(&bad);
            assert_eq!(
                (q.theme, q.ribbon_collapsed, q.tab.as_str()),
                (Brightness::Light, false, "")
            );
        }
    }

    /// What a browser keeps: the UI keys only and workspace-relative recent names, never a path
    /// of a disk, a server or a token.
    #[test]
    fn ui_only_drops_everything_else() {
        let mut p = Prefs {
            theme: Brightness::Dark,
            ..Prefs::default()
        };
        p.add_recent(Path::new(r"C:\secret\Sales.pbix"), 1);
        p.add_ws_recent("ws1", "reports/Shop.pbip", 2);
        p.add_server("https://bi.example.com", "sales");
        let mut v = p.to_json();
        if let Some(o) = v.as_object_mut() {
            o.insert("token".into(), json!("abc"));
        }
        let kept = ui_only(&v);
        let keys: Vec<&String> = kept.as_object().map(|o| o.keys().collect()).unwrap_or_default();
        assert!(keys.iter().all(|k| UI_KEYS.contains(&k.as_str())), "{keys:?}");
        let s = kept.to_string();
        for leak in ["secret", "Sales", "example", "sales", "abc"] {
            assert!(!s.contains(leak), "{s}");
        }
        assert_eq!(kept["theme"], "dark");
        let back = Prefs::from_json(&kept);
        assert_eq!(back.theme, Brightness::Dark);
        assert_eq!(back.recent.len(), 1);
        assert_eq!(back.recent[0].path, "reports/Shop.pbip");
    }
}

/// What a recent command id may look like: dotted lower-case words (`measure.add`), at most 64
/// bytes.
fn command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.contains('.')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn recent_commands_are_kept_most_recent_first() {
        let mut p = Prefs::default();
        for i in 0..20 {
            p.add_command(&format!("a.c{i}"));
        }
        p.add_command("a.c5");
        p.add_command("not an id");
        p.add_command("https://x.y/z");
        assert_eq!(p.commands.len(), MAX_RECENT_COMMANDS);
        assert_eq!(p.commands[0], "a.c5");
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        assert_eq!(Prefs::from_json(&ui_only(&p.to_json())).commands, p.commands);
        let bad = Prefs::from_json(&json!({"commands": ["ok.id", 3, "../x", "UPPER.x"]}));
        assert_eq!(bad.commands, vec!["ok.id".to_string()]);
    }
}

#[cfg(test)]
mod pane_tests {
    use super::*;

    /// The panes' state round-trips (the browser keeps it too); widths are kept within the
    /// allowed range; damaged values give the defaults.
    #[test]
    fn pane_state_and_widths_are_kept() {
        let mut p = Prefs::default();
        assert!(p.panes.data && p.panes.build, "both panes open by default");
        p.panes = PanePrefs {
            data: false,
            data_width: 300.0,
            build: true,
            build_width: 222.0,
            filters: false,
            filters_width: 300.0,
            selection: true,
            selection_width: 300.0,
            bookmarks: true,
            bookmarks_width: 300.0,
            sync: true,
            sync_width: 300.0,
        };
        assert_eq!(Prefs::from_json(&p.to_json()).panes, p.panes);
        assert_eq!(Prefs::from_json(&ui_only(&p.to_json())).panes, p.panes);
        let wide = Prefs::from_json(&json!({"panes": {"data_width": 1e9, "build_width": -4}}));
        assert_eq!(wide.panes.data_width, *PANE_WIDTHS.end());
        assert_eq!(wide.panes.build_width, *PANE_WIDTHS.start());
        for bad in [
            json!({"panes": 3}),
            json!({"panes": {"data": 1, "data_width": "x", "build": "maybe"}}),
            json!({"panes": null}),
        ] {
            assert_eq!(Prefs::from_json(&bad).panes, PanePrefs::default(), "{bad}");
        }
    }
}
