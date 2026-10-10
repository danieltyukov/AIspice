#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Latin1,
}

/// Detect the encoding of a byte slice and decode it to a String.
///
/// Detection order:
/// 1. UTF-16LE BOM (0xFF 0xFE) — decode as UTF-16LE, stripping the BOM.
/// 2. Valid UTF-8 — return as-is.
/// 3. Fallback — treat every byte as a Latin-1 code point.
pub fn detect_and_decode(bytes: &[u8]) -> (String, Encoding) {
    // Check UTF-16LE BOM (0xFF 0xFE)
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16_iter = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
        let text: String = char::decode_utf16(u16_iter)
            .map(|r| r.unwrap_or('\u{FFFD}'))
            .collect();
        return (text, Encoding::Utf16Le);
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), Encoding::Utf8),
        Err(_) => {
            let text: String = bytes.iter().map(|&b| b as char).collect();
            (text, Encoding::Latin1)
        }
    }
}

/// Encode a string back to bytes in the given encoding.
pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => {
            let mut bytes = vec![0xFF, 0xFE]; // BOM
            for c in text.encode_utf16() {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            bytes
        }
        Encoding::Latin1 => text.chars().map(|c| c as u8).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_roundtrip() {
        let original = "Hello, world!";
        let bytes = encode(original, Encoding::Utf8);
        let (decoded, enc) = detect_and_decode(&bytes);
        assert_eq!(enc, Encoding::Utf8);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_utf16le_roundtrip() {
        let original = "Hello, world!";
        let bytes = encode(original, Encoding::Utf16Le);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0xFE);
        let (decoded, enc) = detect_and_decode(&bytes);
        assert_eq!(enc, Encoding::Utf16Le);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_latin1_decode() {
        // Byte 0xE9 is 'e with acute' in Latin-1, but invalid as standalone UTF-8
        let bytes: Vec<u8> = vec![0x48, 0x65, 0x6C, 0x6C, 0xE9]; // "Hell\xe9"
        let (decoded, enc) = detect_and_decode(&bytes);
        assert_eq!(enc, Encoding::Latin1);
        assert!(decoded.ends_with('\u{00E9}'));
    }
}
