//! `craws-mcp` — the MCP server binary. Speaks JSON-RPC over stdio; a client
//! (pooprusteek, Claude Desktop, …) spawns it as a child process.
//!
//! stdout is the protocol channel — this binary logs only to stderr.

use anyhow::Result;
use craws_mcp::Craws;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

#[tokio::main]
async fn main() -> Result<()> {
    eprintln!("craws-mcp: serving on stdio (protocol 2024-11-05)");
    let service = Craws::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
