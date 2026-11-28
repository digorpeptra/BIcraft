//! Server mode (web stage 1, lot 1; `docs/server-mode.md`): a workspace root that every
//! client-supplied filesystem path is resolved under, and the per-command policy
//! ([`ServerPolicy`]) that refuses what a server must not do (launch OS processes) and gates what
//! needs a deployment setting (network access).
//!
//! A [`Session`] built with [`Session::new`] is in local mode: no root, no policy, every command
//! behaves exactly as before. [`Session::server`] (or [`Session::from_env`] with
//! `BICRAFT_SERVER_ROOT` set) turns server mode on for the life of the session.
//!
//! Path rules (server mode): a path parameter is relative to the root; absolute paths, drive
//! letters, UNC and device paths (`\\server\share`, `\\?\`, `\\.\`), `..` components, NUL
//! bytes, and (on Windows) alternate data streams (`:`), reserved device names (`CON`, `NUL`,
//! `COM1`…) and components ending in a dot or a space are refused. `/` and `\` both separate
//! components on every OS. The longest existing prefix of the joined path is canonicalized
//! (symbolic links and junctions resolved) and must stay under the canonical root; the rest
//! (a save target that does not exist yet) must not exist at all, so a dangling link is refused.

use std::borrow::Cow;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::{Command, Session};

/// Environment variable that turns server mode on: the workspace root folder.
pub const SERVER_ROOT_ENV: &str = "BICRAFT_SERVER_ROOT";
/// Environment variable that allows network access in server mode (`1`/`true`/`yes`/`on`):
/// the Databricks connector of `model.refresh`. Off by default.
pub const SERVER_NETWORK_ENV: &str = "BICRAFT_SERVER_NETWORK";

/// Longest path parameter accepted, in bytes.
const MAX_PATH_BYTES: usize = 4096;
/// Most components in a path parameter.
const MAX_COMPONENTS: usize = 256;
/// Most entries visited when a project folder is checked for links leaving the root.
const MAX_TREE_ENTRIES: usize = 200_000;
/// Deepest folder level visited by that check.
const MAX_TREE_DEPTH: usize = 64;

/// What a command may do in server mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Runs in server mode.
    Allowed,
    /// Refused in server mode; the text says why.
    Denied(&'static str),
    /// May open network connections; the command checks [`ServerConfig::network_allowed`] at
    /// the point where it would connect (a run that needs no connection is unaffected).
    Network,
}

/// How a path parameter is resolved in server mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A path (string). Rewritten to the resolved path before the command runs.
    One,
    /// A path, or `null` / `""` meaning "none" (left as is). Rewritten like [`PathKind::One`].
    Clearable,
    /// A path or a list of paths (strings). Other values (inline objects) are left as is.
    Many,
    /// A file path or inline text (`table.import {csv}`): the command resolves a single-line
    /// value itself and reads it only when it is an existing file under the root.
    PathOrText,
    /// Passed unchanged to a private server-mode session that resolves it (`file.convert`).
    Delegated,
}

/// A parameter that names a file or folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathParam {
    pub name: &'static str,
    pub kind: PathKind,
}

impl PathParam {
    pub const fn one(name: &'static str) -> Self {
        Self {
            name,
            kind: PathKind::One,
        }
    }
    pub const fn clearable(name: &'static str) -> Self {
        Self {
            name,
            kind: PathKind::Clearable,
        }
    }
    pub const fn many(name: &'static str) -> Self {
        Self {
            name,
            kind: PathKind::Many,
        }
    }
    pub const fn path_or_text(name: &'static str) -> Self {
        Self {
            name,
            kind: PathKind::PathOrText,
        }
    }
    pub const fn delegated(name: &'static str) -> Self {
        Self {
            name,
            kind: PathKind::Delegated,
        }
    }
}

/// A command's server-mode policy: whether it runs, and which parameters are paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerPolicy {
    pub access: Access,
    pub paths: &'static [PathParam],
    /// What the command does to the session and the workspace, for read-only callers
    /// ([`Effect`]). Every constructor says [`Effect::Mutating`]: a command is open to readers
    /// only when its row says so ([`ServerPolicy::read_only`] and the like).
    pub effect: Effect,
}

