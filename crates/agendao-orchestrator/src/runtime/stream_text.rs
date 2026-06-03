use crate::runtime::events::{CancelToken, LoopError, LoopEvent};
use crate::runtime::normalizer;
use crate::runtime::traits::ModelCaller;
use futures::StreamExt;

/// Consume a provider stream and aggregate normalized text chunks.
///
/// This keeps StreamEvent interpretation inside runtime so adapters can
/// consume a stable text output without matching provider events.
pub async fn collect_text_chunks(
    model: &dyn ModelCaller,
    raw_stream: agendao_provider::StreamResult,
    cancel: &dyn CancelToken,
) -> Result<String, LoopError> {
    let mut text = String::new();
    let mut stream = agendao_provider::assemble_tool_calls(raw_stream);

    while !cancel.is_cancelled() {
        let Some(event_result) = stream.next().await else {
            break;
        };

        match event_result {
            Ok(stream_event) => {
                for event in normalizer::normalize(stream_event) {
                    match event {
                        LoopEvent::TextChunk(delta) => text.push_str(&delta),
                        LoopEvent::Error(err) => {
                            return Err(LoopError::ModelError(
                                crate::runtime::events::ModelFailure::Message(err),
                            ))
                        }
                        _ => {}
                    }
                }
            }
            Err(err) => {
                return Err(LoopError::ModelError(
                    model.model_failure_from_provider_error(&err),
                ))
            }
        }
    }

    Ok(text)
}
