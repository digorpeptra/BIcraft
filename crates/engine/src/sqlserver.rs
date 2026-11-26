//! The SQL Server connector of the session (lot 11): its credentials, held in memory for the
//! session by `source.credentials` (never written to a file, a journal, a log or an answer), and
//! the connector `allow_connectors: ["sqlserver"]` registers (`refresh::connectors`).
//!
//! A server's credentials are looked up by its normalized name (host, port, instance), then in
//! the environment variables `BICRAFT_SQLSERVER_USER` / `BICRAFT_SQLSERVER_PASSWORD`. They are
//! read when a connection opens. See docs/formats/tds.md and docs/mcp.md, "Get data".

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bicraft_sqlserver::{Credential, Password, Settings, SqlServerConnector};
use serde_json::{Value, json};

use crate::Session;
use crate::params::{allow, opt_bool, opt_str, str_param};

/// Credentials by normalized server name, shared by the connectors of the session.
#[derive(Clone, Default)]
pub struct SqlCredentials(Arc<Mutex<HashMap<String, Credential>>>);

impl std::fmt::Debug for SqlCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.lock().map(|m| m.len()).unwrap_or(0);
        write!(f, "SqlCredentials({n} servers)")
    }
}

impl SqlCredentials {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Credential>> {
        match self.0.lock() {
            Ok(m) => m,
            Err(p) => p.into_inner(),
        }
    }

    pub fn set(&self, server: &str, c: Credential) {
        self.map().insert(key(server), c);
    }

    pub fn clear(&self, server: &str) -> bool {
        self.map().remove(&key(server)).is_some()
    }

    pub fn get(&self, server: &str) -> Option<Credential> {
        self.map().get(&key(server)).cloned()
    }
}

/// The name credentials are kept under: `host[\instance][,port]`, lower case.
fn key(server: &str) -> String {
    match bicraft_sqlserver::server::parse(server) {
        Ok(s) => {
            let mut k = s.host;
            if let Some(i) = s.instance {
                k.push('\\');
                k.push_str(&i.to_ascii_lowercase());
            }
            if let Some(p) = s.port {
                k.push_str(&format!(",{p}"));
            }
            k
        }
        Err(_) => server.trim().to_ascii_lowercase(),
    }
}

/// The credentials of `server`: the session's, else the environment's.
fn credential(store: &SqlCredentials, server: &str) -> Result<Credential, String> {
    if let Some(c) = store.get(server) {
        return Ok(c);
    }
    let user = bicraft_env::var(bicraft_sqlserver::USER_ENV)
        .ok()
        .filter(|u| !u.trim().is_empty());
    let password = bicraft_env::var(bicraft_sqlserver::PASSWORD_ENV).ok();
    match (user, password) {
        (Some(user), Some(password)) => Ok(Credential::Sql {
            user,
            password: Password::new(&password),
        }),
        _ => Err(format!(
            "no SQL Server credentials for `{server}`: enter them in Get data › SQL Server database \
             (source.credentials), or set {} and {} in the shell that runs bicraft; they are kept in \
             memory only, never stored",
            bicraft_sqlserver::USER_ENV,
            bicraft_sqlserver::PASSWORD_ENV
        )),
    }
}

/// The connector of the session.
pub(crate) fn connector(s: &Session) -> Result<Arc<SqlServerConnector>, String> {
    crate::sandbox::require_network(s, "the SQL Server connector")?;
    let settings = match &s.sql_settings {
        Some(st) => st.clone(),
        None => Settings::from_env()?,
    };
    let store = s.sql_credentials.clone();
    Ok(Arc::new(SqlServerConnector::new(
        Arc::new(move |server: &str| credential(&store, server)),
        settings,
    )))
}

/// `source.credentials`: the credentials of a SQL Server for this session.
pub(crate) fn source_credentials(s: &mut Session, p: &Value) -> Result<Value, String> {
    // a web source's credential: its kind, or without a kind a parameter only web sources take
    let web = match opt_str(p, "kind")? {
        Some(k) => ["web", "odata", "sharepoint"]
            .iter()
            .any(|w| k.eq_ignore_ascii_case(w)),
        None => ["url", "key", "header", "token"]
            .iter()
            .any(|n| p.get(*n).is_some()),
    };
    if web {
        return crate::source_settings::web_credentials(s, p);
    }
    allow(
        p,
        &["kind", "server", "authentication", "user", "password", "clear"],
    )?;
    match opt_str(p, "kind")?.unwrap_or("sqlserver") {
        k if k.eq_ignore_ascii_case("sqlserver") => {}
        k => {
            return Err(format!(
                "unknown `kind` `{k}` (expected sqlserver, web, odata or sharepoint)"
            ));
        }
    }
    let server = str_param(p, "server")?;
    bicraft_sqlserver::server::parse(server)?;
    if opt_bool(p, "clear")?.unwrap_or(false) {
        let cleared = s.sql_credentials.clear(server);
        return Ok(json!({"kind": "sqlserver", "server": key(server), "stored": false, "cleared": cleared}));
    }
    let auth = opt_str(p, "authentication")?.unwrap_or("sql");
    let c = match auth.to_ascii_lowercase().as_str() {
        "sql" | "database" => {
            let user = str_param(p, "user")?;
            if user.trim().is_empty() || user.chars().count() > 128 {
                return Err("`user` must be a login of 1 to 128 characters".into());
            }
            let password = match p.get("password") {
                None | Some(Value::Null) => "",
                Some(Value::String(pw)) => pw.as_str(),
                Some(_) => return Err("`password` must be text".into()),
            };
            if password.chars().count() > 128 {
                return Err("`password` is longer than 128 characters".into());
            }
            Credential::Sql {
                user: user.to_string(),
                password: Password::new(password),
            }
        }
        "windows" => {
            return Err(
                "Windows (integrated) authentication is not supported by bicraft's SQL Server connector: \
                 use a SQL Server login and password"
                    .into(),
            );
        }
        other => return Err(format!("unknown `authentication` `{other}` (expected sql)")),
    };
    s.sql_credentials.set(server, c);
    Ok(json!({"kind": "sqlserver", "server": key(server), "stored": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_debug() {
        assert_eq!(key(" DB.example.com , 1500"), "db.example.com,1500");
        assert_eq!(key(r"srv\SQLExpress"), r"srv\sqlexpress");
        let c = SqlCredentials::default();
        c.set(
            "srv",
            Credential::Sql {
                user: "u".into(),
                password: Password::new("hunter2"),
            },
        );
        assert!(c.get("SRV").is_some());
        assert!(!format!("{c:?}").contains("hunter2"));
        assert!(c.clear("srv"));
        assert!(c.get("srv").is_none());
    }
}
