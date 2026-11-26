//! TDS packets ([MS-TDS] 2.2.3): an 8-byte header (type, status, big-endian length including the
//! header, SPID, packet id, window) and up to `packet size − 8` bytes of a message; the last
//! packet of a message has the status bit `EOM`.

use std::io::{self, Read, Write};

/// Header size.
pub const HEADER: usize = 8;
/// Largest packet the length field allows.
pub const MAX_PACKET: usize = 32_767;
/// Packet types ([MS-TDS] 2.2.3.1.1).
pub const SQL_BATCH: u8 = 0x01;
pub const REPLY: u8 = 0x04;
pub const LOGIN7: u8 = 0x10;
pub const PRELOGIN: u8 = 0x12;
/// Status bit: end of message.
const EOM: u8 = 0x01;

/// Writes `payload` as one message of packets of at most `packet_size` bytes.
pub fn write_message<W: Write>(w: &mut W, ty: u8, payload: &[u8], packet_size: usize) -> io::Result<()> {
    let chunk = packet_size.clamp(512, MAX_PACKET) - HEADER;
    let count = payload.len().div_ceil(chunk).max(1);
    let mut out = Vec::with_capacity(payload.len() + count * HEADER);
    for i in 0..count {
        let start = i * chunk;
        let body = payload
            .get(start..payload.len().min(start + chunk))
            .unwrap_or_default();
        let last = i + 1 == count;
        // `body.len() + HEADER` ≤ MAX_PACKET by the clamp above
        let len = u16::try_from(body.len() + HEADER).unwrap_or(u16::MAX);
        out.push(ty);
        out.push(if last { EOM } else { 0 });
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.push(((i + 1) % 256) as u8);
        out.push(0);
        out.extend_from_slice(body);
    }
    w.write_all(&out)?;
    w.flush()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// One message read packet by packet, as a byte stream that ends after the `EOM` packet.
/// Packets must be of the types in `types`; the message may hold at most `cap` payload bytes.
pub struct Message<'a, R: Read> {
    inner: &'a mut R,
    types: &'static [u8],
    buf: Vec<u8>,
    pos: usize,
    done: bool,
    total: u64,
    cap: u64,
}

impl<'a, R: Read> Message<'a, R> {
    pub fn new(inner: &'a mut R, types: &'static [u8], cap: u64) -> Self {
        Self {
            inner,
            types,
            buf: Vec::new(),
            pos: 0,
            done: false,
            total: 0,
            cap,
        }
    }

    /// Reads the next packet of the message into `buf`.
    fn packet(&mut self) -> io::Result<()> {
        let mut h = [0u8; HEADER];
        self.inner.read_exact(&mut h)?;
        let [ty, status, l0, l1, ..] = h;
        if !self.types.contains(&ty) {
            return Err(invalid(format!("unexpected TDS packet type 0x{ty:02X}")));
        }
        let len = usize::from(u16::from_be_bytes([l0, l1]));
        if !(HEADER..=MAX_PACKET).contains(&len) {
            return Err(invalid(format!("TDS packet length {len} out of range")));
        }
        let body = len - HEADER;
        self.total = self.total.saturating_add(body as u64);
        if self.total > self.cap {
            return Err(invalid(format!("the server's answer exceeds {} bytes", self.cap)));
        }
        self.buf.resize(body, 0);
        self.inner.read_exact(&mut self.buf)?;
        self.pos = 0;
        self.done = status & EOM != 0;
        Ok(())
    }
}

impl<R: Read> Read for Message<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            if self.done {
                return Ok(0);
            }
            self.packet()?;
        }
        let rest = self.buf.get(self.pos..).unwrap_or_default();
        let n = rest.len().min(out.len());
        if let (Some(dst), Some(src)) = (out.get_mut(..n), rest.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_split_and_join() {
        let payload: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let mut wire = Vec::new();
        write_message(&mut wire, SQL_BATCH, &payload, 512).unwrap();
        // 504 payload bytes per packet
        assert_eq!(wire.len(), payload.len() + 10 * HEADER);
        assert_eq!(&wire[..8], &[0x01, 0x00, 0x02, 0x00, 0, 0, 1, 0]);
        let mut r = wire.as_slice();
        let mut back = Vec::new();
        Message::new(&mut r, &[SQL_BATCH], 1 << 20)
            .read_to_end(&mut back)
            .unwrap();
        assert_eq!(back, payload);
        // an empty message is one header
        let mut wire = Vec::new();
        write_message(&mut wire, SQL_BATCH, &[], 4096).unwrap();
        assert_eq!(wire, [0x01, 0x01, 0x00, 0x08, 0, 0, 1, 0]);
    }

    #[test]
    fn bad_packets_are_errors() {
        for wire in [
            &[0x07u8, 1, 0, 8, 0, 0, 1, 0][..],
            &[0x04, 1, 0, 7, 0, 0, 1, 0],
            &[0x04, 1, 0xFF, 0xFF, 0, 0, 1, 0],
            &[0x04, 1, 0, 20, 0, 0, 1, 0, 1, 2],
            &[0x04, 0, 0, 8, 0, 0, 1, 0],
        ] {
            let mut r = wire;
            let mut out = Vec::new();
            assert!(
                Message::new(&mut r, &[REPLY], 1 << 20)
                    .read_to_end(&mut out)
                    .is_err()
            );
        }
        let mut wire = Vec::new();
        write_message(&mut wire, REPLY, &[0; 3000], 1024).unwrap();
        let mut r = wire.as_slice();
        let mut out = Vec::new();
        assert!(
            Message::new(&mut r, &[REPLY], 2000)
                .read_to_end(&mut out)
                .is_err()
        );
    }
}
