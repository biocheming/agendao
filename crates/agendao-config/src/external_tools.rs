use agendao_types::ToolCatalogMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExternalToolCatalogFile {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, ExternalToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExternalToolConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ExternalToolSource>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<ToolCatalogMetadata>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExternalToolExecution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalToolExecution {
    pub kind: ExternalToolExecutionKind,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_schema_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalToolExecutionKind {
    ScriptRunner,
}

impl ExternalToolConfig {
    pub fn is_executable(&self) -> bool {
        self.execution.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExternalToolSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}
