use super::blueprint::{CapabilityId, ExecutionLimits, ToolId};
use super::catalog::{EffectClass, SchedulerCatalog};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyEnvelope {
    pub hard_limits: ExecutionLimits,
    pub allowed_tools: BTreeSet<ToolId>,
    pub allowed_effects: BTreeSet<EffectClass>,
    pub allowed_capabilities: BTreeSet<CapabilityId>,
    pub workspace_limits: WorkspaceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLimits {
    pub max_files: u32,
    pub max_total_bytes: u64,
    pub min_free_disk_bytes: u64,
    pub operation_timeout_ms: u64,
}

impl PolicyEnvelope {
    pub fn allow_catalog(hard_limits: ExecutionLimits, catalog: &SchedulerCatalog) -> Self {
        Self {
            hard_limits,
            allowed_tools: catalog.tools.keys().cloned().collect(),
            allowed_effects: catalog
                .tools
                .values()
                .map(|tool| tool.effect)
                .chain(
                    catalog
                        .capabilities
                        .values()
                        .map(|capability| capability.effect),
                )
                .collect(),
            allowed_capabilities: catalog.capabilities.keys().cloned().collect(),
            workspace_limits: WorkspaceLimits {
                max_files: 10_000,
                max_total_bytes: 1_073_741_824,
                min_free_disk_bytes: 536_870_912,
                operation_timeout_ms: 30_000,
            },
        }
    }
}
