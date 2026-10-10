//! Running a circuit: choosing a simulator, translating the netlist,
//! resolving models, applying the safety policy, and keeping the results.
//!
//! The policy check is not optional and not up to the caller: every deck
//! that reaches a simulator goes through [`Runner::run_netlist`], which
//! refuses anything the policy refuses.

use crate::project::{Project, ProjectError, now_ms};
use aispice_core::netlist::{self, Netlist, Policy};
use aispice_sim::backend::ltspice::Ltspice;
use aispice_sim::backend::spectre::{Spectre, SpectreConfig};
use aispice_sim::backend::{self, Detection, SimId, SimJob, SimOutput, Simulator};
use aispice_sim::dialect::{self, Target};
use aispice_sim::models;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("{0}")]
    Netlist(String),
    #[error("refused for safety:\n{0}")]
    Policy(String),
    #[error(
        "no simulator is available: install ngspice (recommended), LTspice or Xyce, or configure Spectre"
    )]
    NoSimulator,
    #[error("{0} is not available on this machine")]
    Unavailable(String),
    #[error("{0}")]
    Sim(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    /// `auto`, `ngspice`, `xyce`, `ltspice` or `spectre`.
    pub simulator: String,
    pub ltspice_exe: Option<PathBuf>,
    pub spectre: Option<SpectreConfig>,
    pub allow_control_blocks: bool,
    pub timeout_secs: u64,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            simulator: "auto".into(),
            ltspice_exe: None,
            spectre: None,
            allow_control_blocks: false,
            timeout_secs: 300,
        }
    }
}

/// A finished run, kept so measurements, plots and specs can reuse it.
#[derive(Debug)]
pub struct StoredRun {
    pub id: String,
    pub circuit: String,
    pub simulator: SimId,
    pub output: SimOutput,
    /// The deck exactly as the simulator received it.
    pub deck: String,
    /// Translation and model notes worth telling the user.
    pub notes: Vec<String>,
    pub time: u64,
}

/// What to change for one run without touching the file.
#[derive(Debug, Clone, Default)]
pub struct RunMods {
    /// Replace every analysis directive with this one (`.ac dec 50 1 1Meg`).
    pub analysis: Option<String>,
    /// Directives added at the end (`.meas ...`, `.options plotwinsize=0`).
    pub extra: Vec<String>,
}

const KEEP_RUNS: usize = 32;

pub struct Runner {
    cfg: RwLock<RunnerConfig>,
    detected: OnceCell<Vec<(SimId, Detection)>>,
    runs: Mutex<VecDeque<Arc<StoredRun>>>,
    counter: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner").finish_non_exhaustive()
    }
}

impl Default for Runner {
    fn default() -> Self {
        Self::new(RunnerConfig::default())
    }
}

impl Runner {
    pub fn new(cfg: RunnerConfig) -> Self {
        Self {
            cfg: RwLock::new(cfg),
            detected: OnceCell::new(),
            runs: Mutex::new(VecDeque::new()),
            counter: Default::default(),
        }
    }

    pub fn config(&self) -> RunnerConfig {
        self.cfg.read().expect("runner config").clone()
    }

    pub fn set_config(&self, cfg: RunnerConfig) {
        *self.cfg.write().expect("runner config") = cfg;
    }

    fn ltspice(&self) -> Ltspice {
        Ltspice {
            exe: self.config().ltspice_exe,
            ..Default::default()
        }
    }

    /// LTspice's library folders (with sym, sub and cmp inside), if installed.
    pub fn ltspice_lib_dirs(&self) -> Vec<PathBuf> {
        self.ltspice()
            .lib_dirs()
            .into_iter()
            .filter(|d| d.is_dir())
            .collect()
    }

    /// The first LTspice symbol folder, for the symbol library.
    pub fn ltspice_symbols(&self) -> Option<PathBuf> {
        self.ltspice_lib_dirs()
            .into_iter()
            .map(|d| d.join("sym"))
            .find(|d| d.is_dir())
    }

