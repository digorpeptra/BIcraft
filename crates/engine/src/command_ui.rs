//! Where each command's button lives in the app, its shortcut, when it can run, and the form of
//! its parameters (ROADMAP M11.1, AGENTS.md §9 "every command has its button").
//!
//! Every row of [`crate::COMMANDS`] declares these next to its `server` policy, so a new command
//! has to decide them:
//!
//! - `place`: a ribbon tab and group ([`Place::ribbon`]), a context menu ([`Place::menu`]) or the
//!   command palette only ([`Place::palette`]), with a short UI label and an icon name (the
//!   kebab-case names of the UI's code-drawn icons; `docs/design.md` §6);
//! - `shortcut`: an optional key chord (`Ctrl+S`, `F5`, `Delete`), unique across the table;
//! - `enabled`: what the session needs before the button is enabled ([`Need`]), checked without
//!   side effects by [`Session::command_enabled`], which adds the server policy and the caller's
//!   role;
//! - `form`: the parameters as a small schema ([`Param`]) the palette builds its form from (M11.2).
//!
//! `list_commands` / `describe` / MCP show them (`place`, `shortcut`, `form`, and with a session
//! `enabled` / `disabled_reason`); `docs/commands-ui.md` is the placement table, written by
//! `cargo xtask commands-ui` from this table and checked by `cargo xtask docs`.

use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};

use crate::sandbox::{Access, Effect};
use crate::{Session, Source};

/// A ribbon tab, in the order the tab row shows them. `File` is the backstage (Power BI
/// Desktop's File menu); `Visual`, `Table` and `Measure` are contextual tabs that appear with a
/// selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tab {
    File,
    Home,
    Insert,
    Report,
    Model,
    Data,
    Analyze,
    View,
    Visual,
    Table,
    Measure,
}

impl Tab {
    pub const ALL: [Tab; 11] = [
        Tab::File,
        Tab::Home,
        Tab::Insert,
        Tab::Report,
        Tab::Model,
        Tab::Data,
        Tab::Analyze,
        Tab::View,
        Tab::Visual,
        Tab::Table,
        Tab::Measure,
    ];

    /// The stable name (`"home"`, `"visual"`).
    pub fn name(self) -> &'static str {
        match self {
            Tab::File => "file",
            Tab::Home => "home",
            Tab::Insert => "insert",
            Tab::Report => "report",
            Tab::Model => "model",
            Tab::Data => "data",
            Tab::Analyze => "analyze",
            Tab::View => "view",
            Tab::Visual => "visual",
            Tab::Table => "table",
            Tab::Measure => "measure",
        }
    }

    /// The tab row's caption.
    pub fn caption(self) -> &'static str {
        match self {
            Tab::File => "File",
            Tab::Home => "Home",
            Tab::Insert => "Insert",
            Tab::Report => "Report",
            Tab::Model => "Model",
            Tab::Data => "Data",
            Tab::Analyze => "Analyze",
            Tab::View => "View",
            Tab::Visual => "Visual",
            Tab::Table => "Table",
            Tab::Measure => "Measure",
        }
    }

    /// Whether the tab appears only with a selection (a visual, a table or column, a measure).
    pub fn contextual(self) -> bool {
        matches!(self, Tab::Visual | Tab::Table | Tab::Measure)
    }
}

/// A context menu (right click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Menu {
    /// A page tab under the canvas or a page in the pages pane.
    Page,
    /// A visual on the canvas.
    Visual,
    /// A table, column, measure, hierarchy or other object in the fields pane or model diagram.
    ModelObject,
}

impl Menu {
    pub const ALL: [Menu; 3] = [Menu::Page, Menu::Visual, Menu::ModelObject];

    pub fn name(self) -> &'static str {
        match self {
            Menu::Page => "page",
            Menu::Visual => "visual",
            Menu::ModelObject => "model_object",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            Menu::Page => "Page tab",
            Menu::Visual => "Visual",
            Menu::ModelObject => "Model object",
        }
    }
}

/// Where a button lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// A group of a ribbon tab.
    Ribbon { tab: Tab, group: &'static str },
    /// A context menu.
    Menu(Menu),
    /// Only the command palette (Ctrl+Shift+P).
    Palette,
}

/// A command's button: where it is, its short label and its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub at: At,
    /// The button's text (`"Save as"`), shorter than the command's label.
    pub label: &'static str,
    /// A kebab-case icon name (`"folder"`), drawn by the UI.
    pub icon: &'static str,
}

impl Place {
    /// A button of ribbon tab `tab`, group `group`.
    pub const fn ribbon(tab: Tab, group: &'static str, label: &'static str, icon: &'static str) -> Self {
        Self {
            at: At::Ribbon { tab, group },
            label,
            icon,
        }
    }

    /// An item of context menu `menu`.
    pub const fn menu(menu: Menu, label: &'static str, icon: &'static str) -> Self {
        Self {
            at: At::Menu(menu),
            label,
            icon,
        }
    }

