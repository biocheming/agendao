use crate::{
    extract_methodology_template_from_markdown, write::parse_skill_document, LoadedSkill,
    LoadedSkillFile, SkillConditions, SkillDetailView, SkillError, SkillFilter,
    SkillGovernanceAuthority, SkillMeta, SkillMetaView, SkillMethodologyTemplate,
};
use rocode_config::ConfigStore;
use rocode_types::{
    SkillRuntimeCompositionHint, SkillRuntimeCompositionHintKind, SkillVitalityState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInstructionSource {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSkillSourceKind {
    LegacyMarkdown,
    InstructionProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSkillSpec {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source_kind: RuntimeSkillSourceKind,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSkillMaterializationAction {
    Created,
    Refreshed,
    Unchanged,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSkillMaterialization {
    pub skill_name: String,
    pub action: RuntimeSkillMaterializationAction,
    pub source_kind: RuntimeSkillSourceKind,
    pub source_path: Option<PathBuf>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSkillBootstrapReport {
    pub materializations: Vec<RuntimeSkillMaterialization>,
    pub imported_legacy_sources: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl RuntimeSkillBootstrapReport {
    pub fn is_empty(&self) -> bool {
        self.materializations.is_empty() && self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRuntimeResolutionDiagnostic {
    pub inspection_available: bool,
    pub runtime_available: bool,
    pub vitality_state: SkillVitalityState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSkillPromptBodyKind {
    Methodology,
    CompactBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSkillPromptPacket {
    pub meta: SkillMeta,
    pub detail: SkillDetailView,
    pub vitality_state: SkillVitalityState,
    pub body_kind: RuntimeSkillPromptBodyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub methodology: Option<SkillMethodologyTemplate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_body: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub governance_hints: Vec<SkillRuntimeCompositionHint>,
}

impl RuntimeSkillPromptPacket {
    pub fn render_prompt_block(
        &self,
        arguments_block: Option<&str>,
        prompt_block: Option<&str>,
    ) -> String {
        let mut lines = vec![format!(
            "<skill_runtime_packet name=\"{}\">",
            self.meta.name
        )];
        lines.push(String::new());
        lines.push(format!("# Skill: {}", self.meta.name));
        lines.push(String::new());
        lines.push(format!("Description: {}", self.meta.description));
        lines.push(format!(
            "Runtime vitality: {}",
            runtime_vitality_label(self.vitality_state)
        ));

        if !self.governance_hints.is_empty() {
            lines.push(String::new());
            lines.push("## Governance Hints".to_string());
            for hint in &self.governance_hints {
                let label = match hint.kind {
                    SkillRuntimeCompositionHintKind::PreferCanonicalSkill => "prefer canonical",
                    SkillRuntimeCompositionHintKind::ComplementaryBundle => "keep complementary",
                };
                lines.push(format!("- {label}: {}", hint.summary));
            }
        }

        append_runtime_conditions(&mut lines, &self.meta.conditions);
        append_runtime_requirements(&mut lines, &self.detail);

        if let Some(methodology) = &self.methodology {
            append_methodology_sections(&mut lines, methodology);
        } else if let Some(body) = self.compact_body.as_deref() {
            lines.push(String::new());
            lines.push("## Execution Notes".to_string());
            lines.push(body.to_string());
        }

        if !self.meta.supporting_files.is_empty() {
            lines.push(String::new());
            lines.push("## Supporting Files".to_string());
            lines.push(
                "Use `skill_view(name, file_path)` to inspect linked files only when they are needed."
                    .to_string(),
            );
            for file in &self.meta.supporting_files {
                lines.push(format!("- {}", file.relative_path));
            }
        }

        lines.push(String::new());
        lines.push(format!(
            "Base directory: {}",
            self.meta
                .location
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .display()
        ));

        if let Some(arguments) = arguments_block.filter(|value| !value.trim().is_empty()) {
            lines.push(String::new());
            lines.push("## Arguments".to_string());
            lines.push("```json".to_string());
            lines.push(arguments.trim().to_string());
            lines.push("```".to_string());
        }

        if let Some(prompt) = prompt_block.filter(|value| !value.trim().is_empty()) {
            lines.push(String::new());
            lines.push("## Additional Instructions".to_string());
            lines.push(prompt.trim().to_string());
        }

        lines.push(String::new());
        lines.push("</skill_runtime_packet>".to_string());
        lines.join("\n")
    }
}

#[derive(Clone)]
pub struct SkillRuntimeResolver {
    governance_authority: SkillGovernanceAuthority,
}

impl SkillRuntimeResolver {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        Self {
            governance_authority: SkillGovernanceAuthority::new(base_dir, config_store),
        }
    }

    pub fn from_governance(governance_authority: SkillGovernanceAuthority) -> Self {
        Self {
            governance_authority,
        }
    }

    pub fn list_skill_meta(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillMetaView>, SkillError> {
        let skills = self.list_skill_catalog(filter)?;
        Ok(skills.iter().map(SkillMetaView::from).collect())
    }

    pub fn list_skill_catalog(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillMeta>, SkillError> {
        let mut visible = Vec::new();
        for meta in self
            .governance_authority
            .skill_authority()
            .list_skill_catalog(filter)?
            .into_iter()
        {
            if self.runtime_catalog_visible(&meta) {
                visible.push(meta);
            }
        }
        Ok(visible)
    }

    pub fn resolve_skill(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<SkillMeta, SkillError> {
        let meta = self
            .governance_authority
            .skill_authority()
            .resolve_skill_for_inspection(name, filter)?;
        self.require_runtime_visible_meta(meta)
    }

    pub fn load_skill(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<LoadedSkill, SkillError> {
        let meta = self.resolve_skill(name, filter)?;
        self.governance_authority()
            .skill_authority()
            .load_resolved_skill_for_inspection(meta)
    }

    pub fn load_skill_source(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<String, SkillError> {
        let meta = self.resolve_skill(name, filter)?;
        self.governance_authority()
            .skill_authority()
            .load_resolved_skill_source_for_inspection(&meta)
    }

    pub fn load_skill_detail(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<SkillDetailView, SkillError> {
        let meta = self.resolve_skill(name, filter)?;
        self.governance_authority()
            .skill_authority()
            .load_skill_detail_for_meta_for_inspection(&meta)
    }

    pub fn load_skill_prompt_packet(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
        selected_skill_names: Option<&[String]>,
    ) -> Result<RuntimeSkillPromptPacket, SkillError> {
        let skill = self.load_skill(name, filter)?;
        let detail = self
            .governance_authority()
            .skill_authority()
            .load_skill_detail_for_meta_for_inspection(&skill.meta)?;
        Ok(self.build_prompt_packet_for_loaded_skill(&skill, &detail, selected_skill_names))
    }

    pub fn build_prompt_packet_for_loaded_skill(
        &self,
        skill: &LoadedSkill,
        detail: &SkillDetailView,
        selected_skill_names: Option<&[String]>,
    ) -> RuntimeSkillPromptPacket {
        let selected_skill_names = selected_skill_names
            .map(normalize_selected_skill_names)
            .unwrap_or_else(|| vec![skill.meta.name.clone()]);
        let governance_hints = self
            .governance_authority()
            .runtime_skill_composition_hints(&selected_skill_names)
            .into_iter()
            .filter(|hint| {
                hint.skill_names
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(&skill.meta.name))
                    || hint
                        .preferred_skill_name
                        .as_deref()
                        .map(|value| value.eq_ignore_ascii_case(&skill.meta.name))
                        .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        let methodology = extract_methodology_template_from_markdown(&skill.content);
        let body_kind = if methodology.is_some() {
            RuntimeSkillPromptBodyKind::Methodology
        } else {
            RuntimeSkillPromptBodyKind::CompactBody
        };
        let compact_body = methodology
            .is_none()
            .then(|| compact_runtime_body(&skill.content))
            .flatten();

        RuntimeSkillPromptPacket {
            meta: skill.meta.clone(),
            detail: detail.clone(),
            vitality_state: self
                .governance_authority()
                .effective_skill_vitality_state(&skill.meta.name),
            body_kind,
            methodology,
            compact_body,
            governance_hints,
        }
    }

    pub fn load_skill_file(
        &self,
        name: &str,
        file_path: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<LoadedSkillFile, SkillError> {
        let meta = self.resolve_skill(name, filter)?;
        self.governance_authority()
            .skill_authority()
            .load_resolved_skill_file_for_inspection(&meta, file_path)
    }

    pub fn runtime_resolution_diagnostic(
        &self,
        skill_name: &str,
    ) -> SkillRuntimeResolutionDiagnostic {
        let vitality_state = self
            .governance_authority
            .effective_skill_vitality_state(skill_name);
        SkillRuntimeResolutionDiagnostic {
            inspection_available: true,
            runtime_available: matches!(
                vitality_state,
                SkillVitalityState::Active | SkillVitalityState::ReviewCandidate
            ),
            vitality_state,
        }
    }

    fn require_runtime_visible_meta(&self, meta: SkillMeta) -> Result<SkillMeta, SkillError> {
        self.governance_authority
            .ensure_skill_runtime_available(&meta.name)?;
        Ok(meta)
    }

    fn runtime_catalog_visible(&self, meta: &SkillMeta) -> bool {
        matches!(
            self.governance_authority
                .effective_skill_vitality_state(&meta.name),
            SkillVitalityState::Active | SkillVitalityState::ReviewCandidate
        )
    }

    fn governance_authority(&self) -> &SkillGovernanceAuthority {
        &self.governance_authority
    }
}

pub(crate) fn collect_runtime_skill_specs(
    base_dir: &Path,
    instructions: &[RuntimeInstructionSource],
) -> (Vec<RuntimeSkillSpec>, Vec<String>) {
    let mut specs = BTreeMap::<String, RuntimeSkillSpec>::new();
    let mut warnings = Vec::new();

    for instruction in instructions {
        for spec in collect_explicit_specs(base_dir, instruction, &mut warnings) {
            specs.insert(spec.name.to_ascii_lowercase(), spec);
        }
    }

    for instruction in instructions {
        for spec in collect_skill_reference_specs(base_dir, instruction, &mut warnings) {
            specs.entry(spec.name.to_ascii_lowercase()).or_insert(spec);
        }
    }

    (specs.into_values().collect(), warnings)
}

pub fn infer_runtime_skill_names(
    base_dir: &Path,
    instructions: &[RuntimeInstructionSource],
) -> Vec<String> {
    let (specs, _warnings) = collect_runtime_skill_specs(base_dir, instructions);
    specs.into_iter().map(|spec| spec.name).collect()
}

fn collect_explicit_specs(
    base_dir: &Path,
    instruction: &RuntimeInstructionSource,
    warnings: &mut Vec<String>,
) -> Vec<RuntimeSkillSpec> {
    let mut specs = Vec::new();
    let lines = instruction.content.lines().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !is_numbered_item(trimmed) {
            index += 1;
            continue;
        }

        let mut block = vec![trimmed.to_string()];
        index += 1;
        while index < lines.len() {
            let next = lines[index].trim();
            if next.starts_with("## ") || is_numbered_item(next) {
                break;
            }
            block.push(next.to_string());
            index += 1;
        }

        if let Some(spec) = parse_explicit_block(base_dir, instruction, &block, warnings) {
            specs.push(spec);
        }
    }

    specs
}

fn parse_explicit_block(
    base_dir: &Path,
    instruction: &RuntimeInstructionSource,
    block: &[String],
    warnings: &mut Vec<String>,
) -> Option<RuntimeSkillSpec> {
    let headline = block.first()?.trim();
    let mut source_kind = None;
    let mut source_rel = None;

    if let Some(path) = first_backtick_value(headline) {
        source_kind = Some(RuntimeSkillSourceKind::LegacyMarkdown);
        source_rel = Some(path);
    } else if headline
        .to_ascii_lowercase()
        .contains("harness protocol itself")
    {
        source_kind = Some(RuntimeSkillSourceKind::InstructionProtocol);
    }

    let mut name = None;
    let mut description = None;
    for line in block.iter().skip(1) {
        let trimmed = line.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("- target workspace skill:") {
            name = first_backtick_value(trimmed);
            continue;
        }
        if lowered.starts_with("- description:") {
            description = Some(strip_wrapping_quotes(
                trimmed
                    .split_once(':')
                    .map(|(_, value)| value.trim())
                    .unwrap_or_default(),
            ));
        }
    }

    let name = name?;
    let description = description?;
    build_runtime_skill_spec(
        base_dir,
        instruction,
        &name,
        &description,
        source_kind,
        source_rel.as_deref(),
        warnings,
    )
}

fn collect_skill_reference_specs(
    base_dir: &Path,
    instruction: &RuntimeInstructionSource,
    warnings: &mut Vec<String>,
) -> Vec<RuntimeSkillSpec> {
    let mut descriptions = BTreeMap::<String, String>::new();
    let mut specs = Vec::new();

    for line in instruction.content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }

        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("- target workspace skill:") {
            if let Some(name) = first_backtick_value(trimmed) {
                let description = trimmed
                    .split("--")
                    .nth(1)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(strip_wrapping_quotes)
                    .unwrap_or_default();
                descriptions.insert(name, description);
            }
            continue;
        }

        if lowered.starts_with("- legacy reference source:") {
            let values = backtick_values(trimmed);
            if values.len() < 2 {
                continue;
            }
            let source_rel = &values[0];
            let target_name = &values[1];
            let description = descriptions
                .get(target_name)
                .cloned()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    format!("Runtime materialized workspace skill `{target_name}`.")
                });
            if let Some(spec) = build_runtime_skill_spec(
                base_dir,
                instruction,
                target_name,
                &description,
                Some(RuntimeSkillSourceKind::LegacyMarkdown),
                Some(source_rel),
                warnings,
            ) {
                specs.push(spec);
            }
        }
    }

    specs
}

fn build_runtime_skill_spec(
    base_dir: &Path,
    instruction: &RuntimeInstructionSource,
    name: &str,
    description: &str,
    source_kind: Option<RuntimeSkillSourceKind>,
    source_rel: Option<&str>,
    warnings: &mut Vec<String>,
) -> Option<RuntimeSkillSpec> {
    let source_kind = source_kind?;
    match source_kind {
        RuntimeSkillSourceKind::InstructionProtocol => Some(RuntimeSkillSpec {
            name: name.trim().to_string(),
            description: description.trim().to_string(),
            body: instruction.content.trim().to_string(),
            source_kind,
            source_path: Some(relativize_path(base_dir, &instruction.path)),
        }),
        RuntimeSkillSourceKind::LegacyMarkdown => {
            let source_rel = source_rel?.trim();
            if source_rel.is_empty() {
                return None;
            }
            let resolved = resolve_instruction_relative_path(&instruction.path, source_rel);
            let body = match fs::read_to_string(&resolved) {
                Ok(content) => content.replace("\r\n", "\n").trim().to_string(),
                Err(error) => {
                    warnings.push(format!(
                        "Failed to import legacy skill source `{}` for `{}`: {}",
                        source_rel, name, error
                    ));
                    return None;
                }
            };
            if body.is_empty() {
                warnings.push(format!(
                    "Legacy skill source `{}` for `{}` was empty.",
                    source_rel, name
                ));
                return None;
            }
            Some(RuntimeSkillSpec {
                name: name.trim().to_string(),
                description: description.trim().to_string(),
                body,
                source_kind,
                source_path: Some(relativize_path(base_dir, &resolved)),
            })
        }
    }
}

fn resolve_instruction_relative_path(instruction_path: &Path, raw: &str) -> PathBuf {
    let parent = instruction_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(raw)
}

fn relativize_path(base_dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(base_dir)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn is_numbered_item(value: &str) -> bool {
    let digits = value.chars().take_while(|ch| ch.is_ascii_digit()).count();
    digits > 0 && value[digits..].starts_with(". ")
}

fn first_backtick_value(value: &str) -> Option<String> {
    backtick_values(value).into_iter().next()
}

fn backtick_values(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, ch) in value.char_indices() {
        if ch != '`' {
            continue;
        }
        if let Some(open) = start.take() {
            if index > open + 1 {
                out.push(value[open + 1..index].trim().to_string());
            }
        } else {
            start = Some(index);
        }
    }
    out
}

fn strip_wrapping_quotes(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

fn normalize_selected_skill_names(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn append_runtime_conditions(lines: &mut Vec<String>, conditions: &SkillConditions) {
    let mut items = Vec::new();
    if !conditions.requires_tools.is_empty() {
        items.push(format!(
            "required tools: {}",
            conditions.requires_tools.join(", ")
        ));
    }
    if !conditions.requires_toolsets.is_empty() {
        items.push(format!(
            "required toolsets: {}",
            conditions.requires_toolsets.join(", ")
        ));
    }
    if !conditions.stage_filter.is_empty() {
        items.push(format!(
            "stage filters: {}",
            conditions.stage_filter.join(", ")
        ));
    }
    if !conditions.fallback_for_tools.is_empty() {
        items.push(format!(
            "fallback when tools are absent: {}",
            conditions.fallback_for_tools.join(", ")
        ));
    }
    if !conditions.fallback_for_toolsets.is_empty() {
        items.push(format!(
            "fallback when toolsets are absent: {}",
            conditions.fallback_for_toolsets.join(", ")
        ));
    }
    if items.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push("## Runtime Conditions".to_string());
    for item in items {
        lines.push(format!("- {item}"));
    }
}

fn append_runtime_requirements(lines: &mut Vec<String>, detail: &SkillDetailView) {
    let mut items = Vec::new();
    if !detail.required_environment_variables.is_empty() {
        items.push(format!(
            "environment variables: {}",
            detail
                .required_environment_variables
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !detail.required_commands.is_empty() {
        items.push(format!("commands: {}", detail.required_commands.join(", ")));
    }
    if !detail.missing_required_environment_variables.is_empty() {
        items.push(format!(
            "currently missing env vars: {}",
            detail.missing_required_environment_variables.join(", ")
        ));
    }
    if !detail.missing_required_commands.is_empty() {
        items.push(format!(
            "currently missing commands: {}",
            detail.missing_required_commands.join(", ")
        ));
    }
    if items.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push("## Runtime Requirements".to_string());
    for item in items {
        lines.push(format!("- {item}"));
    }
}

fn append_methodology_sections(lines: &mut Vec<String>, methodology: &SkillMethodologyTemplate) {
    if !methodology.when_to_use.is_empty() {
        lines.push(String::new());
        lines.push("## When To Use".to_string());
        for item in &methodology.when_to_use {
            lines.push(format!("- {item}"));
        }
    }
    if !methodology.prerequisites.is_empty() {
        lines.push(String::new());
        lines.push("## Workflow Prerequisites".to_string());
        for item in &methodology.prerequisites {
            lines.push(format!("- {item}"));
        }
    }
    if !methodology.core_steps.is_empty() {
        lines.push(String::new());
        lines.push("## Core Steps".to_string());
        for (index, step) in methodology.core_steps.iter().enumerate() {
            let mut line = format!("{}. **{}**: {}", index + 1, step.title, step.action);
            if let Some(outcome) = step
                .outcome
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                line.push_str(&format!(" Outcome: {outcome}"));
            }
            if !step.experienced_tools.is_empty() {
                line.push_str(&format!(
                    " Experienced tools: {}",
                    step.experienced_tools.join(", ")
                ));
            }
            lines.push(line);
        }
    }
    if !methodology.success_criteria.is_empty() {
        lines.push(String::new());
        lines.push("## Success Criteria".to_string());
        for item in &methodology.success_criteria {
            lines.push(format!("- [ ] {item}"));
        }
    }
    if !methodology.validation.is_empty() {
        lines.push(String::new());
        lines.push("## Validation".to_string());
        for item in &methodology.validation {
            lines.push(format!("- [ ] {item}"));
        }
    }
    let mut boundaries = methodology.when_not_to_use.clone();
    boundaries.extend(methodology.pitfalls.clone());
    if !boundaries.is_empty() {
        lines.push(String::new());
        lines.push("## Boundaries and Pitfalls".to_string());
        for item in boundaries {
            lines.push(format!("- {item}"));
        }
    }
    if !methodology.references.is_empty() {
        lines.push(String::new());
        lines.push("## References".to_string());
        for reference in &methodology.references {
            lines.push(format!("- `{}` - {}", reference.path, reference.label));
        }
    }
}

fn compact_runtime_body(content: &str) -> Option<String> {
    let body = parse_skill_document(content)
        .map(|document| document.body)
        .unwrap_or_else(|_| content.trim().to_string());
    let mut lines = Vec::new();
    let mut char_count = 0usize;
    let mut previous_blank = true;
    for raw_line in body.lines() {
        let line = raw_line.trim_end();
        let is_blank = line.trim().is_empty();
        if is_blank && previous_blank {
            continue;
        }
        previous_blank = is_blank;
        let normalized = if is_blank {
            String::new()
        } else {
            line.trim().to_string()
        };
        let projected = char_count + normalized.len();
        if lines.len() >= 36 || projected > 2_600 {
            lines.push("...".to_string());
            break;
        }
        char_count = projected;
        lines.push(normalized);
    }
    let excerpt = lines.join("\n").trim().to_string();
    (!excerpt.is_empty()).then_some(excerpt)
}

fn runtime_vitality_label(state: SkillVitalityState) -> &'static str {
    match state {
        SkillVitalityState::Active => "active",
        SkillVitalityState::ReviewCandidate => "review_candidate",
        SkillVitalityState::Retired => "retired",
        SkillVitalityState::Archived => "archived",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_methodology_skill_body;
    use rocode_types::{
        SkillCapabilityGroupKind, SkillCapabilityMember, SkillCapabilityMemberRole,
    };
    use tempfile::tempdir;

    #[test]
    fn collect_runtime_skill_specs_parses_explicit_mapping_and_legacy_refs() {
        let dir = tempdir().unwrap();
        let agents_path = dir.path().join("AGENTS.md");
        let legacy_path = dir.path().join("harness/skills/propose_modifications.md");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, "# Propose\nUse ./tools/mol propose.").unwrap();

        let instruction = RuntimeInstructionSource {
            path: agents_path,
            content: r#"
Use the following explicit create or refresh mapping:

1. For `harness/skills/propose_modifications.md`
   - target workspace skill: `drug-discovery-propose-modifications`
   - target path: `.rocode/skills/drug-discovery-propose-modifications/SKILL.md`
   - description: `Generate local molecular modifications with the workspace ./tools/mol wrapper.`

4. For the harness protocol itself
   - target workspace skill: `drug-discovery-harness`
   - target path: `.rocode/skills/drug-discovery-harness/SKILL.md`
   - description: `Workspace-local harness for molecular optimization using ./tools/mol.`

## Skill References

- Target workspace skill: `drug-discovery-propose-modifications` -- candidate generation guidance
- Legacy reference source: `harness/skills/propose_modifications.md` -> if `drug-discovery-propose-modifications` does not exist, create it
"#
            .to_string(),
        };

        let (specs, warnings) = collect_runtime_skill_specs(dir.path(), &[instruction]);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "drug-discovery-harness");
        assert_eq!(
            specs[0].source_kind,
            RuntimeSkillSourceKind::InstructionProtocol
        );
        assert_eq!(specs[1].name, "drug-discovery-propose-modifications");
        assert_eq!(specs[1].source_kind, RuntimeSkillSourceKind::LegacyMarkdown);
        assert!(specs[1].body.contains("./tools/mol propose"));
    }

    #[test]
    fn infer_runtime_skill_names_returns_sorted_runtime_skill_names() {
        let dir = tempdir().unwrap();
        let agents_path = dir.path().join("AGENTS.md");
        let legacy_path = dir.path().join("harness/skills/propose_modifications.md");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(&legacy_path, "# Propose\nUse ./tools/mol propose.").unwrap();

        let instruction = RuntimeInstructionSource {
            path: agents_path,
            content: r#"
Use the following explicit create or refresh mapping:

1. For `harness/skills/propose_modifications.md`
   - target workspace skill: `drug-discovery-propose-modifications`
   - target path: `.rocode/skills/drug-discovery-propose-modifications/SKILL.md`
   - description: `Generate local molecular modifications with the workspace ./tools/mol wrapper.`

4. For the harness protocol itself
   - target workspace skill: `drug-discovery-harness`
   - target path: `.rocode/skills/drug-discovery-harness/SKILL.md`
   - description: `Workspace-local harness for molecular optimization using ./tools/mol.`
"#
            .to_string(),
        };

        let names = infer_runtime_skill_names(dir.path(), &[instruction]);
        assert_eq!(
            names,
            vec![
                "drug-discovery-harness".to_string(),
                "drug-discovery-propose-modifications".to_string()
            ]
        );
    }

    #[test]
    fn load_skill_prompt_packet_prefers_methodology_and_governance_hints() {
        let dir = tempdir().unwrap();
        let governance = SkillGovernanceAuthority::new(dir.path(), None);
        let methodology = render_methodology_skill_body(
            "provider-refresh",
            &SkillMethodologyTemplate {
                when_to_use: vec!["refresh provider auth".to_string()],
                prerequisites: vec!["provider must already exist".to_string()],
                core_steps: vec![crate::SkillMethodologyStep {
                    title: "Inspect".to_string(),
                    action: "read provider config".to_string(),
                    outcome: Some("resolved current state".to_string()),
                    experienced_tools: vec!["provider".to_string()],
                }],
                success_criteria: vec!["auth validated".to_string()],
                validation: vec!["config explains selected transport".to_string()],
                pitfalls: vec!["do not rewrite unrelated providers".to_string()],
                ..SkillMethodologyTemplate::default()
            },
        )
        .unwrap();
        governance
            .create_skill(
                crate::CreateSkillRequest {
                    name: "provider-refresh".to_string(),
                    description: "canonical provider refresh".to_string(),
                    body: methodology,
                    frontmatter: None,
                    category: Some("ops".to_string()),
                    directory_name: None,
                },
                "test:create",
            )
            .unwrap();
        governance
            .create_skill(
                crate::CreateSkillRequest {
                    name: "provider-refresh-gitlab".to_string(),
                    description: "gitlab variant".to_string(),
                    body: "Read gitlab provider state.\nThen refresh the token.".to_string(),
                    frontmatter: None,
                    category: Some("ops".to_string()),
                    directory_name: None,
                },
                "test:create",
            )
            .unwrap();
        governance
            .activate_skill_capability_group(
                Some("provider-refresh-family"),
                SkillCapabilityGroupKind::CanonicalFamily,
                Some("provider-refresh"),
                vec![
                    SkillCapabilityMember {
                        skill_name: "provider-refresh".to_string(),
                        role: SkillCapabilityMemberRole::Canonical,
                    },
                    SkillCapabilityMember {
                        skill_name: "provider-refresh-gitlab".to_string(),
                        role: SkillCapabilityMemberRole::Specialization,
                    },
                ],
                vec!["gitlab variant should defer to canonical provider refresh".to_string()],
                "test:activate",
            )
            .unwrap();

        let resolver = SkillRuntimeResolver::from_governance(governance);
        let packet = resolver
            .load_skill_prompt_packet(
                "provider-refresh-gitlab",
                None,
                Some(&[
                    "provider-refresh-gitlab".to_string(),
                    "provider-refresh".to_string(),
                ]),
            )
            .unwrap();

        assert_eq!(packet.body_kind, RuntimeSkillPromptBodyKind::CompactBody);
        assert!(packet.compact_body.is_some());
        assert!(packet.methodology.is_none());
        assert!(packet
            .governance_hints
            .iter()
            .any(|hint| hint.kind == SkillRuntimeCompositionHintKind::PreferCanonicalSkill));

        let canonical_packet = resolver
            .load_skill_prompt_packet("provider-refresh", None, None)
            .unwrap();
        assert_eq!(
            canonical_packet.body_kind,
            RuntimeSkillPromptBodyKind::Methodology
        );
        assert!(canonical_packet.methodology.is_some());
        let rendered = canonical_packet.render_prompt_block(None, None);
        assert!(rendered.contains("## Core Steps"));
        assert!(rendered.contains("## Validation"));
        assert!(rendered.contains("Runtime vitality: active"));
    }
}
