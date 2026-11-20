//! Bugcheck header of a 64-bit kernel minidump (`PAGEDU64`). Only the fixed
//! header is read; nothing in the dump is executed or symbolised.

/// Bytes the helper reads from the start of each file.
pub const HEADER_LEN: usize = 0x60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpHeader {
    pub bugcheck_code: u32,
    pub parameters: [u64; 4],
}

/// DUMP_HEADER64: Signature "PAGE" @0, ValidDump "DU64" @4,
/// BugCheckCode u32 @0x38, BugCheckParameter1..4 u64 @0x40.
pub fn parse_dump_header(b: &[u8]) -> Result<DumpHeader, String> {
    if b.len() < HEADER_LEN || &b[0..4] != b"PAGE" || &b[4..8] != b"DU64" {
        return Err("not a 64-bit kernel dump header".into());
    }
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or_default());
    Ok(DumpHeader {
        bugcheck_code: u32::from_le_bytes(b[0x38..0x3c].try_into().unwrap_or_default()),
        parameters: [u64_at(0x40), u64_at(0x48), u64_at(0x50), u64_at(0x58)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header() {
        let mut b = vec![0u8; 0x2000];
        b[0..8].copy_from_slice(b"PAGEDU64");
        b[0x38..0x3c].copy_from_slice(&0x0000_00d1u32.to_le_bytes());
        b[0x40..0x48].copy_from_slice(&0xffff_f800_1234_0000u64.to_le_bytes());
        b[0x58..0x60].copy_from_slice(&7u64.to_le_bytes());
        let h = parse_dump_header(&b).unwrap();
        assert_eq!(h.bugcheck_code, 0xd1);
        assert_eq!(h.parameters, [0xffff_f800_1234_0000, 0, 0, 7]);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse_dump_header(b"PAGEDUMP").is_err());
        let mut b = vec![0u8; HEADER_LEN];
        b[0..8].copy_from_slice(b"PAGEDUMP"); // 32-bit dump
        assert!(parse_dump_header(&b).is_err());
        b[0..8].copy_from_slice(b"MDMP\x93\xa7\0\0"); // user-mode minidump
        assert!(parse_dump_header(&b).is_err());
    }
}
