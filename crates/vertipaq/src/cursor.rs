//! Bounds-checked little-endian reader over a byte slice.

use crate::{Error, FileKind};

pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    kind: FileKind,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(data: &'a [u8], kind: FileKind) -> Self {
        Cursor { data, pos: 0, kind }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn is_at_end(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn truncated(&self, what: &'static str) -> Error {
        Error::Truncated {
            kind: self.kind,
            offset: self.pos,
            what,
        }
    }

    pub(crate) fn malformed(&self, message: impl Into<String>) -> Error {
        Error::Malformed {
            kind: self.kind,
            offset: self.pos,
            message: message.into(),
        }
    }

    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.truncated(what))?;
        let slice = self.data.get(self.pos..end).ok_or_else(|| self.truncated(what))?;
        self.pos = end;
        Ok(slice)
    }

    /// Takes `count * size` bytes, failing (without overflow) when that exceeds the input.
    pub(crate) fn take_array(
        &mut self,
        count: u64,
        size: usize,
        what: &'static str,
    ) -> Result<&'a [u8], Error> {
        let n = usize::try_from(count)
            .ok()
            .and_then(|c| c.checked_mul(size))
            .ok_or_else(|| self.truncated(what))?;
        self.take(n, what)
    }

    fn array<const N: usize>(&mut self, what: &'static str) -> Result<[u8; N], Error> {
        let s = self.take(N, what)?;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8, Error> {
        Ok(self.array::<1>(what)?[0])
    }

    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn i32(&mut self, what: &'static str) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn u64(&mut self, what: &'static str) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.array(what)?))
    }

    pub(crate) fn i64(&mut self, what: &'static str) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.array(what)?))
    }

    /// Reads exactly `tag` or fails with a `Malformed` error naming it.
    pub(crate) fn expect(&mut self, tag: &[u8], name: &'static str) -> Result<(), Error> {
        let at = self.pos;
        let got = self.take(tag.len(), name)?;
        if got != tag {
            return Err(Error::Malformed {
                kind: self.kind,
                offset: at,
                message: format!("expected marker {name}"),
            });
        }
        Ok(())
    }

    /// True when the next bytes equal `tag` (nothing is consumed).
    pub(crate) fn peek_is(&self, tag: &[u8]) -> bool {
        self.pos
            .checked_add(tag.len())
            .and_then(|end| self.data.get(self.pos..end))
            .is_some_and(|s| s == tag)
    }
}

/// Little-endian `u64` words of a byte slice whose length is a multiple of 8.
pub(crate) fn words(bytes: &[u8]) -> impl Iterator<Item = u64> + '_ {
    bytes.as_chunks::<8>().0.iter().map(|w| u64::from_le_bytes(*w))
}
