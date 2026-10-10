//! Small helpers for writing SVG text by hand.

/// Format a coordinate with at most two decimals and no trailing zeros, so
/// grid positions stay integers in the output and snapshots stay readable.
pub(crate) fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

/// Escape text for use in element content and double-quoted attributes.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_drop_needless_decimals() {
        assert_eq!(num(16.0), "16");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(2.0 / 3.0), "0.67");
        assert_eq!(num(-12.004), "-12");
    }

    #[test]
    fn escaping_covers_markup_characters() {
        assert_eq!(esc("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
        assert_eq!(esc("x\u{1}y"), "xy");
    }
}
