use agendao_command_render::cli_style::CliStyle;
use agendao_command_render::live_semantic_consumer::LiveSemanticConsumer;
use agendao_command_render::output_blocks::{
    render_cli_block_rich, MessageBlock, MessagePhase, OutputBlock, ReasoningBlock, ToolPhase,
};

const CLI_TRANSCRIPT_MAX_LINES: usize = 1200;

#[derive(Debug, Clone)]
enum TranscriptEntry {
    Committed {
        rendered_ansi: String,
    },
    LiveSlot {
        slot_key: String,
        rendered_ansi: String,
        rendered_plain: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct CliVisibleTranscript {
    entries: Vec<TranscriptEntry>,
    max_lines: usize,
    ansi_capable: bool,
}

impl CliVisibleTranscript {
    pub(super) fn new(ansi_capable: bool) -> Self {
        Self {
            entries: Vec::new(),
            max_lines: CLI_TRANSCRIPT_MAX_LINES,
            ansi_capable,
        }
    }

    pub(super) fn append_committed(&mut self, rendered_ansi: &str) {
        for line in rendered_ansi.split_inclusive('\n') {
            self.entries.push(TranscriptEntry::Committed {
                rendered_ansi: line.to_string(),
            });
        }
        self.trim_to_budget();
    }

    pub(super) fn upsert_live_slot(
        &mut self,
        slot_key: &str,
        rendered_ansi: String,
        rendered_plain: String,
    ) {
        for entry in &mut self.entries {
            if let TranscriptEntry::LiveSlot {
                slot_key: ref existing_key,
                ..
            } = entry
            {
                if existing_key == slot_key {
                    *entry = TranscriptEntry::LiveSlot {
                        slot_key: slot_key.to_string(),
                        rendered_ansi,
                        rendered_plain,
                    };
                    return;
                }
            }
        }

        self.entries.push(TranscriptEntry::LiveSlot {
            slot_key: slot_key.to_string(),
            rendered_ansi,
            rendered_plain,
        });
    }

    pub(super) fn finalize_live_slot(
        &mut self,
        slot_key: &str,
        suffix_ansi: String,
        suffix_plain: String,
    ) {
        for entry in &mut self.entries {
            if let TranscriptEntry::LiveSlot {
                slot_key: ref existing_key,
                rendered_ansi,
                rendered_plain,
            } = entry
            {
                if existing_key == slot_key {
                    if !suffix_ansi.is_empty() {
                        rendered_ansi.push_str(&suffix_ansi);
                    }
                    if !suffix_plain.is_empty() {
                        rendered_plain.push_str(&suffix_plain);
                    }
                    *entry = TranscriptEntry::Committed {
                        rendered_ansi: rendered_ansi.clone(),
                    };
                    return;
                }
            }
        }
    }

    pub(super) fn rendered_text(&self) -> String {
        if self.ansi_capable {
            self.visible_ansi()
        } else {
            self.visible_plain()
        }
    }

    fn visible_ansi(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                TranscriptEntry::Committed { rendered_ansi } => out.push_str(rendered_ansi),
                TranscriptEntry::LiveSlot { rendered_ansi, .. } => out.push_str(rendered_ansi),
            }
        }
        out
    }

    fn visible_plain(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            match entry {
                TranscriptEntry::Committed { rendered_ansi } => {
                    out.push_str(&agendao_util::util::color::strip_ansi(rendered_ansi));
                }
                TranscriptEntry::LiveSlot { rendered_plain, .. } => out.push_str(rendered_plain),
            }
        }
        out
    }

    fn trim_to_budget(&mut self) {
        let line_count = self.entries.len();
        if line_count > self.max_lines {
            let overflow = line_count - self.max_lines;
            self.entries.drain(0..overflow);
        }
    }
}

impl Default for CliVisibleTranscript {
    fn default() -> Self {
        Self::new(CliStyle::detect().color)
    }
}

