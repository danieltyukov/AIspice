//! Tools that simulate, measure, check specs and size circuits.

use super::schematic_tools::spec;
use crate::runner::{RunMods, StoredRun};
use crate::workspace::Workspace;
use aispice_agent::tool::parse_input;
use aispice_agent::{Tool, ToolContext, ToolOutput, ToolSpec};
use aispice_core::edit::{EditOp, apply};
use aispice_core::units;
use aispice_sim::dataset::{AnalysisKind, Dataset, Quantity};
use aispice_sim::measure::{Measure, MeasureResult, measure};
use aispice_sim::montecarlo::{self, Tolerance};
use aispice_sim::optimize::{
    self, CmaEs, CmaEsOptions, Goal, NelderMead, NelderMeadOptions, Objective, Param, RunOptions,
};
use aispice_sim::plot::{PlotRequest, plot_svg};
use aispice_sim::spec::{Spec, SpecReport, evaluate, parse_specs};
use aispice_sim::sweep::{self, SweepParam};
use async_trait::async_trait;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const MEASURE_SYNTAX: &str = "Measurements are written `name = kind(args)`, for example `f3db = bandwidth_3db(V(out))`, `gain = gain_db_at(V(out)/V(in), 1k)`, `pm = phase_margin(V(out))`, `tr = rise_time(V(out), 10, 90)`, `vmax = max(V(out), 1m, 5m)`. Kinds: value_at(expr, at), min/max/pp/avg/rms/integral(expr[, from, to]), crossing(expr, level[, rise|fall|either, nth]), rise_time/fall_time(expr[, low_pct, high_pct]), overshoot_pct/undershoot_pct(expr), settling_time(expr[, tolerance_pct]), delay(from_expr, to_expr[, level_pct]), frequency/period/duty_cycle(expr), thd(expr, fundamental[, harmonics]), gain_db_at/phase_at(expr, freq), bandwidth_3db(expr[, dc|peak]), unity_gain_freq/phase_margin/gain_margin/peak_gain(expr), freq_at_db(expr, db). Expressions use V(node), V(a,b), I(R1), + - * /, and db(), mag(), ph().";

const SPEC_SYNTAX: &str = "Specs are one per line: `name = measurement op limit`, with op one of `>=`, `<=`, `in a..b` or `= target +- tol`, e.g. `bw = bandwidth_3db(V(out)) in 1Meg..2Meg`, `gain = gain_db_at(V(out)/V(in), 1k) >= 20`, `pm = phase_margin(V(out)) >= 45`.";

fn parse_measures(lines: &[String]) -> Result<Vec<(String, Measure)>, String> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let (name, body) = match l.split_once('=') {
                Some((n, b)) if !n.contains('(') => (n.trim().to_string(), b.trim()),
                _ => (format!("m{}", i + 1), l.trim()),
            };
            Measure::parse(body)
                .map(|m| (name, m))
                .map_err(|e| format!("measurement `{l}`: {e}"))
        })
        .collect()
}

/// Specs for a circuit: given text, or the `<circuit>.specs` file beside it.
fn load_specs(
    ws: &Workspace,
    circuit: &str,
    given: Option<&str>,
) -> Result<(Vec<Spec>, String), String> {
    let p = ws.project().map_err(|e| e.to_string())?;
    let text = match given {
        Some(t) => t.to_string(),
        None => p.read_text(&specs_file(circuit)).map_err(|_| {
            format!(
                "no specs given and no {} file; pass specs, for example: {SPEC_SYNTAX}",
                specs_file(circuit)
            )
        })?,
    };
    let specs = parse_specs(&text).map_err(|e| format!("specs: {e}. {SPEC_SYNTAX}"))?;
    if specs.is_empty() {
        return Err(format!("no specs found. {SPEC_SYNTAX}"));
    }
    Ok((specs, text))
}

/// Save specs beside the circuit, with the user's approval when the front end
/// asks for it. Specs files are plain text and only ever `<circuit>.specs`.
async fn save_specs(ws: &Workspace, p: &crate::project::Project, circuit: &str, text: &str) {
    let file = specs_file(circuit);
    if !ws
        .approve(circuit, &format!("Save specs to {file}"), text)
        .await
    {
        return;
    }
    if let Ok(path) = p.resolve(&file) {
        let _ = crate::project::atomic_write(&path, text.as_bytes());
    }
}

pub fn specs_file(circuit: &str) -> String {
    match circuit.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.specs"),
        None => format!("{circuit}.specs"),
    }
}

