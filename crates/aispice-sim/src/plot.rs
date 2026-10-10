//! Waveforms to standalone SVG.
//!
//! Plots are for the agent's answers, reports and the CLI, so they have to
//! read well as a static picture: one SVG file with no scripts or external
//! references, which a browser shows as is and resvg turns into a PNG for a
//! multimodal model. Transients are drawn against linear time, AC results as
//! a Bode plot (magnitude in dB above phase in degrees, two panes on one log
//! frequency axis, never two scales on one pane), stepped runs as one trace
//! per step.
//!
//! Colours follow a fixed categorical order validated for colour-vision
//! deficiency in light and dark, defined as CSS variables so a page can
//! restyle or theme the plot, with plain attribute fallbacks for renderers
//! that ignore CSS. Long transients are thinned to about `max_points` per
//! trace by keeping the minimum and maximum of each pixel column, so a
//! million-point switching waveform stays a small file and its spikes
//! survive.

use crate::dataset::Dataset;
use crate::expr::{self, Expr, Series};
use crate::measure::{Measure, MeasureResult, human};
use aispice_core::units;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlotKind {
    /// Bode for complex (AC) traces, values otherwise.
    #[default]
    Auto,
    /// Magnitude in dB above phase in degrees.
    Bode,
    /// Magnitude in dB only.
    Magnitude,
    /// Phase in degrees only.
    Phase,
    /// The values themselves (the magnitude of complex ones).
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Light, switching to dark with the viewer's system setting.
    #[default]
    Auto,
    Light,
    Dark,
}

/// Which pane of a Bode plot a marker belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    /// The magnitude pane, or the only pane.
    #[default]
    Main,
    Phase,
}

/// An annotation: a measured crossing, a spec limit, a point of interest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Marker {
    /// A vertical line at `x` across every pane.
    X {
        x: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A horizontal line at level `y`.
    Y {
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default)]
        pane: Pane,
    },
    /// A dot at (`x`, `y`).
    Point {
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default)]
        pane: Pane,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PlotRequest {
    /// Expressions to draw, one trace each (per step): `V(out)`,
    /// `V(out)/V(in)`, `I(R1)`.
    pub traces: Vec<String>,
    pub title: Option<String>,
    /// Index of the dataset to plot; by default the first one that has
    /// every vector the traces name.
    pub dataset: Option<usize>,
    pub kind: PlotKind,
    /// Logarithmic x axis; by default for frequency axes only.
    pub x_log: Option<bool>,
    /// Limit the x axis to `[from, to]`.
    pub x_range: Option<[f64; 2]>,
    /// Override the y axis title.
    pub y_label: Option<String>,
    pub markers: Vec<Marker>,
    /// Size in pixels; 0 picks a default for the plot kind.
    pub width: u32,
    pub height: u32,
    pub theme: Theme,
    /// Thin each trace to about this many points.
    pub max_points: usize,
}

impl Default for PlotRequest {
    fn default() -> Self {
        PlotRequest {
            traces: Vec::new(),
            title: None,
            dataset: None,
            kind: PlotKind::Auto,
            x_log: None,
            x_range: None,
            y_label: None,
            markers: Vec::new(),
            width: 0,
            height: 0,
            theme: Theme::Auto,
            max_points: 2000,
        }
    }
}

