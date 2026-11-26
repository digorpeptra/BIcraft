//! A Windows shortcut (`.lnk`) writer for the Start menu entry of `--register`, from the public
//! specifications [MS-SHLLINK] (Shell Link Binary File Format) and [MS-PROPSTORE] (the property
//! store that carries `System.AppUserModel.ID`), so the shortcut, the window and the taskbar
//! button share one AppUserModelID. Pure Rust, no COM: written the same on every platform.

/// `{00021401-0000-0000-C000-000000000046}`, the shell link class.
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
/// `{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}`, the AppUserModel property set (`System.AppUserModel.*`).
const APP_USER_MODEL_FMTID: [u8; 16] = [
    0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3,
];
/// `System.AppUserModel.ID`'s property id in that set.
const PID_APP_USER_MODEL_ID: u32 = 5;

// LinkFlags
const HAS_LINK_INFO: u32 = 0x0000_0002;
const HAS_NAME: u32 = 0x0000_0004;
const HAS_WORKING_DIR: u32 = 0x0000_0010;
const HAS_ICON_LOCATION: u32 = 0x0000_0040;
const IS_UNICODE: u32 = 0x0000_0080;
/// FILE_ATTRIBUTE_ARCHIVE
const ARCHIVE: u32 = 0x20;
/// SW_SHOWNORMAL
const SHOW_NORMAL: u32 = 1;
/// DRIVE_FIXED
const DRIVE_FIXED: u32 = 3;
/// VolumeIDAndLocalBasePath
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 1;
/// PropertyStoreDataBlock
const PROPERTY_STORE_SIGNATURE: u32 = 0xA000_0009;
/// '1SPS'
const SPS_VERSION: u32 = 0x5350_5331;
/// VT_LPWSTR
const VT_LPWSTR: u16 = 0x001F;

/// What the shortcut points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The target's absolute path (`C:\…\bicraft-desktop.exe`).
    pub target: String,
    pub working_dir: String,
    pub description: String,
    /// The icon file (the target itself) and its index 0.
    pub icon: String,
    pub app_user_model_id: String,
}

fn u16le(b: &mut Vec<u8>, x: u16) {
    b.extend_from_slice(&x.to_le_bytes());
}
fn u32le(b: &mut Vec<u8>, x: u32) {
    b.extend_from_slice(&x.to_le_bytes());
}
fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// StringData: a count of UTF-16 code units, then the units (no terminator).
fn string_data(b: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().take(u16::MAX as usize).collect();
    u16le(b, u16::try_from(units.len()).unwrap_or(u16::MAX));
    for u in units {
        u16le(b, u);
    }
}

/// LinkInfo with a VolumeID and the local base path (ANSI with `?` for non-ASCII characters,
/// and Unicode).
fn link_info(target: &str) -> Vec<u8> {
    const HEADER: usize = 0x24;
    let mut volume = Vec::new();
    u32le(&mut volume, 0x11); // VolumeIDSize: 16 + an empty label
    u32le(&mut volume, DRIVE_FIXED);
    u32le(&mut volume, 0); // serial number
    u32le(&mut volume, 0x10); // VolumeLabelOffset
    volume.push(0);
    let ansi: Vec<u8> = target
        .chars()
        .map(|c| if c.is_ascii() && c != '\0' { c as u8 } else { b'?' })
        .chain([0])
        .collect();
    let mut unicode = Vec::new();
    for u in target.encode_utf16().chain([0]) {
        u16le(&mut unicode, u);
    }
    let volume_at = HEADER;
    let base_at = volume_at + volume.len();
    let suffix_at = base_at + ansi.len();
    let base_u_at = suffix_at + 1;
    let suffix_u_at = base_u_at + unicode.len();
    let size = suffix_u_at + 2;
    let mut b = Vec::with_capacity(size);
    u32le(&mut b, len32(size));
    u32le(&mut b, len32(HEADER));
    u32le(&mut b, VOLUME_ID_AND_LOCAL_BASE_PATH);
    u32le(&mut b, len32(volume_at));
    u32le(&mut b, len32(base_at));
    u32le(&mut b, 0); // CommonNetworkRelativeLinkOffset
    u32le(&mut b, len32(suffix_at));
    u32le(&mut b, len32(base_u_at));
    u32le(&mut b, len32(suffix_u_at));
    b.extend_from_slice(&volume);
    b.extend_from_slice(&ansi);
    b.push(0); // CommonPathSuffix: empty
    b.extend_from_slice(&unicode);
    u16le(&mut b, 0); // CommonPathSuffixUnicode: empty
    b
}