/// What a command does, as read-only callers (`reader` grants of `bicraft-cli serve`,
/// `docs/auth.md` "Roles") see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Changes the model, the report, the session's settings or view state, writes or deletes
    /// files, or uses the network. The default: refused to readers and hidden from them.
    Mutating,
    /// Reads only: the model, the report, the open file's data and files under the workspace.
    /// A run that names one of `unless` (with any value, `null` too) writes or changes something
    /// and counts as mutating (`report.render {out}`).
    ReadOnly { unless: &'static [&'static str] },
    /// Replaces the session's open file (`file.open`): a reader may run it only while the
    /// session holds no unsaved changes, so it never discards an editor's work.
    Opens,
}

impl Effect {
    /// Whether readers see the command at all (`list_commands`, `tools/list`).
    pub fn readable(self) -> bool {
        !matches!(self, Effect::Mutating)
    }
}

impl ServerPolicy {
    /// Runs in server mode and takes no filesystem path.
    pub const ALLOWED: Self = Self {
        access: Access::Allowed,
        paths: &[],
        effect: Effect::Mutating,
    };

    /// Runs in server mode; `paths` are resolved under the workspace root.
    pub const fn paths(paths: &'static [PathParam]) -> Self {
        Self {
            access: Access::Allowed,
            paths,
            effect: Effect::Mutating,
        }
    }

    /// Refused in server mode.
    pub const fn denied(reason: &'static str) -> Self {
        Self {
            access: Access::Denied(reason),
            paths: &[],
            effect: Effect::Mutating,
        }
    }

    /// May open network connections (gated by the server's network setting); `paths` as for
    /// [`ServerPolicy::paths`].
    pub const fn network(paths: &'static [PathParam]) -> Self {
        Self {
            access: Access::Network,
            paths,
            effect: Effect::Mutating,
        }
    }

    /// This policy for a command that only reads ([`Effect::ReadOnly`]).
    #[must_use]
    pub const fn read_only(self) -> Self {
        self.read_only_unless(&[])
    }

    /// This policy for a command that only reads unless one of `unless` is given.
    #[must_use]
    pub const fn read_only_unless(mut self, unless: &'static [&'static str]) -> Self {
        self.effect = Effect::ReadOnly { unless };
        self
    }

    /// This policy for a command that replaces the open file ([`Effect::Opens`]).
    #[must_use]
    pub const fn opens(mut self) -> Self {
        self.effect = Effect::Opens;
        self
    }
}

/// Why a path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    TooLong,
    Nul,
    /// `\\server\share`, `\\?\…`, `\\.\…`, `//…`.
    UncOrDevice,
    /// `C:…`.
    DriveLetter,
    /// `/…` or `\…`.
    Absolute,
    /// A `..` component.
    Parent,
    /// A `:` (alternate data stream) on Windows.
    Stream,
    /// A reserved device name, or a component ending in `.` or ` `, on Windows.
    Reserved(String),
    /// The resolved path (links followed) leaves the root.
    Escapes,
    /// An existing entry on the way cannot be resolved (a dangling link, no permission).
    Unresolvable(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "an empty path"),
            Self::TooLong => write!(
                f,
                "longer than {MAX_PATH_BYTES} bytes or {MAX_COMPONENTS} components"
            ),
            Self::Nul => write!(f, "contains a NUL byte"),
            Self::UncOrDevice => write!(f, "a UNC or device path"),
            Self::DriveLetter => write!(f, "a drive letter"),
            Self::Absolute => write!(f, "an absolute path"),
            Self::Parent => write!(f, "a `..` component"),
            Self::Stream => write!(f, "a `:` (alternate data stream)"),
            Self::Reserved(c) => write!(f, "`{c}` is a reserved Windows name"),
            Self::Escapes => write!(f, "it leaves the workspace root (through a link)"),
            Self::Unresolvable(c) => write!(f, "`{c}` exists but cannot be resolved (a dangling link?)"),
        }
    }
}

impl Refusal {
    /// The classes a single-line `csv` value is refused for (rather than read as CSV text): it
    /// plainly names a file outside the root.
    fn names_outside(&self) -> bool {
        matches!(
            self,
            Self::UncOrDevice | Self::DriveLetter | Self::Absolute | Self::Parent | Self::Nul
        )
    }
}

