//! L3: the SQL Server connector of bicraft's Power Query evaluator.
//!
//! Registered in [`bicraft_data::SourceOptions::connectors`] only when the caller explicitly
//! allows it (`allow_connectors: ["sqlserver"]`), it runs the M functions Power BI Desktop
//! writes for SQL Server, `Sql.Database(server, database, options)` and
//! `Sql.Databases(server, options)`, over **TDS 7.4** ([MS-TDS], Microsoft's public Open
//! Specification) on TCP, always encrypted with TLS ([`tls`]), with SQL Server authentication:
//!
//! - navigation: the database's user tables and views (`Name` `schema.item`, `Schema`, `Item`,
//!   `Kind` `Table`/`View`), or its schemas then their tables with `HierarchicalNavigation =
//!   true`; `Sql.Databases` lists the databases first (`Kind = "Database"`);
//! - the folded query of a table ([`bicraft_data::connect::RemoteQuery`]: columns, row filter,
//!   sort, row limit) as one `SELECT` ([`sql`]); `[Query = "…"]` runs the native query as given,
//!   and nothing folds into it;
//! - SQL Server types mapped to M types ([`types`]); server errors become catchable
//!   `DataSource.Error`s ("Microsoft SQL: …"), a refused login a credentials error.
//!
//! Network access goes only to the server the query names (and its SQL Server Browser on UDP
//! 1434 for a named instance without a port, or the host an Azure SQL gateway routes to). The
//! credentials come from a [`CredentialProvider`] the caller supplies, asked when the first
//! connection opens; they are never logged, displayed or written. Windows (integrated)
//! authentication is refused with a message: no pure-Rust implementation of it is used here.
//! See `docs/formats/tds.md`.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod conn;
mod login;
mod packet;
pub mod server;
pub mod sql;
pub mod tls;
pub mod token;
pub mod types;
mod wire;

#[cfg(feature = "fake-server")]
pub mod fake;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bicraft_data::connect::{
    Arg, ConnectError, ConnectErrorKind, Connector, NavEntry, RemoteColumn, RemoteQuery, RemoteRows, Source,
};
use rustls::ClientConfig;

use crate::conn::{Conn, Opened, ResultSet};
use crate::token::Column;

/// The environment variables the engine's default credential provider and [`Settings::from_env`]
/// read.
pub const USER_ENV: &str = "BICRAFT_SQLSERVER_USER";
pub const PASSWORD_ENV: &str = "BICRAFT_SQLSERVER_PASSWORD";
pub const CA_FILE_ENV: &str = "BICRAFT_SQLSERVER_CA_FILE";
pub const TRUST_ENV: &str = "BICRAFT_SQLSERVER_TRUST_SERVER_CERTIFICATE";

/// A password. Its `Debug` output hides it; it has no `Display`.
#[derive(Clone)]
pub struct Password(Arc<str>);

impl Password {
    pub fn new(text: &str) -> Self {
        Self(text.into())
    }

    fn text(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(***)")
    }
}

/// How to log in.
#[derive(Debug, Clone)]
pub enum Credential {
    /// SQL Server authentication: a login and its password.
    Sql { user: String, password: Password },
    /// Windows (integrated) authentication: refused when the connection opens.
    Windows,
}

/// Where the credentials of a server come from, asked with the server text of the query when a
/// connection opens (`Err`: why there are none, shown to the user).
pub type CredentialProvider = Arc<dyn Fn(&str) -> Result<Credential, String> + Send + Sync>;

/// How the connector connects.
#[derive(Debug, Clone)]
pub struct Settings {
    pub connect_timeout: Duration,
    /// Longest wait for an answer (`CommandTimeout` of the query overrides it).
    pub command_timeout: Duration,
    /// Accept any server certificate (still encrypted, but not authenticated).
    pub trust_server_certificate: bool,
    /// More trusted CA certificates (DER), beside the bundled Mozilla roots.
    pub extra_roots: Vec<Vec<u8>>,
    /// Largest single value read (bytes).
    pub max_value_bytes: usize,
    /// Wait for the SQL Server Browser's answer.
    pub browser_timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            command_timeout: Duration::from_secs(10 * 60),
            trust_server_certificate: false,
            extra_roots: Vec::new(),
            max_value_bytes: 256 << 20,
            browser_timeout: Duration::from_secs(3),
        }
    }
}

