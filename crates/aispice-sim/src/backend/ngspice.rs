//! ngspice in batch mode.
//!
//! `ngspice -b -r deck.raw deck.cir`, with HOME pointed at the private run
//! directory and aispice's own `.spiceinit` there: ngspice executes the
//! `.spiceinit` it finds in HOME and in the working directory, and those are
//! the only two places it looks, so neither the user's nor a project's file
//! can run. The one we write turns on LTspice compatibility
//! (`ngbehavior=ltpsa`), which aispice's netlists rely on.
//!
//! ngspice refuses to evaluate `.meas` in batch mode when it writes a raw
//! file, so a deck with measurements runs twice: once for the raw file and
//! once, without it, for the measurements. ngspice runs take milliseconds;
//! the job's timeout covers both.

use super::process::{self, ProcessSpec};
use super::{
    Detection, SimError, SimId, SimJob, SimOutput, Simulator, Workspace, finish, read_raw_limited,
    which,
};
use crate::log::parse_ngspice_output;
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct Ngspice {
    /// The executable; found on PATH when unset.
    pub exe: Option<PathBuf>,
    /// The `ngbehavior` compatibility setting. `ltpsa` matches LTspice;
    /// `None` runs plain ngspice.
    pub compat: Option<String>,
}

impl Default for Ngspice {
    fn default() -> Self {
        Self {
            exe: None,
            compat: Some("ltpsa".into()),
        }
    }
}

impl Ngspice {
    fn exe(&self) -> Option<PathBuf> {
        match &self.exe {
            Some(p) => p.is_file().then(|| p.clone()),
            None => which("ngspice"),
        }
    }

    /// The `.spiceinit` aispice runs ngspice with.
    pub fn spiceinit(&self) -> String {
        let mut s = String::from("* written by aispice for this run\n");
        if let Some(c) = &self.compat {
            s.push_str(&format!("set ngbehavior={c}\n"));
        }
        s
    }

    /// The argument vector for a deck named `stem`.
    pub fn args(stem: &str) -> Vec<String> {
        vec![
            "-b".into(),
            "-r".into(),
            format!("{stem}.raw"),
            format!("{stem}.cir"),
        ]
    }

    /// The second run that evaluates `.meas` lines.
    pub fn meas_args(stem: &str) -> Vec<String> {
        vec!["-b".into(), format!("{stem}_meas.cir")]
    }
}

