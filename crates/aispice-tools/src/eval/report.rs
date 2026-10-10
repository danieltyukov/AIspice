//! Results of a suite run: one record per task run, a summary per model, and
//! the text tables the CLI prints.

use super::judge::{ToolRecord, Verdict};
use super::task::Difficulty;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One model working on one task once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRun {
    pub task: String,
    pub difficulty: Difficulty,
    pub provider: String,
    pub model: String,
    /// Which repetition, from 1.
    pub attempt: u32,
    pub pass: bool,
    /// `pass`, or the failures joined.
    pub reason: String,
    /// Everything that failed: the run itself (step budget, provider error)
    /// first, then the judge's findings.
    pub failures: Vec<String>,
    /// Model calls made.
    pub steps: u32,
    pub max_steps: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    /// Calls per tool.
    pub tools: BTreeMap<String, u32>,
    /// Every tool call in order: input, whether it failed, and the start of
    /// what it returned.
    #[serde(default)]
    pub calls: Vec<ToolRecord>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Time the agent ran.
    pub agent_ms: u64,
    /// Time the judge took (one simulation, lint, constraints).
    pub judge_ms: u64,
    pub stop_reason: String,
    /// A failure of the run rather than of the result: a provider error, a
    /// timeout, a project that could not be set up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Not run, because the provider kept failing.
    #[serde(default)]
    pub skipped: bool,
    pub verdict: Verdict,
    /// The model's last message.
    pub answer: String,
}