/// A PropertyStoreDataBlock holding one string property of the AppUserModel set.
fn property_store(id: &str) -> Vec<u8> {
    // TypedPropertyValue: VT_LPWSTR, padding, then a UnicodeString (length in characters with
    // the terminator, the characters, zero padding to a multiple of 4 bytes)
    let units: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let mut value = Vec::new();
    u16le(&mut value, VT_LPWSTR);
    u16le(&mut value, 0);
    u32le(&mut value, len32(units.len()));
    for u in &units {
        u16le(&mut value, *u);
    }
    while value.len() % 4 != 0 {
        value.push(0);
    }
    // Serialized Property Value (integer name): size, id, reserved byte, value
    let mut prop = Vec::new();
    u32le(&mut prop, len32(4 + 4 + 1 + value.len()));
    u32le(&mut prop, PID_APP_USER_MODEL_ID);
    prop.push(0);
    prop.extend_from_slice(&value);
    // Serialized Property Storage: size, version, format id, the values, a zero-size value
    let mut storage = Vec::new();
    u32le(&mut storage, len32(4 + 4 + 16 + prop.len() + 4));
    u32le(&mut storage, SPS_VERSION);
    storage.extend_from_slice(&APP_USER_MODEL_FMTID);
    storage.extend_from_slice(&prop);
    u32le(&mut storage, 0);
    // the store ends with a zero-size storage
    let mut block = Vec::new();
    u32le(&mut block, len32(4 + 4 + storage.len() + 4));
    u32le(&mut block, PROPERTY_STORE_SIGNATURE);
    block.extend_from_slice(&storage);
    u32le(&mut block, 0);
    block
}

/// The `.lnk` file's bytes.
pub fn shortcut_bytes(s: &Shortcut) -> Vec<u8> {
    let mut b = Vec::new();
    // ShellLinkHeader (76 bytes)
    u32le(&mut b, 0x4C);
    b.extend_from_slice(&LINK_CLSID);
    u32le(
        &mut b,
        HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ICON_LOCATION | IS_UNICODE,
    );
    u32le(&mut b, ARCHIVE);
    b.extend_from_slice(&[0; 24]); // creation, access and write times
    u32le(&mut b, 0); // FileSize
    u32le(&mut b, 0); // IconIndex
    u32le(&mut b, SHOW_NORMAL);
    u16le(&mut b, 0); // HotKey
    u16le(&mut b, 0);
    u32le(&mut b, 0);
    u32le(&mut b, 0);
    b.extend_from_slice(&link_info(&s.target));
    // StringData in the specification's order: name, working dir, icon location
    string_data(&mut b, &s.description);
    string_data(&mut b, &s.working_dir);
    string_data(&mut b, &s.icon);
    // ExtraData: the property store, then the TerminalBlock
    b.extend_from_slice(&property_store(&s.app_user_model_id));
    u32le(&mut b, 0);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    #[test]
    fn layout_follows_ms_shllink() {
        let s = Shortcut {
            target: r"C:\Users\Zoë\AppData\Local\bicraft\bicraft-desktop.exe".into(),
            working_dir: r"C:\Users\Zoë\AppData\Local\bicraft".into(),
            description: "BIcraft".into(),
            icon: r"C:\Users\Zoë\AppData\Local\bicraft\bicraft-desktop.exe".into(),
            app_user_model_id: "BIcraft.Desktop".into(),
        };
        let b = shortcut_bytes(&s);
        assert_eq!(u32_at(&b, 0), 0x4C);
        assert_eq!(&b[4..20], &LINK_CLSID);
        assert_eq!(u32_at(&b, 20), 0xD6);
        // LinkInfo right after the header, its size and its Unicode path
        let info = 0x4C;
        let size = u32_at(&b, info) as usize;
        assert_eq!(u32_at(&b, info + 4), 0x24);
        let base_u = u32_at(&b, info + 28) as usize;
        let path: Vec<u16> = b[info + base_u..info + size]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        assert_eq!(String::from_utf16(&path).unwrap(), s.target);
        // the ANSI path replaces the non-ASCII character
        let base = u32_at(&b, info + 16) as usize;
        assert!(b[info + base..].starts_with(br"C:\Users\Zo?\"));
        // the description's StringData follows LinkInfo
        let sd = info + size;
        assert_eq!(u16::from_le_bytes([b[sd], b[sd + 1]]), 7);
        // the property store: signature, '1SPS', the format id, PID 5, VT_LPWSTR, the id
        let at = b
            .windows(4)
            .position(|w| w == PROPERTY_STORE_SIGNATURE.to_le_bytes())
            .unwrap()
            - 4;
        let block = u32_at(&b, at) as usize;
        assert_eq!(at + block + 4, b.len(), "the terminal block ends the file");
        assert_eq!(u32_at(&b, at + 12), SPS_VERSION);
        assert_eq!(&b[at + 16..at + 32], &APP_USER_MODEL_FMTID);
        assert_eq!(u32_at(&b, at + 36), PID_APP_USER_MODEL_ID);
        assert_eq!(u16::from_le_bytes([b[at + 41], b[at + 42]]), VT_LPWSTR);
        assert_eq!(u32_at(&b, at + 45), 16, "15 characters and the terminator");
        let storage = u32_at(&b, at + 8) as usize;
        assert_eq!(
            u32_at(&b, at + 8 + storage),
            0,
            "the store ends with a zero-size storage"
        );
        assert_eq!(
            u32_at(&b, at + 8 + storage - 4),
            0,
            "the storage ends with a zero-size value"
        );
    }
}
