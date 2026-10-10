//! `aispice mcp`: the tool set served over the Model Context Protocol on
//! stdio, for Claude Code, Cursor, Codex, Claude Desktop and any other client.

use aispice_agent::{ContentBlock, Registry, ToolContext};
use aispice_tools::{Project, ProjectOptions, Workspace, registry};
use anyhow::{Context, Result};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock as Content,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

const INSTRUCTIONS: &str = "aispice edits LTspice schematics and simulates them. Start with list_circuits and read_schematic. Change circuits only through edit_schematic, using part names and PART.PIN references; never compute coordinates. Run lint after edits. Every edit is saved with undo available through the history tool.";

pub struct AispiceServer {
    registry: Arc<Registry>,
}

impl AispiceServer {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }
}

fn to_mcp_content(block: &ContentBlock) -> Option<Content> {
    match block {
        ContentBlock::Text { text } => Some(Content::text(text.clone())),
        ContentBlock::Image {
            media_type,
            data_base64,
        } => Some(Content::image(data_base64.clone(), media_type.clone())),
        _ => None,
    }
}

impl ServerHandler for AispiceServer {
    fn get_info(&self) -> ServerConfig {
        let mut server = Implementation::new("aispice", env!("CARGO_PKG_VERSION"));
        server.title = Some("aispice".into());
        server.website_url = Some("https://danieltyukov.github.io/aispice/".into());
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(server)
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .registry
            .specs()
            .into_iter()
            .map(|s| {
                let schema = match s.input_schema {
                    Value::Object(map) => map,
                    _ => serde_json::Map::new(),
                };
                Tool::new(s.name, s.description, Arc::new(schema))
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(tool) = self.registry.get(&request.name) else {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Unknown tool `{}`. Available: {}",
                request.name,
                self.registry.names().join(", ")
            ))])
            .into());
        };
        let input = Value::Object(request.arguments.unwrap_or_default());
        let out = tool
            .call(&ToolContext::new(context.ct.clone()), input)
            .await;
        let content: Vec<Content> = out.content.iter().filter_map(to_mcp_content).collect();
        Ok(if out.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        }
        .into())
    }
}

/// Serve over stdin and stdout until the client disconnects.
pub async fn serve(project_dir: &Path, options: ProjectOptions) -> Result<()> {
    let project = Project::open(project_dir, options)
        .with_context(|| format!("opening {}", project_dir.display()))?;
    let ws = Arc::new(Workspace::with_project(project));
    let server = AispiceServer::new(Arc::new(registry(ws)));
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .context("starting the MCP server")?;
    running.waiting().await.context("MCP server stopped")?;
    Ok(())
}
