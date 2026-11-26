//! A scripted TDS server on 127.0.0.1 for tests (feature `fake-server`): it answers `PRELOGIN`,
//! runs the TLS handshake inside `PRELOGIN` packets with a test certificate for `localhost` and
//! `127.0.0.1` (signed by [`CA_PEM`]), checks `LOGIN7`'s login and password, then answers each
//! SQL batch with the token stream its script returns, written by hand from [MS-TDS] with the
//! builders of [`tok`]. No real SQL Server is involved.

use std::io::{self, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::login::{ENCRYPT_ON, unscramble};
use crate::packet::{self, LOGIN7, PRELOGIN, REPLY, SQL_BATCH};
use crate::wire::utf16;

/// The test CA that signed the fake server's certificate.
pub const CA_PEM: &[u8] = include_bytes!("../testdata/ca.pem");
const CERT_PEM: &[u8] = include_bytes!("../testdata/server.pem");
const KEY_PEM: &[u8] = include_bytes!("../testdata/server.key");

/// The answer to a batch: a token stream (without `DONE`, added after it), or `None` for
/// "Invalid object name".
pub type Answer = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// What the fake server does.
#[derive(Clone)]
pub struct Script {
    pub user: String,
    pub password: String,
    /// The `PRELOGIN` encryption answer (`ENCRYPT_ON` by default).
    pub encryption: u8,
    pub answer: Answer,
}

impl Script {
    pub fn new(user: &str, password: &str, answer: Answer) -> Self {
        Self {
            user: user.into(),
            password: password.into(),
            encryption: ENCRYPT_ON,
            answer,
        }
    }
}

/// What the fake server saw.
#[derive(Debug, Default, Clone)]
pub struct Seen {
    /// The SQL batches, in order.
    pub batches: Vec<String>,
    /// The logins tried: (login, database).
    pub logins: Vec<(String, String)>,
}

/// A running fake server; it stops with the test process.
pub struct FakeServer {
    pub port: u16,
    seen: Arc<Mutex<Seen>>,
    _thread: JoinHandle<()>,
}

impl FakeServer {
    /// The server text of `Sql.Database` for it: `127.0.0.1,<port>`.
    pub fn server(&self) -> String {
        format!("127.0.0.1,{}", self.port)
    }

    pub fn seen(&self) -> Seen {
        self.seen.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn config() -> io::Result<Arc<ServerConfig>> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(CERT_PEM)
        .collect::<Result<_, _>>()
        .map_err(|e| other(format!("{e:?}")))?;
    let key = PrivateKeyDer::from_pem_slice(KEY_PEM).map_err(|e| other(format!("{e:?}")))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    // SQL Server negotiates TLS 1.2 inside TDS 7.x
    let c = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(other)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(other)?;
    Ok(Arc::new(c))
}

/// Starts a server with `script` on an ephemeral port of 127.0.0.1.
pub fn start(script: Script) -> io::Result<FakeServer> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let tls = config()?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s2 = Arc::clone(&seen);
    let thread = std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (script, tls, seen) = (script.clone(), Arc::clone(&tls), Arc::clone(&s2));
            std::thread::spawn(move || {
                let _ = serve(stream, &script, tls, &seen);
            });
        }
    });
    Ok(FakeServer {
        port,
        seen,
        _thread: thread,
    })
}

fn read_message<R: Read>(r: &mut R, types: &'static [u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    packet::Message::new(r, types, 64 << 20).read_to_end(&mut out)?;
    Ok(out)
}

/// The server's `PRELOGIN` answer.
fn prelogin_answer(encryption: u8) -> Vec<u8> {
    // VERSION (16.0.1000), ENCRYPTION, then the terminator: 2 × 5 + 1 bytes of header
    let mut v = vec![0, 0, 11, 0, 6, 1, 0, 17, 0, 1, 0xFF];
    v.extend_from_slice(&[16, 0, 0x03, 0xE8, 0, 0]);
    v.push(encryption);
    v
}

