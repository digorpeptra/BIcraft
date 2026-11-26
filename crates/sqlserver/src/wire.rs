//! Little-endian reads of the token stream, bounded: every length read from the wire is checked
//! against a cap before anything is allocated for it, and buffers grow only with bytes that
//! actually arrived.

use std::io::{self, Read};

use bicraft_data::connect::{ConnectError, ConnectErrorKind};

/// A protocol error (a malformed or truncated stream): the connection is not reused.
pub fn protocol(message: impl Into<String>) -> ConnectError {
    ConnectError::new(
        ConnectErrorKind::Network,
        format!("SQL Server: {}", message.into()),
    )
}

pub fn io_error(e: &io::Error) -> ConnectError {
    match e.kind() {
        io::ErrorKind::UnexpectedEof => protocol("the server's answer ended early"),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => protocol("the server did not answer in time"),
        io::ErrorKind::InvalidData => protocol(e.to_string()),
        _ => protocol(format!("connection failed: {e}")),
    }
}

/// A reader of the token stream.
pub struct Rd<R: Read> {
    pub inner: R,
}

impl<R: Read> Rd<R> {
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    /// One byte, or `None` at the end of the stream.
    pub fn first(&mut self) -> Result<Option<u8>, ConnectError> {
        let mut b = [0u8; 1];
        loop {
            match self.inner.read(&mut b) {
                Ok(0) => return Ok(None),
                Ok(_) => return Ok(Some(b[0])), // a one-byte buffer
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io_error(&e)),
            }
        }
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ConnectError> {
        let mut b = [0u8; N];
        self.inner.read_exact(&mut b).map_err(|e| io_error(&e))?;
        Ok(b)
    }

    pub fn u8(&mut self) -> Result<u8, ConnectError> {
        let [b] = self.array::<1>()?;
        Ok(b)
    }

    pub fn u16(&mut self) -> Result<u16, ConnectError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, ConnectError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, ConnectError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, ConnectError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// `n` bytes, at most `cap`.
    pub fn bytes(&mut self, n: u64, cap: usize) -> Result<Vec<u8>, ConnectError> {
        if n > cap as u64 {
            return Err(protocol(format!(
                "a value of {n} bytes exceeds the {cap}-byte limit"
            )));
        }
        let mut v = Vec::new();
        (&mut self.inner)
            .take(n)
            .read_to_end(&mut v)
            .map_err(|e| io_error(&e))?;
        if v.len() as u64 != n {
            return Err(protocol("the server's answer ended early"));
        }
        Ok(v)
    }

    pub fn skip(&mut self, n: u64) -> Result<(), ConnectError> {
        let got = io::copy(&mut (&mut self.inner).take(n), &mut io::sink()).map_err(|e| io_error(&e))?;
        if got != n {
            return Err(protocol("the server's answer ended early"));
        }
        Ok(())
    }

    /// `B_VARCHAR`: a byte count of UTF-16 code units, then the text.
    pub fn b_varchar(&mut self) -> Result<String, ConnectError> {
        let n = self.u8()?;
        let b = self.bytes(u64::from(n) * 2, 512)?;
        Ok(utf16(&b))
    }

    /// `US_VARCHAR`: a 16-bit count of UTF-16 code units, then the text.
    pub fn us_varchar(&mut self) -> Result<String, ConnectError> {
        let n = self.u16()?;
        let b = self.bytes(u64::from(n) * 2, 131_072)?;
        Ok(utf16(&b))
    }
}

/// UTF-16LE text (an unpaired surrogate becomes U+FFFD).
pub fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    String::from_utf16_lossy(&units)
}

/// UTF-16LE bytes of `s`.
pub fn to_utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A slice reader over an in-memory token body (ENVCHANGE, ERROR, LOGINACK).
pub fn slice(b: &[u8]) -> Rd<&[u8]> {
    Rd::new(b)
}
