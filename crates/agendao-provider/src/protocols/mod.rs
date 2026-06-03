mod bedrock;
mod copilot;
mod ethnopic;
mod gitlab;
mod google;
mod openai;
mod openai_request_body;
mod openai_response;
mod openai_tool_recovery;
mod openai_usage;
pub mod request_sanitizer;
mod thinking_continuation;
mod vertex;

use std::sync::Arc;

pub use bedrock::BedrockConverseAdapter;
pub use copilot::GitHubCopilotCloseAiAdapter;
pub use ethnopic::EthnopicAdapter;
pub use gitlab::GitLabCloseAiAdapter;
pub use google::GeminiAdapter;
pub use openai::CloseAiCompatibleAdapter;
pub use thinking_continuation::{
    request_effectively_enables_thinking, request_explicitly_disables_thinking,
    request_explicitly_enables_thinking,
    request_has_tool_call_continuation_missing_reasoning_replay,
    strip_reasoning_provider_options_for_new_continuation,
};
pub use vertex::VertexGeminiAdapter;

use crate::{ProviderAdapter, ProviderProfile, ProviderRuntimeAdapter};

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

pub fn create_provider_adapter_for_profile(profile: &ProviderProfile) -> Arc<dyn ProviderAdapter> {
    create_provider_adapter(ProviderRuntimeAdapter::from_profile(profile))
}
