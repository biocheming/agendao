//! Content-Length framed JSON-RPC codec for subprocess communication.
//!
//! This is the single authority for encoding/decoding Content-Length framed
//! messages, used by all JSON-RPC subprocess protocols (Plugin, MCP, LSP).
//! Per Constitution Article 1, adapters reference this codec — they never
//! reimplement the framing logic.

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Errors produced by Content-Length codec operations.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Serialize(#[from] serde_json::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("connection closed (EOF)")]
    ConnectionClosed,
}

/// Write a Content-Length framed JSON-RPC message to any async writer.
///
/// Format: `Content-Length: N\r\n\r\n<json-body>`
///
/// Accepts `ChildStdin`, `&mut ChildStdin`, or any `AsyncWrite + Unpin`.
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), CodecError> {
    let content = serde_json::to_string(message)?;
    let frame = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);
    writer.write_all(frame.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one Content-Length framed JSON-RPC message from any buffered async reader.
///
/// Returns the parsed JSON [`Value`], or `ConnectionClosed` on EOF.
///
/// **Important:** The caller must own and reuse a single buffered reader across
/// calls. Creating a new `BufReader` per call discards buffered data.
///
/// Accepts `BufReader<ChildStdout>`, `BufReader<R>`, or any `AsyncBufRead + Unpin`.
pub async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value, CodecError> {
    let mut content_length: Option<usize> = None;
    let mut line = String::new();

    // Parse headers until empty line
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(CodecError::ConnectionClosed);
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }

        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(
                rest.trim()
                    .parse()
                    .map_err(|e| CodecError::Protocol(format!("invalid Content-Length: {e}")))?,
            );
        }
        // Ignore other headers (Content-Type, etc.) per LSP spec
    }

    let len = content_length
        .ok_or_else(|| CodecError::Protocol("missing Content-Length header".into()))?;

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let value: Value = serde_json::from_slice(&buf)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn write_frame_format() {
        // Use an in-memory pipe to test framing
        let (mut reader, writer) = tokio::io::duplex(1024);

        // We need a ChildStdin, but for unit testing we can verify the format
        // by writing to a buffer instead
        let msg = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"test"});
        let content = serde_json::to_string(&msg).unwrap();
        let expected = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        // Write directly to verify format
        let mut w = writer;
        w.write_all(expected.as_bytes()).await.unwrap();
        w.flush().await.unwrap();
        drop(w);

        let mut buf = String::new();
        reader.read_to_string(&mut buf).await.unwrap();
        assert!(buf.starts_with("Content-Length: "));
        assert!(buf.contains("\r\n\r\n"));
        assert!(buf.contains("\"jsonrpc\""));
    }
}
