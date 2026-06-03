use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use std::pin::Pin;

use crate::protocol_loader::DecoderConfig;
use crate::ProviderError;

/// Manifest-driven SSE decoder.
#[derive(Debug, Clone)]
pub struct SseDecoder {
    delimiter: String,
    prefix: String,
    done_signal: String,
}

impl SseDecoder {
    pub fn default_sse() -> Self {
        Self {
            delimiter: "\n\n".to_string(),
            prefix: "data: ".to_string(),
            done_signal: "[DONE]".to_string(),
        }
    }

    pub fn from_config(cfg: &DecoderConfig) -> Self {
        let mut decoder = Self::default_sse();
        if let Some(delimiter) = &cfg.delimiter {
            decoder.delimiter = delimiter.clone();
        }
        if let Some(prefix) = &cfg.prefix {
            decoder.prefix = prefix.clone();
        }
        if let Some(done_signal) = &cfg.done_signal {
            decoder.done_signal = done_signal.clone();
        }
        decoder
    }

    pub fn decode_stream(
        &self,
        input: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<serde_json::Value, ProviderError>> + Send>> {
        let delimiter = self.delimiter.clone();
        let prefix = self.prefix.clone();
        let done_signal = self.done_signal.clone();
        let delimiter_len = delimiter.len();

        let sse_stream = stream::unfold((input, String::new()), move |(mut input, mut buf)| {
            let delimiter = delimiter.clone();
            let prefix = prefix.clone();
            let done_signal = done_signal.clone();

            async move {
                let is_done = |raw: &str| {
                    let t = raw.trim();
                    t == done_signal
                        || t == format!("data: {done_signal}")
                        || t == format!("data:{done_signal}")
                };
                let extract_data_line = |frame: &str| -> Option<String> {
                    // SSE frames may contain multiple lines (e.g. "event:xxx\ndata:{...}").
                    // Scan all lines to find the one starting with "data:" or the
                    // configured prefix, and extract its payload.
                    for line in frame.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }
                        if line.starts_with(&prefix) {
                            return Some(line[prefix.len()..].trim().to_string());
                        }
                        if let Some(rest) = line.strip_prefix("data:") {
                            return Some(rest.trim_start().to_string());
                        }
                    }
                    None
                };

                let parse_payload = |frame: &str| -> Option<serde_json::Value> {
                    let payload = extract_data_line(frame)?;
                    if payload.is_empty() || is_done(&payload) {
                        return None;
                    }
                    serde_json::from_str(&payload).ok()
                };

                loop {
                    if let Some(idx) = buf.find(&delimiter) {
                        let frame = buf[..idx].to_string();
                        let rest_start = idx + delimiter_len;
                        buf = if rest_start <= buf.len() {
                            buf[rest_start..].to_string()
                        } else {
                            String::new()
                        };

                        if is_done(&frame) {
                            return None;
                        }
                        if let Some(payload) = parse_payload(&frame) {
                            return Some((Ok(payload), (input, buf)));
                        }
                        continue;
                    }

                    match input.next().await {
                        Some(Ok(bytes)) => {
                            buf.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(err)) => {
                            return Some((
                                Err(ProviderError::StreamError(err.to_string())),
                                (input, buf),
                            ));
                        }
                        None => {
                            if is_done(&buf) {
                                return None;
                            }
                            if let Some(payload) = parse_payload(&buf) {
                                return Some((Ok(payload), (input, String::new())));
                            }
                            return None;
                        }
                    }
                }
            }
        });

        Box::pin(sse_stream)
    }
}
