//! Waveform data for the UI's plots, thinned to a point budget.

use aispice_sim::dataset::{AnalysisKind, Dataset, Quantity};
use aispice_sim::expr::{self, Series};
use serde_json::{Value, json};

/// The `WaveData` shape in `app/src/ipc/types.ts`.
pub fn wave_data(ds: &Dataset, signals: &[String], max_points: usize) -> Result<Value, String> {
    let axis = ds
        .axis_vector()
        .ok_or("this analysis has no independent axis")?;
    let x_all = axis.data.real();
    let ac = ds.kind == AnalysisKind::Ac;
    let mut x_out: Option<Vec<f64>> = None;
    let mut series = Vec::new();
    for step in &ds.steps {
        let range = step.range.clone();
        let n = range.len();
        let stride = n.div_ceil(max_points.max(2)).max(1);
        let idx: Vec<usize> = range.clone().step_by(stride).collect();
        let xs: Vec<f64> = idx.iter().map(|&i| x_all[i].abs()).collect();
        if x_out.is_none() {
            x_out = Some(xs);
        }
        for s in signals {
            let parsed = expr::parse(s).map_err(|e| e.to_string())?;
            let values = parsed.eval(ds, range.clone()).map_err(|e| e.to_string())?;
            let base = range.start;
            match values {
                Series::Real(v) => series.push(json!({"name": s, "step": step.label, "y": idx.iter().map(|&i| v[i - base]).collect::<Vec<_>>()})),
                Series::Complex(v) => {
                    let mut ph: Vec<f64> = v.iter().map(|c| c.phase_deg()).collect();
                    expr::unwrap_degrees(&mut ph);
                    series.push(json!({
                        "name": s,
                        "step": step.label,
                        "y": idx.iter().map(|&i| v[i - base].db()).collect::<Vec<_>>(),
                        "phase": idx.iter().map(|&i| ph[i - base]).collect::<Vec<_>>(),
                    }))
                }
            }
        }
    }
    Ok(json!({
        "kind": ds.kind,
        "x_name": if axis.quantity == Quantity::Frequency { "frequency" } else { axis.name.as_str() },
        "x_unit": axis.quantity.unit(),
        "log_x": ac,
        "x": x_out.unwrap_or_default(),
        "series": series,
    }))
}
