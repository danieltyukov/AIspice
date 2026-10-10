//! Text encodings LTspice files arrive in.
//!
//! LTspice XVII writes schematics as Latin-1 (really Windows-1252) or UTF-8.
//! LTspice 24 writes UTF-16LE, usually without a byte order mark. Whatever a
//! file came in as is what it goes back out as, so a round trip through aispice
//! never changes how LTspice reads it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    /// UTF-16 little endian. `bom` records whether the file started with FF FE.
    Utf16Le { bom: bool },
    /// One byte per character. Decoded as Latin-1, which maps every byte.
    Latin1,
}

/// Detect the encoding of `bytes` and decode them.
///
/// Order: a UTF-16LE byte order mark; UTF-16LE without a mark, recognised by a
/// NUL in every odd position of the first few characters (ASCII text encoded
/// as UTF-16LE always looks like that); valid UTF-8; otherwise Latin-1.
pub fn decode(bytes: &[u8]) -> (String, Encoding) {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return (decode_utf16le(&bytes[2..]), Encoding::Utf16Le { bom: true });
    }
    if looks_like_utf16le(bytes) {
        return (decode_utf16le(bytes), Encoding::Utf16Le { bom: false });
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(text) => (text.to_string(), Encoding::Utf8),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    }
}

/// Encode `text` in `encoding`. Characters Latin-1 cannot hold become `?`.
pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le { bom } => {
            let mut out = Vec::with_capacity(text.len() * 2 + 2);
            if bom {
                out.extend_from_slice(&[0xFF, 0xFE]);
            }
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
        Encoding::Latin1 => text
            .chars()
            .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
            .collect(),
    }
}

fn looks_like_utf16le(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(64) & !1];
    if probe.len() < 4 {
        return false;
    }
    let (pairs, _) = probe.as_chunks::<2>();
    pairs.iter().all(|&[lo, hi]| hi == 0 && lo != 0)
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&pair| u16::from_le_bytes(pair));
    char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_round_trip() {
        let (text, enc) = decode(b"Version 4\nSHEET 1 880 680\n");
        assert_eq!(enc, Encoding::Utf8);
        assert_eq!(encode(&text, enc), b"Version 4\nSHEET 1 880 680\n");
    }

    #[test]
    fn utf16_with_and_without_bom() {
        for bom in [true, false] {
            let bytes = encode(
                "Version 4\r\nSHEET 1 880 680\r\n",
                Encoding::Utf16Le { bom },
            );
            let (text, enc) = decode(&bytes);
            assert_eq!(enc, Encoding::Utf16Le { bom });
            assert!(text.starts_with("Version 4"));
            assert_eq!(encode(&text, enc), bytes);
        }
    }

    #[test]
    fn latin1_fallback_keeps_micro_sign() {
        let bytes = b"SYMATTR Value 10\xb5\n";
        let (text, enc) = decode(bytes);
        assert_eq!(enc, Encoding::Latin1);
        assert!(text.contains('\u{b5}'));
        assert_eq!(encode(&text, enc), bytes);
    }
}
