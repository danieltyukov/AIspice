//! The style sheet embedded in every SVG.
//!
//! Colours and fonts come from CSS custom properties with the light theme as
//! fallback, so a page restyles the drawing by setting the properties on any
//! ancestor. Standalone SVGs get the fallbacks written in directly: resvg and
//! most image viewers do not implement custom properties.

/// One themeable CSS custom property with its light and dark defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeVar {
    pub name: &'static str,
    pub light: &'static str,
    pub dark: &'static str,
    pub role: &'static str,
}

/// Every custom property the style sheet reads. The desktop app and the site
/// set these from their own token files; `dark` is a ready palette for dark
/// backgrounds.
pub const THEME_VARIABLES: &[ThemeVar] = &[
    ThemeVar {
        name: "--sch-bg",
        light: "#ffffff",
        dark: "#16191f",
        role: "sheet background; themed SVGs are transparent unless it is set",
    },
    ThemeVar {
        name: "--sch-wire",
        light: "#2b6cb0",
        dark: "#63b3ed",
        role: "wires, bus taps and junction dots",
    },
    ThemeVar {
        name: "--sch-symbol",
        light: "#1a202c",
        dark: "#e2e8f0",
        role: "symbol drawings",
    },
    ThemeVar {
        name: "--sch-attr",
        light: "#2d3748",
        dark: "#cbd5e0",
        role: "instance names, values and pin names",
    },
    ThemeVar {
        name: "--sch-flag",
        light: "#1a202c",
        dark: "#e2e8f0",
        role: "ground symbols, label anchors and port outlines",
    },
    ThemeVar {
        name: "--sch-label",
        light: "#1a202c",
        dark: "#f7fafc",
        role: "net label text",
    },
    ThemeVar {
        name: "--sch-directive",
        light: "#1a202c",
        dark: "#f7fafc",
        role: "SPICE directives",
    },
    ThemeVar {
        name: "--sch-comment",
        light: "#2c7a7b",
        dark: "#81e6d9",
        role: "comments",
    },
    ThemeVar {
        name: "--sch-shape",
        light: "#718096",
        dark: "#a0aec0",
        role: "lines, boxes and arcs drawn on the sheet",
    },
    ThemeVar {
        name: "--sch-pin",
        light: "#c53030",
        dark: "#fc8181",
        role: "markers on unconnected pins",
    },
    ThemeVar {
        name: "--sch-missing",
        light: "#c53030",
        dark: "#fc8181",
        role: "placeholder for a symbol that could not be found",
    },
    ThemeVar {
        name: "--sch-added",
        light: "#2f855a",
        dark: "#68d391",
        role: "highlight: added part",
    },
    ThemeVar {
        name: "--sch-removed",
        light: "#c53030",
        dark: "#fc8181",
        role: "highlight: removed part",
    },
    ThemeVar {
        name: "--sch-changed",
        light: "#b7791f",
        dark: "#f6e05e",
        role: "highlight: changed part",
    },
    ThemeVar {
        name: "--sch-added-bg",
        light: "rgba(47,133,90,0.14)",
        dark: "rgba(104,211,145,0.18)",
        role: "highlight halo: added part",
    },
    ThemeVar {
        name: "--sch-removed-bg",
        light: "rgba(197,48,48,0.12)",
        dark: "rgba(252,129,129,0.18)",
        role: "highlight halo: removed part",
    },
    ThemeVar {
        name: "--sch-changed-bg",
        light: "rgba(183,121,31,0.14)",
        dark: "rgba(246,224,94,0.18)",
        role: "highlight halo: changed part",
    },
    ThemeVar {
        name: "--sch-font",
        light: "Arial, Helvetica, 'Liberation Sans', 'DejaVu Sans', sans-serif",
        dark: "Arial, Helvetica, 'Liberation Sans', 'DejaVu Sans', sans-serif",
        role: "text font",
    },
    ThemeVar {
        name: "--sch-font-directive",
        light: "Arial, Helvetica, 'Liberation Sans', 'DejaVu Sans', sans-serif",
        dark: "Arial, Helvetica, 'Liberation Sans', 'DejaVu Sans', sans-serif",
        role: "directive font; proportional by default because LTspice lays \
               text out in one, and a wider monospace face makes neighbouring \
               text collide",
    },
];