/// The deck for the measurement run. In batch mode ngspice keeps only the
/// vectors it can read out of the `.meas` lines, and it cannot read
/// `vm(out)` or `vdb(out)`, so an AC run measuring those would save nothing
/// and be skipped; `.save all` keeps every node.
fn measurement_deck(deck: &str) -> String {
    let lines: Vec<&str> = deck.lines().collect();
    let end = lines
        .iter()
        .rposition(|l| l.trim().eq_ignore_ascii_case(".end"))
        .unwrap_or(lines.len());
    let mut out: Vec<&str> = lines[..end].to_vec();
    out.push(".save all");
    out.extend_from_slice(&lines[end..]);
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Whether the deck has `.meas` or `.measure` lines.
fn has_meas(deck: &str) -> bool {
    deck.lines().any(|l| {
        let w = l.split_whitespace().next().unwrap_or("");
        w.eq_ignore_ascii_case(".meas") || w.eq_ignore_ascii_case(".measure")
    })
}

#[async_trait::async_trait]
impl Simulator for Ngspice {
    fn id(&self) -> SimId {
        SimId::Ngspice
    }

    async fn detect(&self) -> Detection {
        let Some(exe) = self.exe() else {
            return Detection {
                notes: vec!["ngspice is not on PATH".into()],
                ..Default::default()
            };
        };
        let mut d = Detection {
            found: true,
            path: Some(exe.clone()),
            ..Default::default()
        };
        if let Ok(ws) = Workspace::new() {
            let spec = ProcessSpec::new(&exe, ws.path())
                .arg("--version")
                .env("HOME", ws.path())
                .env("USERPROFILE", ws.path());
            if let Ok(out) =
                process::run(&spec, Duration::from_secs(10), &CancellationToken::new()).await
            {
                d.version = out.combined().lines().find_map(|l| {
                    let i = l.find("ngspice-")?;
                    l[i + 8..].split_whitespace().next().map(str::to_string)
                });
            }
        }
        if let Some(c) = &self.compat {
            d.notes.push(format!("runs with ngbehavior={c}"));
        }
        d
    }

    async fn run(&self, job: &SimJob, cancel: &CancellationToken) -> Result<SimOutput, SimError> {
        let exe = self
            .exe()
            .ok_or_else(|| SimError::NotFound("ngspice".into()))?;
        let stem = job.stem();
        let ws = Workspace::new()?;
        ws.write(&format!("{stem}.cir"), job.netlist_text.as_bytes())?;
        ws.write(".spiceinit", self.spiceinit().as_bytes())?;
        let mut spec = ProcessSpec::new(&exe, ws.path())
            .env("HOME", ws.path())
            .env("USERPROFILE", ws.path());
        for a in Self::args(&stem) {
            spec = spec.arg(a);
        }
        let started = std::time::Instant::now();
        let out = process::run(&spec, job.timeout, cancel).await?;
        let mut log = out.combined();
        let mut report = parse_ngspice_output(&log);
        if has_meas(&job.netlist_text) && report.errors.is_empty() {
            // ngspice will not evaluate .meas in batch mode when writing a
            // raw file, so the measurements come from a second run without
            // one, within the same deadline.
            let left = job.timeout.saturating_sub(started.elapsed());
            ws.write(
                &format!("{stem}_meas.cir"),
                measurement_deck(&job.netlist_text).as_bytes(),
            )?;
            let mut spec2 = ProcessSpec::new(&exe, ws.path())
                .env("HOME", ws.path())
                .env("USERPROFILE", ws.path());
            for a in Self::meas_args(&stem) {
                spec2 = spec2.arg(a);
            }
            let m = process::run(&spec2, left, cancel).await?;
            let mlog = m.combined();
            let mreport = parse_ngspice_output(&mlog);
            report.measurements = mreport.measurements;
            for w in mreport.warnings {
                if !report.warnings.contains(&w) {
                    report.warnings.push(w);
                }
            }
            log.push_str("\n[aispice: measurement run]\n");
            log.push_str(&mlog);
        }
        let raw = ws.file(&format!("{stem}.raw"));
        let datasets = if raw.exists() {
            read_raw_limited(&raw)?
        } else {
            Vec::new()
        };
        let files = ws.keep(
            &job.run_dir,
            &[format!("{stem}.cir"), format!("{stem}.raw")],
        )?;
        let raw_path = files
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "raw"))
            .cloned();
        let result = SimOutput {
            datasets,
            log,
            measurements: report.measurements,
            errors: report.errors,
            warnings: report.warnings,
            duration: out.elapsed,
            raw_path,
            files,
        };
        finish(result, out.success(), "ngspice wrote no results")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_spiceinit() {
        assert_eq!(Ngspice::args("rc"), vec!["-b", "-r", "rc.raw", "rc.cir"]);
        assert_eq!(Ngspice::meas_args("rc"), vec!["-b", "rc_meas.cir"]);
        assert_eq!(
            measurement_deck("t\nR1 a 0 1\n.meas ac x FIND vm(a) AT=1k\n.end\n"),
            "t\nR1 a 0 1\n.meas ac x FIND vm(a) AT=1k\n.save all\n.end\n"
        );
        assert_eq!(
            measurement_deck("t\nR1 a 0 1\n"),
            "t\nR1 a 0 1\n.save all\n"
        );
        assert!(has_meas("t\nR1 a 0 1\n  .MEAS tran x MAX V(a)\n"));
        assert!(!has_meas("t\nR1 a 0 1\n* .meas in a comment\n"));
        assert!(
            Ngspice::default()
                .spiceinit()
                .contains("set ngbehavior=ltpsa\n")
        );
        let plain = Ngspice {
            compat: None,
            ..Default::default()
        };
        assert!(!plain.spiceinit().contains("ngbehavior"));
    }
}
