//! One connection: TCP, `PRELOGIN`, the TLS handshake, `LOGIN7`, then SQL batches whose first
//! result set is read.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use bicraft_data::connect::{ConnectError, ConnectErrorKind, RemoteValue};
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use crate::login::{self, ENCRYPT_NOT_SUP, ENCRYPT_OFF, ENCRYPT_ON, ENCRYPT_REQ, Login};
use crate::packet::{self, LOGIN7, PRELOGIN, REPLY, SQL_BATCH};
use crate::token::{Column, ServerMessage, Token, Tokens};
use crate::wire::{io_error, protocol};

/// Server error numbers of a refused login ([MS-TDS] leaves them to the server; these are SQL
/// Server's documented login failures).
const LOGIN_FAILED: &[i32] = &[18452, 18456, 18470, 18486, 18487, 18488];

/// What [`open`] needs.
pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    pub password: &'a str,
    pub database: &'a str,
    pub tls: Arc<ClientConfig>,
    pub connect_timeout: Duration,
    pub command_timeout: Duration,
    pub max_value: usize,
}

/// An open, logged-in connection.
pub struct Conn {
    stream: StreamOwned<ClientConnection, TcpStream>,
    packet_size: usize,
    max_value: usize,
}

/// The connection, or where the server sends the client instead (Azure SQL's redirect).
pub enum Opened {
    Conn(Box<Conn>),
    Routed { host: String, port: u16 },
}

/// The first result set of a batch.
#[derive(Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<RemoteValue>>,
}

fn network(message: impl Into<String>) -> ConnectError {
    ConnectError::new(ConnectErrorKind::Network, message.into())
}

/// A server error as the M error Power Query shows (`DataSource.Error`, "Microsoft SQL: …").
pub fn server_error(m: &ServerMessage) -> ConnectError {
    let kind = if LOGIN_FAILED.contains(&m.number) {
        ConnectErrorKind::Credentials
    } else {
        ConnectErrorKind::DataSource
    };
    ConnectError::new(kind, format!("Microsoft SQL: {}", m.text))
}

fn tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, ConnectError> {
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| network(format!("SQL Server: cannot resolve `{host}` ({e})")))?
        .take(16)
        .collect();
    let mut last = None;
    for a in &addrs {
        match TcpStream::connect_timeout(a, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(network(match last {
        Some(e) => format!("SQL Server: cannot connect to {host}:{port} ({e})"),
        None => format!("SQL Server: `{host}` has no address"),
    }))
}

/// Connects and logs in.
pub fn open(t: &Target<'_>) -> Result<Opened, ConnectError> {
    let mut tcp = tcp(t.host, t.port, t.connect_timeout)?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(t.connect_timeout))
        .and_then(|()| tcp.set_write_timeout(Some(t.connect_timeout)))
        .map_err(|e| io_error(&e))?;
    packet::write_message(&mut tcp, PRELOGIN, &login::prelogin(), 4096).map_err(|e| io_error(&e))?;
    let mut answer = Vec::new();
    std::io::Read::read_to_end(&mut packet::Message::new(&mut tcp, &[REPLY], 65_536), &mut answer)
        .map_err(|e| io_error(&e))?;
    let enc = login::prelogin_encryption(&answer)?;
    match enc {
        ENCRYPT_ON | ENCRYPT_REQ => {}
        ENCRYPT_OFF | ENCRYPT_NOT_SUP => {
            return Err(network(
                "SQL Server: the server does not offer an encrypted connection; bicraft connects \
                 only with encryption (configure a certificate on the server)",
            ));
        }
        other => {
            return Err(network(format!(
                "SQL Server: unknown PRELOGIN encryption answer {other:#04x}"
            )));
        }
    }
    let tls = crate::tls::handshake(&mut tcp, Arc::clone(&t.tls), t.host)?;
    tcp.set_read_timeout(Some(t.command_timeout))
        .and_then(|()| tcp.set_write_timeout(Some(t.command_timeout)))
        .map_err(|e| io_error(&e))?;
    let mut stream = StreamOwned::new(tls, tcp);
    let login7 = login::login7(&Login {
        host_name: &host_name(),
        user: t.user,
        password: t.password,
        app_name: "BIcraft",
        server_name: t.host,
        database: t.database,
        packet_size: 4096,
    })?;
    packet::write_message(&mut stream, LOGIN7, &login7, 4096).map_err(|e| io_error(&e))?;
    let mut acked = false;
    let mut size = 4096usize;
    let mut error: Option<ServerMessage> = None;
    let mut routed = None;
    {
        let msg = packet::Message::new(&mut stream, &[REPLY], 1 << 20);
        let mut tokens = Tokens::new(msg, 1 << 16);
        while let Some(tok) = tokens.next_token()? {
            match tok {
                Token::LoginAck { .. } => acked = true,
                Token::PacketSize(n) => size = n as usize,
                Token::Routing { host, port } => routed = Some((host, port)),
                Token::Error(m) => {
                    error.get_or_insert(m);
                }
                _ => {}
            }
        }
    }
    if let Some(e) = &error {
        return Err(server_error(e));
    }
    if !acked {
        return Err(protocol("the server did not acknowledge the login"));
    }
    if let Some((host, port)) = routed {
        return Ok(Opened::Routed { host, port });
    }
    Ok(Opened::Conn(Box::new(Conn {
        stream,
        packet_size: size,
        max_value: t.max_value,
    })))
}

/// This machine's name for `LOGIN7` (what `sys.dm_exec_sessions.host_name` shows).
fn host_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok())
        .unwrap_or_default()
}

impl Conn {
    /// Runs `sql` and reads its first result set: at most `max_rows` rows (more is
    /// [`ConnectErrorKind::Limit`], and the connection must be dropped).
    pub fn query(&mut self, sql: &str, max_rows: usize) -> Result<ResultSet, ConnectError> {
        packet::write_message(&mut self.stream, SQL_BATCH, &login::batch(sql), self.packet_size)
            .map_err(|e| io_error(&e))?;
        let msg = packet::Message::new(&mut self.stream, &[REPLY], u64::MAX);
        let mut tokens = Tokens::new(msg, self.max_value);
        let mut out = ResultSet::default();
        let mut sets = 0usize;
        let mut error: Option<ServerMessage> = None;
        while let Some(tok) = tokens.next_token()? {
            match tok {
                Token::Columns(c) => {
                    sets += 1;
                    if sets == 1 {
                        out.columns = c;
                    }
                }
                Token::Row(r) if sets == 1 => {
                    if out.rows.len() >= max_rows {
                        return Err(ConnectError::new(
                            ConnectErrorKind::Limit,
                            format!("SQL Server: the query returns more than {max_rows} rows"),
                        ));
                    }
                    out.rows.push(r);
                }
                Token::Error(m) if m.class > 10 => {
                    error.get_or_insert(m);
                }
                _ => {}
            }
        }
        match error {
            Some(e) => Err(server_error(&e)),
            None => Ok(out),
        }
    }
}
