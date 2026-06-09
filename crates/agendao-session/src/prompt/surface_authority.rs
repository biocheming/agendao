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
use agendao_provider::ToolDefinition;
use agendao_types::MemoryRetrievalPacket;

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
            Some(PresetPromptExtension::new("sisyphus", "delegation-first orchestrator")),
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
}