fn value(name: &str, standalone: bool) -> String {
    let var = THEME_VARIABLES
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("theme variable {name} is not declared"));
    if standalone {
        var.light.to_string()
    } else {
        format!("var({}, {})", var.name, var.light)
    }
}

/// The `<style>` body. Every rule is scoped to the `aispice-sch` root class so
/// the generic class names cannot leak into a page that inlines the SVG.
pub(crate) fn style_sheet(standalone: bool) -> String {
    let v = |name: &str| value(name, standalone);
    let bg = if standalone {
        value("--sch-bg", true)
    } else {
        "var(--sch-bg, none)".to_string()
    };
    let stroke = "fill:none;stroke-width:2;stroke-linecap:round;stroke-linejoin:round";
    let mut rules = vec![
        format!(".aispice-sch .bg{{fill:{bg}}}"),
        format!(
            ".aispice-sch text{{font-family:{};white-space:pre}}",
            v("--sch-font")
        ),
        format!(
            ".aispice-sch .wire,.aispice-sch .bustap{{{stroke};stroke:{}}}",
            v("--sch-wire")
        ),
        format!(".aispice-sch .junction{{fill:{}}}", v("--sch-wire")),
        format!(
            ".aispice-sch .body{{{stroke};stroke:{}}}",
            v("--sch-symbol")
        ),
        format!(
            ".aispice-sch .fill,.aispice-sch .sym-text{{fill:{}}}",
            v("--sch-symbol")
        ),
        format!(
            ".aispice-sch .attr,.aispice-sch .pin-label{{fill:{}}}",
            v("--sch-attr")
        ),
        format!(
            ".aispice-sch .missing .body{{stroke:{};stroke-dasharray:6 4}}",
            v("--sch-missing")
        ),
        format!(
            ".aispice-sch .missing .sym-text,.aispice-sch .missing .fill{{fill:{}}}",
            v("--sch-missing")
        ),
        format!(".aispice-sch .mark{{{stroke};stroke:{}}}", v("--sch-flag")),
        format!(".aispice-sch .label{{fill:{}}}", v("--sch-label")),
        format!(
            ".aispice-sch .directive{{fill:{};font-family:{}}}",
            v("--sch-directive"),
            v("--sch-font-directive")
        ),
        format!(".aispice-sch .comment{{fill:{}}}", v("--sch-comment")),
        format!(
            ".aispice-sch .shape{{{stroke};stroke:{}}}",
            v("--sch-shape")
        ),
        format!(
            ".aispice-sch .pin{{fill:none;stroke-width:1.5;stroke:{}}}",
            v("--sch-pin")
        ),
        format!(".aispice-sch .pin-dot{{fill:{}}}", v("--sch-pin")),
        ".aispice-sch .hl-box{fill:none;stroke:none}".to_string(),
    ];
    for kind in ["added", "removed", "changed"] {
        let colour = v(&format!("--sch-{kind}"));
        let halo = v(&format!("--sch-{kind}-bg"));
        let dash = if kind == "removed" {
            ";stroke-dasharray:6 4"
        } else {
            ""
        };
        rules.push(format!(
            ".aispice-sch .hl-{kind} .body{{stroke:{colour}{dash}}}"
        ));
        rules.push(format!(
            ".aispice-sch .hl-{kind} .attr,.aispice-sch .hl-{kind} .sym-text,.aispice-sch .hl-{kind} .fill{{fill:{colour}}}"
        ));
        rules.push(format!(".aispice-sch .hl-{kind} .hl-box{{fill:{halo}}}"));
    }
    rules.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themed_sheet_uses_custom_properties_with_fallbacks() {
        let css = style_sheet(false);
        assert!(css.contains("stroke:var(--sch-wire, #2b6cb0)"), "{css}");
        assert!(css.contains("fill:var(--sch-bg, none)"));
    }

    #[test]
    fn standalone_sheet_has_no_custom_properties() {
        let css = style_sheet(true);
        assert!(!css.contains("var("), "{css}");
        assert!(css.contains("stroke:#2b6cb0"));
        assert!(css.contains(".aispice-sch .bg{fill:#ffffff}"));
    }

    #[test]
    fn variable_names_are_unique() {
        let mut names: Vec<_> = THEME_VARIABLES.iter().map(|v| v.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), THEME_VARIABLES.len());
    }
}
