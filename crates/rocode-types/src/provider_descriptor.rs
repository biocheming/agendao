use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfileDescriptorView {
    pub provider_id: String,
    pub npm: String,
    pub api_family: String,
    pub api_shape: String,
    pub transport: String,
    pub usage_shape: String,
    pub cache_family: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quirks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderConnectionDescriptorCandidate {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ProviderProfileDescriptorView>,
}
