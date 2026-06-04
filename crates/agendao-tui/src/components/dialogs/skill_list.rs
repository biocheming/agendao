use agendao_skill::{
    extract_methodology_template_from_markdown, render_methodology_skill_body,
    SkillMethodologyReference, SkillMethodologyStep, SkillMethodologyTemplate,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::api::{
    ManagedSkillRecord, SkillArtifactCacheEntry, SkillAuditEvent, SkillCatalogEntry,
    SkillDetailResponse, SkillDistributionRecord, SkillGovernanceTimelineEntry, SkillGuardReport,
    SkillHubPolicy, SkillManagedLifecycleRecord, SkillRemoteInstallPlan, SkillSourceIndexSnapshot,
    SkillSyncPlan,
};
use crate::{theme::Theme, ui::RenderSurface};

#[derive(Clone, Debug, Default)]
struct TextEditorState {
    text: String,
    cursor: usize,
    scroll: u16,
}

impl TextEditorState {
    fn with_text(text: String) -> Self {
        let cursor = text.len();
        let mut state = Self {
            text,
            cursor,
            scroll: 0,
        };
        state.sync_scroll_to_cursor();
        state
    }

    fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.sync_scroll_to_cursor();
    }

    fn insert_newline(&mut self) {
        self.text.insert(self.cursor, '\n');
        self.cursor += 1;
        self.sync_scroll_to_cursor();
    }

    fn backspace(&mut self) {
        if let Some(prev) = prev_char_boundary(&self.text, self.cursor) {
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
            self.sync_scroll_to_cursor();
        }
    }

    fn delete(&mut self) {
        if let Some(next) = next_char_boundary(&self.text, self.cursor) {
            self.text.replace_range(self.cursor..next, "");
            self.sync_scroll_to_cursor();
        }
    }

    fn move_left(&mut self) {
        if let Some(prev) = prev_char_boundary(&self.text, self.cursor) {
            self.cursor = prev;
            self.sync_scroll_to_cursor();
        }
    }

    fn move_right(&mut self) {
        if let Some(next) = next_char_boundary(&self.text, self.cursor) {
            self.cursor = next;
            self.sync_scroll_to_cursor();
        }
    }

    fn move_home(&mut self) {
        self.cursor = line_start(&self.text, self.cursor);
        self.sync_scroll_to_cursor();
    }

    fn move_end(&mut self) {
        self.cursor = line_end(&self.text, self.cursor);
        self.sync_scroll_to_cursor();
    }

    fn cursor_row_col(&self) -> (u16, u16) {
        let prefix = &self.text[..self.cursor.min(self.text.len())];
        let row = prefix.bytes().filter(|byte| *byte == b'\n').count() as u16;
        let col = prefix
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count() as u16;
        (row, col)
    }

    fn sync_scroll_to_cursor(&mut self) {
        let (row, _) = self.cursor_row_col();
        self.scroll = row.saturating_sub(4);
    }

    fn scroll_up(&mut self) {
        if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillManageEditorMode {
    Methodology,
    Raw,
}

impl SkillManageEditorMode {
    fn toggle(&mut self) {
        *self = match self {
            Self::Methodology => Self::Raw,
            Self::Raw => Self::Methodology,
        };
    }

    fn label(self) -> &'static str {
        match self {
            Self::Methodology => "methodology",
            Self::Raw => "raw markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillMethodologySection {
    WhenToUse,
    WhenNotToUse,
    Prerequisites,
    CoreSteps,
    SuccessCriteria,
    Validation,
    Pitfalls,
    References,
}

impl SkillMethodologySection {
    const ALL: [Self; 8] = [
        Self::WhenToUse,
        Self::WhenNotToUse,
        Self::Prerequisites,
        Self::CoreSteps,
        Self::SuccessCriteria,
        Self::Validation,
        Self::Pitfalls,
        Self::References,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::WhenToUse => "When To Use",
            Self::WhenNotToUse => "When Not To Use",
            Self::Prerequisites => "Prerequisites",
            Self::CoreSteps => "Core Steps",
            Self::SuccessCriteria => "Success Criteria",
            Self::Validation => "Validation",
            Self::Pitfalls => "Boundaries / Pitfalls",
            Self::References => "References",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::WhenToUse => "One trigger per line.",
            Self::WhenNotToUse => "One boundary per line.",
            Self::Prerequisites => "One requirement per line.",
            Self::CoreSteps => "One step per line: Title | Action | Outcome(optional)",
            Self::SuccessCriteria => "One checklist item per line.",
            Self::Validation => "One validation check per line.",
            Self::Pitfalls => "One pitfall per line.",
            Self::References => "One reference per line: path | label",
        }
    }

    fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Debug)]
struct SkillMethodologyDraft {
    selected_section: SkillMethodologySection,
    when_to_use: TextEditorState,
    when_not_to_use: TextEditorState,
    prerequisites: TextEditorState,
    core_steps: TextEditorState,
    success_criteria: TextEditorState,
    validation: TextEditorState,
    pitfalls: TextEditorState,
    references: TextEditorState,
}

impl Default for SkillMethodologyDraft {
    fn default() -> Self {
        Self {
            selected_section: SkillMethodologySection::WhenToUse,
            when_to_use: TextEditorState::default(),
            when_not_to_use: TextEditorState::default(),
            prerequisites: TextEditorState::default(),
            core_steps: TextEditorState::default(),
            success_criteria: TextEditorState::default(),
            validation: TextEditorState::default(),
            pitfalls: TextEditorState::default(),
            references: TextEditorState::default(),
        }
    }
}

impl SkillMethodologyDraft {
    fn from_template(template: &SkillMethodologyTemplate) -> Self {
        Self {
            selected_section: SkillMethodologySection::WhenToUse,
            when_to_use: TextEditorState::with_text(template.when_to_use.join("\n")),
            when_not_to_use: TextEditorState::with_text(template.when_not_to_use.join("\n")),
            prerequisites: TextEditorState::with_text(template.prerequisites.join("\n")),
            core_steps: TextEditorState::with_text(
                template
                    .core_steps
                    .iter()
                    .map(|step| {
                        let mut line = format!("{} | {}", step.title, step.action);
                        if let Some(outcome) =
                            step.outcome.as_deref().filter(|value| !value.is_empty())
                        {
                            line.push_str(" | ");
                            line.push_str(outcome);
                        }
                        line
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            success_criteria: TextEditorState::with_text(template.success_criteria.join("\n")),
            validation: TextEditorState::with_text(template.validation.join("\n")),
            pitfalls: TextEditorState::with_text(template.pitfalls.join("\n")),
            references: TextEditorState::with_text(
                template
                    .references
                    .iter()
                    .map(|reference| format!("{} | {}", reference.path, reference.label))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }

    fn selected_editor(&self) -> &TextEditorState {
        match self.selected_section {
            SkillMethodologySection::WhenToUse => &self.when_to_use,
            SkillMethodologySection::WhenNotToUse => &self.when_not_to_use,
            SkillMethodologySection::Prerequisites => &self.prerequisites,
            SkillMethodologySection::CoreSteps => &self.core_steps,
            SkillMethodologySection::SuccessCriteria => &self.success_criteria,
            SkillMethodologySection::Validation => &self.validation,
            SkillMethodologySection::Pitfalls => &self.pitfalls,
            SkillMethodologySection::References => &self.references,
        }
    }

    fn selected_editor_mut(&mut self) -> &mut TextEditorState {
        match self.selected_section {
            SkillMethodologySection::WhenToUse => &mut self.when_to_use,
            SkillMethodologySection::WhenNotToUse => &mut self.when_not_to_use,
            SkillMethodologySection::Prerequisites => &mut self.prerequisites,
            SkillMethodologySection::CoreSteps => &mut self.core_steps,
            SkillMethodologySection::SuccessCriteria => &mut self.success_criteria,
            SkillMethodologySection::Validation => &mut self.validation,
            SkillMethodologySection::Pitfalls => &mut self.pitfalls,
            SkillMethodologySection::References => &mut self.references,
        }
    }

    fn to_template(&self) -> Result<SkillMethodologyTemplate, String> {
        Ok(SkillMethodologyTemplate {
            when_to_use: split_non_empty_lines(&self.when_to_use.text),
            when_not_to_use: split_non_empty_lines(&self.when_not_to_use.text),
            prerequisites: split_non_empty_lines(&self.prerequisites.text),
            core_steps: parse_methodology_steps_input(&self.core_steps.text)?,
            success_criteria: split_non_empty_lines(&self.success_criteria.text),
            validation: split_non_empty_lines(&self.validation.text),
            pitfalls: split_non_empty_lines(&self.pitfalls.text),
            references: parse_methodology_references_input(&self.references.text)?,
        })
    }

    fn preview(&self, skill_name: &str) -> Result<String, String> {
        let template = self.to_template()?;
        render_methodology_skill_body(skill_name, &template).map_err(|error| error.to_string())
    }

    fn section_summary(&self, section: SkillMethodologySection) -> String {
        let count = split_non_empty_lines(match section {
            SkillMethodologySection::WhenToUse => &self.when_to_use.text,
            SkillMethodologySection::WhenNotToUse => &self.when_not_to_use.text,
            SkillMethodologySection::Prerequisites => &self.prerequisites.text,
            SkillMethodologySection::CoreSteps => &self.core_steps.text,
            SkillMethodologySection::SuccessCriteria => &self.success_criteria.text,
            SkillMethodologySection::Validation => &self.validation.text,
            SkillMethodologySection::Pitfalls => &self.pitfalls.text,
            SkillMethodologySection::References => &self.references.text,
        })
        .len();
        format!("{} ({})", section.title(), count)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillCreateField {
    Name,
    Description,
    Category,
    EditorMode,
    MethodologySection,
    MethodologyEditor,
    Body,
}

impl SkillCreateField {
    fn next(self, mode: SkillManageEditorMode) -> Self {
        match self {
            Self::Name => Self::Description,
            Self::Description => Self::Category,
            Self::Category => Self::EditorMode,
            Self::EditorMode => match mode {
                SkillManageEditorMode::Methodology => Self::MethodologySection,
                SkillManageEditorMode::Raw => Self::Body,
            },
            Self::MethodologySection => Self::MethodologyEditor,
            Self::MethodologyEditor => Self::Name,
            Self::Body => Self::Name,
        }
    }

    fn previous(self, mode: SkillManageEditorMode) -> Self {
        match self {
            Self::Name => match mode {
                SkillManageEditorMode::Methodology => Self::MethodologyEditor,
                SkillManageEditorMode::Raw => Self::Body,
            },
            Self::Description => Self::Name,
            Self::Category => Self::Description,
            Self::EditorMode => Self::Category,
            Self::MethodologySection => Self::EditorMode,
            Self::MethodologyEditor => Self::MethodologySection,
            Self::Body => Self::EditorMode,
        }
    }
}

#[derive(Clone, Debug)]
struct SkillCreateDraft {
    active_field: SkillCreateField,
    editor_mode: SkillManageEditorMode,
    name: String,
    description: String,
    category: String,
    body: TextEditorState,
    methodology: SkillMethodologyDraft,
}

impl Default for SkillCreateDraft {
    fn default() -> Self {
        Self {
            active_field: SkillCreateField::Name,
            editor_mode: SkillManageEditorMode::Methodology,
            name: String::new(),
            description: String::new(),
            category: String::new(),
            body: TextEditorState::default(),
            methodology: SkillMethodologyDraft::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillEditField {
    Description,
    EditorMode,
    MethodologySection,
    MethodologyEditor,
    RawSource,
}

impl SkillEditField {
    fn next(self, mode: SkillManageEditorMode) -> Self {
        match self {
            Self::Description => Self::EditorMode,
            Self::EditorMode => match mode {
                SkillManageEditorMode::Methodology => Self::MethodologySection,
                SkillManageEditorMode::Raw => Self::RawSource,
            },
            Self::MethodologySection => Self::MethodologyEditor,
            Self::MethodologyEditor => Self::Description,
            Self::RawSource => Self::Description,
        }
    }

    fn previous(self, mode: SkillManageEditorMode) -> Self {
        match self {
            Self::Description => match mode {
                SkillManageEditorMode::Methodology => Self::MethodologyEditor,
                SkillManageEditorMode::Raw => Self::RawSource,
            },
            Self::EditorMode => Self::Description,
            Self::MethodologySection => Self::EditorMode,
            Self::MethodologyEditor => Self::MethodologySection,
            Self::RawSource => Self::EditorMode,
        }
    }
}

#[derive(Clone, Debug)]
struct SkillEditDraft {
    name: String,
    description: String,
    category: Option<String>,
    active_field: SkillEditField,
    editor_mode: SkillManageEditorMode,
    source: TextEditorState,
    methodology: SkillMethodologyDraft,
    methodology_loaded: bool,
}

#[derive(Clone, Debug)]
pub enum SkillCreatePayload {
    Raw {
        name: String,
        description: String,
        category: Option<String>,
        body: String,
    },
    Methodology {
        name: String,
        description: String,
        category: Option<String>,
        methodology: SkillMethodologyTemplate,
    },
}

#[derive(Clone, Debug)]
pub enum SkillEditPayload {
    Raw {
        name: String,
        content: String,
    },
    Methodology {
        name: String,
        description: String,
        methodology: SkillMethodologyTemplate,
    },
}

#[derive(Clone, Debug)]
enum SkillListMode {
    Browse,
    Create(SkillCreateDraft),
    Edit(SkillEditDraft),
    ConfirmDelete { name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillBrowsePane {
    Preview,
    Timeline,
}

pub struct SkillListDialog {
    skills: Vec<SkillCatalogEntry>,
    managed_skills: Vec<ManagedSkillRecord>,
    source_indices: Vec<SkillSourceIndexSnapshot>,
    audit_events: Vec<SkillAuditEvent>,
    governance_timeline: Vec<SkillGovernanceTimelineEntry>,
    hub_plan: Option<SkillSyncPlan>,
    remote_install_plan: Option<SkillRemoteInstallPlan>,
    distributions: Vec<SkillDistributionRecord>,
    artifact_cache: Vec<SkillArtifactCacheEntry>,
    hub_policy: Option<SkillHubPolicy>,
    lifecycle_records: Vec<SkillManagedLifecycleRecord>,
    guard_reports: Vec<SkillGuardReport>,
    guard_target_label: Option<String>,
    selected_hub_source: usize,
    filtered: Vec<usize>,
    query: String,
    detail: Option<SkillDetailResponse>,
    detail_error: Option<String>,
    detail_scroll: u16,
    state: ListState,
    mode: SkillListMode,
    browse_pane: SkillBrowsePane,
    open: bool,
}

impl SkillListDialog {
    pub fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            skills: Vec::new(),
            managed_skills: Vec::new(),
            source_indices: Vec::new(),
            audit_events: Vec::new(),
            governance_timeline: Vec::new(),
            hub_plan: None,
            remote_install_plan: None,
            distributions: Vec::new(),
            artifact_cache: Vec::new(),
            hub_policy: None,
            lifecycle_records: Vec::new(),
            guard_reports: Vec::new(),
            guard_target_label: None,
            selected_hub_source: 0,
            filtered: Vec::new(),
            query: String::new(),
            detail: None,
            detail_error: None,
            detail_scroll: 0,
            state,
            mode: SkillListMode::Browse,
            browse_pane: SkillBrowsePane::Preview,
            open: false,
        }
    }

    pub fn set_skills(&mut self, mut skills: Vec<SkillCatalogEntry>) {
        skills.sort_by_key(|skill| skill.name.to_ascii_lowercase());
        skills.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
        self.skills = skills;
        self.clear_detail();
        self.filter();
    }

    pub fn set_hub_state(
        &mut self,
        mut managed_skills: Vec<ManagedSkillRecord>,
        mut source_indices: Vec<SkillSourceIndexSnapshot>,
        mut audit_events: Vec<SkillAuditEvent>,
    ) {
        managed_skills.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        source_indices.sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
        audit_events.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        self.managed_skills = managed_skills;
        self.source_indices = source_indices;
        self.audit_events = audit_events;
        if self.selected_hub_source >= self.source_indices.len() {
            self.selected_hub_source = 0;
        }
    }

    pub fn set_hub_plan(&mut self, plan: SkillSyncPlan) {
        self.hub_plan = Some(plan);
    }

    pub fn set_remote_install_plan(&mut self, plan: SkillRemoteInstallPlan) {
        self.remote_install_plan = Some(plan);
    }

    pub fn set_remote_hub_state(
        &mut self,
        mut distributions: Vec<SkillDistributionRecord>,
        mut artifact_cache: Vec<SkillArtifactCacheEntry>,
        hub_policy: SkillHubPolicy,
        mut lifecycle_records: Vec<SkillManagedLifecycleRecord>,
    ) {
        distributions.sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        artifact_cache
            .sort_by(|left, right| left.artifact.artifact_id.cmp(&right.artifact.artifact_id));
        lifecycle_records.sort_by(|left, right| left.distribution_id.cmp(&right.distribution_id));
        self.distributions = distributions;
        self.artifact_cache = artifact_cache;
        self.hub_policy = Some(hub_policy);
        self.lifecycle_records = lifecycle_records;
    }

    pub fn set_governance_timeline(&mut self, mut entries: Vec<SkillGovernanceTimelineEntry>) {
        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        self.governance_timeline = entries;
    }

    pub fn set_guard_reports(
        &mut self,
        target_label: impl Into<String>,
        mut reports: Vec<SkillGuardReport>,
    ) {
        reports.sort_by(|left, right| left.skill_name.cmp(&right.skill_name));
        self.guard_reports = reports;
        self.guard_target_label = Some(target_label.into());
    }

    pub fn selected_hub_source(&self) -> Option<&crate::api::SkillSourceRef> {
        self.source_indices
            .get(self.selected_hub_source)
            .map(|snapshot| &snapshot.source)
    }

    pub fn selected_hub_source_snapshot(&self) -> Option<&SkillSourceIndexSnapshot> {
        self.source_indices.get(self.selected_hub_source)
    }

    pub fn cycle_hub_source(&mut self) {
        if self.source_indices.is_empty() {
            self.selected_hub_source = 0;
            return;
        }
        self.selected_hub_source = (self.selected_hub_source + 1) % self.source_indices.len();
    }

    pub fn resolve_remote_install_skill_name(&self) -> Option<String> {
        let source = self.selected_hub_source_snapshot()?;
        if let Some(selected_skill) = self.selected_skill() {
            if let Some(entry) = source
                .entries
                .iter()
                .find(|entry| entry.skill_name.eq_ignore_ascii_case(selected_skill))
            {
                return Some(entry.skill_name.clone());
            }
        }

        let query = self.query.trim();
        if !query.is_empty() {
            if let Some(entry) = source
                .entries
                .iter()
                .find(|entry| entry.skill_name.eq_ignore_ascii_case(query))
            {
                return Some(entry.skill_name.clone());
            }
            let normalized = query.to_ascii_lowercase();
            if let Some(entry) = source.entries.iter().find(|entry| {
                entry.skill_name.to_ascii_lowercase().contains(&normalized)
                    || entry
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&normalized)
            }) {
                return Some(entry.skill_name.clone());
            }
        }

        source.entries.first().map(|entry| entry.skill_name.clone())
    }

    pub fn open(&mut self) {
        self.open = true;
        self.mode = SkillListMode::Browse;
        self.browse_pane = SkillBrowsePane::Preview;
        self.query.clear();
        self.detail_scroll = 0;
        self.filter();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.mode = SkillListMode::Browse;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_browse_mode(&self) -> bool {
        matches!(self.mode, SkillListMode::Browse)
    }

    pub fn is_create_mode(&self) -> bool {
        matches!(self.mode, SkillListMode::Create(_))
    }

    pub fn is_edit_mode(&self) -> bool {
        matches!(self.mode, SkillListMode::Edit(_))
    }

    pub fn is_delete_confirm_mode(&self) -> bool {
        matches!(self.mode, SkillListMode::ConfirmDelete { .. })
    }

    pub fn handle_input(&mut self, c: char) {
        self.query.push(c);
        self.filter();
    }

    pub fn handle_backspace(&mut self) {
        self.query.pop();
        self.filter();
    }

    pub fn move_up(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected > 0 {
                self.state.select(Some(selected - 1));
                self.detail_scroll = 0;
            }
        }
    }

    pub fn move_down(&mut self) {
        if let Some(selected) = self.state.selected() {
            if selected < self.filtered.len().saturating_sub(1) {
                self.state.select(Some(selected + 1));
                self.detail_scroll = 0;
            }
        }
    }

    pub fn preview_scroll_up(&mut self) {
        if self.detail_scroll > 0 {
            self.detail_scroll -= 1;
        }
    }

    pub fn preview_scroll_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
    }

    pub fn toggle_browse_pane(&mut self) {
        self.browse_pane = match self.browse_pane {
            SkillBrowsePane::Preview => SkillBrowsePane::Timeline,
            SkillBrowsePane::Timeline => SkillBrowsePane::Preview,
        };
        self.detail_scroll = 0;
    }

    pub fn selected_skill(&self) -> Option<&str> {
        let idx = self.state.selected().and_then(|s| self.filtered.get(s))?;
        self.skills.get(*idx).map(|skill| skill.name.as_str())
    }

    pub fn selected_entry(&self) -> Option<&SkillCatalogEntry> {
        let idx = self.state.selected().and_then(|s| self.filtered.get(s))?;
        self.skills.get(*idx)
    }

    pub fn set_skill_detail(&mut self, detail: SkillDetailResponse) {
        self.detail = Some(detail);
        self.detail_error = None;
        self.detail_scroll = 0;
    }

    pub fn set_skill_detail_error(&mut self, message: impl Into<String>) {
        self.detail = None;
        self.detail_error = Some(message.into());
        self.detail_scroll = 0;
    }

    pub fn clear_detail(&mut self) {
        self.detail = None;
        self.detail_error = None;
        self.detail_scroll = 0;
    }

    pub fn begin_create(&mut self) {
        self.mode = SkillListMode::Create(SkillCreateDraft::default());
    }

    pub fn begin_edit(&mut self) -> Result<(), String> {
        let detail = self
            .detail
            .clone()
            .ok_or_else(|| "Load a skill before editing it.".to_string())?;
        if !detail.writable {
            return Err(
                "This skill is read-only in the current workspace. Only workspace-local skills can be edited.".to_string(),
            );
        }
        let extracted = extract_methodology_template_from_markdown(&detail.source);
        self.mode = SkillListMode::Edit(SkillEditDraft {
            name: detail.skill.meta.name,
            description: detail.skill.meta.description,
            category: detail.skill.meta.category,
            active_field: SkillEditField::Description,
            editor_mode: if extracted.is_some() {
                SkillManageEditorMode::Methodology
            } else {
                SkillManageEditorMode::Raw
            },
            source: TextEditorState::with_text(detail.source),
            methodology: extracted
                .as_ref()
                .map(SkillMethodologyDraft::from_template)
                .unwrap_or_default(),
            methodology_loaded: extracted.is_some(),
        });
        Ok(())
    }

    pub fn begin_delete(&mut self) -> Result<(), String> {
        let entry = self
            .selected_entry()
            .cloned()
            .ok_or_else(|| "Select a skill before deleting it.".to_string())?;
        if !entry.writable {
            return Err(
                "This skill is read-only in the current workspace. Only workspace-local skills can be deleted.".to_string(),
            );
        }
        self.mode = SkillListMode::ConfirmDelete { name: entry.name };
        Ok(())
    }

    pub fn cancel_manage_mode(&mut self) {
        self.mode = SkillListMode::Browse;
    }

    pub fn create_payload(&self) -> Option<Result<SkillCreatePayload, String>> {
        let SkillListMode::Create(draft) = &self.mode else {
            return None;
        };
        let name = draft.name.trim().to_string();
        let description = draft.description.trim().to_string();
        let category =
            (!draft.category.trim().is_empty()).then(|| draft.category.trim().to_string());
        Some(match draft.editor_mode {
            SkillManageEditorMode::Raw => Ok(SkillCreatePayload::Raw {
                name,
                description,
                category,
                body: draft.body.text.clone(),
            }),
            SkillManageEditorMode::Methodology => {
                draft
                    .methodology
                    .to_template()
                    .map(|methodology| SkillCreatePayload::Methodology {
                        name,
                        description,
                        category,
                        methodology,
                    })
            }
        })
    }

    pub fn edit_payload(&self) -> Option<Result<SkillEditPayload, String>> {
        let SkillListMode::Edit(draft) = &self.mode else {
            return None;
        };
        Some(match draft.editor_mode {
            SkillManageEditorMode::Raw => Ok(SkillEditPayload::Raw {
                name: draft.name.clone(),
                content: draft.source.text.clone(),
            }),
            SkillManageEditorMode::Methodology => {
                draft
                    .methodology
                    .to_template()
                    .map(|methodology| SkillEditPayload::Methodology {
                        name: draft.name.clone(),
                        description: draft.description.trim().to_string(),
                        methodology,
                    })
            }
        })
    }

    pub fn delete_payload(&self) -> Option<String> {
        let SkillListMode::ConfirmDelete { name } = &self.mode else {
            return None;
        };
        Some(name.clone())
    }

    pub fn handle_manage_char(&mut self, c: char) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::Name => draft.name.push(c),
                SkillCreateField::Description => draft.description.push(c),
                SkillCreateField::Category => draft.category.push(c),
                SkillCreateField::EditorMode | SkillCreateField::MethodologySection => {}
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().insert_char(c)
                }
                SkillCreateField::Body => draft.body.insert_char(c),
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::Description => draft.description.push(c),
                SkillEditField::EditorMode | SkillEditField::MethodologySection => {}
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().insert_char(c)
                }
                SkillEditField::RawSource => draft.source.insert_char(c),
            },
            _ => {}
        }
    }

    pub fn handle_manage_backspace(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::Name => {
                    draft.name.pop();
                }
                SkillCreateField::Description => {
                    draft.description.pop();
                }
                SkillCreateField::Category => {
                    draft.category.pop();
                }
                SkillCreateField::EditorMode | SkillCreateField::MethodologySection => {}
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().backspace()
                }
                SkillCreateField::Body => draft.body.backspace(),
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::Description => {
                    draft.description.pop();
                }
                SkillEditField::EditorMode | SkillEditField::MethodologySection => {}
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().backspace()
                }
                SkillEditField::RawSource => draft.source.backspace(),
            },
            _ => {}
        }
    }

    pub fn handle_manage_delete(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().delete()
                }
                SkillCreateField::Body => draft.body.delete(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().delete()
                }
                SkillEditField::RawSource => draft.source.delete(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_left(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::EditorMode => draft.editor_mode.toggle(),
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_left()
                }
                SkillCreateField::Body => draft.body.move_left(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::EditorMode => draft.editor_mode.toggle(),
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_left()
                }
                SkillEditField::RawSource => draft.source.move_left(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_right(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::EditorMode => draft.editor_mode.toggle(),
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_right()
                }
                SkillCreateField::Body => draft.body.move_right(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::EditorMode => draft.editor_mode.toggle(),
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_right()
                }
                SkillEditField::RawSource => draft.source.move_right(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_home(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_home()
                }
                SkillCreateField::Body => draft.body.move_home(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_home()
                }
                SkillEditField::RawSource => draft.source.move_home(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_end(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_end()
                }
                SkillCreateField::Body => draft.body.move_end(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().move_end()
                }
                SkillEditField::RawSource => draft.source.move_end(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_enter(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::Name => draft.active_field = SkillCreateField::Description,
                SkillCreateField::Description => draft.active_field = SkillCreateField::Category,
                SkillCreateField::Category => draft.active_field = SkillCreateField::EditorMode,
                SkillCreateField::EditorMode => {
                    draft.editor_mode.toggle();
                    draft.active_field = match draft.editor_mode {
                        SkillManageEditorMode::Methodology => SkillCreateField::MethodologySection,
                        SkillManageEditorMode::Raw => SkillCreateField::Body,
                    };
                }
                SkillCreateField::MethodologySection => {
                    draft.active_field = SkillCreateField::MethodologyEditor
                }
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().insert_newline()
                }
                SkillCreateField::Body => draft.body.insert_newline(),
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::Description => draft.active_field = SkillEditField::EditorMode,
                SkillEditField::EditorMode => {
                    draft.editor_mode.toggle();
                    draft.active_field = match draft.editor_mode {
                        SkillManageEditorMode::Methodology => SkillEditField::MethodologySection,
                        SkillManageEditorMode::Raw => SkillEditField::RawSource,
                    };
                }
                SkillEditField::MethodologySection => {
                    draft.active_field = SkillEditField::MethodologyEditor
                }
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().insert_newline()
                }
                SkillEditField::RawSource => draft.source.insert_newline(),
            },
            _ => {}
        }
    }

    pub fn handle_manage_tab(&mut self, reverse: bool) {
        match &mut self.mode {
            SkillListMode::Create(draft) => {
                draft.active_field = if reverse {
                    draft.active_field.previous(draft.editor_mode)
                } else {
                    draft.active_field.next(draft.editor_mode)
                };
            }
            SkillListMode::Edit(draft) => {
                draft.active_field = if reverse {
                    draft.active_field.previous(draft.editor_mode)
                } else {
                    draft.active_field.next(draft.editor_mode)
                };
            }
            _ => {}
        }
    }

    pub fn handle_manage_page_up(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().scroll_up()
                }
                SkillCreateField::Body => draft.body.scroll_up(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().scroll_up()
                }
                SkillEditField::RawSource => draft.source.scroll_up(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_page_down(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft) => match draft.active_field {
                SkillCreateField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().scroll_down()
                }
                SkillCreateField::Body => draft.body.scroll_down(),
                _ => {}
            },
            SkillListMode::Edit(draft) => match draft.active_field {
                SkillEditField::MethodologyEditor => {
                    draft.methodology.selected_editor_mut().scroll_down()
                }
                SkillEditField::RawSource => draft.source.scroll_down(),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn handle_manage_up(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft)
                if matches!(draft.active_field, SkillCreateField::MethodologySection) =>
            {
                draft.methodology.selected_section = draft.methodology.selected_section.previous();
            }
            SkillListMode::Edit(draft)
                if matches!(draft.active_field, SkillEditField::MethodologySection) =>
            {
                draft.methodology.selected_section = draft.methodology.selected_section.previous();
            }
            _ => {}
        }
    }

    pub fn handle_manage_down(&mut self) {
        match &mut self.mode {
            SkillListMode::Create(draft)
                if matches!(draft.active_field, SkillCreateField::MethodologySection) =>
            {
                draft.methodology.selected_section = draft.methodology.selected_section.next();
            }
            SkillListMode::Edit(draft)
                if matches!(draft.active_field, SkillEditField::MethodologySection) =>
            {
                draft.methodology.selected_section = draft.methodology.selected_section.next();
            }
            _ => {}
        }
    }

    fn filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .skills
            .iter()
            .enumerate()
            .filter(|(_, skill)| {
                skill.name.to_ascii_lowercase().contains(&query)
                    || skill.description.to_ascii_lowercase().contains(&query)
                    || skill
                        .category
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
            })
            .map(|(idx, _)| idx)
            .collect();
        self.state.select(if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        });
        self.detail_scroll = 0;
    }

    pub fn render<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        if !self.open {
            return;
        }

        match &self.mode {
            SkillListMode::Browse => self.render_browse(surface, area, theme),
            SkillListMode::Create(draft) => self.render_create(surface, area, theme, draft),
            SkillListMode::Edit(draft) => self.render_edit(surface, area, theme, draft),
            SkillListMode::ConfirmDelete { name } => {
                self.render_delete_confirm(surface, area, theme, name)
            }
        }
    }

    fn render_browse<S: RenderSurface>(&self, surface: &mut S, area: Rect, theme: &Theme) {
        let total_count = self.skills.len();
        let matched_count = self.filtered.len();
        let title = format!(" Skills ({}/{}) ", matched_count, total_count);
        let dialog_area = centered_rect(88, 22, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                title,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(inner);

        surface.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", Style::default().fg(theme.primary)),
                Span::styled(&self.query, Style::default().fg(theme.text)),
                Span::styled("▏", Style::default().fg(theme.primary)),
            ])),
            layout[0],
        );

        let content_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(layout[1]);

        let items = if self.filtered.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No skills available",
                Style::default().fg(theme.text_muted),
            )))]
        } else {
            self.filtered
                .iter()
                .filter_map(|idx| self.skills.get(*idx))
                .map(|skill| {
                    let managed_record = self.managed_record_for_skill(&skill.name);
                    let latest_guard = self.latest_guard_report_for_skill(&skill.name);
                    let mut lines = vec![Line::from(vec![
                        Span::styled("/", Style::default().fg(theme.primary)),
                        Span::styled(
                            &skill.name,
                            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                        ),
                    ])];
                    let mut detail = String::new();
                    if let Some(category) =
                        skill.category.as_deref().filter(|value| !value.is_empty())
                    {
                        detail.push('[');
                        detail.push_str(category);
                        detail.push_str("] ");
                    }
                    detail.push_str(&skill.description);
                    if !skill.supporting_files.is_empty() {
                        detail.push_str(&format!(" · files {}", skill.supporting_files.len()));
                    }
                    if skill.writable {
                        detail.push_str(" · workspace");
                    } else {
                        detail.push_str(" · read-only");
                    }
                    lines.push(Line::from(Span::styled(
                        detail,
                        Style::default().fg(theme.text_muted),
                    )));
                    let governance = self.governance_summary_line(skill.name.as_str());
                    if !governance.is_empty() {
                        lines.push(Line::from(Span::styled(
                            governance,
                            Style::default().fg(governance_line_color(
                                theme,
                                managed_record,
                                latest_guard,
                            )),
                        )));
                    }
                    lines.push(Line::from(Span::styled(
                        skill.location.as_str(),
                        Style::default().fg(theme.text_muted),
                    )));
                    ListItem::new(Text::from(lines))
                })
                .collect::<Vec<_>>()
        };

        surface.render_stateful_widget(
            List::new(items).highlight_style(
                Style::default()
                    .bg(theme.background_element)
                    .add_modifier(Modifier::BOLD),
            ),
            content_layout[0],
            &mut self.state.clone(),
        );

        let pane_title = match self.browse_pane {
            SkillBrowsePane::Preview => " Preview ",
            SkillBrowsePane::Timeline => " Governance Timeline ",
        };
        let preview_block = Block::default()
            .title(Span::styled(
                pane_title,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let preview_inner = super::dialog_inner(preview_block.inner(content_layout[1]));
        surface.render_widget(preview_block, content_layout[1]);

        let preview_lines = match self.browse_pane {
            SkillBrowsePane::Preview => {
                if let Some(detail) = &self.detail {
                    let meta = &detail.skill.meta;
                    let mut lines = self.hub_preview_lines(theme);
                    lines.extend(vec![
                        Line::from(Span::styled(
                            meta.name.as_str(),
                            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(
                            meta.description.as_str(),
                            Style::default().fg(theme.text_muted),
                        )),
                        Line::from(Span::styled(
                            format!(
                                "{} · {} supporting files · {}",
                                meta.category.as_deref().unwrap_or("uncategorized"),
                                meta.supporting_files.len(),
                                if detail.writable {
                                    "workspace writable"
                                } else {
                                    "read-only"
                                }
                            ),
                            Style::default().fg(theme.text_muted),
                        )),
                        Line::from(Span::styled(
                            meta.location.as_str(),
                            Style::default().fg(theme.text_muted),
                        )),
                        Line::from(""),
                    ]);
                    lines.extend(detail.source.lines().map(|line| {
                        Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(theme.text),
                        ))
                    }));
                    lines
                } else if let Some(message) = &self.detail_error {
                    let mut lines = self.hub_preview_lines(theme);
                    lines.push(Line::from(Span::styled(
                        message.as_str(),
                        Style::default().fg(theme.error),
                    )));
                    lines
                } else {
                    let mut lines = self.hub_preview_lines(theme);
                    lines.push(Line::from(Span::styled(
                        "Select a skill to load its raw SKILL.md preview.",
                        Style::default().fg(theme.text_muted),
                    )));
                    lines
                }
            }
            SkillBrowsePane::Timeline => self.timeline_lines(theme),
        };

        surface.render_widget(
            Paragraph::new(preview_lines)
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .style(Style::default().bg(theme.background_panel)),
            preview_inner,
        );

        let footer = format!(
            "Enter insert /skill  c create  e edit  d delete  g guard-skill  G guard-source  i cycle-source  x refresh-index  p plan-sync  a apply-sync  u/U install plan/apply  v/V update plan/apply  D detach  R remove  r refresh-hub  t preview/timeline  PgUp/PgDn scroll  Esc close  Matched: {}/{}",
            matched_count, total_count
        );
        surface.render_widget(
            Paragraph::new(footer).style(Style::default().fg(theme.text_muted)),
            layout[2],
        );
    }

    fn render_create<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        draft: &SkillCreateDraft,
    ) {
        let dialog_area = centered_rect(94, 28, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Create Skill ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(10),
                Constraint::Length(2),
            ])
            .split(inner);

        render_field_line(
            surface,
            layout[0],
            theme,
            "Name",
            &draft.name,
            matches!(draft.active_field, SkillCreateField::Name),
        );
        render_field_line(
            surface,
            layout[1],
            theme,
            "Description",
            &draft.description,
            matches!(draft.active_field, SkillCreateField::Description),
        );
        render_field_line(
            surface,
            layout[2],
            theme,
            "Category",
            &draft.category,
            matches!(draft.active_field, SkillCreateField::Category),
        );
        render_field_line(
            surface,
            layout[3],
            theme,
            "Editor",
            draft.editor_mode.label(),
            matches!(draft.active_field, SkillCreateField::EditorMode),
        );

        let body_inner = match draft.editor_mode {
            SkillManageEditorMode::Raw => Some(self.render_raw_skill_editor(
                surface,
                layout[4],
                theme,
                "Body",
                "Write markdown body here...",
                &draft.body,
                matches!(draft.active_field, SkillCreateField::Body),
            )),
            SkillManageEditorMode::Methodology => {
                self.render_methodology_editor(
                    surface,
                    layout[4],
                    theme,
                    draft.name.trim(),
                    &draft.methodology,
                    matches!(draft.active_field, SkillCreateField::MethodologySection),
                    matches!(draft.active_field, SkillCreateField::MethodologyEditor),
                );
                None
            }
        };

        let footer = vec![
            Line::from(Span::styled(
                "Tab move fields  Up/Down choose methodology section  Enter edits/toggles  Ctrl+S create  Esc cancel",
                Style::default().fg(theme.text_muted),
            )),
            Line::from(Span::styled(
                "Creates a workspace-local skill via /skill/manage create, using methodology or raw markdown.",
                Style::default().fg(theme.text_muted),
            )),
        ];
        surface.render_widget(Paragraph::new(footer), layout[5]);

        match draft.active_field {
            SkillCreateField::Name => {
                set_line_cursor(
                    surface,
                    layout[0],
                    "Name",
                    draft.name.chars().count() as u16,
                );
            }
            SkillCreateField::Description => {
                set_line_cursor(
                    surface,
                    layout[1],
                    "Description",
                    draft.description.chars().count() as u16,
                );
            }
            SkillCreateField::Category => {
                set_line_cursor(
                    surface,
                    layout[2],
                    "Category",
                    draft.category.chars().count() as u16,
                );
            }
            SkillCreateField::Body => {
                let Some(body_inner) = body_inner else {
                    return;
                };
                let (row, col) = draft.body.cursor_row_col();
                let cursor_y = body_inner
                    .y
                    .saturating_add(row.saturating_sub(draft.body.scroll));
                let cursor_x = body_inner.x.saturating_add(col);
                if cursor_y < body_inner.y.saturating_add(body_inner.height) {
                    surface.set_cursor_position(cursor_x, cursor_y);
                }
            }
            SkillCreateField::MethodologyEditor => {
                self.set_methodology_editor_cursor(surface, &draft.methodology, layout[4]);
            }
            SkillCreateField::EditorMode | SkillCreateField::MethodologySection => {}
        }
    }

    fn render_edit<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        draft: &SkillEditDraft,
    ) {
        let dialog_area = centered_rect(94, 28, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                format!(" Edit Skill: {} ", draft.name),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(2),
            ])
            .split(inner);

        render_field_line(
            surface,
            layout[0],
            theme,
            "Description",
            &draft.description,
            matches!(draft.active_field, SkillEditField::Description),
        );
        render_field_line(
            surface,
            layout[1],
            theme,
            "Editor",
            draft.editor_mode.label(),
            matches!(draft.active_field, SkillEditField::EditorMode),
        );

        let editor_inner = match draft.editor_mode {
            SkillManageEditorMode::Raw => Some(self.render_raw_skill_editor(
                surface,
                layout[3],
                theme,
                "Source",
                "Edit raw SKILL.md source here...",
                &draft.source,
                matches!(draft.active_field, SkillEditField::RawSource),
            )),
            SkillManageEditorMode::Methodology => {
                self.render_methodology_editor(
                    surface,
                    layout[3],
                    theme,
                    draft.name.trim(),
                    &draft.methodology,
                    matches!(draft.active_field, SkillEditField::MethodologySection),
                    matches!(draft.active_field, SkillEditField::MethodologyEditor),
                );
                None
            }
        };

        surface.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!(
                        "Category: {}",
                        draft.category.as_deref().unwrap_or("uncategorized")
                    ),
                    Style::default().fg(theme.text_muted),
                )),
                Line::from(Span::styled(
                    if draft.methodology_loaded {
                        "This skill matched the methodology template, so structured patch mode is available."
                    } else {
                        "This skill did not round-trip into the methodology template; raw markdown remains the safest edit path."
                    },
                    Style::default().fg(theme.text_muted),
                )),
                Line::from(Span::styled(
                    "Ctrl+S save  Esc cancel  Tab move fields  Up/Down choose methodology section",
                    Style::default().fg(theme.text_muted),
                )),
            ]),
            layout[2],
        );

        surface.render_widget(
            Paragraph::new("Saving uses POST /skill/manage action=edit for raw mode, or action=patch with methodology for structured mode.")
                .style(Style::default().fg(theme.text_muted)),
            layout[4],
        );

        match draft.active_field {
            SkillEditField::Description => {
                set_line_cursor(
                    surface,
                    layout[0],
                    "Description",
                    draft.description.chars().count() as u16,
                );
            }
            SkillEditField::RawSource => {
                let Some(editor_inner) = editor_inner else {
                    return;
                };
                let (row, col) = draft.source.cursor_row_col();
                let cursor_y = editor_inner
                    .y
                    .saturating_add(row.saturating_sub(draft.source.scroll));
                let cursor_x = editor_inner.x.saturating_add(col);
                if cursor_y < editor_inner.y.saturating_add(editor_inner.height) {
                    surface.set_cursor_position(cursor_x, cursor_y);
                }
            }
            SkillEditField::MethodologyEditor => {
                self.set_methodology_editor_cursor(surface, &draft.methodology, layout[3]);
            }
            SkillEditField::EditorMode | SkillEditField::MethodologySection => {}
        }
    }

    fn render_raw_skill_editor<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        title: &str,
        placeholder: &str,
        editor: &TextEditorState,
        active: bool,
    ) -> Rect {
        let editor_block = Block::default()
            .title(Span::styled(
                format!(" {} ", title),
                field_block_style(theme, active).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(field_block_style(theme, active))
            .style(Style::default().bg(theme.background_panel));
        let editor_inner = super::dialog_inner(editor_block.inner(area));
        surface.render_widget(editor_block, area);
        surface.render_widget(
            Paragraph::new(if editor.text.is_empty() {
                vec![Line::from(Span::styled(
                    placeholder,
                    Style::default().fg(theme.text_muted),
                ))]
            } else {
                editor
                    .text
                    .lines()
                    .map(|line| {
                        Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(theme.text),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .wrap(Wrap { trim: false })
            .scroll((editor.scroll, 0)),
            editor_inner,
        );
        editor_inner
    }

    fn render_methodology_editor<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        skill_name: &str,
        draft: &SkillMethodologyDraft,
        section_active: bool,
        editor_active: bool,
    ) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(24),
                Constraint::Percentage(33),
                Constraint::Percentage(43),
            ])
            .split(area);

        let selector_block = Block::default()
            .title(Span::styled(
                " Methodology ",
                field_block_style(theme, section_active).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(field_block_style(theme, section_active))
            .style(Style::default().bg(theme.background_panel));
        let selector_inner = super::dialog_inner(selector_block.inner(columns[0]));
        surface.render_widget(selector_block, columns[0]);
        let selector_items = SkillMethodologySection::ALL
            .iter()
            .map(|section| {
                let active_style = if *section == draft.selected_section {
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                };
                ListItem::new(Text::from(vec![
                    Line::from(Span::styled(draft.section_summary(*section), active_style)),
                    Line::from(Span::styled(
                        section.hint(),
                        Style::default().fg(theme.text_muted),
                    )),
                ]))
            })
            .collect::<Vec<_>>();
        surface.render_widget(List::new(selector_items), selector_inner);

        let editor_block = Block::default()
            .title(Span::styled(
                format!(" {} ", draft.selected_section.title()),
                field_block_style(theme, editor_active).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(field_block_style(theme, editor_active))
            .style(Style::default().bg(theme.background_panel));
        let editor_inner = super::dialog_inner(editor_block.inner(columns[1]));
        surface.render_widget(editor_block, columns[1]);
        let selected_editor = draft.selected_editor();
        surface.render_widget(
            Paragraph::new(if selected_editor.text.is_empty() {
                vec![Line::from(Span::styled(
                    draft.selected_section.hint(),
                    Style::default().fg(theme.text_muted),
                ))]
            } else {
                selected_editor
                    .text
                    .lines()
                    .map(|line| {
                        Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(theme.text),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .wrap(Wrap { trim: false })
            .scroll((selected_editor.scroll, 0)),
            editor_inner,
        );

        let preview_block = Block::default()
            .title(Span::styled(
                " Preview ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.background_panel));
        let preview_inner = super::dialog_inner(preview_block.inner(columns[2]));
        surface.render_widget(preview_block, columns[2]);
        let preview_name = if skill_name.trim().is_empty() {
            "draft-skill"
        } else {
            skill_name
        };
        let preview_lines = match draft.preview(preview_name) {
            Ok(preview) => preview
                .lines()
                .map(|line| {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(theme.text),
                    ))
                })
                .collect::<Vec<_>>(),
            Err(error) => vec![
                Line::from(Span::styled(
                    "Methodology preview is incomplete.",
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(error, Style::default().fg(theme.text_muted))),
            ],
        };
        surface.render_widget(
            Paragraph::new(preview_lines)
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(theme.background_panel)),
            preview_inner,
        );
    }

    fn set_methodology_editor_cursor<S: RenderSurface>(
        &self,
        surface: &mut S,
        draft: &SkillMethodologyDraft,
        area: Rect,
    ) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(24),
                Constraint::Percentage(33),
                Constraint::Percentage(43),
            ])
            .split(area);
        let editor_block = Block::default().borders(Borders::ALL);
        let editor_inner = super::dialog_inner(editor_block.inner(columns[1]));
        let selected_editor = draft.selected_editor();
        let (row, col) = selected_editor.cursor_row_col();
        let cursor_y = editor_inner
            .y
            .saturating_add(row.saturating_sub(selected_editor.scroll));
        let cursor_x = editor_inner.x.saturating_add(col);
        if cursor_y < editor_inner.y.saturating_add(editor_inner.height) {
            surface.set_cursor_position(cursor_x, cursor_y);
        }
    }

    fn render_delete_confirm<S: RenderSurface>(
        &self,
        surface: &mut S,
        area: Rect,
        theme: &Theme,
        name: &str,
    ) {
        let dialog_area = centered_rect(64, 8, area);
        surface.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(Span::styled(
                " Delete Skill ",
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning))
            .style(Style::default().bg(theme.background_panel));
        let inner = super::dialog_inner(block.inner(dialog_area));
        surface.render_widget(block, dialog_area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        surface.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    format!("Delete workspace skill `{}`?", name),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "This will call POST /skill/manage action=delete.",
                    Style::default().fg(theme.text_muted),
                )),
            ]),
            layout[0],
        );
        surface.render_widget(
            Paragraph::new("Enter confirm  Esc cancel")
                .style(Style::default().fg(theme.text_muted)),
            layout[1],
        );
    }

    fn hub_preview_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let pane_label = match self.browse_pane {
            SkillBrowsePane::Preview => "preview",
            SkillBrowsePane::Timeline => "timeline",
        };
        let selected_source = self.selected_hub_source().map(|source| {
            format!(
                "{} · {}",
                source.source_id,
                source
                    .revision
                    .as_deref()
                    .unwrap_or(source.locator.as_str())
            )
        });
        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "Hub: managed {} · indexed {} · distributions {} · artifacts {} · lifecycle {} · view {}",
                    self.managed_skills.len(),
                    self.source_indices.len(),
                    self.distributions.len(),
                    self.artifact_cache.len(),
                    self.lifecycle_records.len(),
                    pane_label
                ),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(
                    "Selected source: {}",
                    selected_source.unwrap_or_else(|| "none".to_string())
                ),
                Style::default().fg(theme.text_muted),
            )),
        ];
        if let Some(policy) = &self.hub_policy {
            lines.push(Line::from(Span::styled(
                format!(
                    "Policy: retention {} · timeout {} · download {} · extract {}",
                    format_duration_seconds(policy.artifact_cache_retention_seconds),
                    format_duration_ms(policy.fetch_timeout_ms),
                    format_bytes(policy.max_download_bytes),
                    format_bytes(policy.max_extract_bytes),
                ),
                Style::default().fg(theme.text_muted),
            )));
        }
        if let Some(plan) = &self.hub_plan {
            lines.push(Line::from(Span::styled(
                format!(
                    "Latest sync plan: {} entries for {}",
                    plan.entries.len(),
                    plan.source_id
                ),
                Style::default().fg(theme.text_muted),
            )));
            if let Some(entry) = plan.entries.first() {
                lines.push(Line::from(Span::styled(
                    format!("First action: {} -> {:?}", entry.skill_name, entry.action),
                    Style::default().fg(theme.text_muted),
                )));
            }
        }
        if let Some(skill_name) = self.resolve_remote_install_skill_name() {
            lines.push(Line::from(Span::styled(
                format!("Remote target: {}", skill_name),
                Style::default().fg(theme.text_muted),
            )));
            if let Some(plan) = &self.remote_install_plan {
                if plan.entry.skill_name.eq_ignore_ascii_case(&skill_name) {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Latest remote plan: {:?} via {}",
                            plan.entry.action, plan.source_id
                        ),
                        Style::default().fg(theme.text_muted),
                    )));
                }
            }
            if let Some(source) = self.selected_hub_source_snapshot() {
                if let Some(entry) = source
                    .entries
                    .iter()
                    .find(|entry| entry.skill_name.eq_ignore_ascii_case(&skill_name))
                {
                    let mut entry_summary = String::new();
                    if let Some(category) =
                        entry.category.as_deref().filter(|value| !value.is_empty())
                    {
                        entry_summary.push_str(category);
                    } else {
                        entry_summary.push_str("uncategorized");
                    }
                    if let Some(revision) =
                        entry.revision.as_deref().filter(|value| !value.is_empty())
                    {
                        entry_summary.push_str(" · ");
                        entry_summary.push_str(revision);
                    }
                    if let Some(description) = entry
                        .description
                        .as_deref()
                        .filter(|value| !value.is_empty())
                    {
                        entry_summary.push_str(" · ");
                        entry_summary.push_str(description);
                    }
                    lines.push(Line::from(Span::styled(
                        format!("Remote index: {}", entry_summary),
                        Style::default().fg(theme.text_muted),
                    )));
                }
                if let Some(distribution) = self.distributions.iter().rev().find(|record| {
                    record.source.source_id == source.source.source_id
                        && record.skill_name.eq_ignore_ascii_case(&skill_name)
                }) {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Distribution: {} · version {} · revision {} · {:?}",
                            distribution.distribution_id,
                            distribution.release.version.as_deref().unwrap_or("--"),
                            distribution.release.revision.as_deref().unwrap_or("--"),
                            distribution.lifecycle
                        ),
                        Style::default().fg(theme.text_muted),
                    )));
                    if let Some(cache_entry) = self.artifact_cache.iter().rev().find(|entry| {
                        entry.artifact.artifact_id == distribution.resolution.artifact.artifact_id
                    }) {
                        lines.push(Line::from(Span::styled(
                            format!(
                                "Artifact cache: {:?} @ {}",
                                cache_entry.status, cache_entry.cached_at
                            ),
                            Style::default().fg(theme.text_muted),
                        )));
                        if let Some(error) = cache_entry
                            .error
                            .as_deref()
                            .filter(|value| !value.is_empty())
                        {
                            lines.push(Line::from(Span::styled(
                                format!("Artifact error: {}", error),
                                Style::default().fg(theme.error),
                            )));
                        }
                    }
                }
                if let Some(lifecycle) = self.lifecycle_records.iter().rev().find(|record| {
                    record.source_id == source.source.source_id
                        && record.skill_name.eq_ignore_ascii_case(&skill_name)
                }) {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Lifecycle: {:?} @ {}",
                            lifecycle.state, lifecycle.updated_at
                        ),
                        Style::default().fg(theme.text_muted),
                    )));
                    if let Some(error) =
                        lifecycle.error.as_deref().filter(|value| !value.is_empty())
                    {
                        lines.push(Line::from(Span::styled(
                            format!("Lifecycle error: {}", error),
                            Style::default().fg(theme.error),
                        )));
                    }
                }
                let indexed = source
                    .entries
                    .iter()
                    .take(6)
                    .map(|entry| {
                        let is_target = entry.skill_name.eq_ignore_ascii_case(&skill_name);
                        Line::from(Span::styled(
                            format!(
                                "{} {} ({})",
                                if is_target { ">" } else { "-" },
                                entry.skill_name,
                                entry.revision.as_deref().unwrap_or("unversioned")
                            ),
                            Style::default().fg(if is_target {
                                theme.primary
                            } else {
                                theme.text_muted
                            }),
                        ))
                    })
                    .collect::<Vec<_>>();
                if !indexed.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "Indexed remote entries:",
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                    )));
                    lines.extend(indexed);
                }
            }
        }
        if let Some(target_label) = &self.guard_target_label {
            lines.push(Line::from(Span::styled(
                format!(
                    "Latest guard run: {} report(s) for {}",
                    self.guard_reports.len(),
                    target_label
                ),
                Style::default().fg(theme.text_muted),
            )));
            if let Some(report) = self.guard_reports.first() {
                lines.push(Line::from(Span::styled(
                    format!(
                        "First guard status: {} -> {:?} ({} violations)",
                        report.skill_name,
                        report.status,
                        report.violations.len()
                    ),
                    Style::default().fg(theme.text_muted),
                )));
            }
        }
        lines.push(Line::from(""));
        lines
    }

    fn timeline_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = self.hub_preview_lines(theme);
        let focus_label = self.timeline_focus_label();
        lines.push(Line::from(Span::styled(
            format!("Timeline focus: {}", focus_label),
            Style::default().fg(theme.text_muted),
        )));
        let entries = self.focused_timeline_entries();
        if entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "No governance timeline entries for the current selection.",
                Style::default().fg(theme.text_muted),
            )));
            return lines;
        }

        for entry in entries.iter().take(10) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("[{}] ", timeline_status_label(entry)),
                    Style::default().fg(timeline_status_color(theme, entry)),
                ),
                Span::styled(
                    entry.title.clone(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                format!(
                    "{} · {} · {}",
                    timeline_timestamp_label(entry.created_at),
                    entry.skill_name.as_deref().unwrap_or("--"),
                    entry.source_id.as_deref().unwrap_or("--")
                ),
                Style::default().fg(theme.text_muted),
            )));
            lines.push(Line::from(Span::styled(
                entry.summary.clone(),
                Style::default().fg(theme.text_muted),
            )));
            if let Some(report) = entry.guard_report.as_ref() {
                for violation in report.violations.iter().take(2) {
                    lines.push(Line::from(Span::styled(
                        format!("  - {}: {}", violation.rule_id, violation.message),
                        Style::default().fg(theme.warning),
                    )));
                }
            } else if let Some(record) = entry.managed_record.as_ref() {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  - revision {} · {}",
                        record.installed_revision.as_deref().unwrap_or("--"),
                        if record.deleted_locally {
                            "deleted locally"
                        } else if record.locally_modified {
                            "locally modified"
                        } else {
                            "clean"
                        }
                    ),
                    Style::default().fg(theme.text_muted),
                )));
            }
            lines.push(Line::from(""));
        }
        lines
    }

    fn timeline_focus_label(&self) -> String {
        if let Some(skill_name) = self.selected_skill() {
            return format!("skill `{}`", skill_name);
        }
        if let Some(source) = self.selected_hub_source() {
            return format!("source `{}`", source.source_id);
        }
        "all governance entries".to_string()
    }

    fn focused_timeline_entries(&self) -> Vec<&SkillGovernanceTimelineEntry> {
        if let Some(skill_name) = self.selected_skill() {
            let normalized = skill_name.trim().to_ascii_lowercase();
            let entries = self
                .governance_timeline
                .iter()
                .filter(|entry| {
                    entry
                        .skill_name
                        .as_deref()
                        .map(|name| name.trim().eq_ignore_ascii_case(&normalized))
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>();
            if !entries.is_empty() {
                return entries;
            }
        }

        if let Some(source) = self.selected_hub_source() {
            let entries = self
                .governance_timeline
                .iter()
                .filter(|entry| entry.source_id.as_deref() == Some(source.source_id.as_str()))
                .collect::<Vec<_>>();
            if !entries.is_empty() {
                return entries;
            }
        }

        self.governance_timeline.iter().collect::<Vec<_>>()
    }

    fn managed_record_for_skill(&self, skill_name: &str) -> Option<&ManagedSkillRecord> {
        self.managed_skills
            .iter()
            .find(|record| record.skill_name.eq_ignore_ascii_case(skill_name))
    }

    fn latest_guard_report_for_skill(
        &self,
        skill_name: &str,
    ) -> Option<&crate::api::SkillGuardReport> {
        self.governance_timeline
            .iter()
            .find(|entry| {
                entry
                    .skill_name
                    .as_deref()
                    .map(|name| name.eq_ignore_ascii_case(skill_name))
                    .unwrap_or(false)
                    && entry.guard_report.is_some()
            })
            .and_then(|entry| entry.guard_report.as_ref())
    }

    fn governance_summary_line(&self, skill_name: &str) -> String {
        let mut parts = Vec::new();
        if let Some(record) = self.managed_record_for_skill(skill_name) {
            let source_label = record
                .source
                .as_ref()
                .map(|source| source.source_id.as_str())
                .unwrap_or("workspace-local");
            parts.push(format!("source {}", source_label));
            if record.deleted_locally {
                parts.push("deleted locally".to_string());
            } else if record.locally_modified {
                parts.push("locally modified".to_string());
            } else {
                parts.push("managed clean".to_string());
            }
        }
        if let Some(report) = self.latest_guard_report_for_skill(skill_name) {
            parts.push(format!(
                "guard {} ({})",
                timeline_guard_status_label(report),
                report.violations.len()
            ));
        }
        parts.join(" · ")
    }
}

fn split_non_empty_lines(input: &str) -> Vec<String> {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_methodology_steps_input(input: &str) -> Result<Vec<SkillMethodologyStep>, String> {
    let mut steps = Vec::new();
    for (index, line) in split_non_empty_lines(input).into_iter().enumerate() {
        let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(format!(
                "core step line {} must use `Title | Action | Outcome(optional)`",
                index + 1
            ));
        }
        let title = parts[0];
        let action = parts[1];
        if title.is_empty() || action.is_empty() {
            return Err(format!(
                "core step line {} must include both title and action",
                index + 1
            ));
        }
        let outcome = parts
            .get(2)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        steps.push(SkillMethodologyStep {
            title: title.to_string(),
            action: action.to_string(),
            outcome,
            experienced_tools: vec![],
        });
    }
    Ok(steps)
}

fn parse_methodology_references_input(
    input: &str,
) -> Result<Vec<SkillMethodologyReference>, String> {
    let mut references = Vec::new();
    for (index, line) in split_non_empty_lines(input).into_iter().enumerate() {
        let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(format!(
                "reference line {} must use `path | label`",
                index + 1
            ));
        }
        let path = parts[0];
        let label = parts[1];
        if path.is_empty() || label.is_empty() {
            return Err(format!(
                "reference line {} must include both path and label",
                index + 1
            ));
        }
        references.push(SkillMethodologyReference {
            label: label.to_string(),
            path: path.to_string(),
        });
    }
    Ok(references)
}