/// Server-mode settings of a [`Session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// The canonical root (on Windows the `\\?\` form), for containment checks.
    canonical: PathBuf,
    /// The same folder in plain form, for joining and messages.
    root: PathBuf,
    allow_network: bool,
}

impl ServerConfig {
    /// Server mode rooted at `root`, an existing folder. Network access is off.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err("server root: empty path".into());
        }
        let canonical =
            std::fs::canonicalize(root).map_err(|e| format!("server root {}: {e}", root.display()))?;
        if !canonical.is_dir() {
            return Err(format!("server root {}: not a folder", root.display()));
        }
        Ok(Self {
            root: plain(&canonical),
            canonical,
            allow_network: false,
        })
    }

    /// Allows (or not) network access: the Databricks connector of `model.refresh`.
    #[must_use]
    pub fn allow_network(mut self, allow: bool) -> Self {
        self.allow_network = allow;
        self
    }

    /// The workspace root (canonical, plain form).
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn network_allowed(&self) -> bool {
        self.allow_network
    }

    /// Server mode from the environment: `BICRAFT_SERVER_ROOT` (unset or empty: `None`, local
    /// mode) and `BICRAFT_SERVER_NETWORK`.
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_lookup(bicraft_env::var_os)
    }

    pub(crate) fn from_lookup(
        get: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Option<Self>, String> {
        let Some(root) = get(SERVER_ROOT_ENV).filter(|r| !r.is_empty()) else {
            return Ok(None);
        };
        let network = Self::network_from_lookup(&get)?;
        let config = Self::new(PathBuf::from(root)).map_err(|e| format!("{SERVER_ROOT_ENV}: {e}"))?;
        Ok(Some(config.allow_network(network)))
    }

    /// The network setting alone from the environment (`BICRAFT_SERVER_NETWORK`; unset: off), for
    /// servers that pick their roots themselves (`bicraft-cli serve`: one root per workspace).
    pub fn network_from_env() -> Result<bool, String> {
        Self::network_from_lookup(&bicraft_env::var_os)
    }

    pub(crate) fn network_from_lookup(
        get: &impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> Result<bool, String> {
        match get(SERVER_NETWORK_ENV) {
            None => Ok(false),
            Some(v) => match v.to_str().map(|t| t.trim().to_ascii_lowercase()).as_deref() {
                Some("" | "0" | "false" | "no" | "off") => Ok(false),
                Some("1" | "true" | "yes" | "on") => Ok(true),
                _ => Err(format!(
                    "{SERVER_NETWORK_ENV}: expected 1/true/yes/on or 0/false/no/off"
                )),
            },
        }
    }

    /// Resolves a client-supplied path under the root (see the module docs). The result is
    /// absolute and in plain form; what exists of it is canonical.
    pub fn resolve(&self, raw: &str) -> Result<PathBuf, Refusal> {
        let parts = lexical(raw)?;
        let mut existing = self.canonical.clone();
        let mut rest: Vec<&str> = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let next = existing.join(part);
            match std::fs::symlink_metadata(&next) {
                Ok(_) => match std::fs::canonicalize(&next) {
                    Ok(c) => {
                        if !c.starts_with(&self.canonical) {
                            return Err(Refusal::Escapes);
                        }
                        existing = c;
                    }
                    Err(_) => {
                        return Err(Refusal::Unresolvable(
                            parts.get(..=i).unwrap_or_default().join("/"),
                        ));
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // below a file (Windows reports NotFound there, Unix ENOTDIR): not a folder
                    if !existing.is_dir() {
                        return Err(Refusal::Unresolvable(
                            parts.get(..i).unwrap_or_default().join("/"),
                        ));
                    }
                    rest = parts.get(i..).unwrap_or_default().to_vec();
                    break;
                }
                Err(_) => {
                    return Err(Refusal::Unresolvable(
                        parts.get(..=i).unwrap_or_default().join("/"),
                    ));
                }
            }
        }
        let mut out = plain(&existing);
        for part in rest {
            out.push(part);
        }
        Ok(out)
    }

    /// Whether an existing path (a file or folder bicraft found itself, such as a project's item
    /// folder) is under the root once links are resolved.
    pub(crate) fn contains(&self, path: &Path) -> bool {
        std::fs::canonicalize(path).is_ok_and(|c| c.starts_with(&self.canonical))
    }

    /// Refuses a folder (recursively, bounded) holding a link that leaves the root or cannot be
    /// resolved. The folder itself must be under the root.
    pub(crate) fn check_tree(&self, dir: &Path) -> Result<(), String> {
        if !self.contains(dir) {
            return Err(format!(
                "server mode: {} leaves the workspace root",
                self.shown(dir)
            ));
        }
        let mut stack = vec![(dir.to_path_buf(), 0usize)];
        let mut seen = 0usize;
        while let Some((d, depth)) = stack.pop() {
            let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", self.shown(&d)))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("{}: {e}", self.shown(&d)))?;
                seen = seen.saturating_add(1);
                if seen > MAX_TREE_ENTRIES {
                    return Err(format!(
                        "server mode: {} holds more than {MAX_TREE_ENTRIES} entries",
                        self.shown(dir)
                    ));
                }
                let path = entry.path();
                let kind = entry
                    .file_type()
                    .map_err(|e| format!("{}: {e}", self.shown(&path)))?;
                if kind.is_symlink() {
                    if !self.contains(&path) {
                        return Err(format!(
                            "server mode: {} is a link that leaves the workspace root",
                            self.shown(&path)
                        ));
                    }
                } else if kind.is_dir() {
                    if depth >= MAX_TREE_DEPTH {
                        return Err(format!(
                            "server mode: {} is nested deeper than {MAX_TREE_DEPTH} levels",
                            self.shown(&path)
                        ));
                    }
                    stack.push((path, depth.saturating_add(1)));
                }
            }
        }
        Ok(())
    }

    /// Checks the folders and files a project `<dir>/<name>.pbip` occupies (when they exist)
    /// before it is written there.
    pub(crate) fn check_project_target(&self, dir: &Path, name: &str) -> Result<(), String> {
        for item in [
            format!("{name}.pbip"),
            format!("{name}.SemanticModel"),
            format!("{name}.Report"),
        ] {
            let path = dir.join(item);
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() && !self.contains(&path) {
                return Err(format!(
                    "server mode: {} is a link that leaves the workspace root",
                    self.shown(&path)
                ));
            }
            if path.is_dir() {
                self.check_tree(&path)?;
            }
        }
        Ok(())
    }

    /// Checks a project `file.open` located: its folders stay under the root and hold no link
    /// leaving it.
    pub(crate) fn check_project(&self, project: &bicraft_pbip::Project) -> Result<(), String> {
        if !self.contains(&project.root) {
            return Err(format!(
                "server mode: {} leaves the workspace root",
                self.shown(&project.root)
            ));
        }
        for dir in [&project.semantic_model, &project.report].into_iter().flatten() {
            self.check_tree(dir)?;
        }
        Ok(())
    }

    /// A path as messages show it: relative to the root when it is under it.
    fn shown(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.canonical))
            .map_or_else(|_| path.display().to_string(), |p| p.display().to_string())
    }
}

