use agendao_command_render::cli_style::{rendered_row_count, CliStyle};
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

impl TranscriptEntry {
    fn rendered_ansi(&self) -> &str {
        match self {
            Self::Committed { rendered_ansi } => rendered_ansi,
            Self::LiveSlot { rendered_ansi, .. } => rendered_ansi,
        }
    }

    fn is_live_slot(&self) -> bool {
        matches!(self, Self::LiveSlot { .. })
    }
}

#[derive(Debug, Clone)]
pub(super) struct CliVisibleTranscript {
    entries: Vec<TranscriptEntry>,
    max_lines: usize,
    ansi_capable: bool,
    // ── Incremental terminal sync ─────────────────────────────────────
    // entries[..printed_frontier] are on screen and will never change.
    // entries[printed_frontier..] form the volatile tail (live slots plus
    // anything appended after them); it is erased and reprinted per update.
    printed_frontier: usize,
    // Terminal rows currently occupied by the volatile tail on screen.
    volatile_rows: usize,
    ever_printed: bool,
    // Width used for the last print; a change forces a full resync because
    // terminals reflow already-printed wrapped rows on resize, which makes
    // the stored row count unreliable.
    last_width: Option<u16>,
}

impl CliVisibleTranscript {
    pub(super) fn new(ansi_capable: bool) -> Self {
        Self {
            entries: Vec::new(),
            max_lines: CLI_TRANSCRIPT_MAX_LINES,
            ansi_capable,
            printed_frontier: 0,
            volatile_rows: 0,
            ever_printed: false,
            last_width: None,
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

    /// Produce the next incremental terminal update: erase the previously
    /// printed volatile tail, append newly stable lines, then reprint the
    /// live-slot tail. Replaces the old clear-screen-and-reprint-everything
    /// per event, which was O(transcript) per SSE event.
    ///
    /// Every emitted update leaves the cursor at column 0 of a fresh row, so
    /// the next update can erase exactly `volatile_rows` rows with CUU + ED.
    pub(super) fn incremental_screen_update(&mut self, width: u16) -> String {
        // Defensive: the frontier must never sit past a live slot (a printed
        // slot can be finalized in place, which pulls it back).
        if let Some(first_live) = self.entries.iter().position(|e| e.is_live_slot()) {
            self.printed_frontier = self.printed_frontier.min(first_live);
        }

        let mut out = String::new();
        let width_changed = matches!(self.last_width, Some(previous) if previous != width);
        self.last_width = Some(width);
        if !self.ever_printed || width_changed {
            // Full resync on first draw and on width changes: reflowed
            // wrapped rows invalidate the stored row count, so the tail
            // cannot be erased reliably after a resize.
            out.push_str("\x1B[2J\x1B[1;1H");
            self.ever_printed = true;
            self.printed_frontier = 0;
            self.volatile_rows = 0;
        } else if self.volatile_rows > 0 {
            out.push_str(&format!("\x1B[{}A\x1B[J", self.volatile_rows));
        }

        let stable_end = self
            .entries
            .iter()
            .position(|e| e.is_live_slot())
            .unwrap_or(self.entries.len());
        for entry in &self.entries[self.printed_frontier..stable_end] {
            out.push_str(entry.rendered_ansi());
        }
        self.printed_frontier = stable_end;

        let mut volatile = String::new();
        for entry in &self.entries[stable_end..] {
            volatile.push_str(entry.rendered_ansi());
        }
        let mut rows = 0usize;
        if !volatile.is_empty() {
            if !volatile.ends_with('\n') {
                // Terminate the last row so the cursor parks at column 0 of
                // a fresh row; the newline itself does not occupy a row.
                volatile.push('\n');
            }
            // Count rows over the concatenated text: two entries ("a" and
            // "b") render as one row "ab\n", not two.
            rows = rendered_row_count(&volatile, width);
            out.push_str(&volatile);
        }
        self.volatile_rows = rows;
        out
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
            // Trimmed entries were the oldest; if they were already printed,
            // their rows simply scroll up into history (never erased again).
            self.printed_frontier = self.printed_frontier.saturating_sub(overflow);
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

#[cfg(test)]
mod incremental_tests {
    use super::*;

    fn plain_style() -> CliStyle {
        CliStyle {
            color: false,
            width: 80,
        }
    }

    #[test]
    fn first_update_clears_screen_then_prints_committed_lines() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("hello\nworld\n");
        let update = transcript.incremental_screen_update(80);
        assert!(update.starts_with("\x1B[2J\x1B[1;1H"));
        assert!(update.contains("hello\nworld\n"));
    }

    #[test]
    fn new_committed_lines_append_without_clearing() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("first\n");
        transcript.incremental_screen_update(80);
        transcript.append_committed("second\n");
        let update = transcript.incremental_screen_update(80);
        assert!(
            !update.contains("\x1B[2J"),
            "no full clear after first draw"
        );
        assert!(!update.contains("first"), "stable lines are not reprinted");
        assert!(update.contains("second\n"));
    }

    #[test]
    fn live_slot_redraw_erases_only_volatile_rows() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("stable\n");
        transcript.incremental_screen_update(80);

        transcript.upsert_live_slot(
            "m1:text",
            "draft line\n".to_string(),
            "draft line\n".to_string(),
        );
        let update = transcript.incremental_screen_update(80);
        assert!(
            !update.contains("\x1B["),
            "first live draw has nothing to erase"
        );
        assert!(update.contains("draft line\n"));

        transcript.upsert_live_slot(
            "m1:text",
            "draft line grown\n".to_string(),
            "draft line grown\n".to_string(),
        );
        let update = transcript.incremental_screen_update(80);
        assert!(
            update.starts_with("\x1B[1A\x1B[J"),
            "erase must move up exactly the previous volatile row count"
        );
        assert!(update.contains("draft line grown\n"));
        assert!(!update.contains("stable"), "stable prefix untouched");
    }

    #[test]
    fn live_slot_without_trailing_newline_gets_parked_on_fresh_row() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.upsert_live_slot("m1:text", "partial".to_string(), "partial".to_string());
        let update = transcript.incremental_screen_update(80);
        assert!(update.ends_with("partial\n"));

        // The synthetic newline counts as one volatile row: erasing the next
        // update moves up exactly one row.
        transcript.upsert_live_slot("m1:text", "partial!".to_string(), "partial!".to_string());
        let update = transcript.incremental_screen_update(80);
        assert!(update.starts_with("\x1B[1A\x1B[J"));
    }