impl Settings {
    /// The defaults with `BICRAFT_SQLSERVER_CA_FILE` (a PEM file of CA certificates) and
    /// `BICRAFT_SQLSERVER_TRUST_SERVER_CERTIFICATE` (`1`/`true`) applied.
    pub fn from_env() -> Result<Self, String> {
        let mut s = Self::default();
        if let Ok(v) = bicraft_env::var(TRUST_ENV) {
            s.trust_server_certificate =
                matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
        if let Some(path) = bicraft_env::var_os(CA_FILE_ENV).filter(|p| !p.is_empty()) {
            let text = std::fs::read(&path).map_err(|e| format!("{CA_FILE_ENV}: {e}"))?;
            s.extra_roots = tls::pem_certificates(&text).map_err(|e| format!("{CA_FILE_ENV}: {e}"))?;
        }
        Ok(s)
    }
}

/// The SQL Server connector ([`Connector`] named `sqlserver`).
pub struct SqlServerConnector {
    credentials: CredentialProvider,
    settings: Settings,
    tls: Mutex<Option<Arc<ClientConfig>>>,
}

impl SqlServerConnector {
    pub fn new(credentials: CredentialProvider, settings: Settings) -> Self {
        Self {
            credentials,
            settings,
            tls: Mutex::new(None),
        }
    }

    fn tls(&self) -> Result<Arc<ClientConfig>, ConnectError> {
        let mut slot = match self.tls.lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        if let Some(c) = slot.as_ref() {
            return Ok(Arc::clone(c));
        }
        let c = tls::config(&self.settings.extra_roots, self.settings.trust_server_certificate)?;
        *slot = Some(Arc::clone(&c));
        Ok(c)
    }
}

fn unsupported(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Unsupported, message)
}

/// Options of `Sql.Database` that change nothing in the rows bicraft reads (navigation
/// columns, parallelism, failover, cross-database folding, session context).
const IGNORED_OPTIONS: &[&str] = &[
    "CreateNavigationProperties",
    "NavigationPropertyNameGenerator",
    "MaxDegreeOfParallelism",
    "MultiSubnetFailover",
    "UnsafeTypeConversions",
    "EnableCrossDatabaseFolding",
    "ContextInfo",
    "OmitSRGColumns",
];

/// A duration option, bounded to 1 s – 24 h.
fn duration(function: &str, name: &str, v: &Arg) -> Result<Duration, ConnectError> {
    match v {
        Arg::Duration(ms) if *ms >= 1000 && *ms <= 86_400_000 => Ok(Duration::from_millis(ms.unsigned_abs())),
        Arg::Duration(_) => Err(unsupported(format!(
            "{function}: {name} must be between 1 second and 24 hours"
        ))),
        _ => Err(unsupported(format!("{function}: {name} must be a duration"))),
    }
}

impl Connector for SqlServerConnector {
    fn name(&self) -> &'static str {
        "sqlserver"
    }

    fn handles(&self, function: &str) -> bool {
        matches!(function, "Sql.Database" | "Sql.Databases")
    }

    fn open(&self, function: &str, args: &[Arg]) -> Result<Box<dyn Source>, ConnectError> {
        let text = |i: usize, what: &str| match args.get(i) {
            Some(Arg::Text(t)) => Ok(t.clone()),
            _ => Err(unsupported(format!("{function}: the {what} must be text"))),
        };
        let all = function == "Sql.Databases";
        let server_text = text(0, "server")?;
        let server = server::parse(&server_text).map_err(unsupported)?;
        let database = if all { None } else { Some(text(1, "database")?) };
        if database
            .as_ref()
            .is_some_and(|d| d.trim().is_empty() || d.chars().count() > 128)
        {
            return Err(unsupported(format!(
                "{function}: the database name must be 1 to 128 characters"
            )));
        }
        let at = if all { 1 } else { 2 };
        if args.len() > at + 1 {
            return Err(unsupported(format!(
                "{function} takes at most {} arguments",
                at + 1
            )));
        }
        let mut settings = self.settings.clone();
        let mut query = None;
        let mut hierarchical = false;
        match args.get(at) {
            None | Some(Arg::Null) => {}
            Some(Arg::Record(fields)) => {
                for (name, v) in fields {
                    match (name.as_str(), v) {
                        ("Query", Arg::Text(q)) if !all => query = Some(q.clone()),
                        ("Query", Arg::Null) => {}
                        ("HierarchicalNavigation", Arg::Logical(b)) => hierarchical = *b,
                        ("HierarchicalNavigation", Arg::Null) => {}
                        ("CommandTimeout", v) => settings.command_timeout = duration(function, name, v)?,
                        ("ConnectionTimeout", v) => settings.connect_timeout = duration(function, name, v)?,
                        (n, _) if IGNORED_OPTIONS.contains(&n) => {}
                        (n, _) => {
                            return Err(unsupported(format!(
                                "{function}: option `{n}` is not supported by bicraft"
                            )));
                        }
                    }
                }
            }
            Some(_) => return Err(unsupported(format!("{function}: the options must be a record"))),
        }
        if query.as_ref().is_some_and(|q| q.trim().is_empty()) {
            return Err(unsupported(format!("{function}: the Query option is empty")));
        }
        Ok(Box::new(SqlSource {
            server,
            server_text,
            all,
            database,
            query,
            hierarchical,
            settings,
            credentials: Arc::clone(&self.credentials),
            tls: self.tls()?,
            conn: None,
            listings: HashMap::new(),
            tables: HashMap::new(),
        }))
    }
}