    fn simulator(&self, id: SimId) -> Box<dyn Simulator> {
        match id {
            SimId::Ltspice => Box::new(self.ltspice()),
            SimId::Spectre => Box::new(Spectre {
                config: self.config().spectre,
            }),
            other => backend::simulator(other),
        }
    }

    /// Which simulators are installed. Detected once per process.
    pub async fn detect(&self) -> Vec<(SimId, Detection)> {
        self.detected
            .get_or_init(|| async {
                let mut out = Vec::new();
                for id in SimId::ALL {
                    out.push((id, self.simulator(id).detect().await));
                }
                out
            })
            .await
            .clone()
    }

    async fn available(&self, id: SimId) -> bool {
        self.detect().await.iter().any(|(i, d)| *i == id && d.found)
    }

    /// Pick a simulator. Automatic choice prefers ngspice (fast, and LTspice
    /// compatible in its `ltpsa` mode), then LTspice, then Xyce. Spectre is
    /// used only when asked for by name.
    pub async fn choose(&self, requested: Option<&str>) -> Result<SimId, RunError> {
        let wanted = requested
            .map(str::to_string)
            .unwrap_or_else(|| self.config().simulator);
        let id = match wanted.to_ascii_lowercase().as_str() {
            "ngspice" => Some(SimId::Ngspice),
            "xyce" => Some(SimId::Xyce),
            "ltspice" => Some(SimId::Ltspice),
            "spectre" => Some(SimId::Spectre),
            _ => None,
        };
        if let Some(id) = id {
            return if self.available(id).await {
                Ok(id)
            } else {
                Err(RunError::Unavailable(id.name().into()))
            };
        }
        for id in [SimId::Ngspice, SimId::Ltspice, SimId::Xyce] {
            if self.available(id).await {
                return Ok(id);
            }
        }
        Err(RunError::NoSimulator)
    }

    /// The netlist for a circuit file, plus the LTspice standard libraries it
    /// relies on.
    pub fn netlist_for(
        &self,
        project: &Project,
        circuit: &str,
    ) -> Result<(Netlist, Vec<String>, Vec<String>), RunError> {
        if circuit.to_ascii_lowercase().ends_with(".asc") {
            let (sch, _) = project.load(circuit)?;
            let (built, _) = netlist::build(&sch, project.library(), &format!("* {circuit}"));
            Ok((built.netlist, built.standard_libs, built.warnings))
        } else {
            let text = project.read_text(circuit)?;
            Ok((netlist::parse(&text), Vec::new(), Vec::new()))
        }
    }

    pub async fn run_circuit(
        &self,
        project: &Project,
        circuit: &str,
        simulator: Option<&str>,
        mods: &RunMods,
        cancel: &CancellationToken,
    ) -> Result<Arc<StoredRun>, RunError> {
        let (mut netlist, std_libs, warnings) = self.netlist_for(project, circuit)?;
        apply_mods(&mut netlist, mods);
        let run = self
            .run_netlist(project, circuit, &netlist, &std_libs, simulator, cancel)
            .await?;
        if warnings.is_empty() {
            return Ok(run);
        }
        // Netlisting warnings belong with the run's notes.
        let mut notes = warnings;
        notes.extend(run.notes.iter().cloned());
        let run = Arc::new(StoredRun {
            notes,
            ..Arc::try_unwrap(run).unwrap_or_else(|a| clone_run(&a))
        });
        self.remember(run.clone());
        Ok(run)
    }

