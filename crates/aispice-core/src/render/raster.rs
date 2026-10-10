//! SVG to PNG through resvg, so a rendered schematic can be handed to a
//! multimodal model or shown where SVG is not supported.

use super::RenderError;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};
use std::sync::{Arc, OnceLock};

/// Largest width or height of the PNG, in pixels. A runaway scale or a sheet
/// with stray far-away items would otherwise allocate gigabytes.
const MAX_SIDE: u32 = 16384;

/// Families tried, in order, for text that names a generic family or a font
/// that is not installed. They cover Windows, macOS and common Linux setups.
const PREFERRED: &[&str] = &[
    "Arial",
    "Helvetica",
    "Liberation Sans",
    "Arimo",
    "DejaVu Sans",
    "Noto Sans",
    "Inter",
    "Roboto",
    "Segoe UI",
];

/// System fonts, loaded once per process: scanning them takes a noticeable
/// fraction of a second on a desktop with many fonts installed.
fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let has = |name: &str| {
                db.faces()
                    .any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
            };
            let family = PREFERRED
                .iter()
                .find(|n| has(n))
                .map(|n| n.to_string())
                .or_else(|| {
                    db.faces()
                        .next()
                        .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
                });
            if let Some(family) = family {
                // usvg falls back to the serif family when nothing in a
                // font-family list matches, so point both generics at a face
                // that exists.
                db.set_sans_serif_family(family.clone());
                db.set_serif_family(family);
            }
            Arc::new(db)
        })
        .clone()
}

/// Rasterise an SVG at `scale` pixels per SVG unit. The image has an opaque
/// white background, because many viewers and model inputs show transparency
/// as black; a standalone SVG's own background paints over it. Without any
/// system fonts text is skipped and the drawing still renders.
pub fn render_png(svg: &str, scale: f32) -> Result<Vec<u8>, RenderError> {
    rasterise(svg, scale)?
        .encode_png()
        .map_err(|e| RenderError::Encode(e.to_string()))
}

fn rasterise(svg: &str, scale: f32) -> Result<tiny_skia::Pixmap, RenderError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(RenderError::Scale(scale));
    }
    let svg = resolve_css_vars(svg);
    let opt = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| RenderError::Svg(e.to_string()))?;
    let size = tree.size();
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let too_large = RenderError::TooLarge {
        width,
        height,
        max: MAX_SIDE,
    };
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(too_large);
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(too_large)?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// Replace every `var(--name, fallback)` with its fallback. resvg does not
/// implement CSS custom properties, so a themed SVG would otherwise lose its
/// colours. Nested parentheses in the fallback, as in `rgba(...)`, are kept.
pub(crate) fn resolve_css_vars(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(i) = rest.find("var(") {
        out.push_str(&rest[..i]);
        let body = &rest[i + 4..];
        // Find the matching close parenthesis and the first top-level comma.
        let mut depth = 0usize;
        let mut comma = None;
        let mut close = None;
        for (j, c) in body.char_indices() {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => {
                    close = Some(j);
                    break;
                }
                ')' => depth -= 1,
                ',' if depth == 0 && comma.is_none() => comma = Some(j),
                _ => {}
            }
        }
        let Some(close) = close else {
            // Unbalanced: leave the remainder untouched.
            out.push_str(&rest[i..]);
            return out;
        };
        match comma {
            Some(c) => out.push_str(resolve_css_vars(body[c + 1..close].trim()).as_str()),
            // No fallback: nothing sensible to substitute.
            None => out.push_str("currentColor"),
        }
        rest = &body[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_vars_resolve_to_fallbacks() {
        assert_eq!(
            resolve_css_vars("a{stroke:var(--sch-wire, #2b6cb0);fill:var(--x, rgba(1,2,3,0.5))}"),
            "a{stroke:#2b6cb0;fill:rgba(1,2,3,0.5)}"
        );
        assert_eq!(
            resolve_css_vars("font-family:var(--f, Arial, sans-serif)"),
            "font-family:Arial, sans-serif"
        );
        assert_eq!(resolve_css_vars("x:var(--a, var(--b, red))"), "x:red");
        assert_eq!(resolve_css_vars("no vars"), "no vars");
    }

    #[test]
    fn bad_scale_is_rejected() {
        assert_eq!(render_png("<svg/>", 0.0), Err(RenderError::Scale(0.0)));
        assert!(matches!(
            render_png("<svg/>", f32::NAN),
            Err(RenderError::Scale(_))
        ));
    }

    #[test]
    fn invalid_svg_is_an_error() {
        assert!(matches!(
            render_png("not svg", 1.0),
            Err(RenderError::Svg(_))
        ));
    }

    #[test]
    fn oversized_images_are_refused() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"/>"#;
        assert!(matches!(
            render_png(svg, 1000.0),
            Err(RenderError::TooLarge { .. })
        ));
    }

    #[test]
    fn text_is_drawn_when_fonts_exist() {
        if fonts().is_empty() {
            // Nothing to draw text with; render_png still succeeds.
            return;
        }
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><style>.t{font-family:var(--f, sans-serif)}</style><text class="t" x="4" y="22" font-size="20">R1</text></svg>"#;
        let pixmap = rasterise(svg, 1.0).unwrap();
        let ink = pixmap
            .pixels()
            .iter()
            .filter(|p| p.red() < 128 && p.green() < 128 && p.blue() < 128)
            .count();
        assert!(ink > 20, "text left only {ink} dark pixels");
    }

    #[test]
    fn renders_a_png() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="var(--c, #ff0000)"/></svg>"#;
        let png = render_png(svg, 2.0).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        // Width and height from the IHDR chunk.
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 40);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 20);
    }
}