/// A user table or view.
#[derive(Debug, Clone)]
struct Object {
    schema: String,
    item: String,
    view: bool,
}

/// The navigation tree and queries of one connector call.
struct SqlSource {
    server: server::Server,
    server_text: String,
    /// `Sql.Databases`: the first level is the databases.
    all: bool,
    database: Option<String>,
    query: Option<String>,
    hierarchical: bool,
    settings: Settings,
    credentials: CredentialProvider,
    tls: Arc<ClientConfig>,
    conn: Option<Conn>,
    /// Objects of a database (`None`: the login's), listed once.
    listings: HashMap<Option<String>, Vec<Object>>,
    /// Columns of a table by its quoted name.
    tables: HashMap<String, Vec<Column>>,
}

/// Most navigation entries listed.
const MAX_LISTING: usize = 100_000;

fn entry(name: String, kind: &str, leaf: bool, fields: Vec<(String, String)>) -> NavEntry {
    NavEntry {
        name,
        kind: kind.to_string(),
        leaf,
        fields,
    }
}

fn object_entry(o: &Object, name: String) -> NavEntry {
    entry(
        name,
        if o.view { "View" } else { "Table" },
        true,
        vec![
            ("Schema".into(), o.schema.clone()),
            ("Item".into(), o.item.clone()),
        ],
    )
}

fn text_at(row: &[bicraft_data::connect::RemoteValue], i: usize) -> Option<String> {
    match row.get(i) {
        Some(bicraft_data::connect::RemoteValue::Text(t)) => Some(t.clone()),
        _ => None,
    }
}

impl SqlSource {
    fn connect(&mut self) -> Result<&mut Conn, ConnectError> {
        if self.conn.is_none() {
            let (user, password) = match (self.credentials)(&self.server_text)
                .map_err(|e| ConnectError::new(ConnectErrorKind::Credentials, e))?
            {
                Credential::Sql { user, password } => (user, password),
                Credential::Windows => {
                    return Err(ConnectError::new(
                        ConnectErrorKind::Credentials,
                        "Windows (integrated) authentication is not supported by bicraft's SQL Server \
                         connector: use a SQL Server login and password",
                    ));
                }
            };
            let port = match (self.server.port, &self.server.instance) {
                (Some(p), _) => p,
                (None, Some(i)) => server::instance_port(&self.server.host, i, self.settings.browser_timeout)
                    .map_err(|e| ConnectError::new(ConnectErrorKind::Network, e))?,
                (None, None) => server::DEFAULT_PORT,
            };
            let database = self.database.clone().unwrap_or_default();
            let mut host = self.server.host.clone();
            let mut port = port;
            let mut opened = None;
            for _ in 0..2 {
                let target = conn::Target {
                    host: &host,
                    port,
                    user: &user,
                    password: password.text(),
                    database: &database,
                    tls: Arc::clone(&self.tls),
                    connect_timeout: self.settings.connect_timeout,
                    command_timeout: self.settings.command_timeout,
                    max_value: self.settings.max_value_bytes,
                };
                match conn::open(&target)? {
                    Opened::Conn(c) => {
                        opened = Some(*c);
                        break;
                    }
                    Opened::Routed { host: h, port: p } => {
                        host = server::parse(&h)
                            .map_err(|e| ConnectError::new(ConnectErrorKind::Network, e))?
                            .host;
                        port = p;
                    }
                }
            }
            self.conn = Some(opened.ok_or_else(|| {
                ConnectError::new(
                    ConnectErrorKind::Network,
                    "SQL Server: the server redirected the login twice",
                )
            })?);
        }
        self.conn
            .as_mut()
            .ok_or_else(|| ConnectError::new(ConnectErrorKind::Network, "SQL Server: no connection"))
    }