    /// Translate, resolve models, check the policy and run.
    pub async fn run_netlist(
        &self,
        project: &Project,
        circuit: &str,
        netlist: &Netlist,
        std_libs: &[String],
        simulator: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Arc<StoredRun>, RunError> {
        self.run_netlist_with(project, circuit, netlist, std_libs, simulator, cancel, true)
            .await
    }

    /// As [`run_netlist`](Self::run_netlist); with `remember` false the run is
    /// not kept and its folder is removed afterwards (sweeps, Monte Carlo and
    /// optimization make hundreds of runs).
    #[allow(clippy::too_many_arguments)]
    pub async fn run_netlist_with(
        &self,
        project: &Project,
        circuit: &str,
        netlist: &Netlist,
        std_libs: &[String],
        simulator: Option<&str>,
        cancel: &CancellationToken,
        remember: bool,
    ) -> Result<Arc<StoredRun>, RunError> {
        let mut id = self.choose(simulator).await?;
        let lib_dirs = self.ltspice_lib_dirs();
        let mut notes = Vec::new();
        let mut text = self.deck_for(id, netlist, std_libs, &lib_dirs, &mut notes);
        if let Some(t) = &text.1 {
            // ngspice or Xyce cannot run this deck; LTspice can, if present.
            if simulator.is_none() && id != SimId::Ltspice && self.available(SimId::Ltspice).await {
                notes.push(format!(
                    "{} cannot run this circuit ({t}); used LTspice instead",
                    id.name()
                ));
                id = SimId::Ltspice;
                notes.clear();
                text = self.deck_for(id, netlist, std_libs, &lib_dirs, &mut notes);
            } else {
                return Err(RunError::Netlist(format!(
                    "{} cannot run this circuit: {t}",
                    id.name()
                )));
            }
        }
        let deck = text.0;

        let circuit_path = project.resolve(circuit)?;
        let base = circuit_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project.root().to_path_buf());
        let mut allowed = project.allowed_dirs();
        allowed.extend(lib_dirs.iter().cloned());
        let policy = Policy {
            base_dir: base.clone(),
            allowed_dirs: allowed,
            allow_control_blocks: self.config().allow_control_blocks,
        };
        let violations = netlist::check_policy(&deck, &policy);
        if !violations.is_empty() {
            let msg = violations
                .iter()
                .map(|v| format!("  {} ({})", v.reason, v.line.trim()))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(RunError::Policy(msg));
        }
        let deck = absolutize_includes(&deck, &base);
        // Check again after the rewrite, so the deck the simulator receives is
        // exactly a deck the policy accepted.
        let violations = netlist::check_policy(&deck, &policy);
        if !violations.is_empty() {
            let msg = violations
                .iter()
                .map(|v| format!("  {} ({})", v.reason, v.line.trim()))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(RunError::Policy(msg));
        }

        let run_id = format!(
            "{}-{}",
            now_ms(),
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let run_dir = project.run_dir(&run_id)?;
        let stem = Path::new(circuit)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "deck".into());
        let mut job = SimJob::new(deck.clone(), run_dir);
        job.deck_name = stem;
        job.timeout = Duration::from_secs(self.config().timeout_secs);
        let run_dir = job.run_dir.clone();
        let result = self
            .simulator(id)
            .run(&job, cancel)
            .await
            .map_err(|e| RunError::Sim(e.to_string()));
        if !remember {
            let _ = std::fs::remove_dir_all(&run_dir);
        }
        let output = result?;
        let run = Arc::new(StoredRun {
            id: run_id,
            circuit: circuit.to_string(),
            simulator: id,
            output,
            deck,
            notes,
            time: now_ms(),
        });
        if remember {
            self.remember(run.clone());
        }
        Ok(run)
    }

    /// The deck text for a simulator, or the reason it cannot run there.
    fn deck_for(
        &self,
        id: SimId,
        netlist: &Netlist,
        std_libs: &[String],
        lib_dirs: &[PathBuf],
        notes: &mut Vec<String>,
    ) -> (String, Option<String>) {
        let target = id.target();
        let source = if target == Target::Ltspice {
            netlist.clone()
        } else {
            let resolved = models::resolve_with(netlist, std_libs, lib_dirs);
            for a in &resolved.added {
                notes.push(format!("model {} from {:?}", a.name, a.source));
            }
            if !resolved.missing.is_empty() {
                notes.push(format!(
                    "no definition found for: {}",
                    resolved.missing.join(", ")
                ));
            }
            notes.extend(resolved.notes);
            resolved.netlist
        };
        let t = dialect::translate(&source, target);
        notes.extend(t.notes.iter().cloned());
        let blocked = (!t.unsupported.is_empty()).then(|| t.unsupported.join("; "));
        (t.text, blocked)
    }

