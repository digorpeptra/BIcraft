//! Get data › SharePoint folder and SharePoint list with a credential, and Home › Data source
//! settings (credentials and privacy levels), through the accessibility tree (egui_kittest, no
//! GPU) on an engine session, against the in-process SharePoint test site of `bicraft-webdata`
//! (loopback only): each gesture runs its engine command (`source.credentials`,
//! `source.navigate`, `source.load`, `source.settings`, `source.privacy`), and secrets never stay
//! in the sent history.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::backstage_tests::steps;
use super::get_data_ui_tests::{app, click, has_table, last_sent, listed, type_into, until};
use crate::App;

/// The rightmost node labelled `label` (a combo box, not its own label).
fn combo<'a>(h: &'a Harness<'_, App>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label)
        .max_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
        .unwrap()
}

fn pick(h: &mut Harness<'_, App>, combo_label: &str, option: &str) {
    match h.query_by(|n| n.role() == Role::ComboBox && n.label().as_deref() == Some(combo_label)) {
        Some(c) => c.click(),
        None => combo(h, combo_label).click(),
    }
    steps(h, 2);
    h.get_all_by_label(option).last().unwrap().click();
    steps(h, 2);
}

fn secret_kept(h: &Harness<'_, App>) -> bool {
    h.state()
        .sent
        .iter()
        .any(|c| c.params.to_string().contains(bicraft_webdata::fake::SP_TOKEN))
}

/// Get data › `source` with the test site's token; returns the server.
fn connect(h: &mut Harness<'_, App>, source: &str) -> bicraft_webdata::fake::FakeHttp {
    let srv = bicraft_webdata::fake::start_sharepoint().unwrap();
    click(h, "Get data");
    click(h, source);
    type_into(h, "Site URL", &srv.url("/sites/team"));
    pick(h, "Authentication", "Organizational account (access token)");
    type_into(h, "Access token", bicraft_webdata::fake::SP_TOKEN);
    click(h, "Connect");
    srv
}

#[test]
fn get_data_sharepoint_folder_sends_the_token_then_lists_and_loads() {
    let mut h = app("en-US");
    let srv = connect(&mut h, "SharePoint folder");
    let p = last_sent(&h, "source.credentials").expect("Connect stores the credential first");
    assert_eq!(p["kind"], "web");
    assert_eq!(p["authentication"], "bearer");
    assert_eq!(p["url"], json!(srv.url("/sites/team")));
    assert!(!secret_kept(&h), "the token is not kept in the sent history");
    // the form forgot it too
    let d = h.state().cmd.get_data.clone().unwrap();
    assert!(d.auth.token.is_empty());
    until(&mut h, "the items", |a| listed(a, 2));
    let p = last_sent(&h, "source.navigate").unwrap();
    assert_eq!(
        p,
        json!({"kind": "sharepoint_folder", "url": srv.url("/sites/team")})
    );
    h.get_by_label("Load team").click();
    steps(&mut h, 3);
    click(&mut h, "Load");
    let p = last_sent(&h, "source.load").expect("Load runs source.load");
    assert_eq!(p["allow_connectors"], json!(["sharepoint"]), "{p}");
    until(&mut h, "the dialog closes", |a| a.cmd.get_data.is_none());
    until(&mut h, "the table is in the model", |a| has_table(a, "team"));
}

#[test]
fn get_data_sharepoint_list_lists_the_site_lists() {
    let mut h = app("en-US");
    let _srv = connect(&mut h, "SharePoint list");
    until(&mut h, "the lists", |a| listed(a, 2));
    let p = last_sent(&h, "source.navigate").unwrap();
    assert_eq!(p["kind"], "sharepoint_list");
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Tasks  (table)"));
    h.get_by(|n| n.role() == Role::Button && n.label().as_deref() == Some("Documents  (table)"));
}