impl PlotRequest {
    pub fn new<S: Into<String>>(traces: impl IntoIterator<Item = S>) -> Self {
        PlotRequest {
            traces: traces.into_iter().map(Into::into).collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlotError {
    #[error("nothing to plot: give at least one trace expression")]
    NoTraces,
    #[error("at most {MAX_EXPRS} traces fit on one plot, got {0}; split them over several plots")]
    TooManyTraces(usize),
    #[error("{0}")]
    Expr(String),
    #[error("dataset {index} does not exist; there are {count}")]
    NoDataset { index: usize, count: usize },
    #[error("{0}")]
    NoData(String),
}

/// One hue per trace in a fixed order: past eight, a plot stops being
/// readable and should be split instead.
const MAX_EXPRS: usize = 8;

struct Palette {
    surface: &'static str,
    ink: &'static str,
    ink2: &'static str,
    muted: &'static str,
    grid: &'static str,
    axis: &'static str,
    series: [&'static str; 8],
}

/// The dataviz reference palette: categorical order validated for adjacent
/// pairs under colour-vision deficiency, light and dark steps of the same
/// hues, and recessive chrome.
const LIGHT: Palette = Palette {
    surface: "#fcfcfb",
    ink: "#0b0b0b",
    ink2: "#52514e",
    muted: "#898781",
    grid: "#e1e0d9",
    axis: "#c3c2b7",
    series: [
        "#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948",
    ],
};

const DARK: Palette = Palette {
    surface: "#1a1a19",
    ink: "#f0efec",
    ink2: "#c3c2b7",
    muted: "#898781",
    grid: "#2c2c2a",
    axis: "#383835",
    series: [
        "#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300", "#9085e9", "#e66767",
    ],
};

fn css_vars(p: &Palette) -> String {
    let mut s = format!(
        "color-scheme:{};--surface:{};--ink:{};--ink-2:{};--muted:{};--grid:{};--axis:{};",
        if p.surface == LIGHT.surface {
            "light"
        } else {
            "dark"
        },
        p.surface,
        p.ink,
        p.ink2,
        p.muted,
        p.grid,
        p.axis
    );
    for (i, c) in p.series.iter().enumerate() {
        let _ = write!(s, "--s{}:{c};", i + 1);
    }
    s
}

fn stylesheet() -> String {
    let light = css_vars(&LIGHT);
    let dark = css_vars(&DARK);
    let mut s = String::new();
    let _ = write!(
        s,
        ".aispice-plot{{{light}}}\
@media (prefers-color-scheme:dark){{.aispice-plot:not([data-theme=light]){{{dark}}}\
:root[data-theme=light] .aispice-plot:not([data-theme=dark]){{{light}}}}}\
.aispice-plot[data-theme=dark],:root[data-theme=dark] .aispice-plot:not([data-theme=light]){{{dark}}}\
.aispice-plot .bg{{fill:var(--surface)}}\
.aispice-plot .grid{{stroke:var(--grid)}}\
.aispice-plot .grid-minor{{stroke:var(--grid);stroke-opacity:.5}}\
.aispice-plot .axis{{stroke:var(--axis)}}\
.aispice-plot .title{{fill:var(--ink)}}\
.aispice-plot .subtitle,.aispice-plot .axis-title,.aispice-plot .legend text,.aispice-plot .marker-label{{fill:var(--ink-2)}}\
.aispice-plot .tick{{fill:var(--muted)}}\
.aispice-plot .marker{{stroke:var(--ink-2)}}\
.aispice-plot .marker-label{{stroke:var(--surface)}}\
.aispice-plot .marker-dot{{stroke:var(--surface);fill:var(--ink-2)}}\
.aispice-plot .trace{{fill:none}}"
    );
    for i in 1..=8 {
        let _ = write!(s, ".aispice-plot .s{i}{{stroke:var(--s{i})}}");
    }
    s
}

/// Everything about one drawn line.
struct Trace {
    label: String,
    slot: usize,
    /// Many steps of one expression: thin and translucent, so the bundle
    /// shows the spread rather than a wall of colour.
    faint: bool,
    x: Vec<f64>,
    /// Values per pane.
    y: Vec<Vec<f64>>,
}

#[derive(Clone, Copy, PartialEq)]
enum PaneKind {
    Linear,
    Phase,
}

struct PaneSpec {
    name: &'static str,
    title: String,
    kind: PaneKind,
}

/// A linear or logarithmic map from data to pixels.
#[derive(Clone, Copy)]
struct Scale {
    lo: f64,
    hi: f64,
    log: bool,
    p0: f64,
    p1: f64,
}

impl Scale {
    fn map(&self, v: f64) -> f64 {
        let t = if self.log {
            if v <= 0.0 {
                return f64::NAN;
            }
            (v.log10() - self.lo.log10()) / (self.hi.log10() - self.lo.log10())
        } else {
            (v - self.lo) / (self.hi - self.lo)
        };
        (self.p0 + t * (self.p1 - self.p0)).clamp(-1e5, 1e5)
    }
}

/// Render traces from the results of a run as a standalone SVG document.
pub fn plot_svg(datasets: &[Dataset], request: &PlotRequest) -> Result<String, PlotError> {
    if request.traces.is_empty() {
        return Err(PlotError::NoTraces);
    }
    if request.traces.len() > MAX_EXPRS {
        return Err(PlotError::TooManyTraces(request.traces.len()));
    }
    let exprs: Vec<Expr> = request
        .traces
        .iter()
        .map(|t| expr::parse(t).map_err(|e| PlotError::Expr(e.to_string())))
        .collect::<Result<_, _>>()?;
    let ds = pick_dataset(datasets, request, &exprs)?;
    let Some(axis_vec) = ds.axis_vector() else {
        return Err(PlotError::NoData(
            "an operating point is a single value and has nothing to plot; use a measurement"
                .into(),
        ));
    };
    let axis_all = axis_vec.data.real();
    let x_log = request.x_log.unwrap_or_else(|| expr::is_log_axis(ds));

    // Evaluate every trace on every step.
    let steps: Vec<(String, std::ops::Range<usize>)> = if ds.steps.is_empty() {
        vec![(String::new(), 0..ds.len())]
    } else {
        ds.steps
            .iter()
            .map(|s| (s.label.clone(), s.range.clone()))
            .collect()
    };
    let mut evaluated: Vec<Vec<Series>> = Vec::new();
    for e in &exprs {
        let mut per_step = Vec::new();
        for (_, r) in &steps {
            per_step.push(
                e.eval(ds, r.clone())
                    .map_err(|err| PlotError::Expr(err.to_string()))?,
            );
        }
        evaluated.push(per_step);
    }
    let any_complex = evaluated.iter().flatten().any(Series::is_complex);
    let kind = match request.kind {
        PlotKind::Auto if any_complex => PlotKind::Bode,
        PlotKind::Auto => PlotKind::Value,
        k => k,
    };
    let unit_of = |e: &Expr| e.unit(ds);
    let panes: Vec<PaneSpec> = match kind {
        PlotKind::Bode => vec![
            PaneSpec {
                name: "magnitude",
                title: request
                    .y_label
                    .clone()
                    .unwrap_or_else(|| "magnitude (dB)".into()),
                kind: PaneKind::Linear,
            },
            PaneSpec {
                name: "phase",
                title: "phase (°)".into(),
                kind: PaneKind::Phase,
            },
        ],
        PlotKind::Magnitude => vec![PaneSpec {
            name: "magnitude",
            title: request
                .y_label
                .clone()
                .unwrap_or_else(|| "magnitude (dB)".into()),
            kind: PaneKind::Linear,
        }],
        PlotKind::Phase => vec![PaneSpec {
            name: "phase",
            title: request
                .y_label
                .clone()
                .unwrap_or_else(|| "phase (°)".into()),
            kind: PaneKind::Phase,
        }],
        _ => {
            let units: Vec<String> = exprs.iter().map(unit_of).collect();
            let title = request.y_label.clone().unwrap_or_else(|| {
                let u = &units[0];
                let same = units.iter().all(|x| x == u);
                match (exprs.len(), same && !u.is_empty()) {
                    (1, true) => format!("{} ({u})", request.traces[0].trim()),
                    (1, false) => request.traces[0].trim().to_string(),
                    (_, true) => quantity_name(u)
                        .map(|q| format!("{q} ({u})"))
                        .unwrap_or_else(|| format!("({u})")),
                    _ => "value".into(),
                }
            });
            vec![PaneSpec {
                name: "main",
                title,
                kind: PaneKind::Linear,
            }]
        }
    };

    let n_steps = steps.len();
    let total = exprs.len() * n_steps;
    let mut traces = Vec::new();
    for (ei, per_step) in evaluated.iter().enumerate() {
        for (si, series) in per_step.iter().enumerate() {
            let (label, range) = &steps[si];
            let x: Vec<f64> = axis_all
                .get(range.clone())
                .map(<[f64]>::to_vec)
                .unwrap_or_default();
            let y: Vec<Vec<f64>> = match kind {
                PlotKind::Bode => vec![series.db(), series.phase_deg()],
                PlotKind::Magnitude => vec![series.db()],
                PlotKind::Phase => vec![series.phase_deg()],
                _ => vec![series.real_or_magnitude()],
            };
            let expr_text = request.traces[ei].trim();
            let (slot, faint, label) = if total <= MAX_EXPRS {
                let label = match (exprs.len(), label.is_empty()) {
                    (1, false) => label.clone(),
                    (_, true) => expr_text.to_string(),
                    (_, false) => format!("{expr_text} {label}"),
                };
                (ei * n_steps + si, false, label)
            } else {
                (ei, n_steps > 1, format!("{expr_text}, {n_steps} steps"))
            };
            traces.push(Trace {
                label,
                slot,
                faint,
                x,
                y,
            });
        }
    }

    // X range.
    let finite_x = |v: f64| v.is_finite() && (!x_log || v > 0.0);
    let (mut x_lo, mut x_hi) = traces
        .iter()
        .flat_map(|t| t.x.iter().copied())
        .filter(|v| finite_x(*v))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(v), b.max(v))
        });
    if let Some([a, b]) = request.x_range {
        if a.is_nan() || b.is_nan() || a >= b || (x_log && a <= 0.0) {
            return Err(PlotError::NoData(format!(
                "x_range {}..{} is empty{}",
                units::format(a),
                units::format(b),
                if x_log {
                    " or not positive on a log axis"
                } else {
                    ""
                }
            )));
        }
        x_lo = a;
        x_hi = b;
    }
    if !(x_lo.is_finite() && x_hi.is_finite()) {
        return Err(PlotError::NoData(
            "the traces have no plottable points".into(),
        ));
    }
    if x_lo == x_hi {
        let d = if x_lo == 0.0 { 1.0 } else { x_lo.abs() * 0.1 };
        if x_log {
            x_lo /= 2.0;
            x_hi *= 2.0;
        } else {
            x_lo -= d;
            x_hi += d;
        }
    }
    let in_window = |x: f64| x >= x_lo && x <= x_hi;

    // Y range per pane, from points inside the x window and the markers.
    let mut y_ranges: Vec<(f64, f64)> = vec![(f64::INFINITY, f64::NEG_INFINITY); panes.len()];
    for t in &traces {
        for (pi, ys) in t.y.iter().enumerate() {
            for (x, y) in t.x.iter().zip(ys) {
                if y.is_finite() && finite_x(*x) && in_window(*x) {
                    y_ranges[pi].0 = y_ranges[pi].0.min(*y);
                    y_ranges[pi].1 = y_ranges[pi].1.max(*y);
                }
            }
        }
    }
    for m in &request.markers {
        let (y, pane) = match m {
            Marker::Y { y, pane, .. } | Marker::Point { y, pane, .. } => (*y, *pane),
            Marker::X { .. } => continue,
        };
        let pi = pane_index(&panes, pane);
        if y.is_finite() {
            y_ranges[pi].0 = y_ranges[pi].0.min(y);
            y_ranges[pi].1 = y_ranges[pi].1.max(y);
        }
    }
    if y_ranges
        .iter()
        .any(|(a, b)| !(a.is_finite() && b.is_finite()))
    {
        return Err(PlotError::NoData(
            "the traces have no finite values in the plotted range".into(),
        ));
    }

    // Layout.
    let bode = panes.len() > 1;
    let width = if request.width > 0 {
        request.width
    } else {
        760
    } as f64;
    let height = if request.height > 0 {
        request.height
    } else if bode {
        540
    } else {
        420
    } as f64;
    let y_axes: Vec<(Scale, Vec<f64>, Vec<String>)> = panes
        .iter()
        .zip(&y_ranges)
        .map(|(p, &(lo, hi))| {
            let (lo, hi, ticks) = match p.kind {
                PaneKind::Phase => phase_ticks(lo, hi),
                PaneKind::Linear => linear_ticks(lo, hi, 6),
            };
            let step = if ticks.len() > 1 {
                ticks[1] - ticks[0]
            } else {
                hi - lo
            };
            let labels = if p.kind == PaneKind::Phase {
                ticks.iter().map(|t| format!("{}", t.round())).collect()
            } else {
                tick_labels(&ticks, step)
            };
            (
                Scale {
                    lo,
                    hi,
                    log: false,
                    p0: 0.0,
                    p1: 0.0,
                },
                ticks,
                labels,
            )
        })
        .collect();
    let widest = y_axes
        .iter()
        .flat_map(|(_, _, l)| l.iter())
        .map(|l| text_width(l, 11.0))
        .fold(0.0, f64::max);
    let left = (widest + 40.0).clamp(56.0, 120.0);
    let right = 24.0;

    let title = request.title.clone().unwrap_or_else(|| {
        let mut t = request
            .traces
            .iter()
            .map(|s| s.trim())
            .collect::<Vec<_>>()
            .join(", ");
        if t.chars().count() > 80 {
            t = t.chars().take(77).collect::<String>() + "...";
        }
        t
    });
    let subtitle = {
        let mut s = if ds.plotname.is_empty() {
            String::new()
        } else {
            ds.plotname.clone()
        };
        if n_steps > 1 {
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(&format!("{n_steps} steps"));
        }
        s
    };

    // Legend items, one per distinct label, laid out in rows.
    let mut legend: Vec<(String, usize, bool)> = Vec::new();
    for t in &traces {
        if !legend.iter().any(|(l, _, _)| *l == t.label) {
            legend.push((t.label.clone(), t.slot, t.faint));
        }
    }
    // One series (or one bundle of steps) is named by the title instead.
    let show_legend = legend.len() >= 2;
    let mut y_cursor = 26.0;
    let title_y = y_cursor;
    y_cursor += if subtitle.is_empty() { 8.0 } else { 18.0 };
    let subtitle_y = y_cursor - 2.0;
    let mut legend_rows: Vec<Vec<(f64, usize)>> = Vec::new();
    if show_legend {
        let mut row = Vec::new();
        let mut x = left;
        for (i, (label, _, _)) in legend.iter().enumerate() {
            let w = 22.0 + text_width(label, 12.0) + 18.0;
            if x + w > width - right && !row.is_empty() {
                legend_rows.push(std::mem::take(&mut row));
                x = left;
            }
            row.push((x, i));
            x += w;
        }
        legend_rows.push(row);
        y_cursor += 8.0 + 18.0 * legend_rows.len() as f64;
    }
    let legend_top = if subtitle.is_empty() {
        title_y + 18.0
    } else {
        subtitle_y + 20.0
    };
    // Labels of vertical markers sit in a strip above the plot, clear of
    // the traces.
    let has_x_labels = request
        .markers
        .iter()
        .any(|m| matches!(m, Marker::X { label: Some(_), .. }));
    let plot_top =
        y_cursor + if show_legend { 4.0 } else { 10.0 } + if has_x_labels { 16.0 } else { 0.0 };
    let plot_bottom = height - 48.0;
    if plot_bottom - plot_top < 60.0 {
        return Err(PlotError::NoData(format!(
            "{}x{} pixels leaves no room for the plot; make it larger",
            width, height
        )));
    }
    let gap = 18.0;
    let pane_rects: Vec<(f64, f64)> = if bode {
        let avail = plot_bottom - plot_top - gap;
        let h1 = (avail * 0.56).round();
        vec![
            (plot_top, plot_top + h1),
            (plot_top + h1 + gap, plot_bottom),
        ]
    } else {
        vec![(plot_top, plot_bottom)]
    };
    let x_scale = Scale {
        lo: x_lo,
        hi: x_hi,
        log: x_log,
        p0: left,
        p1: width - right,
    };
    let y_scales: Vec<Scale> = y_axes
        .iter()
        .zip(&pane_rects)
        .map(|((s, _, _), &(top, bottom))| Scale {
            p0: bottom,
            p1: top,
            ..*s
        })
        .collect();

    let palette = if request.theme == Theme::Dark {
        &DARK
    } else {
        &LIGHT
    };
    let uid = format!(
        "p{:08x}",
        fnv(&format!("{:?}{}{}", request.traces, width, height)) as u32
    );
    let mut svg = String::with_capacity(64 * 1024);
    let theme_attr = match request.theme {
        Theme::Auto => "",
        Theme::Light => " data-theme=\"light\"",
        Theme::Dark => " data-theme=\"dark\"",
    };
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"aispice-plot\"{theme_attr} width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" role=\"img\" font-family=\"system-ui, -apple-system, 'Segoe UI', Inter, sans-serif\" font-size=\"11\">",
        w = width,
        h = height
    );
    let _ = write!(svg, "<title>{}</title>", esc(&title));
    let _ = write!(svg, "<style>{}</style>", stylesheet());
    svg.push_str("<defs>");
    for (i, &(top, bottom)) in pane_rects.iter().enumerate() {
        let _ = write!(
            svg,
            "<clipPath id=\"{uid}-c{i}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\"/></clipPath>",
            left,
            top - 1.0,
            width - right - left,
            bottom - top + 2.0
        );
    }
    svg.push_str("</defs>");
    let _ = write!(
        svg,
        "<rect class=\"bg\" width=\"{width}\" height=\"{height}\" fill=\"{}\"/>",
        palette.surface
    );
    let _ = write!(
        svg,
        "<text class=\"title\" x=\"{left:.1}\" y=\"{title_y:.1}\" font-size=\"14\" font-weight=\"600\" fill=\"{}\">{}</text>",
        palette.ink,
        esc(&title)
    );
    if !subtitle.is_empty() {
        let _ = write!(
            svg,
            "<text class=\"subtitle\" x=\"{left:.1}\" y=\"{subtitle_y:.1}\" fill=\"{}\">{}</text>",
            palette.ink2,
            esc(&subtitle)
        );
    }
    if show_legend {
        svg.push_str("<g class=\"legend\" font-size=\"12\">");
        for (r, row) in legend_rows.iter().enumerate() {
            let y = legend_top + 18.0 * r as f64;
            for &(x, i) in row {
                let (label, slot, faint) = &legend[i];
                let _ = write!(
                    svg,
                    "<g class=\"legend-item\"><line class=\"s{s}\" x1=\"{x:.1}\" y1=\"{ly:.1}\" x2=\"{x2:.1}\" y2=\"{ly:.1}\" stroke=\"{c}\" stroke-width=\"{sw}\" stroke-linecap=\"round\"{op}/><text x=\"{tx:.1}\" y=\"{ty:.1}\" fill=\"{ink2}\">{label}</text></g>",
                    s = slot + 1,
                    ly = y - 4.0,
                    x2 = x + 16.0,
                    c = palette.series[slot % 8],
                    sw = if *faint { "2" } else { "2.5" },
                    op = if *faint {
                        " stroke-opacity=\"0.6\""
                    } else {
                        ""
                    },
                    tx = x + 22.0,
                    ty = y,
                    ink2 = palette.ink2,
                    label = esc(label)
                );
            }
        }
        svg.push_str("</g>");
    }

