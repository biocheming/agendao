//! Prompt Surface Authority — unified input/output data model for the
//! session prompt surface.
//!
//! ## Constitution Placement
//!
//! This module is the **single authority data contract** for prompt surface
//! construction (AgenDao 土律: 承载之土，贵在归一).  Every path that touches
//! the model-visible prompt surface — session prompt, scheduler preset,
//! memory reflow, provider cache hints — should consume these structures
//! rather than assembling the surface from independent raw inputs.
//!
//! ## Relationship to existing types
//!
//! | Type                     | Role                                          |
//! |--------------------------|-----------------------------------------------|
//! | `SystemPrompt`           | Product header + environment (static layer)   |
//! | `SessionPrompt`          | Session prompt surface authority (orchestrator)|
//! | `PromptSurfaceInputs`    | **New**: aggregate inputs consumed by authority |
//! | `PromptSurfaceSections`  | **New**: canonical output sections             |
//! | `PresetPromptExtension`  | **New**: preset contribution, not full surface |
//!
//! ## Migration contract
//!
//! - **Phase 3 (this commit)**: Define structures only. No call-site changes.
//! - **Phase 5**: Migrate session-side consumers to read from these views.
//! - **Phase 6**: Migrate preset call sites to return `PresetPromptExtension`
//!   instead of raw `String`.
//! - **Phase 7**: Delete legacy full-prompt assembly paths.
//!
//! ## Cache boundary
//!
//! `PromptSurfaceSections` is the authority for **pure prompt-surface
//! projections** only: stable system projection, tool-source grouping,
//! provider policy projections, and output-projection policy.
//!
//! Provider-family cache semantics remain outside this module. In particular,
//! `cache_request_fingerprint(...)` and
//! `closeai_prompt_cache_key_for_fingerprint(...)` stay in
//! `loop_lifecycle.rs` until their provider-specific behavior can be migrated
//! without changing cache guardrail tests.
//!
//! ## Visibility
//!
//! `PromptSurfaceInputs` is the sanctioned cross-crate mutation entry for
//! request construction. Field-level reads stay crate-internal so
//! `SessionPrompt` remains the sole surface assembler.

// Skeleton types are intentionally unused until Phase 5 cut-over.
#![allow(dead_code)]

use std::collections::HashMap;

use agendao_execution_types::CompiledExecutionRequest;
use agendao_provider::cache::ToolSurfaceSourceDigest;
use agendao_provider::cache::{json_fingerprint, text_fingerprint};
use agendao_provider::ToolDefinition;
use agendao_types::{
    FewShotSurfaceItem, MemoryRetrievalPacket, PinnedConstraint,
    PromptSurfaceDriftCategory as PublicPromptSurfaceDriftCategory,
    PromptSurfaceDriftDetail, PromptSurfaceEvidenceSummary,
    PromptSurfaceVolatilityFinding as PublicPromptSurfaceVolatilityFinding,
    PromptSurfaceVolatilityKind as PublicPromptSurfaceVolatilityKind, SessionCacheSeverity,
};

use super::surface_contract::{
    collect_prompt_surface_provider_options, is_dynamic_catalog_header,
    is_stable_governance_header, is_volatile_system_section, looks_like_clock_line,
    normalize_stable_system_line, sanctioned_model_context_projection_for_message,
    PromptSurfaceProviderOptionGroup,
};
use crate::SessionMessage;

// ── Input model ─────────────────────────────────────────────────────────

/// Complete set of inputs consumed during prompt surface construction.
///
/// This is the single data contract that feeds into `SessionPrompt`'s
/// surface assembly.  Every field maps to one of the five phases:
///
/// | Field              | Phase | What it drives                           |
/// |--------------------|-------|------------------------------------------|
/// | `system_prompt`    | 土    | Product header + compatibility overlay   |
/// | `env_context`      | 土    | Working directory, git, platform, date   |
/// | `preset_extension` | 火    | Preset-specific instructions / tone      |
/// | `tools`            | 金    | Tool schema surface (names + schemas)    |
/// | `memory_prefetch`  | 水→木 | Memory recall injected into user message |
/// | `compiled_request` | 火    | Execution request parameters              |
/// | `provider_options` | 金    | Cache hints, reasoning policy, tool policy|
///
/// # Authority
///
/// `SessionPrompt` is the single assembler.  Presets contribute a
/// `PresetPromptExtension`; providers declare capabilities per profile.
/// Neither constructs the full surface independently.
#[derive(Debug, Clone)]
pub struct PromptSurfaceInputs {
    /// Session identity (used for cache key derivation).
    pub(crate) session_id: String,

    /// Static system prompt text produced by `SystemPrompt`.
    /// This is the product header + compatibility overlay, before
    /// environment context, preset instructions, or tool surface.
    pub(crate) system_prompt: Option<String>,

    /// Environment context block (working directory, git status,
    /// platform, date).  Produced by `SystemPrompt::environment()`.
    pub(crate) env_context: Option<String>,

    /// Preset contribution to the prompt surface.
    /// `None` means no preset is active (e.g. direct / untuned run).
    pub(crate) preset_extension: Option<PresetPromptExtension>,

    /// Memory retrieval packet to be injected into the latest user
    /// message as a `<system-reminder>` block.
    pub(crate) memory_prefetch: Option<MemoryRetrievalPacket>,

    /// Long-lived constraints that belong to the stable prompt prefix.
    pub(crate) pinned_constraints: Vec<PinnedConstraint>,

    /// Stable few-shot examples replayed before live history.
    pub(crate) few_shots: Vec<FewShotSurfaceItem>,

    /// Tool definitions visible to the model.
    pub(crate) tools: Vec<ToolDefinition>,