fn rows(a: &App) -> Vec<crate::source_settings::Row> {
    a.cmd
        .source_settings
        .as_ref()
        .and_then(|d| d.rows.clone())
        .unwrap_or_default()
}

#[test]
fn data_source_settings_edit_and_clear_permissions() {
    let mut h = app("en-US");
    let srv = bicraft_webdata::fake::start_sharepoint().unwrap();
    let site = srv.url("/sites/team");
    let m = format!("Table.SelectColumns(SharePoint.Files(\"{site}\"), {{\"Name\"}})");
    crate::ribbon::invoke(
        h.state_mut(),
        "source.credentials",
        json!({"kind": "web", "url": site, "authentication": "bearer", "token": bicraft_webdata::fake::SP_TOKEN}),
    );
    // a model whose query reads the site (loaded as Get data's Load does)
    h.state_mut().run(
        crate::actions::Call {
            id: "source.load",
            params: json!({"tables": [{"name": "Files", "m": m}], "allow_connectors": ["sharepoint"]}),
        },
        crate::app::Then::Nothing,
    );
    until(&mut h, "the table is in the model", |a| has_table(a, "Files"));
    click(&mut h, "Data source settings");
    assert!(
        last_sent(&h, "source.settings").is_some(),
        "the button runs source.settings"
    );
    until(&mut h, "the list", |a| !rows(a).is_empty());
    let r = rows(h.state());
    let i = r.iter().position(|r| r.source == site).unwrap();
    assert_eq!(r[i].credential.as_deref(), Some("bearer"));
    // the list shows the credential and the level as words
    h.get_by_label("Access token");
    h.get_by_label("Not set");
    click(&mut h, &format!("Web · {site}"));
    click(&mut h, "Edit Permissions…");
    pick(&mut h, "Privacy level", "Organizational");
    click(&mut h, "OK");
    let p = last_sent(&h, "source.privacy").expect("OK runs source.privacy");
    assert_eq!(
        p,
        json!({"kind": "web", "source": site, "level": "organizational"})
    );
    // the kind did not change and nothing was typed: the credential stays
    assert_eq!(
        h.state()
            .sent
            .iter()
            .filter(|c| c.id == "source.credentials")
            .count(),
        1
    );
    until(&mut h, "the level is listed", |a| {
        rows(a)
            .iter()
            .any(|r| r.privacy.as_deref() == Some("organizational"))
    });
    click(&mut h, &format!("Web · {site}"));
    click(&mut h, "Clear Permissions");
    let p = last_sent(&h, "source.credentials").unwrap();
    assert_eq!(p, json!({"kind": "web", "url": site, "clear": true}));
    let p = last_sent(&h, "source.privacy").unwrap();
    assert_eq!(p["clear"], true, "{p}");
    until(&mut h, "the permissions are cleared", |a| {
        rows(a)
            .iter()
            .any(|r| r.source == site && r.credential.is_none() && r.privacy.is_none())
    });
    assert!(!secret_kept(&h));
    click(&mut h, "Close");
    assert!(h.state().cmd.source_settings.is_none());
}

#[test]
fn the_sharepoint_entries_and_settings_speak_french() {
    let mut h = app("fr-FR");
    click(&mut h, "Obtenir des données");
    click(&mut h, "Dossier SharePoint");
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("URL du site"));
    pick(&mut h, "Authentification", "De base");
    h.get_by(|n| n.role() == Role::TextInput && n.label().as_deref() == Some("Nom d'utilisateur"));
    h.get_by(|n| n.role() == Role::PasswordInput && n.label().as_deref() == Some("Mot de passe"));
    click(&mut h, "Liste SharePoint");
    h.state_mut().cmd.get_data = None;
    steps(&mut h, 3);
    click(&mut h, "Paramètres de source de données");
    until(&mut h, "the list", |a| {
        a.cmd.source_settings.as_ref().is_some_and(|d| d.rows.is_some())
    });
    h.get_by_label("Modifier les autorisations…");
    h.get_by_label("Fermer");
}