fn cli_render_live_slot_snapshot(
    block: &OutputBlock,
    live_identity: &agendao_types::LiveMessagePartIdentity,
    style: &CliStyle,
) -> String {
    if LiveSemanticConsumer::is_transcript_bearing_kind(&live_identity.part_kind) {
        match live_identity.part_kind {
            agendao_types::LiveMessagePartKind::AssistantText
                if matches!(block, OutputBlock::Message(_)) =>
            {
                let full_rendered = render_cli_block_rich(block, style);
                let end_suffix = render_cli_block_rich(
                    &OutputBlock::Message(MessageBlock::end(
                        agendao_command_render::output_blocks::MessageRole::Assistant,
                    )),
                    style,
                );
                return full_rendered
                    .strip_suffix(&end_suffix)
                    .unwrap_or(full_rendered.as_str())
                    .to_string();
            }
            agendao_types::LiveMessagePartKind::AssistantReasoning
                if matches!(block, OutputBlock::Reasoning(_)) =>
            {
                let full_rendered = render_cli_block_rich(block, style);
                let end_suffix =
                    render_cli_block_rich(&OutputBlock::Reasoning(ReasoningBlock::end()), style);
                return full_rendered
                    .strip_suffix(&end_suffix)
                    .unwrap_or(full_rendered.as_str())
                    .to_string();
            }
            _ => return render_cli_block_rich(block, style),
        }
    }

    render_cli_block_rich(block, style)
}

pub(super) fn cli_live_slot_commit_suffix(
    live_identity: &agendao_types::LiveMessagePartIdentity,
    style: &CliStyle,
) -> String {
    match live_identity.part_kind {
        agendao_types::LiveMessagePartKind::AssistantText => render_cli_block_rich(
            &OutputBlock::Message(MessageBlock::end(
                agendao_command_render::output_blocks::MessageRole::Assistant,
            )),
            style,
        ),
        agendao_types::LiveMessagePartKind::AssistantReasoning => {
            render_cli_block_rich(&OutputBlock::Reasoning(ReasoningBlock::end()), style)
        }
        _ => String::new(),
    }
}

pub(super) fn cli_live_slot_has_visible_content(block: &OutputBlock) -> bool {
    match block {
        OutputBlock::Message(message) => match message.phase {
            MessagePhase::Start | MessagePhase::End => false,
            MessagePhase::Delta | MessagePhase::Full => !message.text.is_empty(),
        },
        OutputBlock::Reasoning(reasoning) => match reasoning.phase {
            MessagePhase::Start | MessagePhase::End => false,
            MessagePhase::Delta | MessagePhase::Full => !reasoning.text.trim().is_empty(),
        },
        OutputBlock::Tool(tool) => match tool.phase {
            ToolPhase::Start => true,
            ToolPhase::Running => tool
                .detail
                .as_deref()
                .is_some_and(|detail| !detail.trim().is_empty()),
            ToolPhase::Done | ToolPhase::Error => true,
        },
        _ => true,
    }
}

pub(super) fn cli_apply_live_slot_update(
    transcript: &mut CliVisibleTranscript,
    block: &OutputBlock,
    live_identity: &agendao_types::LiveMessagePartIdentity,
    style: &CliStyle,
) {
    if !LiveSemanticConsumer::is_transcript_bearing_kind(&live_identity.part_kind) {
        return;
    }

    let slot_key = format!("{}:{}", live_identity.message_id, live_identity.part_key);
    if live_identity.phase == agendao_types::LivePartPhase::End {
        if cli_live_slot_has_visible_content(block) {
            let snapshot_rendered = cli_render_live_slot_snapshot(block, live_identity, style);
            let snapshot_plain = agendao_util::util::color::strip_ansi(&snapshot_rendered);
            transcript.upsert_live_slot(&slot_key, snapshot_rendered, snapshot_plain);
        }
        let suffix_ansi = cli_live_slot_commit_suffix(live_identity, style);
        let suffix_plain = agendao_util::util::color::strip_ansi(&suffix_ansi);
        transcript.finalize_live_slot(&slot_key, suffix_ansi, suffix_plain);
        return;
    }

    if cli_live_slot_has_visible_content(block) {
        let snapshot_rendered = cli_render_live_slot_snapshot(block, live_identity, style);
        let snapshot_plain = agendao_util::util::color::strip_ansi(&snapshot_rendered);
        transcript.upsert_live_slot(&slot_key, snapshot_rendered, snapshot_plain);
    }
}