    /// Source-group digests for tool surface fingerprinting.
    pub(crate) tool_source_digests: Vec<ToolSurfaceSourceDigest>,

    /// Compiled execution request (model, agent, scheduler profile, etc.).
    pub(crate) compiled_request: CompiledExecutionRequest,

    /// Provider-level options that influence the prompt surface:
    /// cache stage/preset/repo, reasoning mode, tool policy, etc.
    /// These are collected by `collect_prompt_surface_provider_options`.
    pub(crate) provider_options: HashMap<String, serde_json::Value>,
}

// ── Output model ─────────────────────────────────────────────────────────

/// Canonical sections of a constructed prompt surface.
///
/// These are the output of `SessionPrompt`'s surface assembly, consumed by:
/// - `loop_lifecycle.rs` (stable fields for cache fingerprinting)
/// - `PromptSurfaceStateSnapshot` (cache evidence)
/// - Provider request construction (actual API call)
///
/// Not every section is present in every surface; `None` means
/// "this section was not produced for this turn."
#[derive(Debug, Clone)]
pub(crate) struct PromptSurfaceSections {
    /// Full system prompt text (product header + env + preset + tools).
    /// This is the model-visible system message.
    pub(crate) system_text: String,

    /// High-stability system prefix rendered ahead of volatile overlays.
    /// This is the provider-visible prefix we try hardest to keep stable.
    pub(crate) stable_system_prefix_text: String,

    /// Dynamic overlay rendered after the stable prefix.
    /// Environment clocks, capability catalogs, and other volatile sections
    /// should accumulate here so they perturb less of the provider-side
    /// cached prefix.
    pub(crate) dynamic_system_overlay_text: String,

    /// Stable system surface hash — the volatile-stripped projection
    /// used for cache fingerprint stability.
    pub(crate) stable_system_surface_hash: String,

    /// Tool surface fingerprint.
    pub(crate) tool_surface_hash: String,

    /// Tool source surface fingerprint.
    pub(crate) tool_source_surface_hash: String,

    /// Provider-level parameters hash (reasoning, tool policy, etc.).
    pub(crate) provider_params_hash: String,

    /// Reasoning-mode policy projection hash.
    pub(crate) reasoning_mode_hash: Option<String>,

    /// Tool-choice / allowed-tools policy projection hash.
    pub(crate) tool_policy_hash: Option<String>,

    /// Preset identity label (e.g. "sisyphus", "atlas").
    pub(crate) preset_identity: Option<String>,

    /// CloseAI-compatible prompt cache key, when the provider family
    /// supports it.
    pub(crate) closeai_prompt_cache_key: Option<String>,

    /// Ingress policy hash for this turn.
    pub(crate) ingress_policy_hash: Option<String>,