    // X ticks, shared by every pane.
    let (x_major, x_minor, x_labels) = if x_log {
        log_ticks(x_lo, x_hi)
    } else {
        // As many ticks as fit with their labels side by side.
        let plot_w = width - left - right;
        let mut target = (plot_w / 90.0).clamp(3.0, 10.0) as usize;
        loop {
            let ticks = linear_ticks_within(x_lo, x_hi, target);
            let step = if ticks.len() > 1 {
                ticks[1] - ticks[0]
            } else {
                x_hi - x_lo
            };
            let labels = tick_labels(&ticks, step);
            let widest = labels
                .iter()
                .map(|l| text_width(l, 11.0))
                .fold(0.0, f64::max);
            let spacing = step / (x_hi - x_lo) * plot_w;
            if spacing >= widest + 14.0 || target <= 2 {
                break (
                    ticks.clone(),
                    Vec::new(),
                    ticks.into_iter().zip(labels).collect(),
                );
            }
            target -= 1;
        }
    };

    let mut x_label_spans: Vec<(f64, f64)> = Vec::new();
    for (pi, pane) in panes.iter().enumerate() {
        let (top, bottom) = pane_rects[pi];
        let ys = &y_scales[pi];
        let (_, y_ticks, y_labels) = &y_axes[pi];
        let _ = write!(svg, "<g class=\"pane\" data-pane=\"{}\">", pane.name);
        // Grid: minor decades first, then major lines, all hairlines.
        for &x in &x_minor {
            let px = x_scale.map(x);
            let _ = write!(
                svg,
                "<line class=\"grid-minor\" x1=\"{px:.1}\" y1=\"{top:.1}\" x2=\"{px:.1}\" y2=\"{bottom:.1}\" stroke=\"{}\" stroke-opacity=\"0.5\"/>",
                palette.grid
            );
        }
        for &x in &x_major {
            let px = x_scale.map(x);
            let _ = write!(
                svg,
                "<line class=\"grid\" x1=\"{px:.1}\" y1=\"{top:.1}\" x2=\"{px:.1}\" y2=\"{bottom:.1}\" stroke=\"{}\"/>",
                palette.grid
            );
        }
        for &y in y_ticks {
            let py = ys.map(y);
            let class = if y == 0.0 && pane.kind == PaneKind::Linear && ys.lo < 0.0 && ys.hi > 0.0 {
                "axis"
            } else {
                "grid"
            };
            let color = if class == "axis" {
                palette.axis
            } else {
                palette.grid
            };
            let _ = write!(
                svg,
                "<line class=\"{class}\" x1=\"{left:.1}\" y1=\"{py:.1}\" x2=\"{:.1}\" y2=\"{py:.1}\" stroke=\"{color}\"/>",
                width - right
            );
        }
        let _ = write!(
            svg,
            "<path class=\"axis\" d=\"M{left:.1},{top:.1}V{bottom:.1}H{:.1}\" fill=\"none\" stroke=\"{}\"/>",
            width - right,
            palette.axis
        );
        // Traces.
        let _ = write!(svg, "<g clip-path=\"url(#{uid}-c{pi})\">");
        for t in &traces {
            let ysrc = &t.y[pi];
            let mut px = Vec::with_capacity(t.x.len());
            let mut py = Vec::with_capacity(t.x.len());
            // Keep one point beyond each side of the window so lines reach
            // the edges.
            let n = t.x.len();
            for (i, (&x, &y)) in t.x.iter().zip(ysrc).enumerate() {
                let inside = in_window(x)
                    || (i + 1 < n && x < x_lo && t.x[i + 1] >= x_lo)
                    || (i > 0 && x > x_hi && t.x[i - 1] <= x_hi);
                if !inside {
                    continue;
                }
                px.push(if finite_x(x) {
                    x_scale.map(x)
                } else {
                    f64::NAN
                });
                py.push(if y.is_finite() { ys.map(y) } else { f64::NAN });
            }
            // A bundle of many faint steps shares the budget, since no one
            // trace in it needs full detail.
            let budget = if t.faint {
                (8 * request.max_points / traces.len().max(1)).max(100)
            } else {
                request.max_points
            };
            let keep = downsample(&px, &py, budget.max(16));
            let d = path_data(&px, &py, &keep);
            if d.is_empty() {
                continue;
            }
            let _ = write!(
                svg,
                "<path class=\"trace s{s}\" d=\"{d}\" stroke=\"{c}\" stroke-width=\"{sw}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{op} fill=\"none\"><title>{label}</title></path>",
                s = t.slot % 8 + 1,
                c = palette.series[t.slot % 8],
                sw = if t.faint { "1" } else { "2" },
                op = if t.faint {
                    " stroke-opacity=\"0.45\""
                } else {
                    ""
                },
                label = esc(&t.label)
            );
        }
        // Marker lines inside the clip.
        for m in &request.markers {
            match m {
                Marker::X { x, .. } => {
                    let px = x_scale.map(*x);
                    if px.is_finite() {
                        let _ = write!(
                            svg,
                            "<line class=\"marker\" x1=\"{px:.1}\" y1=\"{top:.1}\" x2=\"{px:.1}\" y2=\"{bottom:.1}\" stroke=\"{}\" stroke-dasharray=\"4 3\"/>",
                            palette.ink2
                        );
                    }
                }
                Marker::Y { y, pane: mp, .. } if pane_index(&panes, *mp) == pi => {
                    let py = ys.map(*y);
                    let _ = write!(
                        svg,
                        "<line class=\"marker\" x1=\"{left:.1}\" y1=\"{py:.1}\" x2=\"{:.1}\" y2=\"{py:.1}\" stroke=\"{}\" stroke-dasharray=\"4 3\"/>",
                        width - right,
                        palette.ink2
                    );
                }
                _ => {}
            }
        }
        svg.push_str("</g>");
        // Marker dots and labels, unclipped so labels near an edge stay whole.
        for m in &request.markers {
            match m {
                Marker::X {
                    x,
                    label: Some(label),
                } if pi == 0 => {
                    let px = x_scale.map(*x);
                    if px.is_finite() && px >= left && px <= width - right {
                        let w = text_width(label, 11.0);
                        let (tx, anchor, x0, x1) = if px + 6.0 + w > width - right {
                            (px - 5.0, "end", px - 5.0 - w, px)
                        } else {
                            (px + 5.0, "start", px, px + 5.0 + w)
                        };
                        // Above the plot, or just inside its top when that
                        // spot is taken by a neighbouring label.
                        let clash = x_label_spans
                            .iter()
                            .any(|&(a, b)| x0 < b + 6.0 && x1 > a - 6.0);
                        let ty = if clash { top + 13.0 } else { top - 5.0 };
                        if !clash {
                            x_label_spans.push((x0, x1));
                        }
                        let _ = write!(
                            svg,
                            "<text class=\"marker-label\" x=\"{tx:.1}\" y=\"{ty:.1}\" text-anchor=\"{anchor}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">{}</text>",
                            palette.ink2,
                            palette.surface,
                            esc(label)
                        );
                    }
                }
                Marker::Y {
                    y,
                    label: Some(label),
                    pane: mp,
                } if pane_index(&panes, *mp) == pi => {
                    let py = ys.map(*y);
                    let _ = write!(
                        svg,
                        "<text class=\"marker-label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"{}\" stroke=\"{}\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">{}</text>",
                        width - right - 4.0,
                        // Below the line, where it cannot be read as the
                        // label of a grid line above; above it at the floor.
                        if py + 16.0 > bottom {
                            py - 5.0
                        } else {
                            py + 14.0
                        },
                        palette.ink2,
                        palette.surface,
                        esc(label)
                    );
                }
                Marker::Point {
                    x,
                    y,
                    label,
                    pane: mp,
                } if pane_index(&panes, *mp) == pi => {
                    let (px, py) = (x_scale.map(*x), ys.map(*y));
                    if px.is_finite() && py.is_finite() {
                        let _ = write!(
                            svg,
                            "<circle class=\"marker-dot\" cx=\"{px:.1}\" cy=\"{py:.1}\" r=\"4\" fill=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>",
                            palette.ink2, palette.surface
                        );
                        if let Some(label) = label {
                            let w = text_width(label, 11.0);
                            let (tx, anchor) = if px + 8.0 + w > width - right {
                                (px - 8.0, "end")
                            } else {
                                (px + 8.0, "start")
                            };
                            let ty = if py - 8.0 < top + 10.0 {
                                py + 16.0
                            } else {
                                py - 8.0
                            };
                            let _ = write!(
                                svg,
                                "<text class=\"marker-label\" x=\"{tx:.1}\" y=\"{ty:.1}\" text-anchor=\"{anchor}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"3\" stroke-linejoin=\"round\" paint-order=\"stroke\">{}</text>",
                                palette.ink2,
                                palette.surface,
                                esc(label)
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        // Y tick labels and title.
        svg.push_str("<g class=\"ticks\" font-variant-numeric=\"tabular-nums\">");
        for (y, label) in y_ticks.iter().zip(y_labels) {
            let py = ys.map(*y);
            let _ = write!(
                svg,
                "<text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"{}\">{}</text>",
                left - 8.0,
                py + 3.5,
                palette.muted,
                esc(label)
            );
        }
        svg.push_str("</g>");
        let cy = (top + bottom) / 2.0;
        let _ = write!(
            svg,
            "<text class=\"axis-title\" x=\"14\" y=\"{cy:.1}\" text-anchor=\"middle\" transform=\"rotate(-90 14 {cy:.1})\" fill=\"{}\">{}</text>",
            palette.ink2,
            esc(&pane.title)
        );
        svg.push_str("</g>");
    }

    // X tick labels and title under the last pane.
    let bottom = pane_rects[pane_rects.len() - 1].1;
    svg.push_str("<g class=\"ticks\" font-variant-numeric=\"tabular-nums\">");
    for (x, label) in &x_labels {
        let px = x_scale.map(*x);
        // Centred under the tick, unless that would run off the image.
        let half = text_width(label, 11.0) / 2.0;
        let (px, anchor) = if px - half < 2.0 {
            (2.0f64.max(px - half), "start")
        } else if px + half > width - 2.0 {
            ((width - 2.0).min(px + half), "end")
        } else {
            (px, "middle")
        };
        let _ = write!(
            svg,
            "<text class=\"tick\" x=\"{px:.1}\" y=\"{:.1}\" text-anchor=\"{anchor}\" fill=\"{}\">{}</text>",
            bottom + 16.0,
            palette.muted,
            esc(label)
        );
    }
    svg.push_str("</g>");
    let x_title = {
        let name = axis_vec.name.as_str();
        let unit = axis_vec.quantity.unit();
        if unit.is_empty() {
            name.to_string()
        } else {
            format!("{name} ({unit})")
        }
    };
    let _ = write!(
        svg,
        "<text class=\"axis-title\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" fill=\"{}\">{}</text>",
        (left + width - right) / 2.0,
        height - 10.0,
        palette.ink2,
        esc(&x_title)
    );
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn quantity_name(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "V" => "voltage",
        "A" => "current",
        "W" => "power",
        "dB" => "magnitude",
        "°" => "phase",
        _ => return None,
    })
}

fn pane_index(panes: &[PaneSpec], pane: Pane) -> usize {
    match pane {
        Pane::Phase => panes
            .iter()
            .position(|p| p.kind == PaneKind::Phase)
            .unwrap_or(0),
        Pane::Main => 0,
    }
}

fn pick_dataset<'a>(
    datasets: &'a [Dataset],
    request: &PlotRequest,
    exprs: &[Expr],
) -> Result<&'a Dataset, PlotError> {
    if let Some(i) = request.dataset {
        return datasets.get(i).ok_or(PlotError::NoDataset {
            index: i,
            count: datasets.len(),
        });
    }
    let mut first_err = None;
    for ds in datasets {
        match exprs.iter().try_for_each(|e| e.eval(ds, 0..0).map(|_| ())) {
            Ok(()) => return Ok(ds),
            Err(e) => {
                first_err.get_or_insert(e.to_string());
            }
        }
    }
    Err(PlotError::Expr(
        first_err.unwrap_or_else(|| "there are no results to plot".into()),
    ))
}

fn fnv(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn esc(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => s.push(' '),
            c => s.push(c),
        }
    }
    s
}

/// Rough width of a label in a proportional sans: good enough to keep
/// labels from running off the edge or into each other.
fn text_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | '(' | ')' | ' ' => 0.32,
            'm' | 'w' | 'M' | 'W' => 0.86,
            c if c.is_ascii_uppercase() => 0.66,
            c if c.is_ascii_digit() => 0.56,
            _ => 0.54,
        })
        .sum::<f64>()
        * size
}

