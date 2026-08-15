use crate::agent_loop::ConversationItem;
use crate::blueprint::{
    AgentId, AgentNode, BlueprintFingerprint, ModelCapability, SkillId, ToolId,
};
use crate::catalog::{CatalogFingerprint, SchedulerCatalog};
use crate::policy::PolicyEnvelope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_miss_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub fn merge(&mut self, other: &Self) {
        self.model_calls = self.model_calls.saturating_add(other.model_calls);
        self.tool_calls = self.tool_calls.saturating_add(other.tool_calls);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_miss_tokens = self
            .cache_miss_tokens
            .saturating_add(other.cache_miss_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub id: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffPacket {
    pub goal: String,
    pub constraints: Vec<String>,
    pub inputs: BTreeMap<String, String>,
    pub artifact_refs: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeResult {
    pub summary: String,
    pub output: Option<String>,
    pub artifact_refs: Vec<ArtifactRef>,
    pub usage: Usage,
}

impl NodeResult {
    pub fn combine(results: impl IntoIterator<Item = Self>) -> Self {
        let mut combined = Self::default();
        for result in results {
            if !result.summary.is_empty() {
                if !combined.summary.is_empty() {
                    combined.summary.push('\n');
                }
                combined.summary.push_str(&result.summary);
            }
            if result.output.is_some() {
                combined.output = result.output;
            }
            combined.artifact_refs.extend(result.artifact_refs);
            combined.usage.merge(&result.usage);
        }
        combined
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptSurface {
    pub stable: Arc<[u8]>,
    pub semi_stable: SemiStableZone,
    pub dynamic: DynamicZone,
    pub fingerprints: CacheFingerprints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemiStableZone {
    pub workspace_summary: String,
    pub handoff: HandoffPacket,
    pub progress_summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicZone {
    pub history_tail: Arc<Vec<ConversationItem>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceFingerprint([u8; 32]);

impl SurfaceFingerprint {
    fn of_serializable(value: &impl Serialize) -> Result<Self, SurfaceError> {
        let bytes = serde_json::to_vec(value)?;
        Ok(Self(Sha256::digest(bytes).into()))
    }
}

impl fmt::Display for SurfaceFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheFingerprints {
    pub catalog: String,
    pub blueprint: String,
    pub agent_surface: SurfaceFingerprint,
    pub tool_surface: SurfaceFingerprint,
    pub skill_bundle: SurfaceFingerprint,
    pub continuation: SurfaceFingerprint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheDiagnostic {
    Cold,
    CatalogChanged,
    BlueprintChanged,
    AgentSurfaceChanged,
    ToolSurfaceChanged,
    SkillBundleChanged,
    ContinuationBoundaryChanged,
    DynamicTailOnly,
}

impl CacheFingerprints {
    pub fn compare(&self, previous: Option<&Self>) -> CacheDiagnostic {
        let Some(previous) = previous else {
            return CacheDiagnostic::Cold;
        };
        if self.catalog != previous.catalog {
            CacheDiagnostic::CatalogChanged
        } else if self.blueprint != previous.blueprint {
            CacheDiagnostic::BlueprintChanged
        } else if self.agent_surface != previous.agent_surface {
            CacheDiagnostic::AgentSurfaceChanged
        } else if self.tool_surface != previous.tool_surface {
            CacheDiagnostic::ToolSurfaceChanged
        } else if self.skill_bundle != previous.skill_bundle {
            CacheDiagnostic::SkillBundleChanged
        } else if self.continuation != previous.continuation {
            CacheDiagnostic::ContinuationBoundaryChanged
        } else {
            CacheDiagnostic::DynamicTailOnly
        }
    }
}

pub(crate) struct PromptSurfaceInput<'a> {
    pub workspace_summary: String,
    pub progress_summary: String,
    pub handoff: HandoffPacket,
    pub history_tail: Arc<Vec<ConversationItem>>,
    pub reasoning_continuation: Option<&'a str>,
}

pub(crate) struct PromptAuthority<'a> {
    blueprint_fingerprint: BlueprintFingerprint,
    catalog_fingerprint: CatalogFingerprint,
    catalog: &'a SchedulerCatalog,
    policy: &'a PolicyEnvelope,
    harness_policy: &'a str,
    stable_surfaces: Mutex<BTreeMap<StableSurfaceKey, CachedStableSurface>>,
    max_stable_surfaces: usize,
}

impl<'a> PromptAuthority<'a> {
    pub(crate) fn new(
        blueprint_fingerprint: BlueprintFingerprint,
        catalog_fingerprint: CatalogFingerprint,
        catalog: &'a SchedulerCatalog,
        policy: &'a PolicyEnvelope,
        harness_policy: &'a str,
    ) -> Self {
        Self {
            blueprint_fingerprint,
            catalog_fingerprint,
            catalog,
            policy,
            harness_policy,
            stable_surfaces: Mutex::new(BTreeMap::new()),
            max_stable_surfaces: policy.hard_limits.max_graph_nodes.max(1) as usize,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SurfaceError {
    #[error("agent '{0}' is absent from the validated catalog")]
    MissingAgent(String),
    #[error("skill '{0}' is absent from the validated catalog")]
    MissingSkill(String),
    #[error("tool '{0}' is absent from the validated catalog")]
    MissingTool(String),
    #[error("stable prompt surface cache exceeded its graph-node bound")]
    CacheCapacity,
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StableSurfaceKey {
    agent: AgentId,
    skills: std::collections::BTreeSet<SkillId>,
    tools: std::collections::BTreeSet<ToolId>,
    required_model_capabilities: std::collections::BTreeSet<ModelCapability>,
    max_steps: u32,
}

impl From<&AgentNode> for StableSurfaceKey {
    fn from(node: &AgentNode) -> Self {
        Self {
            agent: node.agent.clone(),
            skills: node.skills.clone(),
            tools: node.tools.clone(),
            required_model_capabilities: node.required_model_capabilities.clone(),
            max_steps: node.max_steps,
        }
    }
}

#[derive(Clone)]
struct CachedStableSurface {
    stable: Arc<[u8]>,
    agent_surface: SurfaceFingerprint,
    tool_surface: SurfaceFingerprint,
    skill_bundle: SurfaceFingerprint,
}

pub(crate) fn build_prompt_surface(
    authority: &PromptAuthority<'_>,
    node: &AgentNode,
    input: PromptSurfaceInput<'_>,
) -> Result<PromptSurface, SurfaceError> {
    let key = StableSurfaceKey::from(node);
    let cached = {
        let mut surfaces = authority
            .stable_surfaces
            .lock()
            .expect("stable prompt surface cache mutex poisoned");
        if let Some(cached) = surfaces.get(&key) {
            cached.clone()
        } else {
            if surfaces.len() >= authority.max_stable_surfaces {
                return Err(SurfaceError::CacheCapacity);
            }
            let cached = build_stable_surface(authority, node)?;
            surfaces.insert(key, cached.clone());
            cached
        }
    };
    let continuation = SurfaceFingerprint::of_serializable(&input.reasoning_continuation)?;

    Ok(PromptSurface {
        stable: cached.stable,
        semi_stable: SemiStableZone {
            workspace_summary: input.workspace_summary,
            handoff: input.handoff,
            progress_summary: input.progress_summary,
        },
        dynamic: DynamicZone {
            history_tail: input.history_tail,
        },
        fingerprints: CacheFingerprints {
            catalog: authority.catalog_fingerprint.to_string(),
            blueprint: authority.blueprint_fingerprint.to_string(),
            agent_surface: cached.agent_surface,
            tool_surface: cached.tool_surface,
            skill_bundle: cached.skill_bundle,
            continuation,
        },
    })
}

fn build_stable_surface(
    authority: &PromptAuthority<'_>,
    node: &AgentNode,
) -> Result<CachedStableSurface, SurfaceError> {
    let agent = authority
        .catalog
        .agents
        .get(&node.agent)
        .ok_or_else(|| SurfaceError::MissingAgent(node.agent.as_str().to_string()))?;
    let skills = node
        .skills
        .iter()
        .map(|id| {
            authority
                .catalog
                .skills
                .get(id)
                .map(|entry| (id, entry))
                .ok_or_else(|| SurfaceError::MissingSkill(id.as_str().to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let tools = node
        .tools
        .iter()
        .map(|id| {
            authority
                .catalog
                .tools
                .get(id)
                .map(|entry| (id, entry))
                .ok_or_else(|| SurfaceError::MissingTool(id.as_str().to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let stable = Arc::from(stable_prompt(
        authority.harness_policy,
        &agent.system_policy,
        &skills,
    ));
    let agent_surface = SurfaceFingerprint::of_serializable(&(
        authority.harness_policy,
        &node.agent,
        &agent.system_policy,
        node.max_steps,
        &agent.model_capabilities,
        &node.required_model_capabilities,
        authority.policy,
    ))?;
    let tool_surface = SurfaceFingerprint::of_serializable(&tools)?;
    let skill_bundle = SurfaceFingerprint::of_serializable(&skills)?;

    Ok(CachedStableSurface {
        stable,
        agent_surface,
        tool_surface,
        skill_bundle,
    })
}

fn stable_prompt(
    harness_policy: &str,
    agent_policy: &str,
    skills: &BTreeMap<&crate::blueprint::SkillId, &crate::catalog::SkillCatalogEntry>,
) -> Vec<u8> {
    let mut prompt = String::with_capacity(
        harness_policy.len()
            + agent_policy.len()
            + skills
                .values()
                .map(|skill| {
                    skill
                        .hydrated_prompt
                        .as_ref()
                        .map_or(skill.summary.len(), |prompt| prompt.len())
                })
                .sum::<usize>()
            + 96,
    );
    prompt.push_str(harness_policy.trim());
    if !agent_policy.trim().is_empty() {
        prompt.push_str("\n\n## Agent Policy\n");
        prompt.push_str(agent_policy.trim());
    }
    if !skills.is_empty() {
        prompt.push_str("\n\n## Selected Skills\n");
        for (id, skill) in skills {
            prompt.push_str("### ");
            prompt.push_str(id.as_str());
            prompt.push('\n');
            prompt.push_str(
                skill
                    .hydrated_prompt
                    .as_deref()
                    .unwrap_or(&skill.summary)
                    .trim(),
            );
            prompt.push('\n');
        }
    }
    prompt.into_bytes()
}
