//! The token stream of a server message ([MS-TDS] 2.2.7): `COLMETADATA`, `ROW`, `NBCROW`,
//! `DONE*`, `ERROR`, `INFO`, `LOGINACK`, `ENVCHANGE` and the tokens a batch may carry and the
//! reader skips (`ORDER`, `RETURNSTATUS`, `COLINFO`, `TABNAME`, `FEATUREEXTACK`, `SSPI`). Any
//! other token, a truncated stream or a length out of bounds is an error; nothing read from the
//! wire sizes an allocation without a cap.

use std::io::Read;

use bicraft_data::connect::{ConnectError, RemoteValue};

use crate::types::TypeInfo;
use crate::wire::{Rd, protocol, slice};

/// Most columns of a result (SQL Server's own limit is 4096).
pub const MAX_COLUMNS: usize = 4096;

const COLMETADATA: u8 = 0x81;
const ROW: u8 = 0xD1;
const NBCROW: u8 = 0xD2;
const DONE: u8 = 0xFD;
const DONEPROC: u8 = 0xFE;
const DONEINPROC: u8 = 0xFF;
const ERROR: u8 = 0xAA;
const INFO: u8 = 0xAB;
const LOGINACK: u8 = 0xAD;
const ENVCHANGE: u8 = 0xE3;
const ORDER: u8 = 0xA9;
const RETURNSTATUS: u8 = 0x79;
const COLINFO: u8 = 0xA5;
const TABNAME: u8 = 0xA4;
const FEATUREEXTACK: u8 = 0xAE;
const SSPI: u8 = 0xED;

/// `DONE` status bits.
pub const DONE_MORE: u16 = 0x1;
pub const DONE_ERROR: u16 = 0x2;
pub const DONE_ATTN: u16 = 0x20;
pub const DONE_SRVERROR: u16 = 0x100;

/// A column of a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: TypeInfo,
}

/// An `ERROR` or `INFO` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerMessage {
    pub number: i32,
    pub class: u8,
    pub text: String,
}

/// A token, with the rows decoded against the last `COLMETADATA`.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Columns(Vec<Column>),
    Row(Vec<RemoteValue>),
    Done {
        status: u16,
        rows: u64,
    },
    Error(ServerMessage),
    Info(ServerMessage),
    LoginAck {
        tds_version: u32,
    },
    /// `ENVCHANGE` of the packet size.
    PacketSize(u32),
    /// `ENVCHANGE` routing (Azure SQL's redirect): another host and port to log in to.
    Routing {
        host: String,
        port: u16,
    },
    /// A token without meaning here (another `ENVCHANGE`, `ORDER`, …).
    Skipped,
}

/// Reads the tokens of one message.
pub struct Tokens<R: Read> {
    r: Rd<R>,
    columns: Option<Vec<Column>>,
    /// Largest value read (bytes).
    max_value: usize,
}

impl<R: Read> Tokens<R> {
    pub fn new(inner: R, max_value: usize) -> Self {
        Self {
            r: Rd::new(inner),
            columns: None,
            max_value,
        }
    }

    /// The columns of the result being read.
    pub fn columns(&self) -> Option<&[Column]> {
        self.columns.as_deref()
    }

    /// The next token; `None` at the end of the message.
    pub fn next_token(&mut self) -> Result<Option<Token>, ConnectError> {
        let Some(t) = self.r.first()? else {
            return Ok(None);
        };
        let tok = match t {
            COLMETADATA => {
                let cols = self.colmetadata()?;
                self.columns = Some(cols.clone());
                Token::Columns(cols)
            }
            ROW | NBCROW => {
                let cols = self
                    .columns
                    .as_ref()
                    .ok_or_else(|| protocol("a row came before its columns"))?;
                let nulls = if t == NBCROW {
                    self.r.bytes(cols.len().div_ceil(8) as u64, MAX_COLUMNS / 8)?
                } else {
                    Vec::new()
                };
                let mut row = Vec::with_capacity(cols.len());
                for (i, c) in cols.iter().enumerate() {
                    let null = nulls.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0);
                    row.push(if null {
                        RemoteValue::Null
                    } else {
                        c.ty.value(&mut self.r, self.max_value).map_err(|e| {
                            ConnectError::new(e.kind, format!("{} (column `{}`)", e.message, c.name))
                        })?
                    });
                }
                Token::Row(row)
            }
            DONE | DONEPROC | DONEINPROC => {
                let status = self.r.u16()?;
                let _cmd = self.r.u16()?;
                let rows = self.r.u64()?;
                Token::Done { status, rows }
            }
            ERROR | INFO => {
                let body = self.body()?;
                let m = message(&body)?;
                if t == ERROR {
                    Token::Error(m)
                } else {
                    Token::Info(m)
                }
            }
            LOGINACK => {
                let body = self.body()?;
                let mut b = slice(&body);
                let _interface = b.u8()?;
                Token::LoginAck {
                    tds_version: u32::from_be_bytes(b.array()?),
                }
            }
            ENVCHANGE => {
                let body = self.body()?;
                envchange(&body)?
            }
            ORDER | COLINFO | TABNAME | SSPI => {
                let n = self.r.u16()?;
                self.r.skip(u64::from(n))?;
                Token::Skipped
            }
            RETURNSTATUS => {
                self.r.skip(4)?;
                Token::Skipped
            }
            FEATUREEXTACK => {
                for _ in 0..=255 {
                    let id = self.r.u8()?;
                    if id == 0xFF {
                        return Ok(Some(Token::Skipped));
                    }
                    let n = self.r.u32()?;
                    self.r.skip(u64::from(n))?;
                }
                return Err(protocol("FEATUREEXTACK has no terminator"));
            }
            other => return Err(protocol(format!("unexpected token 0x{other:02X}"))),
        };
        Ok(Some(tok))
    }

    /// A token body of a 16-bit length.
    fn body(&mut self) -> Result<Vec<u8>, ConnectError> {
        let n = self.r.u16()?;
        self.r.bytes(u64::from(n), usize::from(u16::MAX))
    }

    fn colmetadata(&mut self) -> Result<Vec<Column>, ConnectError> {
        let count = self.r.u16()?;
        if count == 0xFFFF {
            return Ok(Vec::new());
        }
        let count = usize::from(count);
        if count > MAX_COLUMNS {
            return Err(protocol(format!("{count} columns, more than {MAX_COLUMNS}")));
        }
        let mut cols = Vec::with_capacity(count);
        for _ in 0..count {
            let _user_type = self.r.u32()?;
            let _flags = self.r.u16()?;
            let ty = TypeInfo::read(&mut self.r)?;
            if ty.has_table_name() {
                let parts = self.r.u8()?;
                for _ in 0..parts {
                    self.r.us_varchar()?;
                }
            }
            let name = self.r.b_varchar()?;
            cols.push(Column { name, ty });
        }
        Ok(cols)
    }
}