fn serve(mut tcp: TcpStream, script: &Script, tls: Arc<ServerConfig>, seen: &Mutex<Seen>) -> io::Result<()> {
    read_message(&mut tcp, &[PRELOGIN])?;
    packet::write_message(&mut tcp, REPLY, &prelogin_answer(script.encryption), 4096)?;
    if script.encryption != ENCRYPT_ON && script.encryption != crate::login::ENCRYPT_REQ {
        return Ok(());
    }
    let mut conn = ServerConnection::new(tls).map_err(other)?;
    while conn.is_handshaking() {
        let msg = read_message(&mut tcp, &[PRELOGIN])?;
        let mut rd = msg.as_slice();
        while !rd.is_empty() {
            conn.read_tls(&mut rd)?;
            conn.process_new_packets().map_err(other)?;
        }
        if conn.wants_write() {
            let mut out = Vec::new();
            while conn.wants_write() {
                conn.write_tls(&mut out)?;
            }
            packet::write_message(&mut tcp, PRELOGIN, &out, 4096)?;
        }
    }
    let mut s = StreamOwned::new(conn, tcp);
    let login = read_message(&mut s, &[LOGIN7])?;
    let field = |at: usize, scrambled: bool| -> Option<String> {
        let off = usize::from(u16::from_le_bytes([*login.get(at)?, *login.get(at + 1)?]));
        let n = usize::from(u16::from_le_bytes([*login.get(at + 2)?, *login.get(at + 3)?]));
        let b = login.get(off..off + n * 2)?;
        Some(utf16(&if scrambled { unscramble(b) } else { b.to_vec() }))
    };
    let (user, password, database) = (
        field(40, false).unwrap_or_default(),
        field(44, true).unwrap_or_default(),
        field(68, false).unwrap_or_default(),
    );
    if let Ok(mut g) = seen.lock() {
        g.logins.push((user.clone(), database));
    }
    if user != script.user || password != script.password {
        let mut t = tok::error(18456, 14, &format!("Login failed for user '{user}'."));
        t.extend_from_slice(&tok::done_error());
        return packet::write_message(&mut s, REPLY, &t, 4096);
    }
    let mut t = tok::envchange_database("master");
    t.extend_from_slice(&tok::loginack());
    t.extend_from_slice(&tok::done(0));
    packet::write_message(&mut s, REPLY, &t, 4096)?;
    loop {
        let batch = read_message(&mut s, &[SQL_BATCH])?;
        let headers = batch
            .get(..4)
            .and_then(|h| <[u8; 4]>::try_from(h).ok())
            .map_or(0, |h| u32::from_le_bytes(h) as usize);
        let sql = utf16(batch.get(headers..).unwrap_or_default());
        if let Ok(mut g) = seen.lock() {
            g.batches.push(sql.clone());
        }
        let reply = match (script.answer)(&sql) {
            Some(mut t) => {
                t.extend_from_slice(&tok::done(0));
                t
            }
            None => {
                let mut t = tok::error(208, 16, "Invalid object name 'x'.");
                t.extend_from_slice(&tok::done_error());
                t
            }
        };
        packet::write_message(&mut s, REPLY, &reply, 4096)?;
    }
}

/// Token stream builders ([MS-TDS] 2.2.7).
pub mod tok {
    use crate::wire::to_utf16;

    fn us_varchar(out: &mut Vec<u8>, s: &str) {
        let b = to_utf16(s);
        out.extend_from_slice(&((b.len() / 2) as u16).to_le_bytes());
        out.extend_from_slice(&b);
    }

    fn b_varchar(out: &mut Vec<u8>, s: &str) {
        let b = to_utf16(s);
        out.push((b.len() / 2) as u8);
        out.extend_from_slice(&b);
    }

    /// `COLMETADATA`: each column's name and `TYPE_INFO` bytes.
    pub fn columns(cols: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = vec![0x81];
        out.extend_from_slice(&(cols.len() as u16).to_le_bytes());
        for (name, ty) in cols {
            out.extend_from_slice(&[0, 0, 0, 0, 0x01, 0x00]);
            out.extend_from_slice(ty);
            b_varchar(&mut out, name);
        }
        out
    }