/// The 1, 2 or 5 times a power of ten nearest `raw` (in ratio).
fn nice_step(raw: f64) -> f64 {
    if !(raw > 0.0 && raw.is_finite()) {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nice = if f < 1.5 {
        1.0
    } else if f < 3.5 {
        2.0
    } else if f < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Widen a value range by a small margin so traces do not run along the
/// frame, and give a flat signal some height.
fn padded(lo: f64, hi: f64) -> (f64, f64) {
    let (mut lo, mut hi) = (lo, hi);
    if lo == hi {
        let d = if lo == 0.0 { 1.0 } else { lo.abs() * 0.1 };
        lo -= d;
        hi += d;
    }
    let pad = (hi - lo) * 0.05;
    (lo - pad, hi + pad)
}

/// A value axis: the padded data range, with nice ticks inside it. The
/// bounds are not rounded out to whole steps, which would waste up to a
/// step of height on each side.
fn linear_ticks(lo: f64, hi: f64, target: usize) -> (f64, f64, Vec<f64>) {
    let (lo, hi) = padded(lo, hi);
    (lo, hi, linear_ticks_within(lo, hi, target))
}

/// Ticks inside a fixed range (the x axis keeps the data's extent).
fn linear_ticks_within(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let step = nice_step((hi - lo) / target.max(1) as f64);
    steps_between((lo / step - 1e-9).ceil() * step, hi, step)
}

fn steps_between(start: f64, end: f64, step: f64) -> Vec<f64> {
    let n = ((end - start) / step + 1e-9).floor();
    if !(n.is_finite() && n >= 0.0) || n > 1000.0 {
        return vec![start];
    }
    // Ticks go through a short decimal so 3 x 0.2 is 0.6, not
    // 0.6000000000000001, which matters to anyone comparing them.
    let decimals = ((-step.log10()).ceil().max(0.0) as usize + 1).min(17);
    (0..=n as usize)
        .map(|k| {
            let v = start + k as f64 * step;
            let snapped = format!("{v:.decimals$}").parse().unwrap_or(v);
            if snapped == 0.0 { 0.0 } else { snapped }
        })
        .collect()
}

/// Phase ticks at a multiple of 5, 15, 30, 45, 90 or 180 degrees.
fn phase_ticks(lo: f64, hi: f64) -> (f64, f64, Vec<f64>) {
    let (mut lo, mut hi) = (lo, hi);
    if hi - lo < 10.0 {
        let mid = (lo + hi) / 2.0;
        lo = mid - 5.0;
        hi = mid + 5.0;
    }
    let (lo, hi) = padded(lo, hi);
    let raw = (hi - lo) / 5.0;
    let step = [5.0, 10.0, 15.0, 30.0, 45.0, 90.0, 180.0, 360.0]
        .into_iter()
        .find(|s| *s >= raw)
        .unwrap_or_else(|| nice_step(raw));
    (
        lo,
        hi,
        steps_between((lo / step - 1e-9).ceil() * step, hi, step),
    )
}

/// Decade ticks for a log axis: major lines at powers of ten, minor lines
/// at 2..9, and labels at decades (or 1-2-5 within a narrow range, or every
/// other decade across a very wide one).
fn log_ticks(lo: f64, hi: f64) -> (Vec<f64>, Vec<f64>, Vec<(f64, String)>) {
    let d0 = lo.log10().floor() as i32;
    let d1 = hi.log10().ceil() as i32;
    let decades = hi.log10() - lo.log10();
    let within = |v: f64| v >= lo * (1.0 - 1e-9) && v <= hi * (1.0 + 1e-9);
    let mut major = Vec::new();
    let mut minor = Vec::new();
    let mut labels = Vec::new();
    let every = if decades > 10.0 { 2 } else { 1 };
    for d in d0..=d1 {
        let base = 10f64.powi(d);
        if within(base) {
            major.push(base);
            if d.rem_euclid(every) == 0 {
                labels.push((base, units::format_with_unit(base, "")));
            }
        }
        for k in 2..=9 {
            let v = base * k as f64;
            if within(v) {
                minor.push(v);
                if decades < 1.5 && (k == 2 || k == 5) {
                    labels.push((v, units::format_with_unit(v, "")));
                }
            }
        }
    }
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    if labels.is_empty() {
        labels.push((lo, units::format_with_unit(lo, "")));
        labels.push((hi, units::format_with_unit(hi, "")));
    }
    (major, minor, labels)
}

/// Tick labels sharing one SI scale chosen from the largest tick, with as
/// many decimals as the step needs, the same on every label:
/// `0, 0.5m, 1.0m, 1.5m`, never `0, 500u, 1m, 1.5m`.
fn tick_labels(ticks: &[f64], step: f64) -> Vec<String> {
    const SCALES: [(f64, &str); 9] = [
        (1e12, "T"),
        (1e9, "G"),
        (1e6, "M"),
        (1e3, "k"),
        (1.0, ""),
        (1e-3, "m"),
        (1e-6, "u"),
        (1e-9, "n"),
        (1e-12, "p"),
    ];
    let maxabs = ticks.iter().map(|t| t.abs()).fold(0.0, f64::max);
    let (scale, suffix) = if maxabs == 0.0 {
        (1.0, "")
    } else {
        SCALES
            .iter()
            .copied()
            .find(|(s, _)| maxabs >= *s * 0.99995)
            .unwrap_or((1e-15, "f"))
    };
    let decimals = if step > 0.0 {
        (-(step / scale).log10() - 1e-9).ceil().clamp(0.0, 6.0) as usize
    } else {
        0
    };
    ticks
        .iter()
        .map(|t| {
            let v = t / scale;
            if v.abs() < 1e-12 {
                return "0".into();
            }
            let s = format!("{v:.decimals$}");
            format!("{s}{suffix}")
        })
        .collect()
}

/// Indices to draw: everything when it fits, otherwise the first and last
/// point plus the minimum and maximum of each pixel bucket, in order, so the
/// envelope (and every spike) survives. Non-finite points are kept, one per
/// bucket, so gaps in the data stay gaps.
fn downsample(px: &[f64], py: &[f64], max_points: usize) -> Vec<usize> {
    let n = px.len().min(py.len());
    if n <= max_points {
        return (0..n).collect();
    }
    let buckets = (max_points / 2).max(1);
    let finite: Vec<f64> = px.iter().copied().filter(|v| v.is_finite()).collect();
    let (x0, x1) = finite
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if x1.is_nan() || x0.is_nan() || x1 <= x0 {
        return (0..n).step_by(n.div_ceil(max_points)).collect();
    }
    let width = (x1 - x0) / buckets as f64;
    let mut keep = vec![0];
    let mut i = 0;
    while i < n {
        let b = if px[i].is_finite() {
            (((px[i] - x0) / width) as usize).min(buckets - 1)
        } else {
            usize::MAX
        };
        let mut j = i;
        let (mut lo, mut hi, mut gap) = (None::<usize>, None::<usize>, None::<usize>);
        while j < n {
            let bj = if px[j].is_finite() {
                (((px[j] - x0) / width) as usize).min(buckets - 1)
            } else {
                usize::MAX
            };
            if bj != b {
                break;
            }
            if py[j].is_finite() {
                if lo.is_none_or(|k| py[j] < py[k]) {
                    lo = Some(j);
                }
                if hi.is_none_or(|k| py[j] > py[k]) {
                    hi = Some(j);
                }
            } else if gap.is_none() {
                gap = Some(j);
            }
            j += 1;
        }
        let mut picks: Vec<usize> = [lo, hi, gap].into_iter().flatten().collect();
        picks.sort_unstable();
        picks.dedup();
        for k in picks {
            if keep.last() != Some(&k) {
                keep.push(k);
            }
        }
        i = j;
    }
    if keep.last() != Some(&(n - 1)) {
        keep.push(n - 1);
    }
    keep
}

/// SVG path data, starting a new sub-path after every gap.
fn path_data(px: &[f64], py: &[f64], keep: &[usize]) -> String {
    let mut d = String::with_capacity(keep.len() * 12);
    let mut pen_down = false;
    let mut last = (f64::NAN, f64::NAN);
    for &i in keep {
        let (x, y) = (px[i], py[i]);
        if !(x.is_finite() && y.is_finite()) {
            pen_down = false;
            continue;
        }
        let xr = (x * 10.0).round() / 10.0;
        let yr = (y * 10.0).round() / 10.0;
        if pen_down && (xr, yr) == last {
            continue;
        }
        let _ = write!(d, "{}{xr},{yr}", if pen_down { 'L' } else { 'M' });
        pen_down = true;
        last = (xr, yr);
    }
    d
}

/// Markers that show a measurement on a plot of the same data: vertical
/// lines at crossings, corners and edges, dots at values. Stepped results
/// and results without a value give none.
pub fn markers_for(measure: &Measure, result: &MeasureResult) -> Vec<Marker> {
    let Some(v) = result.value else {
        return Vec::new();
    };
    if result.per_step.len() > 1 {
        return Vec::new();
    }
    let label = format!("{} {}", result.name, human(v, &result.unit));
    let extra = |name: &str| {
        result
            .extra
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.value)
    };
    let x = |x: f64, label: Option<String>| Marker::X { x, label };
    match measure {
        Measure::Crossing { .. } | Measure::UnityGainFreq { .. } | Measure::FreqAtDb { .. } => {
            vec![x(v, Some(label))]
        }
        Measure::Bandwidth3db { .. } => match (extra("f_low"), extra("f_high")) {
            (Some(lo), Some(hi)) => vec![x(lo, None), x(hi, Some(label))],
            _ => vec![x(v, Some(label))],
        },
        Measure::RiseTime { .. } | Measure::FallTime { .. } => {
            match (extra("t_low"), extra("t_high")) {
                (Some(a), Some(b)) => vec![x(a, None), x(b, Some(label))],
                _ => Vec::new(),
            }
        }
        Measure::SettlingTime { .. } => extra("t_settled")
            .map(|t| vec![x(t, Some(label))])
            .unwrap_or_default(),
        Measure::Delay { .. } => match (extra("t_from"), extra("t_to")) {
            (Some(a), Some(b)) => vec![x(a, None), x(b, Some(label))],
            _ => Vec::new(),
        },
        Measure::ValueAt { .. } | Measure::Min { .. } | Measure::Max { .. } => match result.at {
            Some(at) if result.at_unit == "Hz" => vec![x(at, Some(label))],
            Some(at) => vec![Marker::Point {
                x: at,
                y: v,
                label: Some(label),
                pane: Pane::Main,
            }],
            None => Vec::new(),
        },
        Measure::GainDbAt { .. } | Measure::PeakGain { .. } => result
            .at
            .map(|at| {
                vec![Marker::Point {
                    x: at,
                    y: v,
                    label: Some(label),
                    pane: Pane::Main,
                }]
            })
            .unwrap_or_default(),
        Measure::PhaseAt { .. } => result
            .at
            .map(|at| {
                vec![Marker::Point {
                    x: at,
                    y: v,
                    label: Some(label),
                    pane: Pane::Phase,
                }]
            })
            .unwrap_or_default(),
        Measure::PhaseMargin { .. }
        | Measure::GainMargin { .. }
        | Measure::OvershootPct { .. }
        | Measure::UndershootPct { .. } => result
            .at
            .map(|at| vec![x(at, Some(label))])
            .unwrap_or_default(),
        Measure::Avg { .. } => vec![Marker::Y {
            y: v,
            label: Some(label),
            pane: Pane::Main,
        }],
        Measure::Pp { .. } => match (extra("min"), extra("max")) {
            (Some(lo), Some(hi)) => vec![
                Marker::Y {
                    y: lo,
                    label: None,
                    pane: Pane::Main,
                },
                Marker::Y {
                    y: hi,
                    label: Some(label),
                    pane: Pane::Main,
                },
            ],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::{AnalysisKind, Complex};
    use crate::expr::test_support::*;
    use crate::measure::measure_dataset;
    use std::f64::consts::PI;

    /// Element structure of an SVG: one line per element with its class,
    /// indented by depth, runs of identical lines collapsed to `xN`. Numbers
    /// are left out on purpose, so the snapshot pins the structure and not
    /// every coordinate. Panics on unbalanced tags, which doubles as a
    /// well-formedness check.
    fn skeleton(svg: &str) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        let mut rest = svg;
        while let Some(start) = rest.find('<') {
            let end = rest[start..].find('>').expect("unclosed tag") + start;
            let tag = &rest[start + 1..end];
            rest = &rest[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                let open = stack.pop().expect("close without open");
                assert_eq!(open, name.trim(), "mismatched tags");
                continue;
            }
            let self_closing = tag.ends_with('/');
            let name: String = tag
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '/')
                .collect();
            let class = tag
                .split("class=\"")
                .nth(1)
                .and_then(|c| c.split('"').next())
                .map(|c| format!(".{}", c.replace(' ', ".")))
                .unwrap_or_default();
            let pane = tag
                .split("data-pane=\"")
                .nth(1)
                .and_then(|c| c.split('"').next())
                .map(|c| format!("[{c}]"))
                .unwrap_or_default();
            lines.push(format!("{}{name}{class}{pane}", "  ".repeat(stack.len())));
            if !self_closing {
                stack.push(name);
            }
        }
        assert!(stack.is_empty(), "unclosed: {stack:?}");
        let mut out: Vec<String> = Vec::new();
        let mut count = 0;
        for (i, line) in lines.iter().enumerate() {
            count += 1;
            if lines.get(i + 1) != Some(line) {
                out.push(if count > 1 {
                    format!("{line} x{count}")
                } else {
                    line.clone()
                });
                count = 0;
            }
        }
        out.join("\n")
    }

    fn rc_step() -> Dataset {
        let t = linspace(0.0, 5e-3, 501);
        let v: Vec<f64> = t.iter().map(|t| 1.0 - (-t / 1e-3).exp()).collect();
        tran(t, vec![("V(out)", v)])
    }

    fn lowpass(fc: f64) -> Dataset {
        let f = logspace(10.0, 1e6, 201);
        let h: Vec<Complex> = f.iter().map(|&f| poles(f, &[fc], 1.0)).collect();
        ac(f, vec![("V(out)", h)])
    }

    #[test]
    fn transient_structure_snapshot() {
        let svg = plot_svg(&[rc_step()], &PlotRequest::new(["V(out)"])).unwrap();
        let want = "\
svg.aispice-plot
  title
  style
  defs
    clipPath
      rect
  rect.bg
  text.title
  text.subtitle
  g.pane[main]
    line.grid x11
    line.axis
    line.grid x5
    path.axis
    g
      path.trace.s1
        title
    g.ticks
      text.tick x6
    text.axis-title
  g.ticks
    text.tick x11
  text.axis-title";
        assert_eq!(skeleton(&svg), want, "\n{}", skeleton(&svg));
        // A single trace has no legend; the title names it.
        assert!(svg.contains(">V(out)</text>"));
        assert!(svg.contains(">V(out) (V)</text>"));
        assert!(svg.contains(">time (s)</text>"));
        // Time ticks share one scale and decimals: 0, 0.5m, 1.0m, ...
        for label in [">0<", ">0.5m<", ">1.0m<", ">5.0m<"] {
            assert!(svg.contains(label), "{label}");
        }
    }

    #[test]
    fn bode_structure_snapshot() {
        let ds = lowpass(1e3);
        let mut req = PlotRequest::new(["V(out)"]);
        let r = measure_dataset("bw", &Measure::parse("bandwidth_3db(V(out))").unwrap(), &ds);
        req.markers = markers_for(&Measure::parse("bandwidth_3db(V(out))").unwrap(), &r);
        req.markers.push(Marker::Y {
            y: -3.0,
            label: Some("-3 dB".into()),
            pane: Pane::Main,
        });
        let svg = plot_svg(&[ds], &req).unwrap();
        let sk = skeleton(&svg);
        assert!(
            sk.contains("g.pane[magnitude]") && sk.contains("g.pane[phase]"),
            "{sk}"
        );
        assert_eq!(sk.matches("path.trace.s1").count(), 2);
        // The corner on both panes, and the -3 dB level on magnitude.
        assert_eq!(svg.matches("class=\"marker\"").count(), 3, "{sk}");
        assert!(sk.contains("line.grid-minor"));
        for label in [
            ">100<",
            ">1k<",
            ">10k<",
            ">100k<",
            ">1M<",
            ">-30<",
            ">-90<",
            ">phase (°)<",
            ">magnitude (dB)<",
        ] {
            assert!(svg.contains(label), "{label}");
        }
        assert!(svg.contains(">bw 1kHz<"), "marker label");
        assert!(svg.contains(">frequency (Hz)<"));
    }

    #[test]
    fn stepped_traces_get_distinct_colours_and_a_legend() {
        let parts: Vec<(String, Dataset)> = [1e3, 2e3, 4e3]
            .iter()
            .map(|&fc| {
                (
                    format!("C={}", units::format(1.0 / (2.0 * PI * 1e3 * fc))),
                    lowpass(fc),
                )
            })
            .collect();
        let ds = stepped(parts.iter().map(|(l, d)| (l.as_str(), d.clone())).collect());
        let svg = plot_svg(&[ds], &PlotRequest::new(["V(out)"])).unwrap();
        for s in ["trace s1", "trace s2", "trace s3"] {
            assert!(svg.contains(s), "{s}");
        }
        assert!(svg.contains("class=\"legend\""));
        assert_eq!(svg.matches("class=\"legend-item\"").count(), 3);
        assert!(svg.contains(">C=159.2n<"));
        assert!(svg.contains("AC Analysis, 3 steps"));
        // Many steps collapse into one translucent bundle per expression.
        let many: Vec<(String, Dataset)> = (0..20)
            .map(|i| (format!("mc{i}"), lowpass(1e3 * (1.0 + i as f64 * 0.01))))
            .collect();
        let ds = stepped(many.iter().map(|(l, d)| (l.as_str(), d.clone())).collect());
        let svg = plot_svg(&[ds], &PlotRequest::new(["V(out)"])).unwrap();
        assert_eq!(svg.matches("class=\"trace s1\"").count(), 40);
        assert!(svg.contains("stroke-opacity=\"0.45\""));
        // One bundle is one series: the title names it, no legend.
        assert_eq!(svg.matches("class=\"legend-item\"").count(), 0);
        assert!(svg.contains("AC Analysis, 20 steps"));
    }

    #[test]
    fn large_transients_are_thinned_but_keep_spikes() {
        let n = 400_000;
        let t = linspace(0.0, 1e-3, n);
        let mut v: Vec<f64> = t.iter().map(|&t| (2.0 * PI * 50e3 * t).sin()).collect();
        v[123_457] = 5.0; // a one-sample spike
        v[300_001] = -4.0;
        let ds = tran(t, vec![("V(sw)", v)]);
        let svg = plot_svg(&[ds], &PlotRequest::new(["V(sw)"])).unwrap();
        assert!(svg.len() < 60_000, "{} bytes", svg.len());
        let d = svg
            .split("class=\"trace s1\" d=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let points: Vec<(f64, f64)> = d
            .split(['M', 'L'])
            .filter(|s| !s.is_empty())
            .map(|p| {
                let (x, y) = p.split_once(',').unwrap();
                (x.parse().unwrap(), y.parse().unwrap())
            })
            .collect();
        assert!(
            points.len() <= 2100 && points.len() > 500,
            "{}",
            points.len()
        );
        // The spikes set the y range, so they reach the top and bottom of
        // the plotted value range.
        let ymin = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let ymax = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        assert!(ymax - ymin > 250.0, "spikes lost: {ymin}..{ymax}");
    }

    #[test]
    fn downsample_keeps_extremes_and_gaps() {
        let n = 10_000;
        let px: Vec<f64> = (0..n).map(|i| i as f64 / 10.0).collect();
        let mut py: Vec<f64> = (0..n).map(|i| (i as f64 / 50.0).sin()).collect();
        py[4321] = 9.0;
        py[7000] = f64::NAN;
        let keep = downsample(&px, &py, 500);
        assert!(keep.len() <= 520);
        assert!(keep.contains(&4321));
        assert!(keep.contains(&7000));
        assert_eq!(keep[0], 0);
        assert_eq!(*keep.last().unwrap(), n - 1);
        assert!(keep.windows(2).all(|w| w[0] < w[1]));
        let d = path_data(&px, &py, &keep);
        assert_eq!(d.matches('M').count(), 2, "a gap starts a new sub-path");
    }

    #[test]
    fn ticks_are_nice() {
        assert_eq!(nice_step(0.13), 0.1);
        assert_eq!(nice_step(0.22), 0.2);
        assert_eq!(nice_step(4.0), 5.0);
        assert_eq!(nice_step(80.0), 100.0);
        let (lo, hi, t) = linear_ticks(0.0, 1.0, 5);
        assert!((lo + 0.05).abs() < 1e-12 && (hi - 1.05).abs() < 1e-12);
        assert_eq!(t, vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]);
        assert_eq!(
            tick_labels(&[0.0, 0.5e-3, 1e-3, 1.5e-3], 0.5e-3),
            vec!["0", "0.5m", "1.0m", "1.5m"]
        );
        assert_eq!(
            tick_labels(&[-1.0, -0.5, 0.0, 0.5, 1.0], 0.5),
            vec!["-1.0", "-0.5", "0", "0.5", "1.0"]
        );
        assert_eq!(
            tick_labels(&[3.3, 3.31, 3.32], 0.01),
            vec!["3.30", "3.31", "3.32"]
        );
        assert_eq!(tick_labels(&[0.0, 1e-3, 2e-3], 1e-3), vec!["0", "1m", "2m"]);
        assert_eq!(linear_ticks_within(1e-3, 1.05e-3, 8)[0], 1e-3);
        let t = steps_between(0.1, 0.5, 0.1);
        assert_eq!(t, vec![0.1, 0.2, 0.3, 0.4, 0.5]);
        assert_eq!(steps_between(1e-3, 1.05e-3, 5e-6)[3], 1.015e-3);
        assert_eq!(
            tick_labels(&[1e-3, 1.1e-3, 1.2e-3], 1e-4),
            vec!["1.0m", "1.1m", "1.2m"]
        );
        let (_, _, labels) = log_ticks(10.0, 1e6);
        let l: Vec<&str> = labels.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(l, vec!["10", "100", "1k", "10k", "100k", "1M"]);
        let (_, _, labels) = log_ticks(1e3, 8e3);
        let l: Vec<&str> = labels.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(l, vec!["1k", "2k", "5k"]);
        let (_, _, labels) = log_ticks(1e-3, 1e12);
        assert!(labels.len() <= 8);
        let (lo, hi, t) = phase_ticks(-178.0, 0.0);
        assert!(lo < -178.0 && hi > 0.0);
        assert_eq!(t, vec![-180.0, -135.0, -90.0, -45.0, 0.0]);
    }

    #[test]
    fn themes_markers_and_escaping() {
        let ds = rc_step();
        let mut req = PlotRequest::new(["V(out)", "V(out)*0.5"]);
        req.theme = Theme::Dark;
        req.title = Some("A & B <test>".into());
        let rise = Measure::parse("rise_time(V(out))").unwrap();
        let r = measure_dataset("rise", &rise, &ds);
        req.markers = markers_for(&rise, &r);
        let mx = Measure::parse("max(V(out))").unwrap();
        req.markers
            .extend(markers_for(&mx, &measure_dataset("vmax", &mx, &ds)));
        let svg = plot_svg(std::slice::from_ref(&ds), &req).unwrap();
        assert!(svg.contains("data-theme=\"dark\""));
        assert!(svg.contains("fill=\"#1a1a19\""));
        assert!(svg.contains("A &amp; B &lt;test&gt;"));
        assert!(!svg.contains("<test>"));
        assert_eq!(svg.matches("class=\"marker\"").count(), 2);
        assert!(svg.contains("class=\"marker-dot\""));
        assert!(svg.contains(">rise 2.1"), "rise marker label");
        assert!(svg.contains(">vmax 993.3mV<"), "{svg}");
        assert_eq!(svg.matches("class=\"legend-item\"").count(), 2);
        // Both light and dark variables are defined for viewers that switch.
        assert!(svg.contains("prefers-color-scheme:dark"));
        assert!(svg.contains("--s1:#2a78d6") && svg.contains("--s1:#3987e5"));
        let light = plot_svg(&[ds], &PlotRequest::new(["V(out)"])).unwrap();
        assert!(!light.contains("data-theme=\""));
        assert!(light.contains("fill=\"#fcfcfb\""));
    }

    #[test]
    fn requests_from_json_and_errors() {
        let req: PlotRequest = serde_json::from_str(
            r#"{"traces":["V(out)"],"kind":"magnitude","markers":[{"kind":"x","x":1000}]}"#,
        )
        .unwrap();
        assert_eq!(req.max_points, 2000);
        assert_eq!(req.kind, PlotKind::Magnitude);
        let svg = plot_svg(&[lowpass(1e3)], &req).unwrap();
        assert!(!svg.contains("data-pane=\"phase\""));
        let ds = rc_step();
        assert_eq!(
            plot_svg(std::slice::from_ref(&ds), &PlotRequest::default()),
            Err(PlotError::NoTraces)
        );
        let err = plot_svg(std::slice::from_ref(&ds), &PlotRequest::new(["V(otu)"])).unwrap_err();
        assert!(err.to_string().contains("did you mean V(out)"), "{err}");
        let err = plot_svg(std::slice::from_ref(&ds), &PlotRequest::new(["V(out"])).unwrap_err();
        assert!(matches!(err, PlotError::Expr(_)));
        let nine: Vec<String> = (0..9).map(|_| "V(out)".to_string()).collect();
        assert_eq!(
            plot_svg(std::slice::from_ref(&ds), &PlotRequest::new(nine)),
            Err(PlotError::TooManyTraces(9))
        );
        let mut req = PlotRequest::new(["V(out)"]);
        req.dataset = Some(3);
        assert!(matches!(
            plot_svg(std::slice::from_ref(&ds), &req),
            Err(PlotError::NoDataset { index: 3, count: 1 })
        ));
        let mut op = tran(vec![0.0], vec![("V(out)", vec![1.0])]);
        op.axis = None;
        op.kind = AnalysisKind::Op;
        assert!(
            plot_svg(&[op], &PlotRequest::new(["V(out)"]))
                .unwrap_err()
                .to_string()
                .contains("operating point")
        );
        let nan = tran(linspace(0.0, 1.0, 5), vec![("V(x)", vec![f64::NAN; 5])]);
        assert!(matches!(
            plot_svg(&[nan], &PlotRequest::new(["V(x)"])),
            Err(PlotError::NoData(_))
        ));
        let mut req = PlotRequest::new(["V(out)"]);
        req.x_range = Some([2e-3, 1e-3]);
        assert!(plot_svg(std::slice::from_ref(&ds), &req).is_err());
        req.x_range = Some([1e-3, 2e-3]);
        let svg = plot_svg(std::slice::from_ref(&ds), &req).unwrap();
        assert!(svg.contains(">1.2m<") && svg.contains(">2.0m<"));
        assert!(!svg.contains(">0.5m<"), "ticks outside x_range");
        let mut tiny = PlotRequest::new(["V(out)"]);
        tiny.width = 100;
        tiny.height = 80;
        assert!(plot_svg(&[ds], &tiny).is_err());
    }

    #[test]
    fn picks_the_dataset_with_the_vectors() {
        let tr = rc_step();
        let a = lowpass(1e3);
        let svg = plot_svg(&[tr, a], &PlotRequest::new(["V(out)"])).unwrap();
        assert!(svg.contains(">time (s)<"));
        let f = logspace(10.0, 1e6, 11);
        let only_ac = ac(f, vec![("V(lg)", vec![Complex::new(1.0, 0.0); 11])]);
        let svg = plot_svg(&[rc_step(), only_ac], &PlotRequest::new(["V(lg)"])).unwrap();
        assert!(svg.contains(">frequency (Hz)<"));
    }
}
