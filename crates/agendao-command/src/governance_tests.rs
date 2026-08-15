#[cfg(test)]
mod tests {
    use crate::governance_fixtures::live_transcript_state_fixture;

    #[test]
    fn live_transcript_fixture_matches_the_current_contract() {
        let fixture = live_transcript_state_fixture();
        assert_eq!(fixture.version, 2);
        assert_eq!(fixture.contract_version, "2026-05-26");
        assert_eq!(fixture.canonical_live_stream.events.len(), 11);
        assert_eq!(
            fixture
                .canonical_live_stream
                .expected
                .transcript_blocks
                .order,
            vec!["reasoning", "tool", "tool", "message"]
        );
        assert_eq!(
            fixture.shared_turn_cycles.entries.len(),
            fixture.shared_turn_cycles.expected.assistant_message_count
        );
        assert_eq!(
            fixture
                .shared_turn_cycles
                .entries
                .iter()
                .filter(|entry| entry.tool.is_some())
                .count(),
            fixture.shared_turn_cycles.expected.tool_result_count
        );
        assert!(!fixture
            .tool_progress_exclusion
            .message
            .message_id
            .is_empty());
        assert_eq!(fixture.run_tail_contract.completed_status, "complete");
        assert_eq!(fixture.run_tail_contract.error_status, "error");
        assert_eq!(
            fixture.run_tail_contract.awaiting_user_status,
            "awaiting_user"
        );
        assert!(fixture.run_tail_contract.completed_usage.input_tokens > 0);
        assert!(!fixture.run_tail_contract.error_message.is_empty());
    }
}