    /// Only in the command palette.
    pub const fn palette(label: &'static str, icon: &'static str) -> Self {
        Self {
            at: At::Palette,
            label,
            icon,
        }
    }
}

impl Serialize for Place {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        match self.at {
            At::Ribbon { tab, group } => {
                m.serialize_entry("kind", "ribbon")?;
                m.serialize_entry("tab", tab.name())?;
                m.serialize_entry("contextual", &tab.contextual())?;
                m.serialize_entry("group", group)?;
            }
            At::Menu(menu) => {
                m.serialize_entry("kind", "menu")?;
                m.serialize_entry("menu", menu.name())?;
            }
            At::Palette => m.serialize_entry("kind", "palette")?,
        }
        m.serialize_entry("label", self.label)?;
        m.serialize_entry("icon", self.icon)?;
        m.end()
    }
}

/// What the session needs before a command's button is enabled. Checked in order; the first one
/// missing gives the reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// An open file (PBIX, PBIT, PBIP or model.bim).
    File,
    /// An open PBIX/PBIT file or PBIP project (not a standalone model.bim, not a new model).
    SavedReportFile,
    /// An open PBIP project.
    Project,
    /// Data `model.refresh` can refresh: an open PBIP project or PBIX file (not a PBIT, a thin
    /// report or a model.bim).
    Refreshable,
    /// A semantic model (an open file's, or one started with `model.new`).
    Model,
    /// A report.
    Report,
    /// No report yet (`report.new`).
    NoReport,
    /// Stored data: a PBIX DataModel, or a PBIP project's `.pbi/cache.abf` (or the session's
    /// explicit cache file).
    Data,
    /// An edit to undo.
    Undo,
    /// An undone edit to redo.
    Redo,
}

impl Need {
    pub fn name(self) -> &'static str {
        match self {
            Need::File => "file",
            Need::SavedReportFile => "saved_report_file",
            Need::Project => "project",
            Need::Refreshable => "refreshable",
            Need::Model => "model",
            Need::Report => "report",
            Need::NoReport => "no_report",
            Need::Data => "data",
            Need::Undo => "undo",
            Need::Redo => "redo",
        }
    }

    /// Why the button is disabled when this is missing.
    pub fn reason(self) -> &'static str {
        match self {
            Need::File => "open a file first",
            Need::SavedReportFile => "needs a saved PBIX, PBIT or PBIP file",
            Need::Project => "needs an open PBIP project",
            Need::Refreshable => "needs an open PBIP project or PBIX file",
            Need::Model => "needs a model: open a file or start a new model",
            Need::Report => "needs a report",
            Need::NoReport => "the session already has a report",
            Need::Data => "needs stored data (a PBIX DataModel or a PBIP project's cache.abf)",
            Need::Undo => "nothing to undo",
            Need::Redo => "nothing to redo",
        }
    }

    /// Whether `s` has it. Reads the session only (and, for [`Need::Data`], whether a cache file
    /// exists); never changes anything.
    pub fn met(self, s: &Session) -> bool {
        match self {
            Need::File => s.source.is_some(),
            Need::SavedReportFile => {
                matches!(s.source, Some(Source::Package { .. } | Source::Project { .. }))
            }
            Need::Project => matches!(s.source, Some(Source::Project { .. })),
            Need::Refreshable => match &s.source {
                Some(Source::Project { .. }) => true,
                Some(Source::Package { package, .. }) => {
                    package.get(bicraft_pbix::DATA_MODEL_SCHEMA).is_none()
                        && (package.get(bicraft_pbix::DATA_MODEL).is_some() || package.has_deferred())
                }
                _ => false,
            },
            Need::Model => s.model.is_some(),
            Need::Report => s.report.is_some(),
            Need::NoReport => s.report.is_none(),
            Need::Data => match &s.source {
                Some(Source::Package { package, .. }) => {
                    package.get(bicraft_pbix::DATA_MODEL).is_some() || package.has_deferred()
                }
                Some(Source::Project { .. }) => crate::cache_abf::path(s).is_ok_and(|p| p.is_some()),
                _ => false,
            },
            Need::Undo => s.history.can_undo(),
            Need::Redo => s.history.can_redo(),
        }
    }
}

/// Who asks: an editor, or a read-only caller (a `reader` grant of `bicraft serve`,
/// `docs/auth.md` "Roles").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Editor,
    Reader,
}

/// A model or report object a name parameter refers to, so the form can offer a picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obj {
    Table,
    Column,
    Measure,
    Hierarchy,
    Partition,
    Expression,
    Relationship,
    Role,
    Culture,
    CalculationGroup,
    CalculationItem,
    Page,
    Visual,
    ReportMeasure,
    /// A visual's data role (`Values`, `Category`, `Y`).
    DataRole,
    /// A visual type (`clusteredColumnChart`).
    VisualType,
    /// An environment profile of bicraft.env.yaml.
    Profile,
}

