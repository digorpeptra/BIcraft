//! Test-only builder for synthetic ABF streams, written from [MS-XLDM] §2.1 and the Power BI
//! layout observed in `docs/formats/abf.md`. All content is our own.

use crate::PAGE_SIZE;

pub(crate) const PERSIST_ROOT: &str = r"\\?\C:\Synth\Data\00000000-0000-0000-0000-000000000001.0.db";

pub(crate) struct Options {
    /// Excel flavor: `ErrorCode` true and a 4-byte end marker after every file.
    pub crc_markers: bool,
    /// Encode the virtual directory as UTF-16LE instead of UTF-8.
    pub dir_utf16: bool,
    /// Emit the Power BI bookkeeping files (`ADDITIONAL_LOG`, `PARTITIONS`).
    pub power_bi_bookkeeping: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            crc_markers: false,
            dir_utf16: false,
            power_bi_bookkeeping: true,
        }
    }
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn bom_utf16(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    v.extend(utf16(s));
    v
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn pad_to_page(v: &mut Vec<u8>) {
    let rem = v.len() % PAGE_SIZE;
    if rem != 0 {
        v.resize(v.len() + PAGE_SIZE - rem, 0);
    }
}

/// Builds an ABF holding `files` (relative logical path, payload).
pub(crate) fn build(files: &[(&str, &[u8])], opts: &Options) -> Vec<u8> {
    let marker = if opts.crc_markers { 4 } else { 0 };
    let mut stream = vec![0u8; PAGE_SIZE];
    // (storage name, offset, stored size)
    let mut dir: Vec<(String, usize, usize)> = Vec::new();
    let mut push = |stream: &mut Vec<u8>, name: String, data: &[u8]| {
        let offset = stream.len();
        stream.extend_from_slice(data);
        stream.extend(std::iter::repeat_n(0xA5u8, marker));
        dir.push((name, offset, data.len() + marker));
    };

    if opts.power_bi_bookkeeping {
        push(
            &mut stream,
            "ADDITIONAL_LOG".into(),
            &bom_utf16("<Property><ProductName>Synth</ProductName></Property>"),
        );
        push(&mut stream, "PARTITIONS".into(), &bom_utf16("<Partitions/>"));
    }
    let mut log_files = String::new();
    for (i, (path, data)) in files.iter().enumerate() {
        let storage = format!("{:020X}", 0xABF0_0000u64 + i as u64);
        push(&mut stream, storage.clone(), data);
        log_files.push_str(&format!(
            "<BackupFile><Path>{}\\{}</Path><StoragePath>{storage}</StoragePath>\
             <LastWriteTime>134000000000000000</LastWriteTime><Size>{}</Size></BackupFile>",
            esc(PERSIST_ROOT),
            esc(&path.replace('/', "\\")),
            data.len()
        ));
    }
    let log = format!(
        "<BackupLog><BackupRestoreSyncVersion>11.53</BackupRestoreSyncVersion><ServerRoot>x</ServerRoot>\
         <ObjectName>synth</ObjectName><ObjectId>synth</ObjectId><Write>ReadWrite</Write><IsTabular>true</IsTabular>\
         <Collations/><Languages><Language>1033</Language></Languages><FileGroups><FileGroup><Class>100069</Class>\
         <ID>g</ID><Name>g</Name><PersistLocationPath>{}</PersistLocationPath><StorageLocationPath/>\
         <FileList>{log_files}</FileList></FileGroup></FileGroups></BackupLog>",
        esc(PERSIST_ROOT)
    );
    push(&mut stream, "LOG".into(), &bom_utf16(&log));
    pad_to_page(&mut stream);

    let dir_offset = stream.len();
    let mut dir_xml = String::from("<VirtualDirectory>");
    for (name, offset, size) in &dir {
        dir_xml.push_str(&format!(
            "<BackupFile><Path>{name}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader>\
             <Delete>false</Delete><CreatedTimestamp>1</CreatedTimestamp><Access>1</Access>\
             <LastWriteTime>134000000000000001</LastWriteTime></BackupFile>"
        ));
    }
    dir_xml.push_str("</VirtualDirectory>");
    let dir_bytes = if opts.dir_utf16 {
        utf16(&dir_xml)
    } else {
        dir_xml.into_bytes()
    };
    stream.extend_from_slice(&dir_bytes);
    pad_to_page(&mut stream);

    let header = format!(
        "<BackupLog><BackupRestoreSyncVersion>140</BackupRestoreSyncVersion><Fault>false</Fault>\
         <faultcode>0</faultcode><ErrorCode>{}</ErrorCode><EncryptionFlag>false</EncryptionFlag>\
         <EncryptionKey>0</EncryptionKey><ApplyCompression>false</ApplyCompression>\
         <m_cbOffsetHeader>{dir_offset}</m_cbOffsetHeader><DataSize>{}</DataSize><Files>{}</Files>\
         <ObjectID>00000000-0000-0000-0000-000000000002</ObjectID><m_cbOffsetData>4096</m_cbOffsetData></BackupLog>",
        opts.crc_markers,
        dir_bytes.len(),
        dir.len()
    );
    let mut page = vec![0xFF, 0xFE];
    page.extend(utf16("STREAM_STORAGE_SIGNATURE_)!@#$%^&*("));
    page.extend(utf16(&header));
    assert!(page.len() <= PAGE_SIZE, "synthetic header too large");
    if let Some(dst) = stream.get_mut(..page.len()) {
        dst.copy_from_slice(&page);
    }
    stream
}

/// A small Power BI-shaped model: metadata, one user table, one H$ table, one R$ index.
pub(crate) fn sample_files() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("metadata.sqlitedb", b"SQLite format 3\0synthetic"),
        ("0.CryptKey.bin", b"\x01\x02\x03"),
        (
            "Sales (12).tbl/13.prt/1.Sales (12).Amount (14).0.idf",
            b"idf-amount",
        ),
        (
            "Sales (12).tbl/13.prt/1.Sales (12).Amount (14).0.idfmeta",
            b"<1:CP\0meta",
        ),
        ("Sales (12).tbl/1.Sales (12).Name & Co (15).dictionary", b"dict"),
        ("Sales (12).tbl/1.Sales (12).Amount (14).hidx", b"hidx"),
        (
            "H$Sales (12)$Name (15)$(16).tbl/17.prt/1.H$Sales (12)$Name (15).POS_TO_ID.0.idf",
            b"h-idf",
        ),
        (
            "R$Sales (12)$(18).tbl/19.prt/1.R$Sales (12)$(18).INDEX.0.idf",
            b"r-idf",
        ),
        ("Empty (20).tbl/21.prt/1.Empty (20).Col (22).0.idf", b""),
    ]
}