    fn remember(&self, run: Arc<StoredRun>) {
        let mut runs = self.runs.lock().expect("runs lock");
        runs.retain(|r| r.id != run.id);
        runs.push_back(run);
        while runs.len() > KEEP_RUNS {
            runs.pop_front();
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<StoredRun>> {
        self.runs
            .lock()
            .expect("runs lock")
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// The most recent run of a circuit.
    pub fn latest(&self, circuit: &str) -> Option<Arc<StoredRun>> {
        self.runs
            .lock()
            .expect("runs lock")
            .iter()
            .rev()
            .find(|r| r.circuit == circuit)
            .cloned()
    }
}

fn clone_run(r: &StoredRun) -> StoredRun {
    StoredRun {
        id: r.id.clone(),
        circuit: r.circuit.clone(),
        simulator: r.simulator,
        output: r.output.clone(),
        deck: r.deck.clone(),
        notes: r.notes.clone(),
        time: r.time,
    }
}

/// Apply run-only changes to a netlist.
pub fn apply_mods(netlist: &mut Netlist, mods: &RunMods) {
    use aispice_core::netlist::Line;
    if let Some(analysis) = &mods.analysis {
        netlist.items.retain(
            |l| !matches!(l, Line::Directive { text } if netlist::spice::is_analysis(text)),
        );
        netlist.items.push(Line::Directive {
            text: analysis.trim().to_string(),
        });
    }
    for d in &mods.extra {
        netlist.items.push(Line::Directive {
            text: d.trim().to_string(),
        });
    }
}

/// The simulator runs in a private temporary folder, so relative includes
/// that point at files beside the circuit are made absolute. Only files that
/// exist (and so already passed the policy) are rewritten; bare library names
/// are left for the simulator's own search path.
pub fn absolutize_includes(deck: &str, base: &Path) -> String {
    let mut out = String::with_capacity(deck.len());
    for line in deck.lines() {
        let trimmed = line.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        let kw = lower.split_whitespace().next().unwrap_or("");
        if matches!(kw, ".include" | ".inc" | ".lib") {
            let rest = trimmed[kw.len()..].trim();
            let (arg, tail) = if let Some(q) = rest.strip_prefix('"') {
                let end = q.find('"').unwrap_or(q.len());
                (&q[..end], q.get(end + 1..).unwrap_or(""))
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (&rest[..end], &rest[end..])
            };
            let p = Path::new(arg);
            if !arg.is_empty() && !p.is_absolute() {
                let candidate = base.join(p);
                if let Ok(real) = std::fs::canonicalize(&candidate) {
                    out.push_str(&format!(
                        "{} \"{}\"{}\n",
                        &trimmed[..kw.len()],
                        real.display(),
                        tail
                    ));
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_beside_the_circuit_become_absolute() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("m.lib"), "* x\n").unwrap();
        let deck = "t\n.lib m.lib\n.lib opamp.sub\n.include \"m.lib\" extra\n";
        let out = absolutize_includes(deck, dir.path());
        let real = std::fs::canonicalize(dir.path().join("m.lib")).unwrap();
        assert!(
            out.contains(&format!(".lib \"{}\"", real.display())),
            "{out}"
        );
        assert!(out.contains(".lib opamp.sub"));
        assert!(
            out.contains(&format!(".include \"{}\" extra", real.display())),
            "{out}"
        );
    }

    #[test]
    fn analysis_override_replaces_existing() {
        let mut n = netlist::parse("t\nR1 a 0 1\n.tran 1m\n.op\n");
        apply_mods(
            &mut n,
            &RunMods {
                analysis: Some(".ac dec 10 1 1k".into()),
                extra: vec![".meas ac x max V(a)".into()],
            },
        );
        let d: Vec<&str> = n.directives().collect();
        assert_eq!(d, vec![".ac dec 10 1 1k", ".meas ac x max V(a)"]);
    }
}
