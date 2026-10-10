//! The design task suite behind `aispice eval`: tasks, a runner that lets a
//! model work on each task in a fresh project, and a judge that checks the
//! result by simulation rather than taking the model's word for it.
//!
//! A task is a folder with a `task.toml` (prompt, judged circuit, spec table,
//! constraints, step budget) and an optional `files/` folder that becomes the
//! starting project. The judge simulates the judged circuit on ngspice,
//! evaluates the specs, runs the rule checks and checks the constraints.
//! `evals/README.md` describes the format.

mod judge;
mod pace;
mod report;
mod run;
mod task;

pub use judge::{FileState, ToolRecord, Transcript, Verdict, judge, numbers_in};
pub use pace::{Patient, hinted_delay};
pub use report::{ModelSummary, Report, TaskRun, summarize};
pub use run::{EvalOptions, ModelUnderTest, run_suite, run_task};
pub use task::{Constraint, Difficulty, Task, TaskError, load_suite, load_task};