    /// `ROW` of value bytes (each value already length-prefixed as its type needs).
    pub fn row(values: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0xD1];
        for v in values {
            out.extend_from_slice(v);
        }
        out
    }

    /// `DONE` (final, with a row count).
    pub fn done(rows: u64) -> Vec<u8> {
        let mut out = vec![0xFD, 0x10, 0x00, 0xC1, 0x00];
        out.extend_from_slice(&rows.to_le_bytes());
        out
    }

    pub fn done_error() -> Vec<u8> {
        let mut out = vec![0xFD, 0x02, 0x00, 0x00, 0x00];
        out.extend_from_slice(&0u64.to_le_bytes());
        out
    }

    /// `ERROR`.
    pub fn error(number: i32, class: u8, text: &str) -> Vec<u8> {
        let mut body = number.to_le_bytes().to_vec();
        body.extend_from_slice(&[1, class]);
        us_varchar(&mut body, text);
        b_varchar(&mut body, "fake");
        b_varchar(&mut body, "");
        body.extend_from_slice(&1i32.to_le_bytes());
        let mut out = vec![0xAA];
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    pub fn loginack() -> Vec<u8> {
        let mut body = vec![1];
        body.extend_from_slice(&0x7400_0004u32.to_be_bytes());
        b_varchar(&mut body, "fake");
        body.extend_from_slice(&[16, 0, 0x03, 0xE8]);
        let mut out = vec![0xAD];
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    pub fn envchange_database(name: &str) -> Vec<u8> {
        let mut body = vec![1];
        b_varchar(&mut body, name);
        b_varchar(&mut body, "");
        let mut out = vec![0xE3];
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// `TYPE_INFO` of common types.
    pub const INT: &[u8] = &[0x26, 4];
    pub const BIGINT: &[u8] = &[0x26, 8];
    pub const BIT: &[u8] = &[0x68, 1];
    pub const FLOAT: &[u8] = &[0x6D, 8];
    pub const DATE: &[u8] = &[0x28];
    /// `nvarchar(100)`, `Latin1_General_CI_AS`.
    pub const NVARCHAR: &[u8] = &[0xE7, 200, 0, 0x09, 0x04, 0xD0, 0x00, 0x34];
    /// `varchar(50)`, `SQL_Latin1_General_CP1_CI_AS`.
    pub const VARCHAR: &[u8] = &[0xA7, 50, 0, 0x09, 0x04, 0xD0, 0x00, 0x34];
    /// `decimal(18, 2)`.
    pub const DECIMAL: &[u8] = &[0x6A, 9, 18, 2];
    /// `datetime2(7)`.
    pub const DATETIME2: &[u8] = &[0x2A, 7];

    pub fn int(v: Option<i32>) -> Vec<u8> {
        match v {
            Some(v) => [&[4u8][..], &v.to_le_bytes()].concat(),
            None => vec![0],
        }
    }

    pub fn bigint(v: i64) -> Vec<u8> {
        [&[8u8][..], &v.to_le_bytes()].concat()
    }

    pub fn bit(v: bool) -> Vec<u8> {
        vec![1, u8::from(v)]
    }

    pub fn float(v: f64) -> Vec<u8> {
        [&[8u8][..], &v.to_le_bytes()].concat()
    }

    pub fn nvarchar(v: Option<&str>) -> Vec<u8> {
        match v {
            Some(s) => {
                let b = to_utf16(s);
                [&(b.len() as u16).to_le_bytes()[..], &b].concat()
            }
            None => vec![0xFF, 0xFF],
        }
    }

    /// Days since 0001-01-01.
    pub fn date(days: u32) -> Vec<u8> {
        let d = days.to_le_bytes();
        vec![3, d[0], d[1], d[2]]
    }

    /// `decimal(18, 2)` of `cents / 100`.
    pub fn decimal(cents: i64) -> Vec<u8> {
        let mut out = vec![9, u8::from(cents >= 0)];
        out.extend_from_slice(&cents.unsigned_abs().to_le_bytes());
        out
    }
}
