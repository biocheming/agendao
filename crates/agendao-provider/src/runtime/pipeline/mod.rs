pub mod decode;
pub mod event_map;

use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use std::pin::Pin;

use crate::driver::StreamingEvent;
use crate::profile::{ProviderApiFamily, ProviderProfile};
use crate::protocol::ProviderHttpStreamError;
use crate::protocol_loader::ProtocolManifest;
use crate::runtime::pipeline::decode::SseDecoder;
use crate::runtime::pipeline::event_map::PathEventMapper;
use crate::ProviderError;

fn is_ethnopic_compatible_provider(provider_id: &str) -> bool {
    let lower = provider_id.trim().to_ascii_lowercase();
    lower.contains("anthropic") || lower.contains("ethnopic")
}

pub struct Pipeline {
    decoder: SseDecoder,
    mapper: PathEventMapper,
}

impl Pipeline {
    pub fn from_manifest(manifest: &ProtocolManifest) -> Result<Self, ProviderError> {
        let decoder = manifest
            .streaming
            .as_ref()
            .map(|s| SseDecoder::from_config(&s.decoder))
            .unwrap_or_else(SseDecoder::default_sse);
        let mapper = PathEventMapper::from_manifest(manifest);
        Ok(Self { decoder, mapper })
    }

    pub fn openai_default() -> Self {
        Self {
            decoder: SseDecoder::default_sse(),
            mapper: PathEventMapper::openai_defaults(),
        }
    }

    pub fn ethnopic_default() -> Self {
        Self {
            decoder: SseDecoder::default_sse(),
            mapper: PathEventMapper::ethnopic_defaults(),
        }
    }

    /// Compatibility shell for older internal call sites.
    pub fn ethnopic_default_compat() -> Self {
        Self::ethnopic_default()
    }

    pub fn google_default() -> Self {
        Self {
            decoder: SseDecoder::default_sse(),
            mapper: PathEventMapper::google_defaults(),
        }
    }

    pub fn vertex_default() -> Self {
        Self {
            decoder: SseDecoder::default_sse(),
            mapper: PathEventMapper::vertex_defaults(),
        }
    }

    pub fn for_provider(provider_id: &str) -> Self {
        let id = provider_id.to_ascii_lowercase();
        if is_ethnopic_compatible_provider(&id) {
            Self::ethnopic_default()
        } else if id.contains("google-vertex") || id.contains("vertex") {
            Self::vertex_default()
        } else if id.contains("google") || id.contains("gemini") {
            Self::google_default()
        } else {
            Self::openai_default()
        }
    }

    pub fn for_profile(profile: &ProviderProfile) -> Self {
        let provider_id = profile.provider_id.to_ascii_lowercase();
        match profile.api_family {
            ProviderApiFamily::EthnopicMessages => Self::ethnopic_default(),
            ProviderApiFamily::GeminiGenerate if provider_id.contains("vertex") => {
                Self::vertex_default()
            }
            ProviderApiFamily::GeminiGenerate => Self::google_default(),
            ProviderApiFamily::CloseAiCompatible
            | ProviderApiFamily::BedrockConverse
            | ProviderApiFamily::Custom => Self::openai_default(),
        }
    }

    pub fn process_stream(
        &self,
        input: Pin<Box<dyn Stream<Item = Result<Bytes, ProviderHttpStreamError>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamingEvent, ProviderError>> + Send>> {
        let decoded = self.decoder.decode_stream(input);
        let mapper = self.mapper.clone();

        let mapped = decoded.flat_map(move |frame_result| match frame_result {
            Ok(frame) => {
                let events: Vec<Result<StreamingEvent, ProviderError>> =
                    mapper.map_frame(&frame).into_iter().map(Ok).collect();
                stream::iter(events)
            }
            Err(err) => stream::iter(vec![Err(err)]),
        });

        Box::pin(mapped)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_ethnopic_compatible_provider, Pipeline};
    use crate::driver::StreamingEvent;
    use crate::profile::{ProviderApiFamily, ProviderProfileResolver};
    use futures::StreamExt;
    use std::collections::HashMap;

    #[test]
    fn detects_ethnopic_compatible_provider_ids() {
        assert!(is_ethnopic_compatible_provider("anthropic"));
        assert!(is_ethnopic_compatible_provider("ethnopic-compatible"));
        assert!(!is_ethnopic_compatible_provider("openai-compatible"));
    }

    #[test]
    fn ethnopic_compatible_provider_uses_messages_pipeline() {
        let pipeline = Pipeline::for_provider("ethnopic-compatible");
        let stream = pipeline.process_stream(Box::pin(futures::stream::iter(vec![Ok(
            bytes::Bytes::from_static(
                b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"},\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n\n",
            ),
        )])));

        let events = futures::executor::block_on(async { stream.collect::<Vec<_>>().await });
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(StreamingEvent::PartialContentDelta { content, .. }) if content == "hello"
        )));
    }

    #[test]
    fn profile_family_selects_messages_pipeline() {
        let profile = ProviderProfileResolver::resolve_with_npm(
            "ethnopic",
            "@ai-sdk/anthropic",
            &HashMap::new(),
        );
        assert_eq!(profile.api_family, ProviderApiFamily::EthnopicMessages);

        let pipeline = Pipeline::for_profile(&profile);
        let stream = pipeline.process_stream(Box::pin(futures::stream::iter(vec![Ok(
            bytes::Bytes::from_static(
                b"data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"hello\"}}\n\n",
            ),
        )])));

        let events = futures::executor::block_on(async { stream.collect::<Vec<_>>().await });
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(StreamingEvent::PartialContentDelta { content, .. }) if content == "hello"
        )));
    }

    #[test]
    fn profile_family_selects_google_pipeline() {
        let profile =
            ProviderProfileResolver::resolve_with_npm("google", "@ai-sdk/google", &HashMap::new());
        assert_eq!(profile.api_family, ProviderApiFamily::GeminiGenerate);

        let pipeline = Pipeline::for_profile(&profile);
        let stream = pipeline.process_stream(Box::pin(futures::stream::iter(vec![Ok(
            bytes::Bytes::from_static(
                b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}]}}]}\n\n",
            ),
        )])));

        let events = futures::executor::block_on(async { stream.collect::<Vec<_>>().await });
        assert!(events.iter().any(|event| matches!(
            event,
            Ok(StreamingEvent::PartialContentDelta { content, .. }) if content == "hello"
        )));
    }
}