/// The components of a client path, after the lexical checks.
fn lexical(raw: &str) -> Result<Vec<&str>, Refusal> {
    if raw.trim().is_empty() {
        return Err(Refusal::Empty);
    }
    if raw.len() > MAX_PATH_BYTES {
        return Err(Refusal::TooLong);
    }
    if raw.contains('\0') {
        return Err(Refusal::Nul);
    }
    let sep = |c: char| c == '/' || c == '\\';
    let mut chars = raw.chars();
    let (first, second) = (chars.next(), chars.next());
    if first.is_some_and(sep) && second.is_some_and(sep) {
        return Err(Refusal::UncOrDevice);
    }
    if first.is_some_and(|c| c.is_ascii_alphabetic()) && second == Some(':') {
        return Err(Refusal::DriveLetter);
    }
    if first.is_some_and(sep) || Path::new(raw).has_root() || Path::new(raw).is_absolute() {
        return Err(Refusal::Absolute);
    }
    if Path::new(raw)
        .components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err(Refusal::Absolute);
    }
    let parts: Vec<&str> = raw.split(sep).filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.len() > MAX_COMPONENTS {
        return Err(Refusal::TooLong);
    }
    for part in &parts {
        if *part == ".." {
            return Err(Refusal::Parent);
        }
        windows_checks(part)?;
    }
    Ok(parts)
}

