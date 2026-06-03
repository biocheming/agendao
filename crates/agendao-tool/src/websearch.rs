use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{Tool, ToolContext, ToolError, ToolResult};

// ── Exa MCP defaults ────────────────────────────────────────────────
const DEFAULT_BASE_URL: &str = "https://mcp.exa.ai";
const DEFAULT_ENDPOINT: &str = "/mcp";
const DEFAULT_METHOD: &str = "web_search_exa";
const DEFAULT_SEARCH_TYPE: &str = "auto";
const DEFAULT_NUM_RESULTS: usize = 8;

pub struct WebSearchTool {
    client: Client,
    /// Full URL = base_url + endpoint
    url: String,
    /// MCP tool method name
    method: String,
    /// Default search type when caller omits it
    default_search_type: String,
    /// Default number of results
    default_num_results: usize,
    /// Provider-specific extra options forwarded as MCP arguments
    options: HashMap<String, serde_json::Value>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self::from_config(None)
    }

    /// Build from an optional `WebSearchConfig`.
    /// Every field falls back to Exa MCP defaults when absent.
    pub fn from_config(config: Option<&agendao_config::WebSearchConfig>) -> Self {
        let base_url = config
            .and_then(|c| c.base_url.as_deref())
            .map(|u| u.trim_end_matches('/'))
            .unwrap_or(DEFAULT_BASE_URL);

        let endpoint = config
            .and_then(|c| c.endpoint.as_deref())
            .unwrap_or(DEFAULT_ENDPOINT);

        let method = config
            .and_then(|c| c.method.as_deref())
            .unwrap_or(DEFAULT_METHOD)
            .to_string();

        let default_search_type = config
            .and_then(|c| c.default_search_type.as_deref())
            .unwrap_or(DEFAULT_SEARCH_TYPE)
            .to_string();

        let default_num_results = config
            .and_then(|c| c.default_num_results)
            .unwrap_or(DEFAULT_NUM_RESULTS);

        let options = config.and_then(|c| c.options.clone()).unwrap_or_default();

        let url = format!("{}{}", base_url, endpoint);

        Self {
            client: match Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
            {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(%error, "failed to build configured websearch client, using reqwest default client");
                    Client::new()
                }
            },
            url,
            method,
            default_search_type,
            default_num_results,
            options,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WebSearchInput {
    query: String,
    #[serde(default, alias = "numResults")]
    num_results: Option<usize>,
    #[serde(default)]
    livecrawl: Option<String>,
    #[serde(rename = "type", default)]
    search_type: Option<String>,
    #[serde(default, alias = "contextMaxCharacters")]
    context_max_characters: Option<usize>,
}

#[derive(Debug, Serialize)]
struct McpSearchRequest {
    jsonrpc: String,
    id: u32,
    method: String,
    params: McpSearchParams,
}

#[derive(Debug, Serialize)]
struct McpSearchParams {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct McpSearchResponse {
    result: McpSearchResult,
}

#[derive(Debug, Deserialize)]
struct McpSearchResult {
    content: Vec<McpContent>,
}

#[derive(Debug, Deserialize)]
struct McpContent {
    #[serde(rename = "type")]
    _content_type: String,
    text: String,
}

static DESCRIPTION: &str = r#"Search the web for real-time information using Exa AI search engine.

This tool provides access to current information from across the web. Use it when you need:
- Current events or news
- Latest documentation or library updates
- Real-time data (weather, stock prices, etc.)
- Recent research or publications
- Any information that may have changed since the knowledge cutoff date

The search returns relevant web pages with their content, optimized for LLM context."#;

#[async_trait]
impl Tool for WebSearchTool {
    fn id(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Web search query"
                },
                "numResults": {
                    "type": "integer",
                    "default": 8,
                    "description": "Number of search results to return"
                },
                "num_results": {
                    "type": "integer",
                    "default": 8,
                    "description": "Number of search results to return (snake_case alias)"
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "default": "fallback",
                    "description": "Live crawl mode - 'fallback': use live crawling as backup if cached content unavailable, 'preferred': prioritize live crawling"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "default": "auto",
                    "description": "Search type - 'auto': balanced search, 'fast': quick results, 'deep': comprehensive search"
                },
                "contextMaxCharacters": {
                    "type": "integer",
                    "default": 10000,
                    "description": "Maximum characters for context string optimized for LLMs"
                },
                "context_max_characters": {
                    "type": "integer",
                    "default": 10000,
                    "description": "Maximum characters for context string optimized for LLMs (snake_case alias)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: WebSearchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let num_results = input.num_results.unwrap_or(self.default_num_results);

        ctx.ask_permission(
            crate::PermissionRequest::new("websearch")
                .with_pattern(&input.query)
                .with_scope_key(format!("net:search"))
                .with_metadata("query", serde_json::Value::String(input.query.clone()))
                .with_metadata("numResults", serde_json::Value::Number(num_results.into()))
                .always_allow(),
        )
        .await?;

        // Build MCP arguments: start with provider options, then overlay
        // well-known fields so caller values take precedence.
        let mut arguments = serde_json::Map::new();

        // Inject provider-specific options as base layer
        for (key, value) in &self.options {
            arguments.insert(key.clone(), value.clone());
        }

        // Well-known fields (override options if same key)
        arguments.insert("query".to_string(), serde_json::json!(input.query.clone()));
        arguments.insert(
            "type".to_string(),
            serde_json::json!(input
                .search_type
                .unwrap_or_else(|| self.default_search_type.clone())),
        );
        arguments.insert("numResults".to_string(), serde_json::json!(num_results));
        if let Some(livecrawl) = &input.livecrawl {
            arguments.insert("livecrawl".to_string(), serde_json::json!(livecrawl));
        }
        if let Some(max_chars) = input.context_max_characters {
            arguments.insert(
                "contextMaxCharacters".to_string(),
                serde_json::json!(max_chars),
            );
        }

        let search_request = McpSearchRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "tools/call".to_string(),
            params: McpSearchParams {
                name: self.method.clone(),
                arguments: serde_json::Value::Object(arguments),
            },
        };

        let response = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .json(&search_request)
            .timeout(std::time::Duration::from_secs(25))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ToolError::ExecutionError("Search request timed out".to_string())
                } else {
                    ToolError::ExecutionError(format!("Search request failed: {}", e))
                }
            })?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            return Err(ToolError::ExecutionError(format!(
                "Search error ({}): {}",
                status, response_text
            )));
        }

        let output = parse_sse_response(&response_text).unwrap_or_else(|| {
            "No search results found. Please try a different query.".to_string()
        });

        Ok(ToolResult {
            title: format!("Web search: {}", input.query),
            output,
            metadata: std::collections::HashMap::new(),
            truncated: false,
        })
    }
}

