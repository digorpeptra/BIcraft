//! Golden bytes of the package writer: a fixed package written, read back through the ZIP reader
//! (the `zip` crate) and written again gives the same archive, byte for byte, with the CRC-32 and
//! length pinned here. The pins were taken before the move from `zip` 2 to 9 (docs/gaps.md G38),
//! so a reader or writer change that alters what Desktop gets shows up as a red test. The parallel
//! reader of large stored parts and parts left in the file (deferred) take the same check.

use bicraft_opc::{Package, ReadLimits, WriteOptions};

/// A package shaped like Desktop's: UTF-16 `Version`, XML and JSON parts deflated, the `DataModel`
/// stored (large enough for the parallel reader), an escaped name, a non-ASCII name and an empty
/// part.
fn package() -> Package {
    let mut pkg = Package::default();
    let utf16 = |s: &str| -> Vec<u8> {
        let mut v = vec![0xFF, 0xFE];
        v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        v
    };
    pkg.put("Version", utf16("1.28"), true);
    pkg.put(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="utf-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="json" ContentType="" /></Types>"#.to_vec(),
        false,
    );
    let layout: String = (0..400)
        .map(|i| {
            format!(
                r#"{{"id":{i},"x":{},"y":{},"config":"{{\"name\":\"v{i}\"}}"}},"#,
                i * 7 % 1280,
                i * 13 % 720
            )
        })
        .collect();
    pkg.put(
        "Report/Layout",
        utf16(&format!("{{\"sections\":[{layout}]}}")),
        false,
    );
    // 9 MiB of a repeating pattern with some noise: stored, and over the 8 MiB from which a file's
    // stored part is read on several threads
    let mut model = Vec::with_capacity(9 << 20);
    let mut x: u32 = 0x1234_5678;
    while model.len() < 9 << 20 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        model.extend_from_slice(&x.to_le_bytes()[..(x % 4 + 1) as usize]);
        model.extend_from_slice(b"VertiPaq");
    }
    pkg.put("DataModel", model, true);
    pkg.put(
        "Report/StaticResources/RegisteredResources/My%20Logo.png",
        vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3],
        false,
    );
    pkg.put(
        "Report/StaticResources/SharedResources/BaseThemes/Thème été.json",
        r#"{"name":"été"}"#.as_bytes().to_vec(),
        false,
    );
    pkg.put("SecurityBindings", Vec::new(), true);
    pkg.put("Settings", vec![0; 300], false);
    pkg
}

/// Written as it is, and with every record in its Zip64 form: (CRC-32, length).
const GOLDEN: [(u32, usize); 2] = [(0x34aa_cb3e, 9_443_074), (0x5b6c_51ae, 9_443_534)];

#[test]
fn written_bytes_are_pinned_and_survive_a_read() {
    let pkg = package();
    let dir = std::env::temp_dir().join(format!("bicraft-opc-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, opts) in [WriteOptions::default(), WriteOptions { force_zip64: true }]
        .iter()
        .enumerate()
    {
        let bytes = pkg.to_bytes_with(opts).unwrap();
        let got = (crc32fast::hash(&bytes), bytes.len());
        assert_eq!(
            got, GOLDEN[i],
            "zip64 {}: {:#010x}, {}",
            opts.force_zip64, got.0, got.1
        );
        // read from memory (the ZIP reader), then written again
        let back = Package::read_bytes(&bytes).unwrap();
        assert_eq!(back, pkg);
        assert_eq!(back.to_bytes_with(opts).unwrap(), bytes);
        // read from a file: the large stored part on several threads, then deferred parts
        let path = dir.join(format!("p{i}.pbix"));
        std::fs::write(&path, &bytes).unwrap();
        let opened = Package::open(&path).unwrap();
        assert_eq!(opened.to_bytes_with(opts).unwrap(), bytes);
        let deferred =
            Package::open_deferring(&path, &ReadLimits::default(), &["DataModel", "Report/Layout"]).unwrap();
        assert!(deferred.get("DataModel").unwrap().is_deferred());
        assert_eq!(deferred.to_bytes_with(opts).unwrap(), bytes);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