#[cfg(windows)]
fn windows_checks(part: &str) -> Result<(), Refusal> {
    if part.contains(':') {
        return Err(Refusal::Stream);
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err(Refusal::Reserved(part.to_string()));
    }
    let stem = part.split('.').next().unwrap_or(part).trim_end();
    let upper = stem.to_ascii_uppercase();
    let reserved = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && stem.chars().count() == 4
        && stem
            .chars()
            .nth(3)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')));
    if reserved {
        return Err(Refusal::Reserved(part.to_string()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn windows_checks(_: &str) -> Result<(), Refusal> {
    Ok(())
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\s\x` → `\\s\x` (Windows); unchanged elsewhere.
fn plain(path: &Path) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            let mut c = rest.chars();
            if c.next().is_some_and(|d| d.is_ascii_alphabetic()) && c.next() == Some(':') {
                return PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

/// A refusal as a command error.
fn refused(name: &str, raw: &str, why: &Refusal) -> String {
    format!(
        "server mode: `{name}` must be a path relative to the workspace root; `{}` is refused: {why}",
        raw.escape_debug()
    )
}

/// The resolved path as a parameter value.
fn path_value(name: &str, path: &Path) -> Result<Value, String> {
    path.to_str()
        .map(|s| Value::String(s.to_string()))
        .ok_or_else(|| format!("server mode: `{name}` resolves to a path that is not valid UTF-8"))
}

/// Server mode, before a command runs: refuses a denied command and rewrites its path parameters
/// to resolved paths. Local mode (`None`): the parameters unchanged.
pub(crate) fn prepare<'a>(
    config: Option<&ServerConfig>,
    cmd: &Command,
    params: &'a Value,
) -> Result<Cow<'a, Value>, String> {
    let Some(config) = config else {
        return Ok(Cow::Borrowed(params));
    };
    if let Access::Denied(why) = cmd.server.access {
        return Err(format!("`{}` is not available in server mode: {why}", cmd.id));
    }
    let Value::Object(map) = params else {
        return Ok(Cow::Borrowed(params));
    };
    let mut out: Option<serde_json::Map<String, Value>> = None;
    for p in cmd.server.paths {
        let Some(v) = map.get(p.name) else { continue };
        let resolve = |raw: &str| {
            config
                .resolve(raw)
                .map_err(|why| refused(p.name, raw, &why))
                .and_then(|path| path_value(p.name, &path))
        };
        let new = match (p.kind, v) {
            (PathKind::PathOrText | PathKind::Delegated, _) => continue,
            (PathKind::Clearable, Value::String(s)) if s.trim().is_empty() => continue,
            (PathKind::One | PathKind::Clearable | PathKind::Many, Value::String(s)) => resolve(s)?,
            (PathKind::Many, Value::Array(items)) if items.iter().all(Value::is_string) => Value::Array(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(resolve)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            // other shapes: the command reports them as before
            _ => continue,
        };
        out.get_or_insert_with(|| map.clone())
            .insert(p.name.to_string(), new);
    }
    Ok(match out {
        Some(m) => Cow::Owned(Value::Object(m)),
        None => Cow::Borrowed(params),
    })
}

/// A `csv` parameter that may be a file: local mode, the path as given when it is a file (as
/// before); server mode, a single-line value resolved under the root when it is an existing
/// file there. `Ok(None)`: CSV text.
pub(crate) fn csv_file(s: &Session, text: &str) -> Result<Option<PathBuf>, String> {
    if text.contains('\n') || text.contains('\r') {
        return Ok(None);
    }
    let Some(config) = s.server_config() else {
        return Ok(Path::new(text).is_file().then(|| PathBuf::from(text)));
    };
    match config.resolve(text) {
        Ok(p) => Ok(p.is_file().then_some(p)),
        Err(why) if why.names_outside() => Err(refused("csv", text, &why)),
        // e.g. `time,12:30` on Windows: CSV text, never read as a file
        Err(_) => Ok(None),
    }
}

/// Refuses a network connection in server mode unless the server allows it.
pub(crate) fn require_network(s: &Session, what: &str) -> Result<(), String> {
    match s.server_config() {
        Some(c) if !c.network_allowed() => Err(format!(
            "server mode: {what} needs network access, which this server does not allow \
             ({SERVER_NETWORK_ENV}=1 or ServerConfig::allow_network)"
        )),
        _ => Ok(()),
    }
}

/// The server policy of command `id`, when it exists.
pub fn server_policy(id: &str) -> Option<ServerPolicy> {
    crate::find_command(id).map(|c| c.server)
}

impl Session {
    /// A session in server mode: every path parameter is resolved under `config`'s root and
    /// commands denied in server mode are refused (`docs/server-mode.md`).
    pub fn server(config: ServerConfig) -> Self {
        Self {
            server: Some(config),
            ..Self::default()
        }
    }

    /// Server mode when `BICRAFT_SERVER_ROOT` is set (see [`ServerConfig::from_env`]), else a
    /// local session ([`Session::new`]).
    pub fn from_env() -> Result<Self, String> {
        Ok(match ServerConfig::from_env()? {
            Some(c) => Self::server(c),
            None => Self::new(),
        })
    }

    /// The server-mode settings; `None` in local mode.
    pub fn server_config(&self) -> Option<&ServerConfig> {
        self.server.as_ref()
    }

    /// Whether command `id` exists and may run in this session's mode: in server mode, commands
    /// whose policy is [`Access::Denied`] are not available (local mode: every command).
    pub fn command_available(&self, id: &str) -> bool {
        crate::find_command(id)
            .is_some_and(|c| self.server.is_none() || !matches!(c.server.access, Access::Denied(_)))
    }

    /// [`crate::list_commands`] without the commands this session's mode refuses
    /// ([`Session::command_available`]): what `list_commands` shows a server-mode client.
    pub fn available_commands(&self, filter: Option<&str>) -> Vec<crate::CommandInfo> {
        self.listed(filter, crate::Caller::Editor)
    }

    /// Whether a read-only caller may run command `id` with `params` now (`Ok`), or why not.
    /// The answer comes from the command's [`Effect`]: mutating commands are refused, read-only
    /// ones are refused only when a run names one of their writing parameters, and `file.open`
    /// is refused while the session holds unsaved changes. An unknown id passes (running it
    /// reports it).
    pub fn reader_check(&self, id: &str, params: &Value) -> Result<(), String> {
        let Some(cmd) = crate::find_command(id) else {
            return Ok(());
        };
        match cmd.server.effect {
            Effect::Mutating => Err(format!(
                "`{id}` is not available to a reader of this workspace: it changes the model, the \
                 report, the session or the workspace's files"
            )),
            Effect::ReadOnly { unless } => {
                let named = unless
                    .iter()
                    .find(|p| params.as_object().is_some_and(|o| o.contains_key(**p)));
                match named {
                    Some(p) => Err(format!(
                        "`{id}` with `{p}` is not available to a reader of this workspace: it \
                         changes the session or writes into the workspace (leave out `{p}`)"
                    )),
                    None => Ok(()),
                }
            }
            Effect::Opens if self.dirty || self.report_dirty => Err(format!(
                "`{id}` is not available to a reader of this workspace now: the open file has \
                 unsaved changes that opening another file would discard"
            )),
            Effect::Opens => Ok(()),
        }
    }

    /// [`Session::available_commands`] without the commands readers never run
    /// ([`Effect::readable`]): what `list_commands` shows a read-only caller.
    pub fn reader_commands(&self, filter: Option<&str>) -> Vec<crate::CommandInfo> {
        self.listed(filter, crate::Caller::Reader)
    }

    /// The commands `caller` sees (filtered as [`crate::list_commands`]), each with whether it can
    /// run now and why not ([`Session::command_enabled`]).
    fn listed(&self, filter: Option<&str>, caller: crate::Caller) -> Vec<crate::CommandInfo> {
        crate::list_commands(filter)
            .into_iter()
            .filter(|c| self.command_available(c.id))
            .filter_map(|c| crate::find_command(c.id))
            .filter(|cmd| caller == crate::Caller::Editor || cmd.server.effect.readable())
            .map(|cmd| cmd.info_for(self, caller))
            .collect()
    }

    /// A fresh private session with the same mode (`file.convert`).
    pub(crate) fn private(&self) -> Self {
        Self {
            server: self.server.clone(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
