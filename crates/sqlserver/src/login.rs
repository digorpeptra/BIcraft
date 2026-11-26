//! The messages a client sends: `PRELOGIN` ([MS-TDS] 2.2.6.5), `LOGIN7` (2.2.6.4) and
//! `SQLBatch` (2.2.6.7), and the server's `PRELOGIN` answer.

use bicraft_data::connect::ConnectError;

use crate::wire::{protocol, to_utf16};

/// TDS 7.4.
pub const TDS_74: u32 = 0x7400_0004;

/// `PRELOGIN` encryption values.
pub const ENCRYPT_OFF: u8 = 0x00;
pub const ENCRYPT_ON: u8 = 0x01;
pub const ENCRYPT_NOT_SUP: u8 = 0x02;
pub const ENCRYPT_REQ: u8 = 0x03;

/// A `PRELOGIN` asking for an encrypted connection.
pub fn prelogin() -> Vec<u8> {
    // VERSION, ENCRYPTION, INSTOPT, THREADID, MARS, then the terminator: 5 × 5 + 1 bytes
    let data: [&[u8]; 5] = [&[0, 1, 0, 0, 0, 0], &[ENCRYPT_ON], &[0], &[0, 0, 0, 0], &[0]];
    let mut head = Vec::new();
    let mut body = Vec::new();
    let mut offset = 26u16;
    for (token, d) in data.iter().enumerate() {
        let len = d.len() as u16;
        head.push(token as u8);
        head.extend_from_slice(&offset.to_be_bytes());
        head.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(d);
        offset += len;
    }
    head.push(0xFF);
    head.extend_from_slice(&body);
    head
}

/// The encryption option of the server's `PRELOGIN` answer.
pub fn prelogin_encryption(answer: &[u8]) -> Result<u8, ConnectError> {
    let bad = || protocol("the server's PRELOGIN answer is malformed");
    let mut at = 0usize;
    for _ in 0..64 {
        let token = *answer.get(at).ok_or_else(bad)?;
        if token == 0xFF {
            break;
        }
        let field = |i: usize| -> Option<u16> {
            Some(u16::from_be_bytes([
                *answer.get(at + i)?,
                *answer.get(at + i + 1)?,
            ]))
        };
        let (offset, len) = (
            usize::from(field(1).ok_or_else(bad)?),
            usize::from(field(3).ok_or_else(bad)?),
        );
        let value = answer.get(offset..offset + len).ok_or_else(bad)?;
        if token == 1 {
            return value.first().copied().ok_or_else(bad);
        }
        at += 5;
    }
    Err(protocol("the server's PRELOGIN answer has no encryption option"))
}

/// What `LOGIN7` carries.
pub struct Login<'a> {
    pub host_name: &'a str,
    pub user: &'a str,
    pub password: &'a str,
    pub app_name: &'a str,
    pub server_name: &'a str,
    pub database: &'a str,
    pub packet_size: u32,
}