    /// Runs `sql`; a failure other than a server error drops the connection.
    fn run(&mut self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        let r = self.connect()?.query(sql, max_rows);
        if let Err(e) = &r
            && e.kind != ConnectErrorKind::DataSource
        {
            self.conn = None;
        }
        r
    }

    /// The database a path is in (`Sql.Databases`: its first name) and the rest of the path.
    fn split<'p>(&self, path: &'p [String]) -> Result<(Option<String>, &'p [String]), ConnectError> {
        if self.all {
            match path.split_first() {
                Some((db, rest)) => Ok((Some(db.clone()), rest)),
                None => Err(unsupported("Sql.Databases: name a database first")),
            }
        } else {
            Ok((None, path))
        }
    }

    fn objects(&mut self, db: Option<String>) -> Result<Vec<Object>, ConnectError> {
        if let Some(l) = self.listings.get(&db) {
            return Ok(l.clone());
        }
        let r = self.run(&sql::objects(db.as_deref()), MAX_LISTING)?;
        let list: Vec<Object> = r
            .rows
            .iter()
            .filter_map(|row| {
                Some(Object {
                    schema: text_at(row, 0)?,
                    item: text_at(row, 1)?,
                    view: text_at(row, 2)?.trim().eq_ignore_ascii_case("V"),
                })
            })
            .collect();
        self.listings.insert(db, list.clone());
        Ok(list)
    }

    /// The quoted name of the table at `path`.
    fn table(&mut self, path: &[String]) -> Result<String, ConnectError> {
        let (db, rest) = self.split(path)?;
        let (schema, item) = match (self.hierarchical, rest) {
            (true, [schema, item]) => (schema.clone(), item.clone()),
            (false, [name]) => {
                let list = self.objects(db.clone())?;
                let o = list
                    .iter()
                    .find(|o| format!("{}.{}", o.schema, o.item) == *name)
                    .ok_or_else(|| {
                        ConnectError::new(
                            ConnectErrorKind::DataSource,
                            format!("Microsoft SQL: no table or view `{name}`"),
                        )
                    })?;
                (o.schema.clone(), o.item.clone())
            }
            _ => return Err(unsupported("this SQL Server navigation path is not a table")),
        };
        Ok(match db {
            Some(d) => sql::path(&[&d, &schema, &item]),
            None => sql::path(&[&schema, &item]),
        })
    }

    fn table_columns(&mut self, name: &str) -> Result<Vec<Column>, ConnectError> {
        if let Some(c) = self.tables.get(name) {
            return Ok(c.clone());
        }
        let r = self.run(&sql::columns(name), 0)?;
        self.tables.insert(name.to_string(), r.columns.clone());
        Ok(r.columns)
    }
}

fn remote_columns(cols: &[Column]) -> Result<Vec<RemoteColumn>, ConnectError> {
    cols.iter()
        .map(|c| {
            let (ty, comparable) =
                c.ty.remote()
                    .map_err(|e| ConnectError::new(e.kind, format!("{} (column `{}`)", e.message, c.name)))?;
            Ok(RemoteColumn {
                name: c.name.clone(),
                ty,
                comparable,
            })
        })
        .collect()
}

fn rows(r: ResultSet) -> Result<RemoteRows, ConnectError> {
    Ok(RemoteRows {
        columns: remote_columns(&r.columns)?,
        rows: r.rows,
    })
}

impl Source for SqlSource {
    fn lookup(
        &mut self,
        path: &[String],
        name: &str,
        kind: Option<&str>,
    ) -> Result<Option<NavEntry>, ConnectError> {
        // a database of Sql.Databases needs no listing; anything else is found in the listing
        Ok(match (self.all, path, kind) {
            (true, [], None | Some("Database")) => {
                Some(entry(name.to_string(), "Database", false, Vec::new()))
            }
            _ => None,
        })
    }

