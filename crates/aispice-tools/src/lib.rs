//! aispice-tools: projects, history, and the typed tools that the desktop
//! app, the CLI and the MCP server all share.

pub mod project;
pub mod tools;
pub mod workspace;

pub use project::{CircuitEntry, Project, ProjectError, ProjectOptions, Snapshot};
pub use tools::registry;
pub use workspace::{Approver, Hooks, Workspace};