impl Default for SkillListDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn render_field_line<S: RenderSurface>(
    surface: &mut S,
    area: Rect,
    theme: &Theme,
    label: &str,
    value: &str,
    active: bool,
) {
    let block = Block::default()
        .title(Span::styled(
            format!(" {} ", label),
            field_block_style(theme, active).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(field_block_style(theme, active))
        .style(Style::default().bg(theme.background_panel));
    let inner = super::dialog_inner(block.inner(area));
    surface.render_widget(block, area);
    surface.render_widget(
        Paragraph::new(if value.is_empty() {
            Line::from(Span::styled(" ", Style::default().fg(theme.text_muted)))
        } else {
            Line::from(Span::styled(
                value.to_string(),
                Style::default().fg(theme.text),
            ))
        }),
        inner,
    );
}

fn field_block_style(theme: &Theme, active: bool) -> Style {
    if active {
        Style::default().fg(theme.primary)
    } else {
        Style::default().fg(theme.border)
    }
}

fn timeline_status_label(entry: &SkillGovernanceTimelineEntry) -> &'static str {
    match entry.status {
        crate::api::SkillGovernanceTimelineStatus::Info => "info",
        crate::api::SkillGovernanceTimelineStatus::Success => "ok",
        crate::api::SkillGovernanceTimelineStatus::Warn => "warn",
        crate::api::SkillGovernanceTimelineStatus::Error => "error",
    }
}

fn timeline_status_color(
    theme: &Theme,
    entry: &SkillGovernanceTimelineEntry,
) -> ratatui::style::Color {
    match entry.status {
        crate::api::SkillGovernanceTimelineStatus::Info => theme.primary,
        crate::api::SkillGovernanceTimelineStatus::Success => theme.success,
        crate::api::SkillGovernanceTimelineStatus::Warn => theme.warning,
        crate::api::SkillGovernanceTimelineStatus::Error => theme.error,
    }
}

fn timeline_timestamp_label(timestamp: i64) -> String {
    if timestamp <= 0 {
        "timestamp --".to_string()
    } else {
        format!("timestamp {}", timestamp)
    }
}

fn governance_line_color(
    theme: &Theme,
    managed_record: Option<&ManagedSkillRecord>,
    latest_guard: Option<&crate::api::SkillGuardReport>,
) -> ratatui::style::Color {
    if latest_guard
        .map(|report| report.status == crate::api::SkillGuardStatus::Blocked)
        .unwrap_or(false)
    {
        return theme.error;
    }
    if latest_guard
        .map(|report| report.status == crate::api::SkillGuardStatus::Warn)
        .unwrap_or(false)
        || managed_record
            .map(|record| record.locally_modified || record.deleted_locally)
            .unwrap_or(false)
    {
        return theme.warning;
    }
    theme.text_muted
}

fn timeline_guard_status_label(report: &crate::api::SkillGuardReport) -> &'static str {
    match report.status {
        crate::api::SkillGuardStatus::Passed => "passed",
        crate::api::SkillGuardStatus::Warn => "warn",
        crate::api::SkillGuardStatus::Blocked => "blocked",
    }
}