impl Obj {
    pub fn name(self) -> &'static str {
        match self {
            Obj::Table => "table",
            Obj::Column => "column",
            Obj::Measure => "measure",
            Obj::Hierarchy => "hierarchy",
            Obj::Partition => "partition",
            Obj::Expression => "expression",
            Obj::Relationship => "relationship",
            Obj::Role => "role",
            Obj::Culture => "culture",
            Obj::CalculationGroup => "calculation_group",
            Obj::CalculationItem => "calculation_item",
            Obj::Page => "page",
            Obj::Visual => "visual",
            Obj::ReportMeasure => "report_measure",
            Obj::DataRole => "data_role",
            Obj::VisualType => "visual_type",
            Obj::Profile => "profile",
        }
    }
}

/// The kind of value a parameter takes, for the form's widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Free text (a new name, a description, a format string).
    Text,
    /// The name of an existing object.
    Name(Obj),
    /// A field: `Table[Column]`, `[Measure]` or `Sum(Table[Column])`.
    Field,
    /// DAX text.
    Dax,
    /// Power Query (M) text.
    M,
    /// DAX or M, depending on another parameter (`partition.set {kind}`).
    Code,
    /// A file or folder path (resolved under the workspace in server mode).
    Path,
    /// A path or a list of paths (or, for some, an inline object).
    Paths,
    /// A file path or inline text (`table.import {csv}`).
    PathOrText,
    Bool,
    Integer,
    Number,
    /// One of these values.
    Enum(&'static [&'static str]),
    /// A colour (`#RRGGBB`).
    Color,
    /// A list (of names, fields or values).
    List,
    /// A JSON object (a spec, a theme, a mapping) or one of several shapes.
    Json,
}

/// One parameter of a command's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// What the command does when the parameter is left out, as JSON text (`"true"`), so the
    /// form can show it. `None`: no fixed default (left out means "unchanged", or it depends on
    /// the file or the other values).
    pub default: Option<&'static str>,
}

impl Param {
    /// A required parameter.
    pub const fn req(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            kind,
            required: true,
            default: None,
        }
    }

    /// An optional parameter.
    pub const fn opt(name: &'static str, kind: Kind) -> Self {
        Self {
            name,
            kind,
            required: false,
            default: None,
        }
    }

    /// An optional boolean the command reads as `default` when it is left out.
    pub const fn flag(name: &'static str, default: bool) -> Self {
        Self {
            name,
            kind: Kind::Bool,
            required: false,
            default: Some(if default { "true" } else { "false" }),
        }
    }

    /// The same parameter with the value the command uses when it is left out (JSON text).
    pub const fn with_default(self, default: &'static str) -> Self {
        Self {
            default: Some(default),
            ..self
        }
    }
}

impl Serialize for Param {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("name", self.name)?;
        m.serialize_entry("kind", self.kind.name())?;
        match self.kind {
            Kind::Name(o) => m.serialize_entry("object", o.name())?,
            Kind::Enum(values) => m.serialize_entry("values", values)?,
            _ => {}
        }
        m.serialize_entry("required", &self.required)?;
        if let Some(d) = self
            .default
            .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
        {
            m.serialize_entry("default", &d)?;
        }
        m.end()
    }
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Name(_) => "name",
            Kind::Field => "field",
            Kind::Dax => "dax",
            Kind::M => "m",
            Kind::Code => "code",
            Kind::Path => "path",
            Kind::Paths => "paths",
            Kind::PathOrText => "path_or_text",
            Kind::Bool => "bool",
            Kind::Integer => "integer",
            Kind::Number => "number",
            Kind::Enum(_) => "enum",
            Kind::Color => "color",
            Kind::List => "list",
            Kind::Json => "json",
        }
    }
}

/// Shortcuts the UI keeps for itself; no command may take them.
pub const UI_SHORTCUTS: &[(&str, &str)] = &[("Ctrl+Shift+P", "command palette")];

impl Session {
    /// Whether command `id` can run now for `caller` (`Ok`), or why not: unknown command, refused
    /// by the server policy, not open to a read-only caller, or something the session lacks
    /// ([`Need`]). Reads the session only; the UI greys out a button with the reason as tooltip.
    /// A command that is enabled can still fail on its parameters.
    pub fn command_enabled(&self, id: &str, caller: Caller) -> Result<(), String> {
        let Some(cmd) = crate::find_command(id) else {
            return Err(format!("unknown command `{id}`"));
        };
        if self.server_config().is_some()
            && let Access::Denied(why) = cmd.server.access
        {
            return Err(format!("not available on a server: {why}"));
        }
        if caller == Caller::Reader {
            match cmd.server.effect {
                Effect::Mutating => return Err("read-only access: this command changes the session".into()),
                Effect::Opens if self.dirty || self.report_dirty => {
                    return Err("read-only access: the open file has unsaved changes".into());
                }
                _ => {}
            }
        }
        match cmd.enabled.iter().find(|n| !n.met(self)) {
            Some(n) => Err(n.reason().to_string()),
            None => Ok(()),
        }
    }
}