    fn list(&mut self, path: &[String]) -> Result<Vec<NavEntry>, ConnectError> {
        if self.all && path.is_empty() {
            let r = self.run(sql::databases(), MAX_LISTING)?;
            return Ok(r
                .rows
                .iter()
                .filter_map(|row| text_at(row, 0))
                .map(|n| entry(n, "Database", false, Vec::new()))
                .collect());
        }
        let (db, rest) = self.split(path)?;
        let objects = self.objects(db)?;
        Ok(match (self.hierarchical, rest) {
            (false, []) => objects
                .iter()
                .map(|o| object_entry(o, format!("{}.{}", o.schema, o.item)))
                .collect(),
            (true, []) => {
                let mut schemas: Vec<String> = objects.iter().map(|o| o.schema.clone()).collect();
                schemas.dedup();
                schemas
                    .into_iter()
                    .map(|s| entry(s, "Schema", false, Vec::new()))
                    .collect()
            }
            (true, [schema]) => objects
                .iter()
                .filter(|o| o.schema == *schema)
                .map(|o| object_entry(o, o.item.clone()))
                .collect(),
            _ => return Err(unsupported("this SQL Server navigation path has no children")),
        })
    }

    fn columns(&mut self, path: &[String]) -> Result<Vec<RemoteColumn>, ConnectError> {
        let name = self.table(path)?;
        let cols = self.table_columns(&name)?;
        remote_columns(&cols)
    }

    fn fetch(&mut self, q: &RemoteQuery, max_rows: usize) -> Result<RemoteRows, ConnectError> {
        if q.table.is_empty() {
            let Some(native) = self.query.clone() else {
                return Err(unsupported("a SQL Server navigation table has no rows"));
            };
            return rows(self.run(&native, max_rows)?);
        }
        let name = self.table(&q.table)?;
        let cols = self.table_columns(&name)?;
        let text =
            sql::select(q, &name, &cols).map_err(|e| ConnectError::new(ConnectErrorKind::DataSource, e))?;
        rows(self.run(&text, max_rows)?)
    }

    fn native(&mut self, q: &RemoteQuery) -> Result<Option<String>, ConnectError> {
        if q.table.is_empty() {
            return Ok(self.query.clone());
        }
        let name = self.table(&q.table)?;
        let cols = self.table_columns(&name)?;
        sql::select(q, &name, &cols)
            .map(Some)
            .map_err(|e| ConnectError::new(ConnectErrorKind::DataSource, e))
    }

    fn folds_sort(&self) -> bool {
        true
    }

    fn root_is_table(&self) -> bool {
        self.query.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_checked() {
        let c = SqlServerConnector::new(Arc::new(|_| Err("none".into())), Settings::default());
        let open = |args: Vec<Arg>| c.open("Sql.Database", &args).map(|_| ());
        let t = |s: &str| Arg::Text(s.into());
        assert!(open(vec![t("srv"), t("db")]).is_ok());
        let rec = |f: Vec<(&str, Arg)>| Arg::Record(f.into_iter().map(|(n, v)| (n.to_string(), v)).collect());
        assert!(open(vec![t("srv"), t("db"), rec(vec![("Query", t("select 1"))])]).is_ok());
        assert!(
            open(vec![
                t("srv"),
                t("db"),
                rec(vec![("CreateNavigationProperties", Arg::Logical(false))])
            ])
            .is_ok()
        );
        assert!(
            open(vec![
                t("srv"),
                t("db"),
                rec(vec![("CommandTimeout", Arg::Duration(60_000))])
            ])
            .is_ok()
        );
        assert!(
            open(vec![
                t("srv"),
                t("db"),
                rec(vec![("CommandTimeout", Arg::Duration(5))])
            ])
            .is_err()
        );
        assert!(open(vec![t("srv"), t("db"), rec(vec![("Bogus", Arg::Null)])]).is_err());
        assert!(open(vec![t("srv"), t("")]).is_err());
        assert!(open(vec![t("a b"), t("db")]).is_err());
        assert!(open(vec![t("srv")]).is_err());
        assert!(
            c.open("Sql.Databases", &[t("srv"), rec(vec![("Query", t("x"))])])
                .is_err()
        );
        assert!(c.open("Sql.Databases", &[t("srv")]).is_ok());
        assert_eq!(format!("{:?}", Password::new("secret")), "Password(***)");
    }
}
