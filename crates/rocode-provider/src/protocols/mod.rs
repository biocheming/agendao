mod bedrock;
mod copilot;
mod gitlab;
mod google;
mod messages;
mod openai;
mod openai_request_body;
mod openai_response;
mod openai_tool_recovery;
mod openai_usage;
mod vertex;

use std::sync::Arc;

pub use bedrock::BedrockProtocol;
pub use copilot::CopilotProtocol;
pub use gitlab::GitLabProtocol;
pub use google::GoogleProtocol;
pub use messages::MessagesProtocol;
/// Neutral alias for the generic messages-family protocol implementation.
pub use messages::MessagesProtocol as EthnopicProtocol;
pub use openai::OpenAIProtocol;
pub use vertex::VertexProtocol;

use crate::{Protocol, ProtocolImpl, ProviderProfile};

pub fn create_legacy_protocol_impl(protocol: Protocol) -> Arc<dyn ProtocolImpl> {
    match protocol {
        Protocol::OpenAI => Arc::new(OpenAIProtocol::new()),
        Protocol::Messages => Arc::new(EthnopicProtocol::new()),
        Protocol::Google => Arc::new(GoogleProtocol::new()),
        Protocol::Bedrock => Arc::new(BedrockProtocol::new()),
        Protocol::Vertex => Arc::new(VertexProtocol::new()),
        Protocol::GitHubCopilot => Arc::new(CopilotProtocol::new()),
        Protocol::GitLab => Arc::new(GitLabProtocol::new()),
    }
}

pub fn create_protocol_impl(protocol: Protocol) -> Arc<dyn ProtocolImpl> {
    create_legacy_protocol_impl(protocol)
}

pub fn create_protocol_impl_for_profile(profile: &ProviderProfile) -> Arc<dyn ProtocolImpl> {
    create_legacy_protocol_impl(Protocol::from_profile(profile))
}
