//! Unified JSON-RPC 2.0 envelope types.
//!
//! This is the single authority for JSON-RPC 2.0 message structures used across
//! all subprocess protocols (Plugin, MCP, LSP). Per Constitution Article 1,
//! adapters reference this authority — they never replicate it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 request (client → server, expects a response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 response (server → client, matches a request by `id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 error object.
///
/// Uses `i64` for the error code to accommodate both standard codes
/// (-32700 to -32603) and custom application codes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

// ---------------------------------------------------------------------------
// Notification
// ---------------------------------------------------------------------------

/// A JSON-RPC 2.0 notification (no `id`, no response expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
        }
    }
}

// ---------------------------------------------------------------------------
// Incoming message discriminant
// ---------------------------------------------------------------------------

/// A received JSON-RPC message — either a response or a server notification.
///
/// Discrimination is based on the presence of an `"id"` field (response) vs
/// a `"method"` field without `"id"` (notification).
#[derive(Debug, Clone)]
pub enum JsonRpcMessage {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// Error returned when a raw JSON value cannot be classified as a valid
/// JSON-RPC 2.0 response or notification.
#[derive(Debug, thiserror::Error)]
#[error("invalid JSON-RPC message: missing both 'id' (response) and 'method' (notification)")]
pub struct InvalidJsonRpcMessage;

impl JsonRpcMessage {
    /// Parse a [`serde_json::Value`] into a typed [`JsonRpcMessage`].
    ///
    /// This is the canonical discrimination algorithm — adapters must not
    /// reimplement it.
    pub fn from_value(value: Value) -> Result<Self, JsonRpcError> {
        if value.get("id").is_some() {
            serde_json::from_value(value)
                .map(JsonRpcMessage::Response)
                .map_err(|e| JsonRpcError {
                    code: -32700,
                    message: format!("Failed to parse response: {e}"),
                    data: None,
                })
        } else if value.get("method").is_some() {
            serde_json::from_value(value)
                .map(JsonRpcMessage::Notification)
                .map_err(|e| JsonRpcError {
                    code: -32700,
                    message: format!("Failed to parse notification: {e}"),
                    data: None,
                })
        } else {
            Err(JsonRpcError {
                code: -32600,
                message: "Invalid JSON-RPC message: missing both 'id' and 'method'".into(),
                data: None,
            })
        }
    }

    /// Parse a raw JSON string into a typed [`JsonRpcMessage`].
    pub fn parse_json(s: &str) -> Result<Self, JsonRpcError> {
        let value: Value = serde_json::from_str(s).map_err(|e| JsonRpcError {
            code: -32700,
            message: format!("Parse error: {e}"),
            data: None,
        })?;
        Self::from_value(value)
    }

    /// Returns `true` if this is a progress notification (`notifications/progress`
    /// or `$/progress`). Used by all subprocess clients to extend RPC deadlines.
    pub fn is_progress_notification(&self) -> bool {
        match self {
            JsonRpcMessage::Notification(n) => {
                n.method == "notifications/progress" || n.method == "$/progress"
            }
            _ => false,
        }
    }
}

impl std::str::FromStr for JsonRpcMessage {
    type Err = JsonRpcError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_json(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serialization() {
        let req = JsonRpcRequest::new(1, "initialize", None);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"initialize\""));
        // params is None → should be absent
        assert!(!json.contains("\"params\""));
    }

    #[test]
    fn request_with_params() {
        let req = JsonRpcRequest::new(2, "tools/call", Some(serde_json::json!({"name": "ls"})));
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"params\""));
    }

    #[test]
    fn response_deserialization() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, 1);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn error_deserialization_with_data() {
        let json = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid","data":"extra"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert!(err.data.is_some());
    }

    #[test]
    fn message_from_value_response() {
        let val = serde_json::json!({"jsonrpc":"2.0","id":1,"result":null});
        let msg = JsonRpcMessage::from_value(val).unwrap();
        assert!(matches!(msg, JsonRpcMessage::Response(_)));
    }

    #[test]
    fn message_from_value_notification() {
        let val = serde_json::json!({"jsonrpc":"2.0","method":"$/progress","params":{}});
        let msg = JsonRpcMessage::from_value(val).unwrap();
        assert!(matches!(msg, JsonRpcMessage::Notification(_)));
        assert!(msg.is_progress_notification());
    }

    #[test]
    fn message_from_value_invalid() {
        let val = serde_json::json!({"jsonrpc":"2.0"});
        let err = JsonRpcMessage::from_value(val).unwrap_err();
        assert_eq!(err.code, -32600);
    }

    #[test]
    fn notification_new() {
        let notif = JsonRpcNotification::new("$/cancelRequest", Some(serde_json::json!({"id": 5})));
        let json = serde_json::to_string(&notif).unwrap();
        assert!(json.contains("\"method\":\"$/cancelRequest\""));
        // A notification must not have a top-level "id" field (only requests/responses do).
        // The "id" inside params is fine — we check that there's no `"id":` at the top level
        // by verifying the struct fields (notification has: jsonrpc, method, params only).
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.get("id").is_none(),
            "notification must not have top-level 'id'"
        );
    }

    #[test]
    fn progress_detection() {
        let msg = r#"{"jsonrpc":"2.0","method":"notifications/progress","params":{}}"#
            .parse::<JsonRpcMessage>()
            .unwrap();
        assert!(msg.is_progress_notification());

        let msg = r#"{"jsonrpc":"2.0","method":"$/progress","params":{}}"#
            .parse::<JsonRpcMessage>()
            .unwrap();
        assert!(msg.is_progress_notification());

        let msg = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{}}"#
            .parse::<JsonRpcMessage>()
            .unwrap();
        assert!(!msg.is_progress_notification());
    }
}
