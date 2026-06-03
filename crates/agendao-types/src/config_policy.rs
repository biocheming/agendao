use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConfigPolicyValidationOwner {
    Scheduler,
    SkillTree,
    ProviderProfile,
    ExternalAdapter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConfigPolicyValidationScopeKind {
    SchedulerPath,
    SkillTree,
    Provider,
    ExternalAdapter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConfigPolicyValidationSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConfigPolicyValidationEffect {
    SoftFallback,
    FailClosedBootstrap,
    FailClosedRequestGate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigPolicyValidationScope {
    pub kind: ConfigPolicyValidationScopeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigPolicyValidationItem {
    pub owner: ConfigPolicyValidationOwner,
    pub scope: ConfigPolicyValidationScope,
    pub path: String,
    pub severity: ConfigPolicyValidationSeverity,
    pub effect: ConfigPolicyValidationEffect,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigPolicyValidationSnapshot {
    pub revision: u64,
    pub generated_at_ms: i64,
    #[serde(default)]
    pub reports: Vec<ConfigPolicyValidationItem>,
}
