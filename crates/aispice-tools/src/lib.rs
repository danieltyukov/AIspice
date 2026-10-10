//! aispice-tools: projects, history, and the typed tools that the desktop
//! app, the CLI and the MCP server all share.

pub mod project;

pub use project::{CircuitEntry, Project, ProjectError, ProjectOptions, Snapshot};
