//! SPICE numbers: `4.7k`, `100n`, `1Meg`, `10uF`, `2.2µ`, `1e-3`.
//!
//! Suffixes are case-insensitive, as in every SPICE: `m` and `M` are both
//! milli, mega is `Meg`. Letters after a recognised scale are units and are
//! ignored (`10uF`, `5V`, `1kOhm`). The RKM form `4k7` is accepted as 4.7k
//! because people type it, but it is never produced.

/// Parse a SPICE number. Returns `None` for anything that is not a plain
/// number with an optional scale and unit, such as `{R}`, `SINE(0 1 1k)` or a
/// model name.
pub fn parse(text: &str) -> Option<f64> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end = 1;
    }
    let digits_start = end;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    if end == digits_start {
        return None;
    }
    // Exponent, but only when digits follow: "1e3" yes, "1meg" no.
    if end < bytes.len() && matches!(bytes[end], b'e' | b'E') {
        let mut k = end + 1;
        if k < bytes.len() && matches!(bytes[k], b'+' | b'-') {
            k += 1;
        }
        let exp_digits = k;
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        if k > exp_digits {
            end = k;
        }
    }
    let mantissa: f64 = s[..end].parse().ok()?;
    let rest = &s[end..];
    if rest.is_empty() {
        return Some(mantissa);
    }
    let (scale, consumed) = scale_prefix(rest)?;
    let tail = &rest[consumed..];
    // RKM: 4k7 means 4.7k. Only for an integer mantissa and one or two
    // trailing digits, so a part number like 1N4148 is not read as 1.4148n.
    if (1..=2).contains(&tail.len())
        && tail.bytes().all(|b| b.is_ascii_digit())
        && !s[..end].contains('.')
    {
        let frac: f64 = format!("0.{tail}").parse().ok()?;
        return Some((mantissa + frac.copysign(mantissa)) * scale);
    }
    if tail.chars().all(|c| c.is_alphabetic() || c == '_') {
        Some(mantissa * scale)
    } else {
        None
    }
}

fn scale_prefix(rest: &str) -> Option<(f64, usize)> {
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("meg") {
        return Some((1e6, 3));
    }
    if lower.starts_with("mil") {
        return Some((25.4e-6, 3));
    }
    let mut chars = rest.chars();
    let first = chars.next()?;
    let scale = match first.to_ascii_lowercase() {
        't' => 1e12,
        'g' => 1e9,
        'k' => 1e3,
        'm' => 1e-3,
        'u' | 'µ' | 'μ' => 1e-6,
        'n' => 1e-9,
        'p' => 1e-12,
        'f' => 1e-15,
        'a' => 1e-18,
        // A bare unit with no scale: "5V", "10Ohm", "3A" handled by 'a' above
        // as atto in SPICE too, which is what ngspice does.
        c if c.is_alphabetic() => return Some((1.0, 0)),
        _ => return None,
    };
    Some((scale, first.len_utf8()))
}

/// Format a value the way an engineer writes it in a schematic: at most four
/// significant digits and an LTspice scale suffix (`4.7k`, `100n`, `1Meg`).
pub fn format(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if !value.is_finite() {
        return value.to_string();
    }
    const SCALES: [(f64, &str); 10] = [
        (1e12, "T"),
        (1e9, "G"),
        (1e6, "Meg"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "u"),
        (1e-9, "n"),
        (1e-12, "p"),
        (1e-15, "f"),
    ];
    let mag = value.abs();
    let (scale, suffix) = SCALES
        .iter()
        .copied()
        .find(|(s, _)| mag >= *s * 0.99995)
        .unwrap_or((1e-15, "f"));
    let scaled = value / scale;
    // Four significant digits, then strip trailing zeros.
    let decimals = 3usize.saturating_sub(scaled.abs().log10().floor().max(0.0) as usize);
    let mut text = format!("{scaled:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    format!("{text}{suffix}")
}

/// Format with a unit, for human-facing reports: `1.592kHz`, `3.3V`.
pub fn format_with_unit(value: f64, unit: &str) -> String {
    let base = format(value);
    let base = base.replace("Meg", "M");
    format!("{base}{unit}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs().max(1e-30)
    }

    #[test]
    fn parses_common_values() {
        let cases = [
            ("4.7k", 4.7e3),
            ("100n", 100e-9),
            ("1Meg", 1e6),
            ("1MEG", 1e6),
            ("1m", 1e-3),
            ("1M", 1e-3),
            ("10uF", 10e-6),
            ("2.2µ", 2.2e-6),
            ("1e-3", 1e-3),
            ("1E3", 1e3),
            ("-5", -5.0),
            ("5V", 5.0),
            ("1kOhm", 1e3),
            ("4k7", 4.7e3),
            ("2mil", 50.8e-6),
            ("3.3", 3.3),
            (".5", 0.5),
        ];
        for (text, want) in cases {
            let got = parse(text).unwrap_or_else(|| panic!("{text} did not parse"));
            assert!(close(got, want), "{text}: got {got}, want {want}");
        }
    }

    #[test]
    fn rejects_non_numbers() {
        for text in ["", "{R}", "SINE(0 1 1k)", "1N4148", "abc", "PULSE(0 5 0)", "1k+2"] {
            assert_eq!(parse(text), None, "{text} should not parse");
        }
    }

    #[test]
    fn formats_like_an_engineer() {
        let cases = [
            (4700.0, "4.7k"),
            (100e-9, "100n"),
            (1e6, "1Meg"),
            (1.5916e3, "1.592k"),
            (0.001, "1m"),
            (3.3, "3.3"),
            (0.0, "0"),
            (-2.2e-6, "-2.2u"),
            (999.99, "1k"),
            (47e-12, "47p"),
        ];
        for (value, want) in cases {
            assert_eq!(format(value), want, "{value}");
        }
    }

    #[test]
    fn format_parse_round_trip() {
        for v in [1.0, 4.7e3, 2.2e-6, 15e-12, 33e6, 0.0125, 820.0] {
            assert!(close(parse(&format(v)).unwrap(), v));
        }
    }
}
