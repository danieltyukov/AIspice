//! What every tool shares: the open project and hooks into the front end.
//!
//! The desktop app can switch projects, so the project sits behind a lock;
//! the CLI and the MCP server open one at start. Hooks let a front end react
//! to edits (reload LTspice, refresh the drawing) and, in ask-before-apply
//! mode, approve an edit before it is written.

use crate::project::{Project, ProjectError};
use crate::runner::Runner;
use async_trait::async_trait;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Approves or rejects an edit before it is saved.
#[async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, circuit: &str, summary: &str, diff: &str) -> bool;
}

/// Called after a circuit file changes on disk because of a tool.
pub type AfterSave = Arc<dyn Fn(&Path) + Send + Sync>;

#[derive(Default)]
pub struct Hooks {
    pub approver: Option<Arc<dyn Approver>>,
    pub after_save: Option<AfterSave>,
}

pub struct Workspace {
    project: RwLock<Option<Arc<Project>>>,
    hooks: RwLock<Hooks>,
    /// Simulators and recent results, shared by every tool.
    pub runner: Runner,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field(
                "project",
                &self
                    .project
                    .read()
                    .ok()
                    .and_then(|p| p.as_ref().map(|p| p.root().to_path_buf())),
            )
            .finish()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            project: RwLock::new(None),
            hooks: RwLock::new(Hooks::default()),
            runner: Runner::default(),
        }
    }

    pub fn with_project(project: Project) -> Self {
        let ws = Self::new();
        ws.set_project(project);
        ws
    }

    pub fn set_project(&self, project: Project) {
        *self.project.write().expect("project lock") = Some(Arc::new(project));
    }

    pub fn project(&self) -> Result<Arc<Project>, ProjectError> {
        self.project
            .read()
            .expect("project lock")
            .clone()
            .ok_or_else(|| {
                ProjectError::Other("no project is open; open a folder of circuits first".into())
            })
    }

    pub fn set_hooks(&self, hooks: Hooks) {
        *self.hooks.write().expect("hooks lock") = hooks;
    }

    pub fn approver(&self) -> Option<Arc<dyn Approver>> {
        self.hooks.read().expect("hooks lock").approver.clone()
    }

    /// Ask the user to approve a change, when the front end asks for that
    /// (ask-before-apply mode). Without an approver every change is allowed,
    /// since edits are undoable. Every tool that writes goes through here.
    pub async fn approve(&self, circuit: &str, summary: &str, diff: &str) -> bool {
        match self.approver() {
            Some(a) => a.approve(circuit, summary, diff).await,
            None => true,
        }
    }

    pub fn after_save(&self, path: &Path) {
        let hook = self.hooks.read().expect("hooks lock").after_save.clone();
        if let Some(h) = hook {
            h(path);
        }
    }
}