/// The body of `ERROR`/`INFO` ([MS-TDS] 2.2.7.10).
fn message(body: &[u8]) -> Result<ServerMessage, ConnectError> {
    let mut b = slice(body);
    let number = b.i32()?;
    let _state = b.u8()?;
    let class = b.u8()?;
    let text = b.us_varchar()?;
    Ok(ServerMessage { number, class, text })
}

/// The body of `ENVCHANGE` ([MS-TDS] 2.2.7.9).
fn envchange(body: &[u8]) -> Result<Token, ConnectError> {
    let mut b = slice(body);
    Ok(match b.u8()? {
        4 => {
            let size = b.b_varchar()?;
            let n: u32 = size
                .trim()
                .parse()
                .map_err(|_| protocol(format!("packet size `{size}`")))?;
            if !(512..=32_767).contains(&n) {
                return Err(protocol(format!("packet size {n} out of range")));
            }
            Token::PacketSize(n)
        }
        20 => {
            let _len = b.u16()?;
            if b.u8()? != 0 {
                return Err(protocol("routing to a protocol other than TCP"));
            }
            let port = b.u16()?;
            let host = b.us_varchar()?;
            Token::Routing { host, port }
        }
        _ => Token::Skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::to_utf16;

    #[test]
    fn a_result_with_nulls_and_errors() {
        let mut s = vec![COLMETADATA, 2, 0];
        // INT NOT NULL "id", NVARCHAR(10) "name"
        s.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0x38, 2]);
        s.extend_from_slice(&to_utf16("id"));
        s.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0xE7, 20, 0, 0x09, 0x04, 0xD0, 0x00, 0x34, 4]);
        s.extend_from_slice(&to_utf16("name"));
        s.push(ROW);
        s.extend_from_slice(&7i32.to_le_bytes());
        s.extend_from_slice(&[4, 0]);
        s.extend_from_slice(&to_utf16("ab"));
        s.extend_from_slice(&[NBCROW, 0b10]);
        s.extend_from_slice(&8i32.to_le_bytes());
        let text = to_utf16("Invalid column name 'x'.");
        let mut err = Vec::new();
        err.extend_from_slice(&207i32.to_le_bytes());
        err.extend_from_slice(&[1, 16]);
        err.extend_from_slice(&u16::try_from(text.len() / 2).unwrap().to_le_bytes());
        err.extend_from_slice(&text);
        err.extend_from_slice(&[0, 0, 1, 0, 0, 0]);
        s.push(ERROR);
        s.extend_from_slice(&u16::try_from(err.len()).unwrap().to_le_bytes());
        s.extend_from_slice(&err);
        s.extend_from_slice(&[DONE, 0x12, 0, 0xC1, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        let mut t = Tokens::new(s.as_slice(), 1 << 20);
        let mut got = Vec::new();
        while let Some(tok) = t.next_token().unwrap() {
            got.push(tok);
        }
        assert!(matches!(&got[0], Token::Columns(c) if c.len() == 2 && c[1].name == "name"));
        assert_eq!(
            got[1],
            Token::Row(vec![RemoteValue::Int(7), RemoteValue::Text("ab".into())])
        );
        assert_eq!(got[2], Token::Row(vec![RemoteValue::Int(8), RemoteValue::Null]));
        assert_eq!(
            got[3],
            Token::Error(ServerMessage {
                number: 207,
                class: 16,
                text: "Invalid column name 'x'.".into()
            })
        );
        assert_eq!(
            got[4],
            Token::Done {
                status: 0x12,
                rows: 2
            }
        );
    }

    #[test]
    fn hostile_streams_are_errors() {
        for bad in [
            &[ROW][..],
            &[0x42],
            &[COLMETADATA, 0xFF, 0x7F],
            &[DONE, 0],
            &[ERROR, 0xFF, 0xFF, 1],
            &[FEATUREEXTACK, 1, 0xFF, 0xFF, 0xFF, 0xFF],
            &[ENVCHANGE, 3, 0, 4, 1, b'9'],
        ] {
            let mut t = Tokens::new(bad, 1 << 20);
            let r = (0..10).try_for_each(|_| t.next_token().map(|_| ()));
            assert!(r.is_err(), "{bad:?}");
        }
    }
}
