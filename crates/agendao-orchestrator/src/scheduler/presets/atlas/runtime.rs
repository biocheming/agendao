use crate::scheduler::ground_truth::{load_scheduler_ground_truth, render_scheduler_ground_truth};
use crate::scheduler::profile_state::SchedulerPresetRuntimeState;
use crate::scheduler::{SchedulerExecutionGateDecision, SchedulerExecutionGateStatus};
use crate::OrchestratorContext;

pub(super) fn sync_atlas_runtime_authority(
    runtime: &mut SchedulerPresetRuntimeState,
    ctx: &OrchestratorContext,
) {
    let Some(ground_truth) = load_scheduler_ground_truth(
        &ctx.exec_ctx.workdir,
        runtime.planning_artifact_path.as_deref(),
    ) else {
        runtime.ground_truth_context = None;
        return;
    };

    if runtime.planning_artifact_path.is_none() {
        runtime.planning_artifact_path = Some(ground_truth.plan_path.clone());
    }
    if ground_truth.plan_snapshot.is_some() {
        runtime.planned = ground_truth.plan_snapshot.clone();
    }
    runtime.ground_truth_context = render_scheduler_ground_truth(&ground_truth);
}

pub(super) fn resolve_atlas_gate_terminal_content(
    status: SchedulerExecutionGateStatus,
    decision: &SchedulerExecutionGateDecision,
    _fallback_content: &str,
) -> Option<String> {
    match status {
        SchedulerExecutionGateStatus::Done => decision
            .final_response
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        SchedulerExecutionGateStatus::Blocked => {
            let blocked = decision
                .final_response
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| decision.summary.clone());
            (!blocked.trim().is_empty()).then_some(blocked)
        }
        SchedulerExecutionGateStatus::Continue => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{AgentResolver, ModelResolver, NoopLifecycleHook, ToolExecutor};
    use crate::{
        AgentDescriptor, ExecutionContext, ModelRef, OrchestratorContext, OrchestratorError,
        ToolExecError, ToolOutput,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agendao-orchestrator-{name}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_context(workdir: &std::path::Path, session_id: &str) -> OrchestratorContext {
        OrchestratorContext {
            agent_resolver: Arc::new(TestAgentResolver),
            model_resolver: Arc::new(TestModelResolver),
            tool_executor: Arc::new(NoopToolExecutor),
            lifecycle_hook: Arc::new(NoopLifecycleHook),
            cancel_token: Arc::new(crate::runtime::events::NeverCancel),
            exec_ctx: ExecutionContext {
                session_id: session_id.to_string(),
                workdir: workdir.display().to_string(),
                agent_name: "atlas".to_string(),
                metadata: HashMap::new(),
            },
        }
    }

    #[test]
    fn atlas_runtime_sync_loads_boulder_plan_snapshot() {
        let workdir = temp_dir("atlas-runtime");
        let plan_path = workdir.join(".sisyphus/plans/demo.md");
        fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
        fs::write(&plan_path, "- [ ] task A\n- [x] task B\n").unwrap();
        fs::write(
            workdir.join(".sisyphus/boulder.json"),
            format!(
                r#"{{
  "active_plan": "{}",
  "started_at": "2026-03-09T00:00:00Z",
  "session_ids": ["ses-1", "ses-2"],
  "plan_name": "demo",
  "agent": "atlas",
  "worktree_path": "/tmp/worktree-demo"
}}"#,
                plan_path.display()
            ),
        )
        .unwrap();

        let ctx = test_context(&workdir, "atlas-session");
        let mut runtime = SchedulerPresetRuntimeState::default();

        sync_atlas_runtime_authority(&mut runtime, &ctx);

        assert_eq!(
            runtime.planning_artifact_path.as_deref(),
            Some(".sisyphus/plans/demo.md")
        );
        assert!(runtime
            .planned
            .as_deref()
            .is_some_and(|content| content.contains("- [ ] task A")));
        let ground_truth = runtime
            .ground_truth_context
            .as_deref()
            .expect("ground truth should exist");
        assert!(ground_truth.contains("boulder_state_path"));
        assert!(ground_truth.contains("active_agent: `atlas`"));
    }

    #[test]
    fn atlas_gate_terminal_content_preserves_done_only_when_final_response_exists() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "all tasks verified".to_string(),
            next_input: None,
            final_response: None,
        };
        assert_eq!(
            resolve_atlas_gate_terminal_content(
                SchedulerExecutionGateStatus::Done,
                &decision,
                "worker output",
            ),
            None
        );

        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Done,
            summary: "all tasks verified".to_string(),
            next_input: None,
            final_response: Some("## Delivery Summary\nDone.".to_string()),
        };
        assert!(resolve_atlas_gate_terminal_content(
            SchedulerExecutionGateStatus::Done,
            &decision,
            "worker output",
        )
        .is_some());
    }

    #[test]
    fn atlas_gate_terminal_content_uses_summary_for_blocked_without_final_response() {
        let decision = SchedulerExecutionGateDecision {
            status: SchedulerExecutionGateStatus::Blocked,
            summary: "task B blocked by missing API credential".to_string(),
            next_input: None,
            final_response: None,
        };
        assert_eq!(
            resolve_atlas_gate_terminal_content(
                SchedulerExecutionGateStatus::Blocked,
                &decision,
                "worker output",
            ),
            Some("task B blocked by missing API credential".to_string())
        );
    }

    struct TestAgentResolver;

    #[async_trait]
    impl AgentResolver for TestAgentResolver {
        fn resolve(&self, _name: &str) -> Option<AgentDescriptor> {
            None
        }
    }

    struct TestModelResolver;

    #[async_trait]
    impl ModelResolver for TestModelResolver {
        async fn chat_stream(
            &self,
            _model: Option<&ModelRef>,
            _messages: Vec<agendao_provider::Message>,
            _tools: Vec<agendao_provider::ToolDefinition>,
            _exec_ctx: &ExecutionContext,
        ) -> Result<agendao_provider::StreamResult, OrchestratorError> {
            panic!("model should not be called in atlas runtime authority tests")
        }
    }

    struct NoopToolExecutor;

    #[async_trait]
    impl ToolExecutor for NoopToolExecutor {
        async fn execute(
            &self,
            _tool_name: &str,
            _arguments: serde_json::Value,
            _exec_ctx: &ExecutionContext,
        ) -> Result<ToolOutput, ToolExecError> {
            panic!("tool executor should not be called in atlas runtime authority tests")
        }

        async fn list_ids(&self) -> Vec<String> {
            Vec::new()
        }

        async fn list_definitions(
            &self,
            _exec_ctx: &ExecutionContext,
        ) -> Vec<agendao_provider::ToolDefinition> {
            Vec::new()
        }
    }
}