/// The structured run payload the desktop app renders (`RunView`).
pub fn run_view(run: &StoredRun, measured: &[MeasureResult]) -> Value {
    let datasets: Vec<Value> = run
        .output
        .datasets
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({
                "index": i,
                "plotname": d.plotname,
                "kind": d.kind,
                "axis": d.axis_vector().map(|v| v.name.clone()),
                "vectors": d.vectors.iter().map(|v| json!({"name": v.name, "quantity": v.quantity})).collect::<Vec<_>>(),
                "points": d.len(),
                "steps": d.steps.iter().map(|s| s.label.clone()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut measurements: Vec<Value> = run
        .output
        .measurements
        .iter()
        .map(|m| {
            let display = m.value.map(units::format).unwrap_or_else(|| "failed".into());
            json!({"name": m.name, "value": m.value, "unit": "", "display": display, "note": m.detail})
        })
        .collect();
    measurements.extend(measured.iter().map(|m| json!({"name": m.name, "value": m.value, "unit": m.unit, "display": m.display(), "note": m.note})));
    json!({
        "run_id": run.id,
        "circuit": run.circuit,
        "simulator": run.simulator,
        "datasets": datasets,
        "measurements": measurements,
        "op": op_values(&run.output.datasets),
        "errors": run.output.errors,
        "warnings": run.output.warnings,
        "duration_ms": run.output.duration.as_millis() as u64,
        "log": run.output.log.chars().take(20_000).collect::<String>(),
    })
}

fn op_values(datasets: &[Dataset]) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    if let Some(op) = datasets.iter().find(|d| d.kind == AnalysisKind::Op) {
        for v in &op.vectors {
            if let Some(x) = v.data.real().first() {
                out.insert(v.name.clone(), json!(x));
            }
        }
    }
    out
}

fn run_text(run: &StoredRun, measured: &[MeasureResult]) -> String {
    let o = &run.output;
    let mut t = format!(
        "Simulated {} with {} in {} ms (run {}).\n",
        run.circuit,
        run.simulator.name(),
        o.duration.as_millis(),
        run.id
    );
    for n in &run.notes {
        t.push_str(&format!("  note: {n}\n"));
    }
    for e in &o.errors {
        t.push_str(&format!("  error: {e}\n"));
    }
    for w in o.warnings.iter().take(10) {
        t.push_str(&format!("  warning: {w}\n"));
    }
    for d in &o.datasets {
        let names: Vec<&str> = d.vectors.iter().map(|v| v.name.as_str()).take(40).collect();
        let more = d.vectors.len().saturating_sub(40);
        t.push_str(&format!(
            "  {} ({:?}): {} points{}; vectors: {}{}\n",
            d.plotname,
            d.kind,
            d.len(),
            if d.is_stepped() {
                format!(", {} steps", d.steps.len())
            } else {
                String::new()
            },
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ));
    }
    let op = op_values(&o.datasets);
    if !op.is_empty() {
        t.push_str("Operating point:\n");
        for (k, v) in op.iter().take(60) {
            let unit = if k.starts_with('I') { "A" } else { "V" };
            t.push_str(&format!(
                "  {k} = {}\n",
                units::format_with_unit(v.as_f64().unwrap_or(f64::NAN), unit)
            ));
        }
    }
    if !o.measurements.is_empty() {
        t.push_str("Simulator .meas results:\n");
        for m in &o.measurements {
            let step = m.step.map(|s| format!(" (step {s})")).unwrap_or_default();
            t.push_str(&format!(
                "  {}{step} = {}\n",
                m.name,
                m.value
                    .map(units::format)
                    .unwrap_or_else(|| "failed".into())
            ));
        }
    }
    if !measured.is_empty() {
        t.push_str("Measurements:\n");
        for m in measured {
            t.push_str(&format!("  {}\n", m.display()));
        }
    }
    t
}

async fn fresh_or_given_run(
    ws: &Workspace,
    circuit: &str,
    run_id: Option<&str>,
    ctx: &ToolContext,
) -> Result<Arc<StoredRun>, String> {
    if let Some(id) = run_id {
        return ws
            .runner
            .get(id)
            .ok_or_else(|| format!("no run {id} (runs are kept for the session; simulate again)"));
    }
    if let Some(r) = ws.runner.latest(circuit) {
        return Ok(r);
    }
    let p = ws.project().map_err(|e| e.to_string())?;
    ws.runner
        .run_circuit(&p, circuit, None, &RunMods::default(), &ctx.cancel)
        .await
        .map_err(|e| e.to_string())
}

macro_rules! input {
    ($v:expr) => {
        match parse_input($v) {
            Ok(i) => i,
            Err(e) => return e,
        }
    };
}

pub struct Simulators {
    pub ws: Arc<Workspace>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoArgs {}

#[async_trait]
impl Tool for Simulators {
    fn spec(&self) -> ToolSpec {
        spec::<NoArgs>(
            "simulators",
            "List the simulators installed on this machine (ngspice, LTspice, Xyce, Spectre) with versions and how each will run.",
        )
    }

    async fn call(&self, _ctx: &ToolContext, _input: Value) -> ToolOutput {
        let found = self.ws.runner.detect().await;
        let mut t = String::new();
        for (id, d) in &found {
            t.push_str(&format!(
                "{}: {}{}\n",
                id.name(),
                if d.found { "available" } else { "not found" },
                d.version
                    .as_ref()
                    .map(|v| format!(" ({v})"))
                    .unwrap_or_default()
            ));
            for n in &d.notes {
                t.push_str(&format!("  {n}\n"));
            }
        }
        ToolOutput::text(t).with_data(json!({"kind": "generic", "value": found.iter().map(|(i, d)| json!({"id": i, "found": d.found, "version": d.version, "path": d.path, "notes": d.notes})).collect::<Vec<_>>()}))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SimulateInput {
    pub circuit: String,
    /// `ngspice`, `ltspice`, `xyce` or `spectre`. Default: automatic
    /// (ngspice, falling back to LTspice for LTspice-only constructs).
    #[serde(default)]
    pub simulator: Option<String>,
    /// Run this analysis instead of the circuit's own, without changing the
    /// file, e.g. `.ac dec 50 1 10Meg` or `.tran 0 5m 0 1u`.
    #[serde(default)]
    pub analysis: Option<String>,
    /// Measurements to take on the result (see the tool description).
    #[serde(default)]
    pub measurements: Vec<String>,
}

pub struct Simulate {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for Simulate {
    fn spec(&self) -> ToolSpec {
        spec::<SimulateInput>(
            "simulate",
            &format!(
                "Simulate a circuit and report what came back: analyses, vector names, operating point, the simulator's own .meas results, warnings and errors, plus any measurements you ask for. Fix lint errors first. Use the run id with measure, plot and read_waveform. {MEASURE_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: SimulateInput = input!(input);
        let measures = match parse_measures(&input.measurements) {
            Ok(m) => m,
            Err(e) => return ToolOutput::error(e),
        };
        let p = match self.ws.project() {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let mods = RunMods {
            analysis: input.analysis.clone(),
            extra: Vec::new(),
        };
        let run = match self
            .ws
            .runner
            .run_circuit(
                &p,
                &input.circuit,
                input.simulator.as_deref(),
                &mods,
                &ctx.cancel,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("Simulation failed: {e}")),
        };
        let measured: Vec<MeasureResult> = measures
            .iter()
            .map(|(n, m)| measure(n, m, &run.output.datasets))
            .collect();
        let text = run_text(&run, &measured);
        let out = ToolOutput::text(text)
            .with_data(json!({"kind": "run", "run": run_view(&run, &measured)}));
        if run.output.datasets.is_empty() && !run.output.errors.is_empty() {
            ToolOutput {
                is_error: true,
                ..out
            }
        } else {
            out
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MeasureInput {
    pub circuit: String,
    /// A run id from simulate. Default: the circuit's latest run, simulating
    /// first if there is none.
    #[serde(default)]
    pub run: Option<String>,
    pub measurements: Vec<String>,
}

pub struct MeasureTool {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for MeasureTool {
    fn spec(&self) -> ToolSpec {
        spec::<MeasureInput>(
            "measure",
            &format!(
                "Take measurements on a simulation result without re-running it. {MEASURE_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: MeasureInput = input!(input);
        let measures = match parse_measures(&input.measurements) {
            Ok(m) => m,
            Err(e) => return ToolOutput::error(e),
        };
        let run =
            match fresh_or_given_run(&self.ws, &input.circuit, input.run.as_deref(), ctx).await {
                Ok(r) => r,
                Err(e) => return ToolOutput::error(e),
            };
        let measured: Vec<MeasureResult> = measures
            .iter()
            .map(|(n, m)| measure(n, m, &run.output.datasets))
            .collect();
        let mut t = format!("Measured run {} of {}:\n", run.id, run.circuit);
        for m in &measured {
            t.push_str(&format!("  {}\n", m.display()));
        }
        let data: Vec<Value> = measured.iter().map(|m| json!({"name": m.name, "value": m.value, "unit": m.unit, "display": m.display(), "note": m.note})).collect();
        ToolOutput::text(t).with_data(json!({"kind": "measure", "measurements": data}))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpecsInput {
    pub circuit: String,
    /// Specs, one per line. When omitted, the `<circuit>.specs` file beside
    /// the circuit is used.
    #[serde(default)]
    pub specs: Option<String>,
    /// Save the given specs to `<circuit>.specs` for later checks. Default true.
    #[serde(default = "yes")]
    pub save: bool,
    #[serde(default)]
    pub simulator: Option<String>,
}

fn yes() -> bool {
    true
}

pub struct CheckSpecs {
    pub ws: Arc<Workspace>,
}

fn spec_text(report: &SpecReport) -> String {
    let mut t = format!("{}\n", report.summary);
    for r in &report.rows {
        t.push_str(&format!(
            "  {} {}\n",
            if r.pass { "PASS" } else { "FAIL" },
            r.display
        ));
    }
    t
}

#[async_trait]
impl Tool for CheckSpecs {
    fn spec(&self) -> ToolSpec {
        spec::<SpecsInput>(
            "check_specs",
            &format!(
                "Simulate the circuit and check it against a spec table, reporting pass or fail and the margin for each spec. This is how to prove a design meets its requirements; run it after every change that matters. {SPEC_SYNTAX} {MEASURE_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: SpecsInput = input!(input);
        let (specs, text) = match load_specs(&self.ws, &input.circuit, input.specs.as_deref()) {
            Ok(s) => s,
            Err(e) => return ToolOutput::error(e),
        };
        let p = match self.ws.project() {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        if input.specs.is_some() && input.save {
            save_specs(&self.ws, &p, &input.circuit, &text).await;
        }
        let run = match self
            .ws
            .runner
            .run_circuit(
                &p,
                &input.circuit,
                input.simulator.as_deref(),
                &RunMods::default(),
                &ctx.cancel,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("Simulation failed: {e}")),
        };
        let report = evaluate(&specs, &run.output.datasets);
        let t = format!(
            "{} (run {} on {})\n{}",
            input.circuit,
            run.id,
            run.simulator.name(),
            spec_text(&report)
        );
        ToolOutput::text(t).with_data(json!({"kind": "specs", "report": report}))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlotInput {
    pub circuit: String,
    #[serde(default)]
    pub run: Option<String>,
    /// Expressions to draw, e.g. `V(out)`, `V(out)/V(in)` (a Bode plot for AC
    /// data), `I(R1)`.
    pub traces: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Limit the x axis, e.g. `[0, 0.001]`.
    #[serde(default)]
    pub x_range: Option<[f64; 2]>,
}

pub struct Plot {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for Plot {
    fn spec(&self) -> ToolSpec {
        spec::<PlotInput>(
            "plot",
            "Plot waveforms from a simulation as an image: time-domain traces, or a Bode plot (magnitude in dB and phase) for AC data. Use it to look at behaviour; use measure for numbers.",
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: PlotInput = input!(input);
        let run =
            match fresh_or_given_run(&self.ws, &input.circuit, input.run.as_deref(), ctx).await {
                Ok(r) => r,
                Err(e) => return ToolOutput::error(e),
            };
        let req = PlotRequest {
            traces: input.traces.clone(),
            title: input.title.clone(),
            x_range: input.x_range,
            ..Default::default()
        };
        let svg = match plot_svg(&run.output.datasets, &req) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput::error(format!(
                    "Could not plot: {e}. Vectors in this run: {}",
                    run.output
                        .datasets
                        .iter()
                        .flat_map(|d| d.names())
                        .take(40)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        };
        match aispice_core::render::render_png(&svg, 1.0) {
            Ok(png) => ToolOutput::text(format!(
                "Plot of {} from run {}",
                input.traces.join(", "),
                run.id
            ))
            .with_image(
                "image/png",
                base64::engine::general_purpose::STANDARD.encode(png),
            )
            .with_data(json!({"kind": "plot", "svg": svg})),
            Err(e) => ToolOutput::error(format!("Could not rasterise the plot: {e}")),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaveInput {
    pub circuit: String,
    #[serde(default)]
    pub run: Option<String>,
    pub signals: Vec<String>,
    #[serde(default)]
    pub from: Option<f64>,
    #[serde(default)]
    pub to: Option<f64>,
    /// Number of evenly spaced rows to return (default 40, at most 400).
    #[serde(default)]
    pub points: Option<usize>,
}

pub struct ReadWaveform {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for ReadWaveform {
    fn spec(&self) -> ToolSpec {
        spec::<WaveInput>(
            "read_waveform",
            "Read sampled values of signals from a simulation as a table (evenly spaced over a range). For AC data values are magnitude in dB and phase in degrees.",
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: WaveInput = input!(input);
        let run =
            match fresh_or_given_run(&self.ws, &input.circuit, input.run.as_deref(), ctx).await {
                Ok(r) => r,
                Err(e) => return ToolOutput::error(e),
            };
        let Some(ds) = run.output.datasets.iter().find(|d| {
            input.signals.iter().all(|s| {
                aispice_sim::expr::parse(s)
                    .ok()
                    .and_then(|e| e.eval_all(d).ok())
                    .is_some()
            })
        }) else {
            return ToolOutput::error(format!(
                "No dataset has all of {}. Vectors: {}",
                input.signals.join(", "),
                run.output
                    .datasets
                    .iter()
                    .flat_map(|d| d.names())
                    .take(40)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let Some(axis) = ds.axis_vector() else {
            return ToolOutput::error(
                "this analysis has no independent axis (an operating point); simulate shows its values",
            );
        };
        let x = axis.data.real();
        let n = input.points.unwrap_or(40).clamp(2, 400);
        let lo = input.from.unwrap_or(x.first().copied().unwrap_or(0.0));
        let hi = input.to.unwrap_or(x.last().copied().unwrap_or(0.0));
        let ac = ds.kind == AnalysisKind::Ac;
        let series: Vec<aispice_sim::expr::Series> = input
            .signals
            .iter()
            .filter_map(|s| {
                aispice_sim::expr::parse(s)
                    .ok()
                    .and_then(|e| e.eval_all(ds).ok())
            })
            .collect();
        let mut t = (if axis.quantity == Quantity::Frequency {
            "frequency"
        } else {
            axis.name.as_str()
        })
        .to_string();
        for s in &input.signals {
            t.push_str(&if ac {
                format!("\t{s} dB\t{s} deg")
            } else {
                format!("\t{s}")
            });
        }
        t.push('\n');
        for k in 0..n {
            let target = if ac && lo > 0.0 && hi > 0.0 {
                lo * (hi / lo).powf(k as f64 / (n - 1) as f64)
            } else {
                lo + (hi - lo) * k as f64 / (n - 1) as f64
            };
            let i = x
                .partition_point(|v| *v < target)
                .min(x.len().saturating_sub(1));
            t.push_str(&units::format(x[i]));
            for s in &series {
                match s {
                    aispice_sim::expr::Series::Real(v) => {
                        t.push_str(&format!("\t{}", units::format(v[i])))
                    }
                    aispice_sim::expr::Series::Complex(v) => {
                        t.push_str(&format!("\t{:.3}\t{:.2}", v[i].db(), v[i].phase_deg()))
                    }
                }
            }
            t.push('\n');
        }
        ToolOutput::text(t)
    }
}

/// Run netlist variants in parallel, a few at a time.
async fn run_variants(
    ws: &Workspace,
    circuit: &str,
    variants: Vec<(String, aispice_core::netlist::Netlist)>,
    std_libs: &[String],
    simulator: Option<&str>,
    ctx: &ToolContext,
) -> Vec<(String, Result<Arc<StoredRun>, String>)> {
    let p = match ws.project() {
        Ok(p) => p,
        Err(e) => {
            return variants
                .into_iter()
                .map(|(l, _)| (l, Err(e.to_string())))
                .collect();
        }
    };
    let mut out = Vec::with_capacity(variants.len());
    let total = variants.len();
    for chunk in variants.chunks(6) {
        if ctx.cancel.is_cancelled() {
            break;
        }
        let futs = chunk.iter().map(|(label, n)| {
            let p = p.clone();
            async move {
                (
                    label.clone(),
                    ws.runner
                        .run_netlist_with(&p, circuit, n, std_libs, simulator, &ctx.cancel, false)
                        .await
                        .map_err(|e| e.to_string()),
                )
            }
        });
        out.extend(futures::future::join_all(futs).await);
        ctx.progress(
            format!("{} of {total} runs", out.len()),
            Some(out.len() as f32 / total.max(1) as f32),
        );
    }
    out
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SweepInput {
    pub circuit: String,
    /// Parameters to sweep: an element (`R1`) or a `.param` name, with values
    /// as a list `[1000, 2200, 4700]`, a range `{"kind": "dec", "start": 1000,
    /// "stop": 100000, "points_per_decade": 5}`, or text `"lin 1k 10k 10"`.
    pub params: Vec<SweepParam>,
    pub measurements: Vec<String>,
    #[serde(default)]
    pub analysis: Option<String>,
    #[serde(default)]
    pub simulator: Option<String>,
}

pub struct Sweep {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for Sweep {
    fn spec(&self) -> ToolSpec {
        spec::<SweepInput>(
            "sweep",
            &format!(
                "Sweep component values or .param values (several parameters give every combination, at most 200 runs) and tabulate measurements for each. Works with every simulator. {MEASURE_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: SweepInput = input!(input);
        let measures = match parse_measures(&input.measurements) {
            Ok(m) => m,
            Err(e) => return ToolOutput::error(e),
        };
        let p = match self.ws.project() {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let (mut netlist, std_libs, _) = match self.ws.runner.netlist_for(&p, &input.circuit) {
            Ok(v) => v,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        crate::runner::apply_mods(
            &mut netlist,
            &RunMods {
                analysis: input.analysis.clone(),
                extra: vec![],
            },
        );
        let variants = match sweep::variants_with_limit(&netlist, &input.params, 200) {
            Ok(v) => v,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let results = run_variants(
            &self.ws,
            &input.circuit,
            variants,
            &std_libs,
            input.simulator.as_deref(),
            ctx,
        )
        .await;
        let mut t = format!(
            "variant\t{}\n",
            measures
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join("\t")
        );
        let mut rows = Vec::new();
        for (label, r) in &results {
            match r {
                Ok(run) => {
                    let vals: Vec<MeasureResult> = measures
                        .iter()
                        .map(|(n, m)| measure(n, m, &run.output.datasets))
                        .collect();
                    t.push_str(&format!(
                        "{label}\t{}\n",
                        vals.iter()
                            .map(|v| v.value.map(units::format).unwrap_or_else(|| "n/a".into()))
                            .collect::<Vec<_>>()
                            .join("\t")
                    ));
                    rows.push(json!({"label": label, "values": vals.iter().map(|v| v.value).collect::<Vec<_>>()}));
                }
                Err(e) => t.push_str(&format!("{label}\tfailed: {e}\n")),
            }
        }
        ToolOutput::text(t).with_data(json!({"kind": "generic", "value": {"measurements": measures.iter().map(|(n, _)| n).collect::<Vec<_>>(), "rows": rows}}))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MonteCarloInput {
    pub circuit: String,
    /// Tolerances: target is an element name or a glob such as `R*` or `C*`,
    /// `tol_pct` a percentage, `distribution` uniform (default) or gaussian
    /// (3 sigma = tolerance).
    pub tolerances: Vec<Tolerance>,
    /// Number of runs (default 50, at most 500).
    #[serde(default)]
    pub runs: Option<usize>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Specs to judge each run; default: the `<circuit>.specs` file.
    #[serde(default)]
    pub specs: Option<String>,
    #[serde(default)]
    pub simulator: Option<String>,
}

pub struct MonteCarlo {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for MonteCarlo {
    fn spec(&self) -> ToolSpec {
        spec::<MonteCarloInput>(
            "monte_carlo",
            &format!(
                "Monte Carlo tolerance analysis: vary component values within their tolerances, simulate each sample, and report the yield against the specs with per-spec spread and the worst run. Seeded and reproducible. {SPEC_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: MonteCarloInput = input!(input);
        let (specs, _) = match load_specs(&self.ws, &input.circuit, input.specs.as_deref()) {
            Ok(s) => s,
            Err(e) => return ToolOutput::error(e),
        };
        let p = match self.ws.project() {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let (netlist, std_libs, _) = match self.ws.runner.netlist_for(&p, &input.circuit) {
            Ok(v) => v,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let n = input.runs.unwrap_or(50).clamp(1, 500);
        let variants =
            match montecarlo::variants(&netlist, &input.tolerances, n, input.seed.unwrap_or(1)) {
                Ok(v) => v,
                Err(e) => return ToolOutput::error(e.to_string()),
            };
        let labelled: Vec<(String, aispice_core::netlist::Netlist)> = variants
            .iter()
            .map(|v| (v.label.clone(), v.netlist.clone()))
            .collect();
        let results = run_variants(
            &self.ws,
            &input.circuit,
            labelled,
            &std_libs,
            input.simulator.as_deref(),
            ctx,
        )
        .await;
        let reports: Vec<SpecReport> = results
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok())
            .map(|run| evaluate(&specs, &run.output.datasets))
            .collect();
        let failed_runs = results.len() - reports.len();
        let y = montecarlo::summarize(&reports);
        let mut t = format!("{}\n", y.summary);
        if failed_runs > 0 {
            t.push_str(&format!("  {failed_runs} run(s) failed to simulate\n"));
        }
        for s in &y.specs {
            t.push_str(&format!(
                "  {}\n",
                serde_json::to_string(s).unwrap_or_default()
            ));
        }
        ToolOutput::text(t).with_data(json!({"kind": "montecarlo", "runs": y.runs, "yield_pct": y.yield_pct, "report": y.summary}))
    }
}

#[derive(Debug, Deserialize, JsonSchema, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Fast and reliable for a few parameters.
    #[default]
    NelderMead,
    /// Better for many parameters or rough objectives.
    CmaEs,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OptimizeInput {
    pub circuit: String,
    /// Parameters to size: an element (`R1`) or `.param` name with `min` and
    /// `max` (`"1k"` style values accepted), optional `log_scale` (default true
    /// for R, C, L) and `snap` to an E-series (E6, E12, E24, E48, E96) for the
    /// final answer.
    pub params: Vec<Param>,
    /// Specs to meet; default: the `<circuit>.specs` file.
    #[serde(default)]
    pub specs: Option<String>,
    /// Optionally also minimise or maximise one spec's value once all pass.
    #[serde(default)]
    pub goal: Option<Goal>,
    #[serde(default)]
    pub method: Method,
    /// Simulation budget (default 80, at most 400).
    #[serde(default)]
    pub max_evals: Option<usize>,
    /// Write the best (snapped) values into the schematic as an undoable edit.
    #[serde(default)]
    pub apply: bool,
    /// Save the given specs to `<circuit>.specs`, as the circuit's
    /// requirements from now on. Default true.
    #[serde(default = "yes")]
    pub save_specs: bool,
    #[serde(default)]
    pub simulator: Option<String>,
}

pub struct Optimize {
    pub ws: Arc<Workspace>,
}

#[async_trait]
impl Tool for Optimize {
    fn spec(&self) -> ToolSpec {
        spec::<OptimizeInput>(
            "optimize",
            &format!(
                "Size component values to meet a spec table with the simulator in the loop (Nelder-Mead or CMA-ES over log-scaled ranges), then round to standard E-series values and re-check. You choose the parameters and sensible ranges; the optimizer does the search. {SPEC_SYNTAX}"
            ),
        )
    }

    async fn call(&self, ctx: &ToolContext, input: Value) -> ToolOutput {
        let input: OptimizeInput = input!(input);
        let (specs, spec_source) =
            match load_specs(&self.ws, &input.circuit, input.specs.as_deref()) {
                Ok(s) => s,
                Err(e) => return ToolOutput::error(e),
            };
        let p = match self.ws.project() {
            Ok(p) => p,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        if input.specs.is_some() && input.save_specs {
            save_specs(&self.ws, &p, &input.circuit, &spec_source).await;
        }
        let (netlist, std_libs, _) = match self.ws.runner.netlist_for(&p, &input.circuit) {
            Ok(v) => v,
            Err(e) => return ToolOutput::error(e.to_string()),
        };
        let mut opt: Box<dyn optimize::Optimizer + Send> = match input.method {
            Method::NelderMead => {
                match NelderMead::new(&input.params, NelderMeadOptions::default()) {
                    Ok(o) => Box::new(o),
                    Err(e) => return ToolOutput::error(e.to_string()),
                }
            }
            Method::CmaEs => match CmaEs::new(&input.params, CmaEsOptions::default()) {
                Ok(o) => Box::new(o),
                Err(e) => return ToolOutput::error(e.to_string()),
            },
        };
        let objective = Objective {
            goal: input.goal.clone(),
        };
        let options = RunOptions {
            max_evals: input.max_evals.unwrap_or(80).clamp(4, 400),
            ..Default::default()
        };
        let ws = self.ws.clone();
        let circuit = input.circuit.clone();
        let params = input.params.clone();
        let sim = input.simulator.clone();
        let ctx2 = ctx.clone();
        let (specs_in, libs_in, netlist_in) = (specs.clone(), std_libs.clone(), netlist.clone());
        let handle = tokio::runtime::Handle::current();
        // The optimizer drives a synchronous batch callback; each batch runs
        // its simulations concurrently on the async runtime.
        let result = tokio::task::spawn_blocking(move || {
            optimize::run(opt.as_mut(), &params, &options, |batch: &[Vec<f64>]| {
                let variants: Vec<(String, aispice_core::netlist::Netlist)> = batch
                    .iter()
                    .enumerate()
                    .map(|(i, values)| {
                        (
                            format!("p{i}"),
                            optimize::apply(&netlist_in, &params, values)
                                .unwrap_or_else(|_| netlist_in.clone()),
                        )
                    })
                    .collect();
                let results = handle.block_on(run_variants(
                    &ws,
                    &circuit,
                    variants,
                    &libs_in,
                    sim.as_deref(),
                    &ctx2,
                ));
                results
                    .iter()
                    .map(|(_, r)| match r {
                        Ok(run) => objective.score(&evaluate(&specs_in, &run.output.datasets)),
                        Err(_) => optimize::Evaluation {
                            value: optimize::MISSING_PENALTY,
                            feasible: false,
                        },
                    })
                    .collect()
            })
        })
        .await;
        let result = match result {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("internal error: {e}")),
        };
        let values = result
            .snapped
            .as_ref()
            .map(|s| s.values.clone())
            .unwrap_or_else(|| result.best.clone());
        let feasible = result
            .snapped
            .as_ref()
            .map(|s| s.feasible)
            .unwrap_or(result.feasible);
        let mut best = serde_json::Map::new();
        let mut t = format!(
            "{} after {} simulations ({:?}).\n",
            if feasible {
                "All specs pass"
            } else {
                "Specs not all met"
            },
            result.evaluations,
            result.stop
        );
        for (name, v) in result.names.iter().zip(&values) {
            best.insert(name.clone(), json!(units::format(*v)));
            t.push_str(&format!("  {name} = {}\n", units::format(*v)));
        }
        if input.apply {
            let ops: Vec<EditOp> = result
                .names
                .iter()
                .zip(&values)
                .filter(|(n, _)| !n.starts_with('.'))
                .map(|(n, v)| EditOp::SetValue {
                    name: n.clone(),
                    value: units::format(*v),
                })
                .collect();
            let summary = ops
                .iter()
                .map(|o| match o {
                    EditOp::SetValue { name, value } => format!("{name} = {value}"),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            match p.load(&input.circuit) {
                Ok((mut sch, _)) => match apply(&mut sch, p.library(), &ops) {
                    Ok(_)
                        if !self
                            .ws
                            .approve(
                                &input.circuit,
                                &format!("Apply optimized values: {summary}"),
                                &summary,
                            )
                            .await =>
                    {
                        t.push_str("The user declined applying these values; the schematic is unchanged.\n");
                    }
                    Ok(_) => match p.save(&input.circuit, &sch, "Optimized values") {
                        Ok(_) => {
                            if let Ok(path) = p.resolve(&input.circuit) {
                                self.ws.after_save(&path);
                            }
                            t.push_str("Applied to the schematic (undo is available).\n");
                        }
                        Err(e) => t.push_str(&format!("Could not save: {e}\n")),
                    },
                    Err(e) => t.push_str(&format!(
                        "Could not apply: {e} (.param values must be edited with edit_schematic)\n"
                    )),
                },
                Err(e) => t.push_str(&format!("Could not load: {e}\n")),
            }
        }
        // Re-check the values actually reported (snapped) with a fresh run.
        let final_report = {
            match optimize::apply(&netlist, &input.params, &values) {
                Ok(n) => match self
                    .ws
                    .runner
                    .run_netlist_with(
                        &p,
                        &input.circuit,
                        &n,
                        &std_libs,
                        input.simulator.as_deref(),
                        &ctx.cancel,
                        false,
                    )
                    .await
                {
                    Ok(run) => Some(evaluate(&specs, &run.output.datasets)),
                    Err(_) => None,
                },
                Err(_) => None,
            }
        };
        if let Some(r) = &final_report {
            t.push_str(&spec_text(r));
        }
        ToolOutput::text(t).with_data(json!({"kind": "optimize", "best": best, "evaluations": result.evaluations, "report": final_report}))
    }
}
