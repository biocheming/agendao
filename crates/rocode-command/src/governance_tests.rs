//! Cross-layer governance tests for the three-layer stage architecture.
//!
//! These tests verify consistency across:
//! - **Stage Summary Layer** — `SchedulerStageBlock → StageSummary` projection
//! - **Execution Topology Layer** — `ExecutionRecord → ExecutionNode` (tested in rocode-server)
//! - **Raw SSE Layer** — `StageEvent` filtering, ordering, and field correctness
//!
//! Uses the shared multi-agent replay fixture to ensure golden-path invariants.

#[cfg(test)]
mod tests {
    use crate::governance_fixtures::{
        live_transcript_state_fixture, multi_agent_replay_fixture, ExecutionRecordFixture,
        MultiAgentReplayFixture,
    };
    use crate::output_blocks::SchedulerStageBlock;
    use crate::stage_protocol::*;
    use std::collections::HashSet;

    fn load_fixture() -> MultiAgentReplayFixture {
        multi_agent_replay_fixture()
    }

    // ── 1. Fixture integrity ─────────────────────────────────────────

    #[test]
    fn fixture_loads_and_has_expected_shape() {
        let fixture = load_fixture();
        assert_eq!(fixture.stages.len(), fixture.expected.total_stages);
        assert_eq!(fixture.session_id, "session_gov_1");

        let total_records: usize = fixture
            .stages
            .iter()
            .map(|s| s.execution_records.len())
            .sum();
        assert_eq!(total_records, fixture.expected.total_execution_records);

        let total_events: usize = fixture.stages.iter().map(|s| s.events.len()).sum();
        assert_eq!(total_events, fixture.expected.total_events);
    }

    #[test]
    fn live_transcript_state_fixture_loads_and_matches_expected_shape() {
        let fixture = live_transcript_state_fixture();
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
        assert!(!fixture.scheduler_stage_exclusion.stage_id.is_empty());
        assert_eq!(fixture.run_tail_contract.completed_status, "complete");
        assert_eq!(fixture.run_tail_contract.error_status, "error");
        assert_eq!(
            fixture.run_tail_contract.awaiting_user_status,
            "awaiting_user"
        );
        assert!(fixture.run_tail_contract.completed_usage.input_tokens > 0);
        assert!(!fixture.run_tail_contract.error_message.is_empty());
    }

    #[test]
    fn fixture_stage_ids_match_expected() {
        let fixture = load_fixture();
        let stage_ids: Vec<String> = fixture
            .stages
            .iter()
            .map(|s| s.block.stage_id.clone().unwrap_or_default())
            .collect();
        assert_eq!(stage_ids, fixture.expected.distinct_stage_ids);
    }

    // ── 2. StageSummary projection consistency ───────────────────────