    /// Output projection policy hash.
    pub(crate) output_projection_policy_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptSurfaceVolatilityKind {
    VolatileEnvField,
    DynamicCatalogBeforeStableGovernance,
    OversizedCapabilityProjection,
    ProviderOptionsAffectSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptSurfaceVolatilityFinding {
    pub(crate) kind: PromptSurfaceVolatilityKind,
    pub(crate) field: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PromptSurfaceVolatilityReport {
    pub(crate) findings: Vec<PromptSurfaceVolatilityFinding>,
}

impl PromptSurfaceVolatilityReport {
    pub(crate) fn is_empty(&self) -> bool {
        self.findings.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptSurfaceDriftCategory {
    StableSystemSurface,
    ToolSurface,
    ToolSourceSurface,
    ProviderPolicy,
    ReasoningMode,
    ToolPolicy,
    OutputProjection,
    IngressPolicy,
    CloseAiPromptCacheKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptSurfaceDriftExplanation {
    pub(crate) category: PromptSurfaceDriftCategory,
    pub(crate) field: String,
    pub(crate) detail: String,
    pub(crate) severity: SessionCacheSeverity,
}

// ── Preset contribution (re-export from agendao-types) ─────────────────
//
// The canonical PresetPromptExtension lives in agendao-types to avoid
// a circular dependency between agendao-session and agendao-orchestrator.
// This re-export keeps the surface_authority module as the single
// namespace for prompt surface types within the session crate.
pub use agendao_types::PresetPromptExtension;

#[derive(Debug, Clone, Default)]
struct PromptSurfaceSystemLayers {
    stable_prefix_sections: Vec<String>,
    dynamic_overlay_sections: Vec<String>,
}

impl PromptSurfaceSystemLayers {
    fn stable_system_prefix_text(&self) -> String {
        self.stable_prefix_sections.join("\n\n")
    }

    fn dynamic_system_overlay_text(&self) -> String {
        self.dynamic_overlay_sections.join("\n\n")
    }

    fn system_text(&self) -> String {
        let mut sections = Vec::new();
        let stable = self.stable_system_prefix_text();
        if !stable.is_empty() {
            sections.push(stable);
        }
        let dynamic = self.dynamic_system_overlay_text();
        if !dynamic.is_empty() {
            sections.push(dynamic);
        }
        sections.join("\n\n")
    }
}

// ── Construction helpers (skeleton, no callers yet) ─────────────────────

impl PromptSurfaceInputs {
    /// Construct inputs from the pieces that `SessionPrompt` already
    /// holds internally.
    ///
    /// Compatibility shim during the Phase 7 builder migration.
    /// New production call sites should prefer `builder(...).set_*()`.
    pub(crate) fn from_session_prompt_parts(
        session_id: impl Into<String>,
        system_prompt: Option<String>,
        env_context: Option<String>,
        preset_extension: Option<PresetPromptExtension>,
        memory_prefetch: Option<MemoryRetrievalPacket>,
        pinned_constraints: Vec<PinnedConstraint>,
        few_shots: Vec<FewShotSurfaceItem>,
        tools: Vec<ToolDefinition>,
        tool_source_digests: Vec<ToolSurfaceSourceDigest>,
        compiled_request: CompiledExecutionRequest,
        provider_options: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self::builder(session_id, compiled_request)
            .set_base_system_prompt(system_prompt)
            .set_environment_identity(env_context)
            .set_preset_extension(preset_extension)
            .set_memory_prefetch(memory_prefetch)
            .set_pinned_constraints(pinned_constraints)
            .set_few_shots(few_shots)
            .set_tool_surface(tools, tool_source_digests)
            .set_provider_options(provider_options)
    }

    pub fn builder(
        session_id: impl Into<String>,
        compiled_request: CompiledExecutionRequest,
    ) -> Self {
        let provider_options = compiled_request.provider_options.clone().unwrap_or_default();
        Self {
            session_id: session_id.into(),
            system_prompt: None,
            env_context: None,
            preset_extension: None,
            memory_prefetch: None,
            pinned_constraints: Vec::new(),
            few_shots: Vec::new(),
            tools: Vec::new(),
            tool_source_digests: Vec::new(),
            compiled_request,
            provider_options,
        }
    }

    pub fn set_base_system_prompt(mut self, system_prompt: Option<String>) -> Self {
        self.system_prompt = system_prompt;
        self
    }

    pub fn set_environment_identity(mut self, env_context: Option<String>) -> Self {
        self.env_context = env_context;
        self
    }

    pub fn set_preset_extension(
        mut self,
        preset_extension: Option<PresetPromptExtension>,
    ) -> Self {
        self.preset_extension = preset_extension;
        self
    }

    pub fn set_memory_prefetch(
        mut self,
        memory_prefetch: Option<MemoryRetrievalPacket>,
    ) -> Self {
        self.memory_prefetch = memory_prefetch;
        self
    }

    pub fn set_pinned_constraints(mut self, pinned_constraints: Vec<PinnedConstraint>) -> Self {
        self.pinned_constraints = pinned_constraints;
        self
    }

    pub fn set_few_shots(mut self, few_shots: Vec<FewShotSurfaceItem>) -> Self {
        self.few_shots = few_shots;
        self
    }

    pub fn set_tool_surface(
        mut self,
        tools: Vec<ToolDefinition>,
        tool_source_digests: Vec<ToolSurfaceSourceDigest>,
    ) -> Self {
        self.tools = tools;
        self.tool_source_digests = tool_source_digests;
        self
    }

    pub fn set_provider_options(
        mut self,
        provider_options: HashMap<String, serde_json::Value>,
    ) -> Self {
        self.provider_options = provider_options;
        self
    }

    pub(crate) fn assemble_sections(
        &self,
        output_projection_policy_hash: String,
        ingress_policy_hash: Option<String>,
        closeai_prompt_cache_key: Option<String>,
    ) -> PromptSurfaceSections {
        let tool_surface_hash = agendao_provider::cache::tool_surface_fingerprint(&self.tools);
        let tool_source_surface_hash =
            agendao_provider::cache::tool_source_surface_fingerprint(&if self
                .tool_source_digests
                .is_empty()
            {
                vec![ToolSurfaceSourceDigest {
                    source: agendao_provider::cache::ToolSurfaceSourceKind::Base,
                    tool_count: self.tools.len(),
                    tools_hash: tool_surface_hash.clone(),
                }]
            } else {
                self.tool_source_digests.clone()
            });
        let provider_params_hash = json_fingerprint(&serde_json::json!({
            "max_tokens": self.compiled_request.max_tokens,
            "temperature": self.compiled_request.temperature,
            "top_p": self.compiled_request.top_p,
            "variant": self.compiled_request.variant,
            "provider_options": self.compiled_request.provider_options,
        }));
        let reasoning_mode_hash = provider_option_hash(
            &self.provider_options,
            PromptSurfaceProviderOptionGroup::ReasoningMode,
        );
        let tool_policy_hash = provider_option_hash(
            &self.provider_options,
            PromptSurfaceProviderOptionGroup::ToolPolicy,
        );
        let system_layers = self.assemble_system_layers();
        let system_text = system_layers.system_text();
        let stable_system_prefix_text = system_layers.stable_system_prefix_text();
        let dynamic_system_overlay_text = system_layers.dynamic_system_overlay_text();
        let stable_system_surface_hash =
            text_fingerprint(&stable_system_surface_projection(&stable_system_prefix_text));

        PromptSurfaceSections {
            system_text,
            stable_system_prefix_text,
            dynamic_system_overlay_text,
            stable_system_surface_hash,
            tool_surface_hash,
            tool_source_surface_hash,
            provider_params_hash,
            reasoning_mode_hash,
            tool_policy_hash,
            preset_identity: self
                .preset_extension
                .as_ref()
                .map(|preset| preset.preset_name.clone()),
            closeai_prompt_cache_key,
            ingress_policy_hash,
            output_projection_policy_hash,
        }
    }

    pub(crate) fn detect_volatility(&self) -> PromptSurfaceVolatilityReport {
        let mut findings = Vec::new();

        if let Some(env_context) = self.env_context.as_deref() {
            for line in env_context.lines() {
                if looks_like_clock_line(line) {
                    findings.push(PromptSurfaceVolatilityFinding {
                        kind: PromptSurfaceVolatilityKind::VolatileEnvField,
                        field: "env_context".to_string(),
                        detail: line.trim().to_string(),
                    });
                }
            }
        }

        if let Some(extension) = self.preset_extension.as_ref() {
            if let Some(capability_projection) = extension.capability_projection.as_deref() {
                let trimmed = capability_projection.trim();
                if trimmed.len() > 2_000 {
                    findings.push(PromptSurfaceVolatilityFinding {
                        kind: PromptSurfaceVolatilityKind::OversizedCapabilityProjection,
                        field: "capability_projection".to_string(),
                        detail: format!("{} chars", trimmed.len()),
                    });
                }
            }

            let layers = render_preset_extension_layers(extension);
            if !layers.dynamic_overlay_sections.is_empty()
                && layers.stable_prefix_sections.is_empty()
            {
                findings.push(PromptSurfaceVolatilityFinding {
                    kind: PromptSurfaceVolatilityKind::DynamicCatalogBeforeStableGovernance,
                    field: "preset_extension".to_string(),
                    detail: "dynamic catalog has no stable governance prefix ahead of it"
                        .to_string(),
                });
            }
        }

        let reasoning = collect_prompt_surface_provider_options(
            &self.provider_options,
            PromptSurfaceProviderOptionGroup::ReasoningMode,
        );
        let tool_policy = collect_prompt_surface_provider_options(
            &self.provider_options,
            PromptSurfaceProviderOptionGroup::ToolPolicy,
        );
        if !reasoning.is_empty() || !tool_policy.is_empty() {
            findings.push(PromptSurfaceVolatilityFinding {
                kind: PromptSurfaceVolatilityKind::ProviderOptionsAffectSurface,
                field: "provider_options".to_string(),
                detail: format!(
                    "reasoning keys: {} · tool policy keys: {}",
                    reasoning.len(),
                    tool_policy.len()
                ),
            });
        }

        PromptSurfaceVolatilityReport { findings }
    }

    fn assemble_system_layers(&self) -> PromptSurfaceSystemLayers {
        let mut layers = PromptSurfaceSystemLayers::default();

        if let Some(system_prompt) = self
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layers.stable_prefix_sections.push(system_prompt.to_string());
        }

        if let Some(env_context) = self
            .env_context
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layers
                .dynamic_overlay_sections
                .push(format!("## Environment Context\n{env_context}"));
        }

        if let Some(preset_extension) = self.preset_extension.as_ref() {
            let preset_layers = render_preset_extension_layers(preset_extension);
            layers
                .stable_prefix_sections
                .extend(preset_layers.stable_prefix_sections);
            layers
                .dynamic_overlay_sections
                .extend(preset_layers.dynamic_overlay_sections);
        }

        let pinned_constraints = self
            .pinned_constraints
            .iter()
            .filter_map(|constraint| {
                let title = constraint.title.trim();
                let body = constraint.body.trim();
                (!body.is_empty()).then(|| {
                    if title.is_empty() {
                        body.to_string()
                    } else {
                        format!("### {title}\n{body}")
                    }
                })
            })
            .collect::<Vec<_>>();
        if !pinned_constraints.is_empty() {
            layers.stable_prefix_sections.push(format!(
                "## Pinned Constraints\n{}",
                pinned_constraints.join("\n\n")
            ));
        }

        layers
    }

    pub(crate) fn effective_tool_source_digests(
        base_source_digests: &[ToolSurfaceSourceDigest],
        base_tools: &[ToolDefinition],
        extra_tools: &[ToolDefinition],
    ) -> Vec<ToolSurfaceSourceDigest> {
        let base_names = base_tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<std::collections::HashSet<_>>();
        let effective_extra = extra_tools
            .iter()
            .filter(|tool| !base_names.contains(&tool.name))
            .cloned()
            .collect::<Vec<_>>();

        let mut groups = if base_source_digests.is_empty() {
            vec![ToolSurfaceSourceDigest {
                source: agendao_provider::cache::ToolSurfaceSourceKind::Base,
                tool_count: base_tools.len(),
                tools_hash: agendao_provider::cache::tool_surface_fingerprint(base_tools),
            }]
        } else {
            base_source_digests.to_vec()
        };
        groups.push(ToolSurfaceSourceDigest {
            source: agendao_provider::cache::ToolSurfaceSourceKind::Mcp,
            tool_count: effective_extra.len(),
            tools_hash: agendao_provider::cache::tool_surface_fingerprint(&effective_extra),
        });
        groups
    }

    pub(crate) fn output_projection_policy_hash(prompt_messages: &[SessionMessage]) -> String {
        let projected = prompt_messages
            .iter()
            .filter_map(sanctioned_model_context_projection_for_message)
            .map(|projection| {
                serde_json::json!({
                    "path": projection.path.as_str(),
                    "policy": projection.policy,
                    "legacy_without_policy": projection.legacy_without_policy,
                })
            })
            .collect::<Vec<_>>();

        json_fingerprint(&serde_json::json!({
            "owner": "sanctioned_model_context_projection",
            "entries": projected,
        }))
    }
}

impl PromptSurfaceSections {
    pub(crate) fn describe_drift(
        &self,
        previous: &Self,
    ) -> Vec<PromptSurfaceDriftExplanation> {
        let mut changes = Vec::new();

        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::StableSystemSurface,
            "stableSystemSurfaceHash",
            previous.stable_system_surface_hash != self.stable_system_surface_hash,
            SessionCacheSeverity::HighChange,
            "stable system surface changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::ToolSurface,
            "toolSurfaceHash",
            previous.tool_surface_hash != self.tool_surface_hash,
            SessionCacheSeverity::HighChange,
            "tool surface changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::ToolSourceSurface,
            "toolSourceSurfaceHash",
            previous.tool_source_surface_hash != self.tool_source_surface_hash,
            SessionCacheSeverity::HighChange,
            "tool source surface changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::ProviderPolicy,
            "providerParamsHash",
            previous.provider_params_hash != self.provider_params_hash,
            SessionCacheSeverity::HighChange,
            "provider params changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::ReasoningMode,
            "reasoningModeHash",
            previous.reasoning_mode_hash != self.reasoning_mode_hash,
            SessionCacheSeverity::MediumChange,
            "reasoning mode projection changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::ToolPolicy,
            "toolPolicyHash",
            previous.tool_policy_hash != self.tool_policy_hash,
            SessionCacheSeverity::MediumChange,
            "tool policy projection changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::OutputProjection,
            "outputProjectionPolicyHash",
            previous.output_projection_policy_hash != self.output_projection_policy_hash,
            SessionCacheSeverity::MediumChange,
            "output projection policy changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::IngressPolicy,
            "ingressPolicyHash",
            previous.ingress_policy_hash != self.ingress_policy_hash,
            SessionCacheSeverity::LowChange,
            "ingress policy changed",
        );
        push_drift(
            &mut changes,
            PromptSurfaceDriftCategory::CloseAiPromptCacheKey,
            "closeaiPromptCacheKey",
            previous.closeai_prompt_cache_key != self.closeai_prompt_cache_key,
            SessionCacheSeverity::MediumChange,
            "closeai prompt cache key changed",
        );

        changes
    }

    pub(crate) fn to_prompt_surface_evidence_summary(
        &self,
        previous: &Self,
        volatility_report: Option<&PromptSurfaceVolatilityReport>,
    ) -> Option<PromptSurfaceEvidenceSummary> {
        let drift = self.describe_drift(previous);
        let volatility_findings = volatility_report
            .map(|report| {
                report
                    .findings
                    .iter()
                    .cloned()
                    .map(PublicPromptSurfaceVolatilityFinding::from)
                    .collect::<Vec<PublicPromptSurfaceVolatilityFinding>>()
            })
            .unwrap_or_default();
        if drift.is_empty() && volatility_findings.is_empty() {
            return None;
        }

        let changed_fields = drift.iter().map(|item| item.field.clone()).collect::<Vec<_>>();
        let drift_details = drift.into_iter().map(Into::into).collect::<Vec<_>>();
        let stable_prefix_change = changed_fields
            .iter()
            .any(|field| field == "stableSystemSurfaceHash")
            .then_some(true);
        let dynamic_overlay_reasons = volatility_findings
            .iter()
            .map(|finding| match finding.kind {
                PublicPromptSurfaceVolatilityKind::VolatileEnvField => {
                    format!("dynamic env field · {}", finding.detail)
                }
                PublicPromptSurfaceVolatilityKind::DynamicCatalogBeforeStableGovernance => {
                    finding.detail.clone()
                }
                PublicPromptSurfaceVolatilityKind::OversizedCapabilityProjection => {
                    format!("oversized capability projection · {}", finding.detail)
                }
                PublicPromptSurfaceVolatilityKind::ProviderOptionsAffectSurface => {
                    format!("provider options affect surface · {}", finding.detail)
                }
            })
            .collect::<Vec<_>>();
        let severity = drift_details
            .iter()
            .map(|item: &PromptSurfaceDriftDetail| item.severity)
            .max()
            .unwrap_or(SessionCacheSeverity::LowChange);
        let reason = if changed_fields.is_empty() {
            "surface volatility detected".to_string()
        } else {
            format!("surface changed: {}", changed_fields.join(", "))
        };

        Some(PromptSurfaceEvidenceSummary {
            severity,
            reason,
            changed_fields,
            stable_prefix_change,
            dynamic_overlay_reasons,
            drift_details,
            volatility_findings,
        })
    }
}

impl From<PromptSurfaceDriftCategory> for PublicPromptSurfaceDriftCategory {
    fn from(value: PromptSurfaceDriftCategory) -> Self {
        match value {
            PromptSurfaceDriftCategory::StableSystemSurface => Self::StableSystemSurface,
            PromptSurfaceDriftCategory::ToolSurface => Self::ToolSurface,
            PromptSurfaceDriftCategory::ToolSourceSurface => Self::ToolSourceSurface,
            PromptSurfaceDriftCategory::ProviderPolicy => Self::ProviderPolicy,
            PromptSurfaceDriftCategory::ReasoningMode => Self::ReasoningMode,
            PromptSurfaceDriftCategory::ToolPolicy => Self::ToolPolicy,
            PromptSurfaceDriftCategory::OutputProjection => Self::OutputProjection,
            PromptSurfaceDriftCategory::IngressPolicy => Self::IngressPolicy,
            PromptSurfaceDriftCategory::CloseAiPromptCacheKey => Self::CloseAiPromptCacheKey,
        }
    }
}

impl From<PromptSurfaceDriftExplanation> for PromptSurfaceDriftDetail {
    fn from(value: PromptSurfaceDriftExplanation) -> Self {
        Self {
            category: value.category.into(),
            field: value.field,
            detail: value.detail,
            severity: value.severity,
        }
    }
}

impl From<PromptSurfaceVolatilityKind> for PublicPromptSurfaceVolatilityKind {
    fn from(value: PromptSurfaceVolatilityKind) -> Self {
        match value {
            PromptSurfaceVolatilityKind::VolatileEnvField => Self::VolatileEnvField,
            PromptSurfaceVolatilityKind::DynamicCatalogBeforeStableGovernance => {
                Self::DynamicCatalogBeforeStableGovernance
            }
            PromptSurfaceVolatilityKind::OversizedCapabilityProjection => {
                Self::OversizedCapabilityProjection
            }
            PromptSurfaceVolatilityKind::ProviderOptionsAffectSurface => {
                Self::ProviderOptionsAffectSurface
            }
        }
    }
}

impl From<PromptSurfaceVolatilityFinding> for PublicPromptSurfaceVolatilityFinding {
    fn from(value: PromptSurfaceVolatilityFinding) -> Self {
        Self {
            kind: value.kind.into(),
            field: value.field,
            detail: value.detail,
        }
    }
}

fn stable_system_surface_projection(system_prompt: &str) -> String {
    let mut lines = Vec::new();
    let mut skipping_section = false;

    for line in system_prompt.lines() {
        if let Some(header) = line.strip_prefix("## ") {
            let title = header.trim();
            skipping_section = is_volatile_system_section(title);
            if skipping_section {
                continue;
            }
        }

        if skipping_section {
            continue;
        }

        lines.push(normalize_stable_system_line(line).into_owned());
    }

    lines.join("\n")
}

fn render_preset_extension_layers(extension: &PresetPromptExtension) -> PromptSurfaceSystemLayers {
    let mut layers = PromptSurfaceSystemLayers::default();

    let role_summary = extension.role_summary.trim();
    if !role_summary.is_empty() {
        layers
            .stable_prefix_sections
            .push(format!("## Preset Role Summary\n{role_summary}"));
    }

    for (title, body) in &extension.extra_sections {
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        if is_dynamic_catalog_header(title) {
            layers.dynamic_overlay_sections.push(body.to_string());
        } else if is_stable_governance_header(title) {
            layers.stable_prefix_sections.push(body.to_string());
        } else {
            layers.stable_prefix_sections.push(body.to_string());
        }
    }

    if let Some(tone_augment) = extension
        .tone_augment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        layers
            .stable_prefix_sections
            .push(format!("## Tone Augment\n{tone_augment}"));
    }

    // Keep large runtime capability catalogs late so the prompt prefix
    // stays anchored by higher-stability preset governance text.
    if let Some(capability_projection) = extension
        .capability_projection
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        layers.dynamic_overlay_sections.push(format!(
            "## Capability Projection\n{capability_projection}"
        ));
    }

    layers
}

fn provider_option_hash(
    provider_options: &HashMap<String, serde_json::Value>,
    group: PromptSurfaceProviderOptionGroup,
) -> Option<String> {
    let relevant = collect_prompt_surface_provider_options(provider_options, group);
    (!relevant.is_empty()).then(|| json_fingerprint(&serde_json::Value::Object(relevant)))
}

fn push_drift(
    changes: &mut Vec<PromptSurfaceDriftExplanation>,
    category: PromptSurfaceDriftCategory,
    field: &str,
    changed: bool,
    severity: SessionCacheSeverity,
    detail: &str,
) {
    if changed {
        changes.push(PromptSurfaceDriftExplanation {
            category,
            field: field.to_string(),
            detail: detail.to_string(),
            severity,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Smoke tests: verify structures construct and hold data ──────────

    #[test]
    fn surface_inputs_constructor_populates_all_fields() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("system header".to_string()),
            Some("env: linux".to_string()),
            Some(PresetPromptExtension::new(
                "sisyphus",
                "delegation-first orchestrator",
            )),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        assert_eq!(inputs.session_id, "ses-1");
        assert_eq!(inputs.system_prompt.as_deref(), Some("system header"));
        assert_eq!(inputs.env_context.as_deref(), Some("env: linux"));
        assert!(inputs.preset_extension.is_some());
        assert_eq!(
            inputs.preset_extension.as_ref().unwrap().preset_name,
            "sisyphus"
        );
        assert!(inputs.memory_prefetch.is_none());
        assert!(inputs.tools.is_empty());
    }

    #[test]
    fn builder_setters_match_compat_constructor_shape() {
        let via_compat = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("system header".to_string()),
            Some("env: linux".to_string()),
            Some(PresetPromptExtension::new(
                "sisyphus",
                "delegation-first orchestrator",
            )),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::from([("thinking".to_string(), serde_json::json!(true))]),
        );

        let via_builder = PromptSurfaceInputs::builder("ses-1", CompiledExecutionRequest::default())
            .set_base_system_prompt(Some("system header".to_string()))
            .set_environment_identity(Some("env: linux".to_string()))
            .set_preset_extension(Some(PresetPromptExtension::new(
                "sisyphus",
                "delegation-first orchestrator",
            )))
            .set_memory_prefetch(None)
            .set_tool_surface(vec![], vec![])
            .set_provider_options(HashMap::from([(
                "thinking".to_string(),
                serde_json::json!(true),
            )]));

        assert_eq!(via_builder.session_id, via_compat.session_id);
        assert_eq!(via_builder.system_prompt, via_compat.system_prompt);
        assert_eq!(via_builder.env_context, via_compat.env_context);
        assert_eq!(via_builder.preset_extension, via_compat.preset_extension);
        assert_eq!(via_builder.memory_prefetch, via_compat.memory_prefetch);
        assert_eq!(via_builder.tools.len(), via_compat.tools.len());
        assert_eq!(
            via_builder.tool_source_digests,
            via_compat.tool_source_digests
        );
        assert_eq!(via_builder.provider_options, via_compat.provider_options);
    }

    #[test]
    fn pinned_constraints_flow_into_stable_prefix_and_hash() {
        let first = PromptSurfaceInputs::builder(
            "ses-pinned",
            CompiledExecutionRequest::default(),
        )
        .set_base_system_prompt(Some("base header".to_string()))
        .set_pinned_constraints(vec![
            PinnedConstraint::new("Boundary", "Never bypass verification."),
            PinnedConstraint::new(
                "Continuity",
                "Preserve acceptance constraints across compaction.",
            ),
        ])
        .assemble_sections("projection".to_string(), None, None);

        let second = PromptSurfaceInputs::builder(
            "ses-pinned",
            CompiledExecutionRequest::default(),
        )
        .set_base_system_prompt(Some("base header".to_string()))
        .set_pinned_constraints(vec![PinnedConstraint::new(
            "Boundary",
            "Never bypass verification.",
        )])
        .assemble_sections("projection".to_string(), None, None);

        assert!(first.system_text.contains("## Pinned Constraints"));
        assert!(first
            .stable_system_prefix_text
            .contains("Preserve acceptance constraints across compaction."));
        assert_ne!(
            first.stable_system_surface_hash, second.stable_system_surface_hash,
            "changing pinned constraints must change the stable prefix hash"
        );
    }

    #[test]
    fn preset_extension_builds_with_sections_and_augments() {
        let ext = PresetPromptExtension::new("atlas", "coordination orchestrator")
            .with_section("Execution Charter", "Atlas Mode: delegate and verify.")
            .with_section("Guardrails", "Never pretend delegated work was completed.")
            .with_capability("Agents: explore, code-review, librarian, oracle.")
            .with_tone_augment("Be concise. No flattery.");

        assert_eq!(ext.preset_name, "atlas");
        assert_eq!(ext.role_summary, "coordination orchestrator");
        assert_eq!(ext.extra_sections.len(), 2);
        assert_eq!(ext.extra_sections[0].0, "Execution Charter");
        assert_eq!(ext.extra_sections[1].0, "Guardrails");
        assert!(ext.capability_projection.is_some());
        assert!(ext.tone_augment.is_some());
    }

    #[test]
    fn preset_extension_supports_minimal_identity_only() {
        let ext = PresetPromptExtension::new("hephaestus", "autonomous deep executor");

        assert_eq!(ext.preset_name, "hephaestus");
        assert!(ext.extra_sections.is_empty());
        assert!(ext.capability_projection.is_none());
        assert!(ext.tone_augment.is_none());
    }

    #[test]
    fn assemble_sections_projects_provider_policy_hashes() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("system".to_string()),
            None,
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest {
                provider_options: Some(HashMap::from([
                    ("thinking".to_string(), serde_json::json!(true)),
                    ("tool_choice".to_string(), serde_json::json!("auto")),
                ])),
                ..Default::default()
            },
            HashMap::from([
                ("thinking".to_string(), serde_json::json!(true)),
                ("tool_choice".to_string(), serde_json::json!("auto")),
            ]),
        );

        let sections = inputs.assemble_sections("projection".to_string(), None, None);

        assert!(sections.reasoning_mode_hash.is_some());
        assert!(sections.tool_policy_hash.is_some());
    }

