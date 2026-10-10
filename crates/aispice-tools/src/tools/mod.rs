//! The tools an agent (in the app, or any MCP client) uses to work on circuits.
//!
//! Every tool returns text written for a language model plus, in `data`, a
//! structured payload the desktop app renders as a card. Shapes of `data` are
//! the `ToolData` union in `app/src/ipc/types.ts`.

mod render_tools;
mod schematic_tools;

use crate::workspace::Workspace;
use aispice_agent::{Registry, ToolOutput};
use std::sync::Arc;

pub use render_tools::*;
pub use schematic_tools::*;

/// All tools, bound to one workspace.
pub fn registry(ws: Arc<Workspace>) -> Registry {
    let mut r = Registry::new();
    r.register(ListCircuits { ws: ws.clone() });
    r.register(ReadSchematic { ws: ws.clone() });
    r.register(EditSchematic { ws: ws.clone() });
    r.register(CreateSchematic { ws: ws.clone() });
    r.register(Lint { ws: ws.clone() });
    r.register(NetlistTool { ws: ws.clone() });
    r.register(History { ws: ws.clone() });
    r.register(RenderSchematic { ws: ws.clone() });
    r.register(Symbols { ws });
    r
}

/// Run blocking file work off the async executor and turn a panic or error
/// into a tool error the model can read.
pub(crate) async fn blocking<F>(f: F) -> ToolOutput
where
    F: FnOnce() -> Result<ToolOutput, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(out)) => out,
        Ok(Err(msg)) => ToolOutput::error(msg),
        Err(e) => ToolOutput::error(format!("internal error: {e}")),
    }
}
