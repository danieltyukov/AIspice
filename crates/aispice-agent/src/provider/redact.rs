//! Keeping credentials out of error messages.
//!
//! Upstream error bodies sometimes quote the key that was sent ("Incorrect
//! API key provided: sk-..."), and reqwest errors carry the request URL,
//! which for a custom endpoint can hold a key in the query or user info.
//! Every error leaving a provider passes through [`scrub`], which removes
//! the configured secrets, anything shaped like a common API key, and caps
//! the length.

const REDACTED: &str = "[redacted]";
const MAX_MESSAGE_CHARS: usize = 1000;
/// Configured secrets shorter than this are not real keys, and replacing
/// them would mangle ordinary words.
const MIN_SECRET_LEN: usize = 6;

/// Remove `secrets` and key-shaped tokens from `text`, then truncate it.
pub(crate) fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= MIN_SECRET_LEN {
            out = out.replace(secret, REDACTED);
        }
    }
    truncate(scrub_shapes(&out))
}

/// Replace tokens that look like API keys: `sk-...` (OpenAI, Anthropic,
/// OpenRouter), `AIza...` (Google), `Bearer ...`, and `key=...` in URLs.
fn scrub_shapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if (i == 0 || !is_token_byte(bytes[i - 1]))
            && let Some((label, len)) = key_at_start(rest)
        {
            // The label is ASCII, so slicing after it is safe.
            out.push_str(&rest[..label]);
            out.push_str(REDACTED);
            i += len;
            continue;
        }
        let ch = rest.chars().next().expect("i is below the length");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// A key-shaped token at the start of `rest`: the length of a label to keep
/// (such as `Bearer `) and the total length matched.
fn key_at_start(rest: &str) -> Option<(usize, usize)> {
    if let Some(len) = prefixed_token(rest, b"sk-", 8, is_token_byte) {
        return Some((0, len));
    }
    if let Some(len) = prefixed_token(rest, b"AIza", 20, is_token_byte) {
        return Some((0, len));
    }
    if let Some(len) = prefixed_token(rest, b"bearer ", 8, is_bearer_byte) {
        return Some((7, len));
    }
    prefixed_token(rest, b"key=", 16, is_bearer_byte).map(|len| (4, len))
}

/// Length of `prefix` plus the token after it, if the prefix matches
/// (ASCII case-insensitively) and at least `min_tail` token bytes follow.
fn prefixed_token(
    rest: &str,
    prefix: &[u8],
    min_tail: usize,
    token_byte: fn(u8) -> bool,
) -> Option<usize> {
    let bytes = rest.as_bytes();
    if bytes.len() < prefix.len() || !bytes[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    let tail = bytes[prefix.len()..]
        .iter()
        .take_while(|b| token_byte(**b))
        .count();
    (tail >= min_tail).then_some(prefix.len() + tail)
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

fn is_bearer_byte(b: u8) -> bool {
    is_token_byte(b) || b"._~+/=".contains(&b)
}

fn truncate(text: String) -> String {
    match text.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((cut, _)) => format!("{}... (truncated)", &text[..cut]),
        None => text,
    }
}

/// A URL for display: user info and query string removed, since either can
/// carry a credential.
pub(crate) fn display_url(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    match without_query.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            let host = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host}{path}")
        }
        None => without_query.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_configured_secrets() {
        let out = scrub(
            "bad key my-custom-token-123 rejected",
            &["my-custom-token-123"],
        );
        assert_eq!(out, "bad key [redacted] rejected");
        // Too short to be a key: left alone.
        assert_eq!(scrub("the key k failed", &["k"]), "the key k failed");
    }

    #[test]
    fn removes_key_shapes() {
        let cases = [
            (
                "Incorrect API key provided: sk-proj-abcDEF123456789. Find yours at",
                "Incorrect API key provided: [redacted]. Find yours at",
            ),
            (
                "x-api-key sk-ant-api03-AAAA_BBBB-CCCC",
                "x-api-key [redacted]",
            ),
            (
                "key AIzaSyA1234567890abcdefghijklmnop ok",
                "key [redacted] ok",
            ),
            (
                "Authorization: Bearer eyJhbGciOi.J9.abc_def",
                "Authorization: Bearer [redacted]",
            ),
            (
                "GET /v1beta/models?key=abcdefghijklmnop1234 failed",
                "GET /v1beta/models?key=[redacted] failed",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(scrub(input, &[]), expected, "{input}");
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "risk-free task-runner sk-short",
            "Bearer token missing",
            "the monkey=1 setting",
            "\u{b5}F capacitor, 10\u{2126} resistor",
        ] {
            assert_eq!(scrub(text, &[]), text);
        }
    }

    #[test]
    fn truncates_long_messages() {
        let long = "x".repeat(5000);
        let out = scrub(&long, &[]);
        assert!(out.len() < 1100);
        assert!(out.ends_with("(truncated)"));
    }

    #[test]
    fn display_url_drops_credentials() {
        assert_eq!(
            display_url("https://user:sk-secret@proxy.example:8443/v1?key=abc#x"),
            "https://proxy.example:8443/v1"
        );
        assert_eq!(
            display_url("http://localhost:11434/v1"),
            "http://localhost:11434/v1"
        );
    }
}