/// `LOGIN7` with SQL Server authentication.
pub fn login7(l: &Login<'_>) -> Result<Vec<u8>, ConnectError> {
    const FIXED: usize = 94;
    for (what, s) in [
        ("login", l.user),
        ("password", l.password),
        ("database", l.database),
    ] {
        if s.encode_utf16().count() > 128 {
            return Err(ConnectError::new(
                bicraft_data::connect::ConnectErrorKind::Unsupported,
                format!("the {what} is longer than 128 characters"),
            ));
        }
    }
    let clip = |s: &str| s.chars().take(128).collect::<String>();
    let password: Vec<u8> = to_utf16(l.password)
        .into_iter()
        .map(|b| b.rotate_right(4) ^ 0xA5)
        .collect();
    // host, user, password, app, server, (extension), library, language, database
    let fields: [(Vec<u8>, bool); 9] = [
        (to_utf16(&clip(l.host_name)), true),
        (to_utf16(l.user), true),
        (password, true),
        (to_utf16(&clip(l.app_name)), true),
        (to_utf16(&clip(l.server_name)), true),
        (Vec::new(), false),
        (to_utf16("bicraft"), true),
        (Vec::new(), true),
        (to_utf16(l.database), true),
    ];
    let mut head = Vec::with_capacity(FIXED);
    let mut data = Vec::new();
    head.extend_from_slice(&0u32.to_le_bytes()); // total length, set below
    head.extend_from_slice(&TDS_74.to_le_bytes());
    head.extend_from_slice(&l.packet_size.to_le_bytes());
    head.extend_from_slice(&1u32.to_le_bytes()); // client program version
    head.extend_from_slice(&std::process::id().to_le_bytes());
    head.extend_from_slice(&0u32.to_le_bytes()); // connection id
    // OptionFlags1: USE DB and INIT DB fatal, SET LANG on; OptionFlags2: INIT LANG fatal, ODBC
    // (the ANSI session options); TypeFlags; OptionFlags3
    head.extend_from_slice(&[0xE0, 0x03, 0x00, 0x00]);
    head.extend_from_slice(&0i32.to_le_bytes()); // time zone
    head.extend_from_slice(&0x0409u32.to_le_bytes()); // LCID
    let mut offset = FIXED;
    for (bytes, chars) in &fields {
        let off = u16::try_from(offset).map_err(|_| protocol("LOGIN7 too long"))?;
        let n = if *chars { bytes.len() / 2 } else { bytes.len() };
        head.extend_from_slice(&off.to_le_bytes());
        head.extend_from_slice(
            &u16::try_from(n)
                .map_err(|_| protocol("LOGIN7 too long"))?
                .to_le_bytes(),
        );
        data.extend_from_slice(bytes);
        offset += bytes.len();
    }
    head.extend_from_slice(&[0; 6]); // client id
    let off = u16::try_from(offset).map_err(|_| protocol("LOGIN7 too long"))?;
    for _ in 0..3 {
        // SSPI, attach file, change password: empty
        head.extend_from_slice(&off.to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes());
    }
    head.extend_from_slice(&0u32.to_le_bytes()); // long SSPI
    head.extend_from_slice(&data);
    let total = u32::try_from(head.len()).map_err(|_| protocol("LOGIN7 too long"))?;
    if let Some(first) = head.get_mut(..4) {
        first.copy_from_slice(&total.to_le_bytes());
    }
    Ok(head)
}

/// The password of a `LOGIN7` (for the fake server's checks): the scrambling undone.
#[cfg(any(test, feature = "fake-server"))]
pub fn unscramble(b: &[u8]) -> Vec<u8> {
    b.iter()
        .map(|&x| {
            let y = x ^ 0xA5;
            y.rotate_right(4)
        })
        .collect()
}

/// A `SQLBatch` payload: `ALL_HEADERS` with the transaction descriptor (auto-commit), then the
/// text.
pub fn batch(sql: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(22 + sql.len() * 2);
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(&18u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&to_utf16(sql));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelogin_round_trip() {
        let p = prelogin();
        assert_eq!(&p[..5], &[0, 0, 26, 0, 6]);
        assert_eq!(p[25], 0xFF);
        assert_eq!(prelogin_encryption(&p).unwrap(), ENCRYPT_ON);
        assert!(prelogin_encryption(&[1, 0, 200, 0, 1, 0xFF]).is_err());
        assert!(prelogin_encryption(&[0xFF]).is_err());
        assert!(prelogin_encryption(&[]).is_err());
    }

    #[test]
    fn login7_layout() {
        let l = login7(&Login {
            host_name: "pc",
            user: "sa",
            password: "pw",
            app_name: "BIcraft",
            server_name: "db.example",
            database: "Sales",
            packet_size: 4096,
        })
        .unwrap();
        assert_eq!(u32::from_le_bytes(l[..4].try_into().unwrap()) as usize, l.len());
        assert_eq!(&l[4..8], &TDS_74.to_le_bytes());
        // the user name field: offset 94 + 4 (host "pc"), 2 characters
        assert_eq!(&l[40..44], &[98, 0, 2, 0]);
        let pw_off = usize::from(u16::from_le_bytes([l[44], l[45]]));
        assert_eq!(unscramble(&l[pw_off..pw_off + 4]), to_utf16("pw"));
        assert!(
            login7(&Login {
                host_name: "",
                user: &"u".repeat(129),
                password: "",
                app_name: "",
                server_name: "",
                database: "",
                packet_size: 4096,
            })
            .is_err()
        );
    }
}