fn format_duration_seconds(value: u64) -> String {
    if value % 86_400 == 0 {
        format!("{}d", value / 86_400)
    } else if value % 3_600 == 0 {
        format!("{}h", value / 3_600)
    } else if value % 60 == 0 {
        format!("{}m", value / 60)
    } else {
        format!("{}s", value)
    }
}

fn format_duration_ms(value: u64) -> String {
    if value % 1000 == 0 {
        format!("{}s", value / 1000)
    } else {
        format!("{}ms", value)
    }
}

fn format_bytes(value: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    if value >= MIB && value % MIB == 0 {
        format!("{} MiB", value / MIB)
    } else if value >= KIB && value % KIB == 0 {
        format!("{} KiB", value / KIB)
    } else {
        format!("{} B", value)
    }
}

fn set_line_cursor<S: RenderSurface>(surface: &mut S, area: Rect, label: &str, value_width: u16) {
    let x = area.x.saturating_add(label.len() as u16).saturating_add(4);
    let y = area.y.saturating_add(1);
    surface.set_cursor_position(x.saturating_add(value_width), y);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    super::centered_rect(width, height, area)
}

fn prev_char_boundary(input: &str, cursor_position: usize) -> Option<usize> {
    if cursor_position == 0 || cursor_position > input.len() {
        return None;
    }
    input[..cursor_position]
        .char_indices()
        .last()
        .map(|(idx, _)| idx)
}

