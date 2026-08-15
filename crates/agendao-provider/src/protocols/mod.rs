#[cfg(feature = "http-transport")]
mod anthropic;
#[cfg(feature = "http-transport")]
mod openai;
#[cfg(feature = "http-transport")]
mod openai_request_body;
#[cfg(feature = "http-transport")]
mod openai_response;
#[cfg(feature = "http-transport")]
mod openai_tool_arguments;
#[cfg(feature = "http-transport")]
mod openai_usage;
pub mod request_sanitizer;
mod thinking_continuation;

#[cfg(feature = "http-transport")]
use std::sync::Arc;

#[cfg(feature = "http-transport")]
pub use anthropic::AnthropicAdapter;
#[cfg(feature = "http-transport")]
pub use openai::OpenAiCompatibleAdapter;
pub use thinking_continuation::{
    request_effectively_enables_thinking, request_explicitly_disables_thinking,
    request_explicitly_enables_thinking,
    request_has_tool_call_continuation_missing_reasoning_replay,
    strip_reasoning_provider_options_for_new_continuation,
};

#[cfg(feature = "http-transport")]
use crate::{ProviderAdapter, ProviderProfile, ProviderRuntimeAdapter};

#[cfg(feature = "http-transport")]
pub fn create_provider_adapter(adapter: ProviderRuntimeAdapter) -> Arc<dyn ProviderAdapter> {
    match adapter {
        ProviderRuntimeAdapter::OpenAiCompatible => Arc::new(OpenAiCompatibleAdapter::new()),
        ProviderRuntimeAdapter::Anthropic => Arc::new(AnthropicAdapter::new()),
    }
}

#[cfg(feature = "http-transport")]
pub fn create_provider_adapter_for_profile(profile: &ProviderProfile) -> Arc<dyn ProviderAdapter> {
    create_provider_adapter(ProviderRuntimeAdapter::from_profile(profile))
}
