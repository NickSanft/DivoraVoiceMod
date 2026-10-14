//! `divora-app.exe` must not import Media Foundation at load time.
//!
//! Windows N editions ship without Media Foundation until the Media Feature
//! Pack is installed. A load-time import of `mfplat.dll` or
//! `mfreadwrite.dll` would stop the app from starting at all on those
//! machines — where today it starts, and only screen recording is missing.
//! `divora-capture` therefore loads them at run time (`native::mf`), and
//! this reads the built executable's import tables, normal and delay-load,
//! to prove that no `windows`-crate binding slipped one in.
//!
//! Cargo builds the app binary for this test (`CARGO_BIN_EXE_*`), so it
//! checks exactly what `cargo test` just linked.

#![cfg(windows)]

/// Every DLL `pe` imports, normal and delay-loaded, lowercased.
fn imported_dlls(pe: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| u16::from_le_bytes([pe[o], pe[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes([pe[o], pe[o + 1], pe[o + 2], pe[o + 3]]);
    let pe_at = u32_at(0x3c) as usize;
    assert_eq!(&pe[pe_at..pe_at + 4], b"PE\0\0", "not a PE file");
    let coff = pe_at + 4;
    let sections = usize::from(u16_at(coff + 2));
    let optional = coff + 20;
    let optional_size = usize::from(u16_at(coff + 16));
    // The data directories start 112 bytes into a PE32+ optional header
    // (96 into a PE32 one); each is (RVA, size).
    let dirs = optional + if u16_at(optional) == 0x20b { 112 } else { 96 };
    let section_table = optional + optional_size;
    let to_offset = |rva: u32| -> usize {
        for i in 0..sections {
            let s = section_table + i * 40;
            let (size, va, raw) = (
                u32_at(s + 8).max(u32_at(s + 16)),
                u32_at(s + 12),
                u32_at(s + 20),
            );
            if rva >= va && rva < va + size {
                return (raw + (rva - va)) as usize;
            }
        }
        panic!("RVA {rva:#x} is in no section");
    };
    let c_str = |rva: u32| -> String {
        let at = to_offset(rva);
        let end = pe[at..].iter().position(|&b| b == 0).unwrap();
        String::from_utf8_lossy(&pe[at..at + end]).to_lowercase()
    };

    let mut out = Vec::new();
    // Directory 1: imports. 20-byte descriptors; the name RVA is at +12; a
    // zeroed descriptor ends the table.
    let imports = u32_at(dirs + 8);
    if imports != 0 {
        let mut d = to_offset(imports);
        while u32_at(d + 12) != 0 {
            out.push(c_str(u32_at(d + 12)));
            d += 20;
        }
    }
    // Directory 13: delay-load imports. 32-byte descriptors; the name RVA is
    // at +4.
    let delayed = u32_at(dirs + 13 * 8);
    if delayed != 0 {
        let mut d = to_offset(delayed);
        while u32_at(d + 4) != 0 {
            out.push(c_str(u32_at(d + 4)));
            d += 32;
        }
    }
    out
}

#[test]
fn the_app_does_not_import_media_foundation() {
    let exe = env!("CARGO_BIN_EXE_divora-app");
    let pe = std::fs::read(exe).expect("read divora-app.exe");
    let dlls = imported_dlls(&pe);
    // The reader works: every Windows program imports kernel32.
    assert!(
        dlls.iter().any(|d| d == "kernel32.dll"),
        "import table unreadable: {dlls:?}"
    );
    let mf: Vec<_> = dlls.iter().filter(|d| d.starts_with("mf")).collect();
    assert!(
        mf.is_empty(),
        "divora-app.exe imports Media Foundation at load time: {mf:?}. \
         Resolve it at run time in divora-capture's native::mf instead."
    );
}
