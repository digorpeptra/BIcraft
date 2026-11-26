//! The BIcraft app's UI, one for every shell (docs/webui.md, "One UI for the desktop and the
//! browser"): the window ([`App`]), its report pages drawn by `bicraft-render` at the window's
//! size and DPI with their hit maps, the Model and Validation views, the ribbon built from the
//! engine's command registry (`registry`, `ribbon`, `menus`), the fonts, the chrome's language.
//! The native window (`apps/bicraft-desktop`) and the browser page (`apps/bicraft-web`) are thin
//! shells: each gives a [`shell::Shell`] (its data sources, file dialog, settings, title)
//! and runs [`App::logic`] and [`App::ui`] on every frame.
//!
//! Clicks on a page are mapped with the renderer's hit map to the same `view.interact` actions
//! the viewer sends, so a slicer click or a cross-filter gives the viewer's result
//! (`docs/desktop.md`, "Interactions"). No file, thread or socket is needed here: the crate builds
//! for wasm32 (`cargo xtask webui check`).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod actions;
pub mod analytics_pane;
pub mod analyze_ui;
pub mod answer;
pub mod app;
pub mod autodetect;
pub mod backstage;
pub mod bookmarks_pane;
pub mod canvas;
pub mod conditional_dialog;
pub mod control;
pub mod custom_visuals;
pub mod data_filter;
pub mod data_format_card;
pub mod data_view;
pub mod dax_box;
pub mod dax_query;
pub mod dax_text;
pub mod decomp_ui;
pub mod diagram;
pub mod drill;
pub mod enter_data;
pub mod export_dialog;
pub mod filter_pane_ui;
pub mod filters_pane;
pub mod focus;
pub mod fonts;
pub mod form;
pub mod format_pane;
pub mod formula;
pub mod get_data;
pub mod group_dialog;
pub mod i18n;
pub mod icons;
pub mod interactions;
pub mod key_tips;
pub mod keyboard;
pub mod m_box;
pub mod menus;
pub mod mobile;
pub mod model_explorer;
pub mod model_view;
pub mod offscreen;
pub mod optimize_ui;
pub mod options_page;
pub mod page_tabs;
pub mod pages;
pub mod palette;
pub mod panes;
pub mod param_dialog;
pub mod perf_pane;
pub mod perspective_dialog;
pub mod point_keys;
pub mod pq_deps;
pub mod pq_editor;
pub mod pq_tabs;
pub mod prefs;
pub mod props_pane;
pub mod quick_dialog;
pub mod recovery;
pub mod refresh_policy_dialog;
pub mod registry;
pub mod relationships_dialog;
pub mod remote;
pub mod ribbon;
pub mod rich;
pub mod rich_text;
pub mod role_filter;
pub mod roles_dialog;
pub mod selection_pane;
pub mod shell;
pub mod shortcuts_list;
pub mod source;
pub mod source_settings;
pub mod storage_ui;
pub mod sync_pane;
pub mod theme;
pub mod theme_dialog;
pub mod themes_ui;
pub mod tmdl_view;
pub mod view_as;
pub mod viscalc_dialog;
pub mod widgets;

pub use app::App;

#[cfg(test)]
mod tests;