fn next_char_boundary(input: &str, cursor_position: usize) -> Option<usize> {
    if cursor_position >= input.len() {
        return None;
    }
    let suffix = &input[cursor_position..];
    suffix
        .chars()
        .next()
        .map(|ch| cursor_position + ch.len_utf8())
}

fn line_start(input: &str, cursor_position: usize) -> usize {
    input[..cursor_position.min(input.len())]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0)
}

fn line_end(input: &str, cursor_position: usize) -> usize {
    let cursor_position = cursor_position.min(input.len());
    input[cursor_position..]
        .find('\n')
        .map(|idx| cursor_position + idx)
        .unwrap_or(input.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    use crate::ui::BufferSurface;

    #[test]
    fn skill_list_dialog_renders_to_buffer_surface() {
        let mut dialog = SkillListDialog::new();
        dialog.set_skills(vec![SkillCatalogEntry {
            name: "review".to_string(),
            description: "Review the current implementation".to_string(),
            location: "/tmp/review/SKILL.md".to_string(),
            category: Some("analysis".to_string()),
            supporting_files: vec!["references/checklist.md".to_string()],
            writable: true,
        }]);
        dialog.open();

        let area = Rect::new(0, 0, 120, 36);
        let mut buffer = Buffer::empty(area);
        let mut surface = BufferSurface::new(&mut buffer);

        dialog.render(&mut surface, area, &Theme::dark());

        let rendered = buffer
            .content
            .iter()
            .filter(|cell| !cell.symbol().trim().is_empty())
            .count();
        assert!(rendered > 0);
    }
}