    #[test]
    fn assemble_sections_merges_system_env_and_preset_extension() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("base header".to_string()),
            Some("<env>\n  Working directory: /repo\n</env>".to_string()),
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_section("Identity", "<identity>Atlas</identity>")
                    .with_capability("Agents: explore, review.")
                    .with_tone_augment("Be concise. No flattery."),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let sections = inputs.assemble_sections("projection".to_string(), None, None);

        assert!(sections.system_text.contains("base header"));
        assert!(sections.stable_system_prefix_text.contains("base header"));
        assert!(sections.system_text.contains("## Environment Context"));
        assert!(sections
            .dynamic_system_overlay_text
            .contains("## Environment Context"));
        assert!(sections.system_text.contains("Working directory: /repo"));
        assert!(sections.system_text.contains("## Preset Role Summary"));
        assert!(sections
            .stable_system_prefix_text
            .contains("## Preset Role Summary"));
        assert!(sections.system_text.contains("coordination orchestrator"));
        assert!(sections.system_text.contains("<identity>Atlas</identity>"));
        assert!(sections.system_text.contains("## Capability Projection"));
        assert!(sections
            .dynamic_system_overlay_text
            .contains("## Capability Projection"));
        assert!(sections.system_text.contains("Agents: explore, review."));
        assert!(sections.system_text.contains("## Tone Augment"));
        assert!(sections.system_text.contains("Be concise. No flattery."));
        assert!(
            sections.system_text.find("## Tone Augment").unwrap()
                < sections.system_text.find("## Capability Projection").unwrap()
        );
        assert!(!sections
            .stable_system_prefix_text
            .contains("## Capability Projection"));
        assert!(!sections
            .dynamic_system_overlay_text
            .contains("## Tone Augment"));
    }

    #[test]
    fn environment_context_surface_stays_cache_friendly() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-cache",
            Some("base header".to_string()),
            Some(
                "You are powered by the model named gpt-5. The exact model ID is openai/gpt-5\nHere is some useful information about the environment you are running in:\n<env>\n  Working directory: /repo\n  Is directory a git repo: yes\n  Platform: linux\n</env>"
                    .to_string(),
            ),
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let sections = inputs.assemble_sections("projection".to_string(), None, None);

        assert!(sections.system_text.contains("## Environment Context"));
        assert!(sections.dynamic_system_overlay_text.contains("## Environment Context"));
        assert!(sections.system_text.contains("Working directory: /repo"));
        assert!(!sections.system_text.contains("Today's date:"));
        assert!(!sections.system_text.contains("Current local time:"));
        assert!(!sections.system_text.contains("Local timezone:"));
    }

    #[test]
    fn stable_hash_depends_on_stable_prefix_not_dynamic_overlay_order() {
        let first = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-a",
            Some("base header".to_string()),
            Some(
                "<env>\n  Working directory: /repo\n  Current local time: 10:10:10 UTC\n</env>"
                    .to_string(),
            ),
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_section("Execution Charter", "Always verify delegated work.")
                    .with_capability("Agents: explore, review."),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        )
        .assemble_sections("projection".to_string(), None, None);

        let second = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-a",
            Some("base header".to_string()),
            Some(
                "<env>\n  Working directory: /repo\n  Current local time: 22:22:22 UTC\n</env>"
                    .to_string(),
            ),
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_capability("Agents: explore, review, build.")
                    .with_section("Execution Charter", "Always verify delegated work."),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        )
        .assemble_sections("projection".to_string(), None, None);

        assert_eq!(
            first.stable_system_surface_hash, second.stable_system_surface_hash,
            "dynamic overlay drift must not perturb stable prefix hash"
        );
        assert_eq!(
            first.stable_system_prefix_text, second.stable_system_prefix_text,
            "stable prefix should remain identical across dynamic overlay changes"
        );
        assert_ne!(
            first.dynamic_system_overlay_text, second.dynamic_system_overlay_text,
            "dynamic overlay should still reflect live env/catalog drift"
        );
    }

    #[test]
    fn volatility_report_flags_dynamic_clock_lines() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-clock",
            Some("base header".to_string()),
            Some(
                "<env>\n  Working directory: /repo\n  Today's date: Tue Jun 10 2026\n  Current local time: 10:10:10 UTC\n</env>"
                    .to_string(),
            ),
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let report = inputs.detect_volatility();
        assert!(report.findings.iter().any(|finding| matches!(
            finding.kind,
            PromptSurfaceVolatilityKind::VolatileEnvField
        )));
    }

    #[test]
    fn volatility_report_flags_oversized_capability_projection() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-caps",
            Some("base header".to_string()),
            None,
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_capability("A".repeat(2_100)),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let report = inputs.detect_volatility();
        assert!(report.findings.iter().any(|finding| matches!(
            finding.kind,
            PromptSurfaceVolatilityKind::OversizedCapabilityProjection
        )));
    }

    #[test]
    fn drift_explain_keeps_dynamic_overlay_out_of_stable_hash() {
        let first = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("base header".to_string()),
            Some("<env>\n  Working directory: /repo-a\n</env>".to_string()),
            Some(PresetPromptExtension::new("atlas", "coordination orchestrator")),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        )
        .assemble_sections("projection".to_string(), None, None);

        let second = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("base header".to_string()),
            Some("<env>\n  Working directory: /repo-b\n</env>".to_string()),
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_capability("Agents: explore, review."),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        )
        .assemble_sections("projection".to_string(), None, None);

        let drift = second.describe_drift(&first);
        assert!(!drift
            .iter()
            .any(|item| item.field == "stableSystemSurfaceHash"));
        assert_eq!(
            first.stable_system_surface_hash, second.stable_system_surface_hash,
            "env/capability changes should live in the dynamic overlay, not the stable hash"
        );
        assert_ne!(
            first.dynamic_system_overlay_text, second.dynamic_system_overlay_text,
            "dynamic overlay should still expose the changed env/catalog surface"
        );

        let volatility = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("base header".to_string()),
            Some(
                "<env>\n  Working directory: /repo-b\n  Current local time: 10:10:10 UTC\n</env>"
                    .to_string(),
            ),
            Some(
                PresetPromptExtension::new("atlas", "coordination orchestrator")
                    .with_capability("Agents: explore, review."),
            ),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        )
        .detect_volatility();

        let evidence = second
            .to_prompt_surface_evidence_summary(&first, Some(&volatility))
            .expect("surface evidence");
        assert!(evidence
            .volatility_findings
            .iter()
            .any(|detail| matches!(
                detail.kind,
                agendao_types::PromptSurfaceVolatilityKind::VolatileEnvField
            )));
    }

    #[test]
    fn evidence_summary_can_surface_volatility_without_hash_drift() {
        let inputs = PromptSurfaceInputs::from_session_prompt_parts(
            "ses-1",
            Some("base header".to_string()),
            Some(
                "<env>\n  Working directory: /repo\n  Current local time: 10:10:10 UTC\n</env>"
                    .to_string(),
            ),
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );
        let first = inputs.assemble_sections("projection".to_string(), None, None);
        let second = inputs.assemble_sections("projection".to_string(), None, None);
        let report = inputs.detect_volatility();

        let evidence = second
            .to_prompt_surface_evidence_summary(&first, Some(&report))
            .expect("volatility evidence");

        assert!(evidence.changed_fields.is_empty());
        assert_eq!(evidence.reason, "surface volatility detected");
        assert!(evidence
            .volatility_findings
            .iter()
            .any(|finding| matches!(
                finding.kind,
                agendao_types::PromptSurfaceVolatilityKind::VolatileEnvField
            )));
    }

    #[test]
    fn effective_tool_source_digests_preserve_base_and_append_mcp_group() {
        let base = vec![ToolDefinition {
            name: "read".to_string(),
            description: Some("read files".to_string()),
            parameters: serde_json::json!({"type":"object"}),
        }];
        let extra = vec![
            ToolDefinition {
                name: "read".to_string(),
                description: Some("duplicate".to_string()),
                parameters: serde_json::json!({"type":"object"}),
            },
            ToolDefinition {
                name: "grep".to_string(),
                description: Some("search".to_string()),
                parameters: serde_json::json!({"type":"object"}),
            },
        ];

        let digests = PromptSurfaceInputs::effective_tool_source_digests(&[], &base, &extra);

        assert_eq!(digests.len(), 2);
        assert_eq!(
            digests[0].source,
            agendao_provider::cache::ToolSurfaceSourceKind::Base
        );
        assert_eq!(
            digests[1].source,
            agendao_provider::cache::ToolSurfaceSourceKind::Mcp
        );
        assert_eq!(digests[1].tool_count, 1);
    }
}
