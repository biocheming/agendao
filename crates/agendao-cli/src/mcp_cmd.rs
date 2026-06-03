use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cli::{McpAuthCommands, McpCommands};
use crate::util::{parse_http_json, server_url};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct McpStatusEntry {
    name: String,
    status: String,
    tools: usize,
    resources: usize,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct McpAuthStartResponse {
    authorization_url: String,
    client_id: Option<String>,
    status: String,
}

pub(crate) async fn handle_mcp_command(server: String, action: McpCommands) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    match action {
        McpCommands::List => {
            let endpoint = server_url(&server, "/mcp");
            let status_map: HashMap<String, McpStatusEntry> =
                parse_http_json(client.get(endpoint).send().await?).await?;

            if status_map.is_empty() {
                println!("No MCP servers reported.");
                return Ok(());
            }

            println!("\nMCP servers:\n");
            let mut items: Vec<_> = status_map.into_values().collect();
            items.sort_by(|a, b| a.name.cmp(&b.name));

            for server_info in items {
                println!(
                    "  {:<20} {:<12} tools={} resources={}",
                    server_info.name, server_info.status, server_info.tools, server_info.resources
                );
                if let Some(error) = server_info.error {
                    println!("    error: {}", error);
                }
            }
            println!();
        }
        McpCommands::Add {
            name,
            url,
            command,
            args,
            enabled,
            timeout,
        } => {
            let config = if let Some(url) = url {
                serde_json::json!({
                    "type": "remote",
                    "url": url,
                    "enabled": enabled,
                    "timeout": timeout
                })
            } else if let Some(command) = command {
                serde_json::json!({
                    "command": command,
                    "args": args,
                    "enabled": enabled,
                    "timeout": timeout
                })
            } else {
                anyhow::bail!("`mcp add` requires either --url (remote) or --command (local)");
            };

            let endpoint = server_url(&server, "/mcp");
            let _: HashMap<String, McpStatusEntry> = parse_http_json(
                client
                    .post(endpoint)
                    .json(&serde_json::json!({
                        "name": name,
                        "config": config
                    }))
                    .send()
                    .await?,
            )
            .await?;

            println!("MCP server added.");
        }
        McpCommands::Connect { name } => {
            let endpoint = server_url(&server, &format!("/mcp/{}/connect", name));
            let connected: bool = parse_http_json(client.post(endpoint).send().await?).await?;
            println!("Connected: {}", connected);
        }
        McpCommands::Disconnect { name } => {
            let endpoint = server_url(&server, &format!("/mcp/{}/disconnect", name));
            let disconnected: bool = parse_http_json(client.post(endpoint).send().await?).await?;
            println!("Disconnected: {}", disconnected);
        }
        McpCommands::Auth {
            action,
            name,
            code,
            authenticate,
        } => {
            if matches!(action, Some(McpAuthCommands::List)) {
                let endpoint = server_url(&server, "/mcp");
                let status_map: HashMap<String, McpStatusEntry> =
                    parse_http_json(client.get(endpoint).send().await?).await?;
                let mut items: Vec<_> = status_map.into_values().collect();
                items.sort_by(|a, b| a.name.cmp(&b.name));
                for server_info in items {
                    println!(
                        "  {:<20} {:<12} tools={} resources={}",
                        server_info.name,
                        server_info.status,
                        server_info.tools,
                        server_info.resources
                    );
                }
                return Ok(());
            }

            let name = name.ok_or_else(|| {
                anyhow::anyhow!("Missing MCP server name. Use `agendao mcp auth <name>`.")
            })?;

            if authenticate {
                let endpoint = server_url(&server, &format!("/mcp/{}/auth/authenticate", name));
                let status: McpStatusEntry =
                    parse_http_json(client.post(endpoint).send().await?).await?;
                println!("Auth status: {} ({})", status.name, status.status);
            } else if let Some(code) = code {
                let endpoint = server_url(&server, &format!("/mcp/{}/auth/callback", name));
                let status: McpStatusEntry = parse_http_json(
                    client
                        .post(endpoint)
                        .json(&serde_json::json!({ "code": code }))
                        .send()
                        .await?,
                )
                .await?;
                println!("Auth callback result: {} ({})", status.name, status.status);
            } else {
                let endpoint = server_url(&server, &format!("/mcp/{}/auth", name));
                let auth: McpAuthStartResponse =
                    parse_http_json(client.post(endpoint).send().await?).await?;
                println!("Authorization URL: {}", auth.authorization_url);
                if let Some(client_id) = auth.client_id {
                    println!("Client ID: {}", client_id);
                }
                println!("Status: {}", auth.status);
            }
        }
        McpCommands::Logout { name } => {
            let name = name.ok_or_else(|| {
                anyhow::anyhow!("Missing MCP server name. Use `agendao mcp logout <name>`.")
            })?;
            let endpoint = server_url(&server, &format!("/mcp/{}/auth", name));
            let _: serde_json::Value =
                parse_http_json(client.delete(endpoint).send().await?).await?;
            println!("OAuth credentials removed for MCP server: {}", name);
        }
        McpCommands::Debug { name } => {
            let endpoint = server_url(&server, "/mcp");
            let status_map: HashMap<String, McpStatusEntry> =
                parse_http_json(client.get(endpoint).send().await?).await?;
            let entry = status_map.get(&name).ok_or_else(|| {
                anyhow::anyhow!("MCP server not found in runtime status: {}", name)
            })?;
            println!("{}", serde_json::to_string_pretty(entry)?);
        }
    }

    Ok(())
}