fn parse_sse_response(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(response) = serde_json::from_str::<McpSearchResponse>(data) {
                if !response.result.content.is_empty() {
                    return Some(response.result.content[0].text.clone());
                }
            }
        }
    }
    None
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_exa_defaults() {
        let tool = WebSearchTool::new();
        assert_eq!(tool.url, "https://mcp.exa.ai/mcp");
        assert_eq!(tool.method, "web_search_exa");
        assert_eq!(tool.default_search_type, "auto");
        assert_eq!(tool.default_num_results, 8);
        assert!(tool.options.is_empty());
    }

    #[test]
    fn from_config_none_uses_exa_defaults() {
        let tool = WebSearchTool::from_config(None);
        assert_eq!(tool.url, "https://mcp.exa.ai/mcp");
        assert_eq!(tool.method, "web_search_exa");
    }

    #[test]
    fn from_config_custom_base_url_and_endpoint() {
        let config = agendao_config::WebSearchConfig {
            base_url: Some("https://search.example.com".to_string()),
            endpoint: Some("/v2/query".to_string()),
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(Some(&config));
        assert_eq!(tool.url, "https://search.example.com/v2/query");
    }

    #[test]
    fn from_config_trailing_slash_stripped() {
        let config = agendao_config::WebSearchConfig {
            base_url: Some("https://search.example.com/".to_string()),
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(Some(&config));
        assert_eq!(tool.url, "https://search.example.com/mcp");
    }

    #[test]
    fn from_config_custom_method_and_search_type() {
        let config = agendao_config::WebSearchConfig {
            method: Some("brave_search".to_string()),
            default_search_type: Some("deep".to_string()),
            default_num_results: Some(20),
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(Some(&config));
        assert_eq!(tool.method, "brave_search");
        assert_eq!(tool.default_search_type, "deep");
        assert_eq!(tool.default_num_results, 20);
    }

    #[test]
    fn from_config_options_forwarded() {
        let mut opts = HashMap::new();
        opts.insert("livecrawl".to_string(), serde_json::json!("preferred"));
        opts.insert("region".to_string(), serde_json::json!("cn"));

        let config = agendao_config::WebSearchConfig {
            options: Some(opts),
            ..Default::default()
        };
        let tool = WebSearchTool::from_config(Some(&config));
        assert_eq!(tool.options.get("livecrawl").unwrap(), "preferred");
        assert_eq!(tool.options.get("region").unwrap(), "cn");
    }

    #[test]
    fn from_config_empty_config_uses_all_defaults() {
        let config = agendao_config::WebSearchConfig::default();
        let tool = WebSearchTool::from_config(Some(&config));
        assert_eq!(tool.url, "https://mcp.exa.ai/mcp");
        assert_eq!(tool.method, "web_search_exa");
        assert_eq!(tool.default_search_type, "auto");
        assert_eq!(tool.default_num_results, 8);
        assert!(tool.options.is_empty());
    }
}
