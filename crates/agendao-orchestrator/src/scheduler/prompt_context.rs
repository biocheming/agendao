use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvailableAgentMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub cost: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvailableCategoryMeta {
    pub name: String,
    pub description: String,
}
