use std::collections::HashMap;

use rocode_command::stage_protocol::StageSummary;
use tokio::sync::RwLock;

#[derive(Debug, Default)]
pub(crate) struct StageSummaryStore {
    summaries: RwLock<HashMap<String, HashMap<String, StageSummary>>>,
}

impl StageSummaryStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn upsert(&self, session_id: &str, summary: StageSummary) -> bool {
        let mut guard = self.summaries.write().await;
        let previous = guard
            .entry(session_id.to_string())
            .or_default()
            .insert(summary.stage_id.clone(), summary.clone());
        previous.as_ref() != Some(&summary)
    }

    pub(crate) async fn list_for_session(&self, session_id: &str) -> Vec<StageSummary> {
        let guard = self.summaries.read().await;
        let Some(entries) = guard.get(session_id) else {
            return Vec::new();
        };
        let mut summaries = entries.values().cloned().collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            let left_index = left.index.unwrap_or(u64::MAX);
            let right_index = right.index.unwrap_or(u64::MAX);
            left_index
                .cmp(&right_index)
                .then_with(|| left.stage_id.cmp(&right.stage_id))
        });
        summaries
    }

    #[cfg(test)]
    pub(crate) async fn remove_stage(&self, session_id: &str, stage_id: &str) {
        let mut guard = self.summaries.write().await;
        let Some(entries) = guard.get_mut(session_id) else {
            return;
        };
        entries.remove(stage_id);
        if entries.is_empty() {
            guard.remove(session_id);
        }
    }

    pub(crate) async fn clear_session(&self, session_id: &str) {
        self.summaries.write().await.remove(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocode_command::stage_protocol::StageStatus;

    fn summary(stage_id: &str, index: Option<u64>) -> StageSummary {
        StageSummary {
            stage_id: stage_id.to_string(),
            stage_name: format!("stage-{stage_id}"),
            index,
            total: None,
            step: None,
            step_total: None,
            status: StageStatus::Running,
            prompt_tokens: None,
            context_tokens: None,
            completion_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            focus: None,
            last_event: None,
            waiting_on: None,
            estimated_context_tokens: None,
            skill_tree_budget: None,
            skill_tree_truncation_strategy: None,
            skill_tree_truncated: None,
            retry_attempt: None,
            active_agent_count: 0,
            active_tool_count: 0,
            child_session_count: 0,
            primary_child_session_id: None,
        }
    }

    #[tokio::test]
    async fn lists_sorted_by_index_then_stage_id() {
        let store = StageSummaryStore::new();
        assert!(store.upsert("s1", summary("b", Some(2))).await);
        assert!(store.upsert("s1", summary("a", Some(1))).await);
        assert!(store.upsert("s1", summary("z", None)).await);

        let ids = store
            .list_for_session("s1")
            .await
            .into_iter()
            .map(|entry| entry.stage_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a", "b", "z"]);
    }

    #[tokio::test]
    async fn remove_stage_cleans_empty_session_bucket() {
        let store = StageSummaryStore::new();
        assert!(store.upsert("s1", summary("a", Some(1))).await);
        store.remove_stage("s1", "a").await;
        assert!(store.list_for_session("s1").await.is_empty());
    }

    #[tokio::test]
    async fn upsert_reports_when_summary_is_unchanged() {
        let store = StageSummaryStore::new();
        let entry = summary("a", Some(1));

        assert!(store.upsert("s1", entry.clone()).await);
        assert!(!store.upsert("s1", entry).await);
    }
}
