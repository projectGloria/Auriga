use chrono::{DateTime, Utc};

/// NUL-terminated UTF-16 for passing to Win32.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer (possibly NUL-terminated / padded) to String.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Little-endian byte buffer holding UTF-16 to u16 vector.
pub fn bytes_to_u16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect()
}

/// REG_MULTI_SZ / DEVPROP_TYPE_STRING_LIST to Vec<String>.
pub fn multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// FILETIME (100 ns ticks since 1601-01-01 UTC) to DateTime.
pub fn filetime_to_utc(ticks: u64) -> Option<DateTime<Utc>> {
    const EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    if ticks == 0 {
        return None;
    }
    let secs = (ticks / 10_000_000) as i64 - EPOCH_DIFF_SECS;
    let nanos = ((ticks % 10_000_000) * 100) as u32;
    DateTime::from_timestamp(secs, nanos)
}

pub fn format_guid(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    let d1 = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let d2 = u16::from_le_bytes(bytes[4..6].try_into().ok()?);
    let d3 = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
    let d4 = &bytes[8..16];
    Some(format!(
        "{{{d1:08x}-{d2:04x}-{d3:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}}}",
        d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_epoch() {
        // 2020-01-01T00:00:00Z
        let ft = (1_577_836_800i64 + 11_644_473_600) as u64 * 10_000_000;
        assert_eq!(
            filetime_to_utc(ft).unwrap().to_rfc3339(),
            "2020-01-01T00:00:00+00:00"
        );
        assert!(filetime_to_utc(0).is_none());
    }

    #[test]
    fn multi_sz_split() {
        let v: Vec<u16> = "a\0bc\0\0".encode_utf16().collect();
        assert_eq!(multi_sz(&v), vec!["a", "bc"]);
    }

    #[test]
    fn guid_format() {
        let g = [
            0x72, 0xe9, 0x36, 0x4d, 0x25, 0xe3, 0xce, 0x11, 0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1,
            0x03, 0x18,
        ];
        assert_eq!(
            format_guid(&g).unwrap(),
            "{4d36e972-e325-11ce-bfc1-08002be10318}"
        );
    }
}