impl TaskRun {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSummary {
    pub provider: String,
    pub model: String,
    /// Runs attempted (skipped runs are not counted).
    pub runs: usize,
    pub passed: usize,
    /// Runs that ended in a provider error or timeout.
    pub errors: usize,
    pub skipped: usize,
    /// passed / runs, 0 when nothing ran.
    pub pass_rate: f64,
    pub median_steps: f64,
    pub median_input_tokens: f64,
    pub median_output_tokens: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub median_seconds: f64,
    pub total_seconds: f64,
}

impl ModelSummary {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub aispice_version: String,
    pub suite: String,
    /// Unix milliseconds.
    pub started: u64,
    pub simulator: String,
    pub repeat: u32,
    pub models: Vec<ModelSummary>,
    pub runs: Vec<TaskRun>,
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Summaries in the order the models first appear in `runs`.
pub fn summarize(runs: &[TaskRun]) -> Vec<ModelSummary> {
    let mut order: Vec<(String, String)> = Vec::new();
    for r in runs {
        let key = (r.provider.clone(), r.model.clone());
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
        .into_iter()
        .map(|(provider, model)| {
            let mine: Vec<&TaskRun> = runs
                .iter()
                .filter(|r| r.provider == provider && r.model == model)
                .collect();
            let done: Vec<&&TaskRun> = mine.iter().filter(|r| !r.skipped).collect();
            let passed = done.iter().filter(|r| r.pass).count();
            ModelSummary {
                provider,
                model,
                runs: done.len(),
                passed,
                errors: done.iter().filter(|r| r.error.is_some()).count(),
                skipped: mine.len() - done.len(),
                pass_rate: if done.is_empty() {
                    0.0
                } else {
                    passed as f64 / done.len() as f64
                },
                median_steps: median(done.iter().map(|r| r.steps as f64).collect()),
                median_input_tokens: median(
                    done.iter()
                        .map(|r| {
                            (r.input_tokens + r.cache_read_tokens + r.cache_write_tokens) as f64
                        })
                        .collect(),
                ),
                median_output_tokens: median(done.iter().map(|r| r.output_tokens as f64).collect()),
                total_input_tokens: done
                    .iter()
                    .map(|r| r.input_tokens + r.cache_read_tokens + r.cache_write_tokens)
                    .sum(),
                total_output_tokens: done.iter().map(|r| r.output_tokens).sum(),
                median_seconds: median(done.iter().map(|r| r.agent_ms as f64 / 1000.0).collect()),
                total_seconds: done.iter().map(|r| r.agent_ms as f64 / 1000.0).sum(),
            }
        })
        .collect()
}

fn tokens(n: f64) -> String {
    if n >= 1e6 {
        format!("{:.1}M", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.1}k", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

fn seconds(s: f64) -> String {
    if s >= 120.0 {
        format!("{:.1} min", s / 60.0)
    } else {
        format!("{s:.0} s")
    }
}

/// Lay out rows as left-aligned columns two spaces apart.
fn table(rows: &[Vec<String>], indent: &str) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for r in rows {
        let mut line = String::from(indent);
        for (c, cell) in r.iter().enumerate() {
            if c + 1 == r.len() {
                line.push_str(cell);
            } else {
                line.push_str(&format!("{cell:<w$}  ", w = widths[c]));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

impl Report {
    /// Summaries of models whose pass rate is below `min`.
    pub fn below(&self, min: f64) -> Vec<&ModelSummary> {
        self.models.iter().filter(|m| m.pass_rate < min).collect()
    }

    /// The tables: per-task results for each model, then one line per model.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for m in &self.models {
            out.push_str(&format!("{}\n", m.label()));
            let mut rows = vec![
                [
                    "task",
                    "level",
                    "result",
                    "steps",
                    "tools",
                    "tokens in/out",
                    "time",
                    "reason",
                ]
                .map(String::from)
                .to_vec(),
            ];
            for r in self
                .runs
                .iter()
                .filter(|r| r.provider == m.provider && r.model == m.model)
            {
                let name = if self.repeat > 1 {
                    format!("{} #{}", r.task, r.attempt)
                } else {
                    r.task.clone()
                };
                let result = if r.skipped {
                    "skip"
                } else if r.pass {
                    "pass"
                } else if r.error.is_some() {
                    "ERROR"
                } else {
                    "FAIL"
                };
                rows.push(vec![
                    name,
                    r.difficulty.as_str().into(),
                    result.into(),
                    format!("{}/{}", r.steps, r.max_steps),
                    r.tool_calls.to_string(),
                    format!(
                        "{}/{}",
                        tokens(
                            (r.input_tokens + r.cache_read_tokens + r.cache_write_tokens) as f64
                        ),
                        tokens(r.output_tokens as f64)
                    ),
                    seconds(r.agent_ms as f64 / 1000.0),
                    if r.pass {
                        String::new()
                    } else {
                        r.reason.clone()
                    },
                ]);
            }
            out.push_str(&table(&rows, "  "));
            out.push('\n');
        }
        let mut rows = vec![
            [
                "model",
                "pass rate",
                "median steps",
                "median tokens in/out",
                "median time",
                "total tokens in/out",
                "errors",
            ]
            .map(String::from)
            .to_vec(),
        ];
        for m in &self.models {
            rows.push(vec![
                m.label(),
                format!("{}/{} ({:.0}%)", m.passed, m.runs, m.pass_rate * 100.0),
                format!("{}", m.median_steps),
                format!(
                    "{}/{}",
                    tokens(m.median_input_tokens),
                    tokens(m.median_output_tokens)
                ),
                seconds(m.median_seconds),
                format!(
                    "{}/{}",
                    tokens(m.total_input_tokens as f64),
                    tokens(m.total_output_tokens as f64)
                ),
                if m.skipped > 0 {
                    format!("{} ({} skipped)", m.errors, m.skipped)
                } else {
                    m.errors.to_string()
                },
            ]);
        }
        out.push_str(&table(&rows, ""));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(model: &str, task: &str, pass: bool, steps: u32, ms: u64) -> TaskRun {
        TaskRun {
            task: task.into(),
            difficulty: Difficulty::Easy,
            provider: "p".into(),
            model: model.into(),
            attempt: 1,
            pass,
            reason: if pass {
                "pass".into()
            } else {
                "spec fc: 1kHz FAIL".into()
            },
            failures: Vec::new(),
            steps,
            max_steps: 10,
            tool_calls: steps,
            tool_errors: 0,
            tools: BTreeMap::new(),
            calls: Vec::new(),
            input_tokens: 1000 * steps as u64,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            agent_ms: ms,
            judge_ms: 10,
            stop_reason: "end_turn".into(),
            error: None,
            skipped: false,
            verdict: Verdict::default(),
            answer: String::new(),
        }
    }

    #[test]
    fn summaries_and_tables() {
        let runs = vec![
            run("a", "t1", true, 3, 1000),
            run("a", "t2", false, 5, 3000),
            run("a", "t3", true, 4, 2000),
            run("b", "t1", false, 9, 9000),
        ];
        let s = summarize(&runs);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].runs, s[0].passed), (3, 2));
        assert!((s[0].pass_rate - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(s[0].median_steps, 4.0);
        assert_eq!(s[0].median_seconds, 2.0);
        assert_eq!(s[0].total_input_tokens, 12_000);
        let report = Report {
            aispice_version: "0".into(),
            suite: "evals".into(),
            started: 0,
            simulator: "ngspice".into(),
            repeat: 1,
            models: s,
            runs,
        };
        let text = report.to_text();
        assert!(text.contains("p/a\n"), "{text}");
        assert!(text.contains("spec fc: 1kHz FAIL"), "{text}");
        assert!(text.contains("2/3 (67%)"), "{text}");
        assert_eq!(report.below(0.5).len(), 1);
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
    }
}