    #[test]
    fn finalized_slot_moves_into_stable_region() {
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("stable\n");
        transcript.incremental_screen_update(80);
        transcript.upsert_live_slot("m1:text", "draft\n".to_string(), "draft\n".to_string());
        transcript.incremental_screen_update(80);

        transcript.finalize_live_slot("m1:text", " [done]\n".to_string(), " [done]\n".to_string());
        transcript.append_committed("after\n");
        let update = transcript.incremental_screen_update(80);
        assert!(update.starts_with("\x1B[1A\x1B[J"));
        assert!(update.contains("draft\n [done]\n"));
        assert!(update.contains("after\n"));

        // Everything is committed now; a follow-up with no changes prints
        // nothing and erases nothing.
        let update = transcript.incremental_screen_update(80);
        assert!(update.is_empty());
    }

    #[test]
    fn adjacent_entries_without_trailing_newline_share_one_row() {
        // Two live slots whose text has no trailing newline concatenate into
        // a single on-screen row "ab\n" — the erase math must count 1 row,
        // not 2.
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("stable\n");
        transcript.incremental_screen_update(80);
        transcript.upsert_live_slot("m1:a", "a".to_string(), "a".to_string());
        transcript.upsert_live_slot("m1:b", "b".to_string(), "b".to_string());
        let update = transcript.incremental_screen_update(80);
        assert!(update.contains("ab\n"), "adjacent entries print as one row");
        // One row printed -> the next update erases exactly one row.
        transcript.upsert_live_slot("m1:a", "x".to_string(), "x".to_string());
        transcript.upsert_live_slot("m1:b", "y".to_string(), "y".to_string());
        let update = transcript.incremental_screen_update(80);
        assert!(update.starts_with("\x1B[1A\x1B[J"));
    }

    #[test]
    fn width_change_forces_full_resync() {
        // Terminal resize reflows already-printed wrapped rows, so stored
        // row counts are unreliable: a width change must fall back to a
        // full clear + reprint (including the stable prefix).
        let mut transcript = CliVisibleTranscript::new(false);
        transcript.append_committed("stable line\n");
        transcript.incremental_screen_update(80);
        transcript.append_committed("more\n");
        let update = transcript.incremental_screen_update(80);
        assert!(!update.contains("\x1B[2J"));

        let update = transcript.incremental_screen_update(40);
        assert!(
            update.starts_with("\x1B[2J\x1B[1;1H"),
            "width change must force a full resync"
        );
        assert!(
            update.contains("stable line\n") && update.contains("more\n"),
            "full resync reprints the stable prefix"
        );

        // The resync re-establishes the frontier: a follow-up at the same
        // width goes back to incremental appends without clearing.
        transcript.append_committed("tail\n");
        let update = transcript.incremental_screen_update(40);
        assert!(!update.contains("\x1B[2J"));
        assert!(update.ends_with("tail\n"));
    }

    #[test]
    fn style_helper_row_counts_agree_with_erase_math() {
        // Two wrapped lines plus a hard newline = 3 rows.
        let text = format!("{}\nab", "x".repeat(100));
        assert_eq!(rendered_row_count(&text, 80), 3);
        let _ = plain_style();
    }
}