    #[test]
    fn all_stages_project_to_valid_summaries() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            assert!(!summary.stage_id.is_empty(), "stage_id must not be empty");
            assert!(
                !summary.stage_name.is_empty(),
                "stage_name must not be empty"
            );
            // status must be a valid variant (would fail at deserialization otherwise)
            let _ = serde_json::to_string(&summary.status).unwrap();
        }
    }

    #[test]
    fn summary_stage_id_matches_block_stage_id() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            assert_eq!(
                summary.stage_id,
                entry.block.stage_id.clone().unwrap_or_default()
            );
        }
    }

    #[test]
    fn summary_index_and_total_match_block() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            assert_eq!(summary.index, entry.block.stage_index);
            assert_eq!(summary.total, entry.block.stage_total);
        }
    }

    #[test]
    fn summary_token_fields_match_block() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            assert_eq!(summary.prompt_tokens, entry.block.prompt_tokens);
            assert_eq!(summary.completion_tokens, entry.block.completion_tokens);
            assert_eq!(summary.reasoning_tokens, entry.block.reasoning_tokens);
            assert_eq!(summary.cache_read_tokens, entry.block.cache_read_tokens);
            assert_eq!(summary.cache_write_tokens, entry.block.cache_write_tokens);
        }
    }

    #[test]
    fn summary_active_agent_count_matches_block_active_agents_len() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            assert_eq!(
                summary.active_agent_count,
                entry.block.active_agents.len() as u32,
                "stage {}",
                entry.block.stage.clone()
            );
        }
    }

    #[test]
    fn summary_attached_session_count_matches_block() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            let expected_count = if entry.block.attached_session_id.is_some() {
                1
            } else {
                0
            };
            assert_eq!(summary.attached_session_count, expected_count);
            assert_eq!(
                summary.primary_attached_session_id,
                entry.block.attached_session_id
            );
        }
    }

    #[test]
    fn summary_step_total_parsed_from_loop_budget() {
        let fixture = load_fixture();
        // planning: "step-limit:5" → Some(5)
        let plan_summary = fixture.stages[0].block.to_summary();
        assert_eq!(plan_summary.step_total, Some(5));

        // implementation: "step-limit:8" → Some(8)
        let impl_summary = fixture.stages[1].block.to_summary();
        assert_eq!(impl_summary.step_total, Some(8));

        // review: "step-limit:3" → Some(3)
        let review_summary = fixture.stages[2].block.to_summary();
        assert_eq!(review_summary.step_total, Some(3));
    }

    #[test]
    fn summary_status_matches_block_status_string() {
        let fixture = load_fixture();
        let expected_statuses = [
            StageStatus::Running,
            StageStatus::Running,
            StageStatus::Waiting,
        ];
        for (entry, expected) in fixture.stages.iter().zip(expected_statuses.iter()) {
            let summary = entry.block.to_summary();
            assert_eq!(summary.status, *expected, "stage {}", entry.block.stage);
        }
    }

    // ── 3. StageSummary serde roundtrip for all fixture stages ───────

    #[test]
    fn all_summaries_survive_serde_roundtrip() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let summary = entry.block.to_summary();
            let json = serde_json::to_string(&summary).unwrap();
            let back: StageSummary = serde_json::from_str(&json).unwrap();
            assert_eq!(summary, back, "roundtrip failed for {}", entry.block.stage);
        }
    }

    // ── 4. Block from_metadata consistency ───────────────────────────

    #[test]
    fn block_from_metadata_matches_fixture_block() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let reconstructed =
                SchedulerStageBlock::from_metadata(&entry.message_text, &entry.metadata)
                    .expect("from_metadata should succeed");
            // Core identity fields must match
            assert_eq!(reconstructed.stage_id, entry.block.stage_id);
            assert_eq!(reconstructed.stage, entry.block.stage);
            assert_eq!(reconstructed.stage_index, entry.block.stage_index);
            assert_eq!(reconstructed.stage_total, entry.block.stage_total);
            assert_eq!(reconstructed.step, entry.block.step);
            assert_eq!(reconstructed.status, entry.block.status);
            assert_eq!(reconstructed.prompt_tokens, entry.block.prompt_tokens);
            assert_eq!(
                reconstructed.completion_tokens,
                entry.block.completion_tokens
            );
            assert_eq!(reconstructed.active_agents, entry.block.active_agents);
            assert_eq!(reconstructed.active_skills, entry.block.active_skills);
        }
    }

    #[test]
    fn from_metadata_then_to_summary_yields_same_as_direct_to_summary() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let direct = entry.block.to_summary();
            let via_metadata =
                SchedulerStageBlock::from_metadata(&entry.message_text, &entry.metadata)
                    .unwrap()
                    .to_summary();
            // Core protocol fields must be identical
            assert_eq!(direct.stage_id, via_metadata.stage_id);
            assert_eq!(direct.stage_name, via_metadata.stage_name);
            assert_eq!(direct.index, via_metadata.index);
            assert_eq!(direct.total, via_metadata.total);
            assert_eq!(direct.step, via_metadata.step);
            assert_eq!(direct.status, via_metadata.status);
            assert_eq!(direct.prompt_tokens, via_metadata.prompt_tokens);
            assert_eq!(direct.completion_tokens, via_metadata.completion_tokens);
            assert_eq!(direct.active_agent_count, via_metadata.active_agent_count);
            assert_eq!(
                direct.attached_session_count,
                via_metadata.attached_session_count
            );
        }
    }

    // ── 5. Cross-layer: stage_id consistency ─────────────────────────

    #[test]
    fn every_execution_record_stage_id_belongs_to_known_stage() {
        let fixture = load_fixture();
        let known_stage_ids: HashSet<String> = fixture
            .stages
            .iter()
            .map(|s| s.block.stage_id.clone().unwrap_or_default())
            .collect();

        for entry in &fixture.stages {
            for rec in &entry.execution_records {
                if let Some(ref sid) = rec.stage_id {
                    assert!(
                        known_stage_ids.contains(sid),
                        "execution record {} has unknown stage_id {}",
                        rec.id,
                        sid
                    );
                }
            }
        }
    }

    #[test]
    fn every_event_stage_id_belongs_to_known_stage() {
        let fixture = load_fixture();
        let known_stage_ids: HashSet<String> = fixture
            .stages
            .iter()
            .map(|s| s.block.stage_id.clone().unwrap_or_default())
            .collect();

        for entry in &fixture.stages {
            for evt in &entry.events {
                if let Some(ref sid) = evt.stage_id {
                    assert!(
                        known_stage_ids.contains(sid),
                        "event {} has unknown stage_id {}",
                        evt.event_id,
                        sid
                    );
                }
            }
        }
    }

    #[test]
    fn every_event_execution_id_belongs_to_known_record() {
        let fixture = load_fixture();
        let known_exec_ids: HashSet<String> = fixture
            .stages
            .iter()
            .flat_map(|s| s.execution_records.iter().map(|r| r.id.clone()))
            .collect();

        for entry in &fixture.stages {
            for evt in &entry.events {
                if let Some(ref eid) = evt.execution_id {
                    assert!(
                        known_exec_ids.contains(eid),
                        "event {} has unknown execution_id {}",
                        evt.event_id,
                        eid
                    );
                }
            }
        }
    }

    // ── 6. Event ordering within each stage ──────────────────────────

    #[test]
    fn events_are_ordered_by_timestamp_within_each_stage() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let timestamps: Vec<i64> = entry.events.iter().map(|e| e.ts).collect();
            for window in timestamps.windows(2) {
                assert!(
                    window[0] <= window[1],
                    "events not ordered in stage {}: {} > {}",
                    entry.block.stage,
                    window[0],
                    window[1]
                );
            }
        }
    }

    #[test]
    fn global_events_ordered_across_stages() {
        let fixture = load_fixture();
        let all_timestamps: Vec<i64> = fixture
            .stages
            .iter()
            .flat_map(|s| s.events.iter().map(|e| e.ts))
            .collect();

        for window in all_timestamps.windows(2) {
            assert!(
                window[0] <= window[1],
                "global event ordering violated: {} > {}",
                window[0],
                window[1]
            );
        }
    }

    // ── 7. Event ID uniqueness ───────────────────────────────────────

    #[test]
    fn all_event_ids_are_globally_unique() {
        let fixture = load_fixture();
        let all_ids: Vec<&str> = fixture
            .stages
            .iter()
            .flat_map(|s| s.events.iter().map(|e| e.event_id.as_str()))
            .collect();
        let unique: HashSet<&str> = all_ids.iter().copied().collect();
        assert_eq!(all_ids.len(), unique.len(), "duplicate event IDs found");
    }

    #[test]
    fn all_execution_record_ids_are_globally_unique() {
        let fixture = load_fixture();
        let all_ids: Vec<&str> = fixture
            .stages
            .iter()
            .flat_map(|s| s.execution_records.iter().map(|r| r.id.as_str()))
            .collect();
        let unique: HashSet<&str> = all_ids.iter().copied().collect();
        assert_eq!(
            all_ids.len(),
            unique.len(),
            "duplicate execution record IDs found"
        );
    }

    // ── 8. Aggregate consistency ─────────────────────────────────────

    #[test]
    fn aggregate_token_counts_match_expected() {
        let fixture = load_fixture();
        let total_prompt: u64 = fixture
            .stages
            .iter()
            .filter_map(|s| s.block.prompt_tokens)
            .sum();
        let total_completion: u64 = fixture
            .stages
            .iter()
            .filter_map(|s| s.block.completion_tokens)
            .sum();
        let total_reasoning: u64 = fixture
            .stages
            .iter()
            .filter_map(|s| s.block.reasoning_tokens)
            .sum();

        assert_eq!(total_prompt, fixture.expected.aggregate_prompt_tokens);
        assert_eq!(
            total_completion,
            fixture.expected.aggregate_completion_tokens
        );
        assert_eq!(total_reasoning, fixture.expected.aggregate_reasoning_tokens);
    }

    #[test]
    fn agent_labels_match_expected() {
        let fixture = load_fixture();
        let mut labels: Vec<String> = fixture
            .stages
            .iter()
            .flat_map(|s| {
                s.execution_records
                    .iter()
                    .filter(|r| r.kind == "agent_task")
                    .filter_map(|r| r.label.clone())
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        labels.sort();
        let mut expected = fixture.expected.distinct_agent_labels.clone();
        expected.sort();
        assert_eq!(labels, expected);
    }

    #[test]
    fn tool_labels_match_expected() {
        let fixture = load_fixture();
        let mut labels: Vec<String> = fixture
            .stages
            .iter()
            .flat_map(|s| {
                s.execution_records
                    .iter()
                    .filter(|r| r.kind == "tool_call")
                    .filter_map(|r| r.label.clone())
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        labels.sort();
        let mut expected = fixture.expected.distinct_tool_labels.clone();
        expected.sort();
        assert_eq!(labels, expected);
    }

    #[test]
    fn question_count_matches_expected() {
        let fixture = load_fixture();
        let count = fixture
            .stages
            .iter()
            .flat_map(|s| s.execution_records.iter())
            .filter(|r| r.kind == "question")
            .count();
        assert_eq!(count, fixture.expected.question_count);
    }

    #[test]
    fn attached_session_count_matches_expected() {
        let fixture = load_fixture();
        let count = fixture
            .stages
            .iter()
            .filter(|s| s.block.attached_session_id.is_some())
            .count();
        assert_eq!(count, fixture.expected.stages_with_attached_sessions);
    }

    // ── 9. StageEvent filtering simulation ───────────────────────────

    fn collect_all_events(fixture: &MultiAgentReplayFixture) -> Vec<StageEvent> {
        fixture
            .stages
            .iter()
            .flat_map(|s| s.events.clone())
            .collect()
    }

    #[test]
    fn filter_events_by_stage_id() {
        let fixture = load_fixture();
        let all = collect_all_events(&fixture);

        for stage_id in &fixture.expected.distinct_stage_ids {
            let filtered: Vec<&StageEvent> = all
                .iter()
                .filter(|e| e.stage_id.as_deref() == Some(stage_id.as_str()))
                .collect();
            assert!(!filtered.is_empty(), "no events for stage_id {}", stage_id);
            // All filtered events must belong to this stage
            for evt in &filtered {
                assert_eq!(evt.stage_id.as_deref(), Some(stage_id.as_str()));
            }
        }
    }

    #[test]
    fn filter_events_by_event_type() {
        let fixture = load_fixture();
        let all = collect_all_events(&fixture);

        let stage_started: Vec<&StageEvent> = all
            .iter()
            .filter(|e| e.event_type == "stage_started")
            .collect();
        assert_eq!(
            stage_started.len(),
            3,
            "should have 3 stage_started events (one per stage)"
        );

        let agent_started: Vec<&StageEvent> = all
            .iter()
            .filter(|e| e.event_type == "agent_started")
            .collect();
        assert_eq!(agent_started.len(), 4, "should have 4 agent_started events");

        let question_asked: Vec<&StageEvent> = all
            .iter()
            .filter(|e| e.event_type == "question_asked")
            .collect();
        assert_eq!(
            question_asked.len(),
            2,
            "should have 2 question_asked events"
        );
    }

    #[test]
    fn filter_events_by_since_timestamp() {
        let fixture = load_fixture();
        let all = collect_all_events(&fixture);

        // Only events from implementation stage onwards (ts >= 1710000010000)
        let since_impl: Vec<&StageEvent> = all.iter().filter(|e| e.ts >= 1710000010000).collect();
        // Implementation has 5 events, review has 3 → 8
        assert_eq!(since_impl.len(), 8);

        // Only review events (ts >= 1710000020000)
        let since_review: Vec<&StageEvent> = all.iter().filter(|e| e.ts >= 1710000020000).collect();
        assert_eq!(since_review.len(), 3);
    }

    #[test]
    fn filter_events_combined_stage_and_type() {
        let fixture = load_fixture();
        let all = collect_all_events(&fixture);

        let impl_tools: Vec<&StageEvent> = all
            .iter()
            .filter(|e| {
                e.stage_id.as_deref() == Some("stage_impl_002") && e.event_type == "tool_started"
            })
            .collect();
        assert_eq!(
            impl_tools.len(),
            1,
            "implementation stage should have 1 tool_started event"
        );
        assert_eq!(
            impl_tools[0].payload.get("tool").and_then(|v| v.as_str()),
            Some("write_file")
        );
    }

    // ── 10. InspectBlock construction from events ────────────────────

    #[test]
    fn inspect_block_from_fixture_events() {
        use crate::output_blocks::{InspectBlock, InspectEventRow};

        let fixture = load_fixture();
        let all = collect_all_events(&fixture);

        let block = InspectBlock {
            stage_ids: fixture.expected.distinct_stage_ids.clone(),
            events: all
                .iter()
                .map(|e| InspectEventRow {
                    ts: e.ts,
                    event_type: e.event_type.clone(),
                    execution_id: e.execution_id.clone(),
                    stage_id: e.stage_id.clone(),
                })
                .collect(),
            filter_stage_id: None,
        };

        assert_eq!(block.stage_ids.len(), 3);
        assert_eq!(block.events.len(), fixture.expected.total_events);

        // Verify filtering works
        let filtered = InspectBlock {
            stage_ids: block.stage_ids.clone(),
            events: block
                .events
                .iter()
                .filter(|e| e.stage_id.as_deref() == Some("stage_plan_001"))
                .cloned()
                .collect(),
            filter_stage_id: Some("stage_plan_001".to_string()),
        };
        assert_eq!(filtered.events.len(), 3);
    }

    // ── 11. Decision block presence on review stage ──────────────────

    #[test]
    fn review_stage_has_decision_block() {
        let fixture = load_fixture();
        let review = &fixture.stages[2];
        let decision = review
            .block
            .decision
            .as_ref()
            .expect("review stage must have a decision");
        assert_eq!(decision.kind, "gate");
        assert_eq!(decision.title, "Code Review Decision");
        assert!(!decision.fields.is_empty());
        assert!(!decision.sections.is_empty());

        // Verify decision field content
        let outcome = &decision.fields[0];
        assert_eq!(outcome.label, "Outcome");
        assert_eq!(outcome.value, "approve");
    }

    // ── 12. Execution record parent-child topology ───────────────────

    #[test]
    fn execution_records_form_valid_tree() {
        let fixture = load_fixture();
        let all_records: Vec<&ExecutionRecordFixture> = fixture
            .stages
            .iter()
            .flat_map(|s| s.execution_records.iter())
            .collect();
        let id_set: HashSet<&str> = all_records.iter().map(|r| r.id.as_str()).collect();

        for rec in &all_records {
            if let Some(ref parent_id) = rec.parent_id {
                // Parent must either be in our known set or be the scheduler root
                if parent_id != "exec_scheduler_root" {
                    assert!(
                        id_set.contains(parent_id.as_str()),
                        "record {} has orphaned parent_id {}",
                        rec.id,
                        parent_id
                    );
                }
            }
        }
    }

    #[test]
    fn stage_type_records_are_tree_roots_under_scheduler() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            let stage_records: Vec<&ExecutionRecordFixture> = entry
                .execution_records
                .iter()
                .filter(|r| r.kind == "scheduler_stage")
                .collect();
            assert_eq!(
                stage_records.len(),
                1,
                "each stage should have exactly one scheduler_stage record"
            );
            assert_eq!(
                stage_records[0].parent_id.as_deref(),
                Some("exec_scheduler_root")
            );
        }
    }

    #[test]
    fn tool_records_have_agent_parents() {
        let fixture = load_fixture();
        let agent_ids: HashSet<String> = fixture
            .stages
            .iter()
            .flat_map(|s| {
                s.execution_records
                    .iter()
                    .filter(|r| r.kind == "agent_task")
                    .map(|r| r.id.clone())
            })
            .collect();

        for entry in &fixture.stages {
            for rec in entry
                .execution_records
                .iter()
                .filter(|r| r.kind == "tool_call")
            {
                let parent = rec.parent_id.as_ref().expect("tool must have parent");
                assert!(
                    agent_ids.contains(parent),
                    "tool {} parent {} is not an agent",
                    rec.id,
                    parent
                );
            }
        }
    }

    #[test]
    fn question_records_have_agent_parents() {
        let fixture = load_fixture();
        let agent_ids: HashSet<String> = fixture
            .stages
            .iter()
            .flat_map(|s| {
                s.execution_records
                    .iter()
                    .filter(|r| r.kind == "agent_task")
                    .map(|r| r.id.clone())
            })
            .collect();

        for entry in &fixture.stages {
            for rec in entry
                .execution_records
                .iter()
                .filter(|r| r.kind == "question")
            {
                let parent = rec.parent_id.as_ref().expect("question must have parent");
                assert!(
                    agent_ids.contains(parent),
                    "question {} parent {} is not an agent",
                    rec.id,
                    parent
                );
            }
        }
    }

    // ── 13. Session ID consistency ───────────────────────────────────

    #[test]
    fn all_execution_records_belong_to_fixture_session() {
        let fixture = load_fixture();
        for entry in &fixture.stages {
            for rec in &entry.execution_records {
                assert_eq!(
                    rec.session_id, fixture.session_id,
                    "record {} has wrong session_id",
                    rec.id
                );
            }
        }
    }

    // ── 14. StageEvent::new() builder still works ────────────────────

    #[test]
    fn stage_event_builder_produces_unique_ids() {
        let evt1 = StageEvent::new(
            EventScope::Stage,
            Some("s1".into()),
            None,
            "test",
            serde_json::json!({}),
        );
        let evt2 = StageEvent::new(
            EventScope::Stage,
            Some("s1".into()),
            None,
            "test",
            serde_json::json!({}),
        );
        assert_ne!(
            evt1.event_id, evt2.event_id,
            "builder must produce unique IDs"
        );
        assert!(evt1.ts > 0);
        assert!(evt2.ts >= evt1.ts);
    }
}
