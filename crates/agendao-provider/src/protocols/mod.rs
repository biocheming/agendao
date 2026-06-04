#[cfg(feature = "http-transport")]
mod bedrock;
#[cfg(feature = "http-transport")]
mod copilot;
#[cfg(feature = "http-transport")]
mod ethnopic;
#[cfg(feature = "http-transport")]
mod gitlab;
#[cfg(feature = "http-transport")]
mod google;
#[cfg(feature = "http-transport")]
mod openai;
#[cfg(feature = "http-transport")]
mod openai_request_body;
#[cfg(feature = "http-transport")]
mod openai_response;
#[cfg(feature = "http-transport")]
mod openai_tool_recovery;
#[cfg(feature = "http-transport")]
mod openai_usage;
pub mod request_sanitizer;
mod thinking_continuation;
#[cfg(feature = "http-transport")]
mod vertex;

#[cfg(feature = "http-transport")]
use std::sync::Arc;

#[cfg(feature = "http-transport")]
pub use bedrock::BedrockConverseAdapter;
#[cfg(feature = "http-transport")]
pub use copilot::GitHubCopilotCloseAiAdapter;
#[cfg(feature = "http-transport")]
pub use ethnopic::EthnopicAdapter;
#[cfg(feature = "http-transport")]
pub use gitlab::GitLabCloseAiAdapter;
#[cfg(feature = "http-transport")]
pub use google::GeminiAdapter;
#[cfg(feature = "http-transport")]
pub use openai::CloseAiCompatibleAdapter;
pub use thinking_continuation::{
    request_effectively_enables_thinking, request_explicitly_disables_thinking,
    request_explicitly_enables_thinking,
    request_has_tool_call_continuation_missing_reasoning_replay,
    strip_reasoning_provider_options_for_new_continuation,
};
#[cfg(feature = "http-transport")]
pub use vertex::VertexGeminiAdapter;

#[cfg(feature = "http-transport")]
use crate::{ProviderAdapter, ProviderProfile, ProviderRuntimeAdapter};

#[cfg(feature = "http-transport")]
pub fn create_provider_adapter(adapter: ProviderRuntimeAdapter) -> Arc<dyn ProviderAdapter> {
    match adapter {
        ProviderRuntimeAdapter::CloseAiCompatible => Arc::new(CloseAiCompatibleAdapter::new()),
        ProviderRuntimeAdapter::Ethnopic => Arc::new(EthnopicAdapter::new()),
        ProviderRuntimeAdapter::Gemini => Arc::new(GeminiAdapter::new()),
        ProviderRuntimeAdapter::BedrockConverse => Arc::new(BedrockConverseAdapter::new()),
        ProviderRuntimeAdapter::VertexGemini => Arc::new(VertexGeminiAdapter::new()),
        ProviderRuntimeAdapter::GitHubCopilotCloseAi => {
            Arc::new(GitHubCopilotCloseAiAdapter::new())
        }
        ProviderRuntimeAdapter::GitLabCloseAi => Arc::new(GitLabCloseAiAdapter::new()),
    }
}

#[cfg(feature = "http-transport")]
pub fn create_provider_adapter_for_profile(profile: &ProviderProfile) -> Arc<dyn ProviderAdapter> {
    create_provider_adapter(ProviderRuntimeAdapter::from_profile(profile))
}
