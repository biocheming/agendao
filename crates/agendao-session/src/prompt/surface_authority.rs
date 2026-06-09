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
//! All types and fields are `pub(crate)` — this module is a **crate-internal
//! skeleton**, not a stable public API.  Visibility will be widened only
//! after Phase 7 when the contract is proven.

// Skeleton types are intentionally unused until Phase 5 cut-over.
#![allow(dead_code)]

use std::collections::HashMap;

use agendao_execution_types::CompiledExecutionRequest;
use agendao_provider::cache::ToolSurfaceSourceDigest;
use agendao_provider::cache::{json_fingerprint, text_fingerprint};
use agendao_provider::ToolDefinition;
use agendao_types::MemoryRetrievalPacket;

use super::surface_contract::{
    collect_prompt_surface_provider_options, is_volatile_system_section,
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
pub(crate) struct PromptSurfaceInputs {
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

// ── Preset contribution (re-export from agendao-types) ─────────────────
//
// The canonical PresetPromptExtension lives in agendao-types to avoid
// a circular dependency between agendao-session and agendao-orchestrator.
// This re-export keeps the surface_authority module as the single
// namespace for prompt surface types within the session crate.
pub use agendao_types::PresetPromptExtension;

// ── Construction helpers (skeleton, no callers yet) ─────────────────────

impl PromptSurfaceInputs {
    /// Construct inputs from the pieces that `SessionPrompt` already
    /// holds internally.
    ///
    /// This is the eventual replacement for scattered builder calls.
    /// In Phase 5, session-side consumers (loop_lifecycle,
    /// message_building) will switch to reading from this view.
    pub(crate) fn from_session_prompt_parts(
        session_id: impl Into<String>,
        system_prompt: Option<String>,
        env_context: Option<String>,
        preset_extension: Option<PresetPromptExtension>,
        memory_prefetch: Option<MemoryRetrievalPacket>,
        tools: Vec<ToolDefinition>,
        tool_source_digests: Vec<ToolSurfaceSourceDigest>,
        compiled_request: CompiledExecutionRequest,
        provider_options: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            system_prompt,
            env_context,
            preset_extension,
            memory_prefetch,
            tools,
            tool_source_digests,
            compiled_request,
            provider_options,
        }
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
        let system_text = self.assemble_system_text();
        let stable_system_surface_hash =
            text_fingerprint(&stable_system_surface_projection(&system_text));

        PromptSurfaceSections {
            system_text,
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

    fn assemble_system_text(&self) -> String {
        let mut sections = Vec::new();

        if let Some(system_prompt) = self
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(system_prompt.to_string());
        }

        if let Some(env_context) = self
            .env_context
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(format!("## Environment Context\n{env_context}"));
        }

        if let Some(preset_extension) = self.preset_extension.as_ref() {
            let preset_text = render_preset_extension_for_surface(preset_extension);
            if !preset_text.is_empty() {
                sections.push(preset_text);
            }
        }

        sections.join("\n\n")
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

fn render_preset_extension_for_surface(extension: &PresetPromptExtension) -> String {
    let mut sections = Vec::new();

    let role_summary = extension.role_summary.trim();
    if !role_summary.is_empty() {
        sections.push(format!("## Preset Role Summary\n{role_summary}"));
    }

    sections.extend(
        extension
            .extra_sections
            .iter()
            .map(|(_, body)| body.trim())
            .filter(|body| !body.is_empty())
            .map(str::to_string),
    );

    if let Some(tone_augment) = extension
        .tone_augment
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!("## Tone Augment\n{tone_augment}"));
    }

    // Keep large runtime capability catalogs late so the prompt prefix
    // stays anchored by higher-stability preset governance text.
    if let Some(capability_projection) = extension
        .capability_projection
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(format!(
            "## Capability Projection\n{capability_projection}"
        ));
    }

    sections.join("\n\n")
}

fn provider_option_hash(
    provider_options: &HashMap<String, serde_json::Value>,
    group: PromptSurfaceProviderOptionGroup,
) -> Option<String> {
    let relevant = collect_prompt_surface_provider_options(provider_options, group);
    (!relevant.is_empty()).then(|| json_fingerprint(&serde_json::Value::Object(relevant)))
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
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let sections = inputs.assemble_sections("projection".to_string(), None, None);

        assert!(sections.system_text.contains("base header"));
        assert!(sections.system_text.contains("## Environment Context"));
        assert!(sections.system_text.contains("Working directory: /repo"));
        assert!(sections.system_text.contains("## Preset Role Summary"));
        assert!(sections.system_text.contains("coordination orchestrator"));
        assert!(sections.system_text.contains("<identity>Atlas</identity>"));
        assert!(sections.system_text.contains("## Capability Projection"));
        assert!(sections.system_text.contains("Agents: explore, review."));
        assert!(sections.system_text.contains("## Tone Augment"));
        assert!(sections.system_text.contains("Be concise. No flattery."));
        assert!(
            sections.system_text.find("## Tone Augment").unwrap()
                < sections.system_text.find("## Capability Projection").unwrap()
        );
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
            CompiledExecutionRequest::default(),
            HashMap::new(),
        );

        let sections = inputs.assemble_sections("projection".to_string(), None, None);

        assert!(sections.system_text.contains("## Environment Context"));
        assert!(sections.system_text.contains("Working directory: /repo"));
        assert!(!sections.system_text.contains("Today's date:"));
        assert!(!sections.system_text.contains("Current local time:"));
        assert!(!sections.system_text.contains("Local timezone:"));
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
