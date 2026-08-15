use super::agent_loop::*;
use super::blueprint::*;
use super::catalog::*;
use super::context::*;
use super::engine::*;
use super::events::*;
use super::policy::PolicyEnvelope;
use super::selector::*;
use super::templates::*;
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn limits() -> ExecutionLimits {
    ExecutionLimits {
        max_model_calls: 32,
        max_tool_calls: 64,
        max_total_tokens: 200_000,
        max_wall_time_ms: 300_000,
        max_parallelism: 4,
        max_graph_nodes: 32,
        max_graph_depth: 16,
        max_loop_iterations: 6,
        max_agent_steps: 12,
    }
}

fn catalog() -> SchedulerCatalog {
    let read = ToolId::from("read");
    let write = ToolId::from("write");
    let audit = SkillId::from("audit");
    SchedulerCatalog {
        revision: "catalog-1".to_string(),
        agents: BTreeMap::from([(
            AgentId::from("worker"),
            AgentCatalogEntry {
                id: AgentId::from("worker"),
                system_policy: "Inspect evidence before changing code.".to_string(),
                available_skills: set([audit.clone()]),
                available_tools: set([read.clone(), write.clone()]),
                model_capabilities: set([ModelCapability::ToolCalls, ModelCapability::Reasoning]),
            },
        )]),
        skills: BTreeMap::from([(
            audit.clone(),
            SkillCatalogEntry {
                id: audit,
                summary: "Audit a repository with evidence".to_string(),
                content_fingerprint: "skill-audit-v1".to_string(),
                capability_tags: set(["code-review".to_string()]),
                hydrated_prompt: Some(Arc::from("Follow the repository audit procedure.")),
            },
        )]),
        tools: BTreeMap::from([
            (
                read.clone(),
                ToolCatalogEntry {
                    id: read,
                    effect: EffectClass::ReadOnly,
                    permission: PermissionClass::Automatic,
                },
            ),
            (
                write.clone(),
                ToolCatalogEntry {
                    id: write,
                    effect: EffectClass::WorkspaceMutation,
                    permission: PermissionClass::Ask,
                },
            ),
        ]),
        evaluators: BTreeMap::from([(
            EvaluatorId::from("quality"),
            EvaluatorCatalogEntry {
                id: EvaluatorId::from("quality"),
                kind: EvaluatorKind::Deterministic,
            },
        )]),
        capabilities: BTreeMap::from([
            (
                CapabilityId::from("checkpoint"),
                CapabilityCatalogEntry {
                    id: CapabilityId::from("checkpoint"),
                    kind: CapabilityKind::WorkspaceCheckpoint,
                    effect: EffectClass::WorkspaceMutation,
                },
            ),
            (
                CapabilityId::from("artifacts"),
                CapabilityCatalogEntry {
                    id: CapabilityId::from("artifacts"),
                    kind: CapabilityKind::ArtifactStore,
                    effect: EffectClass::WorkspaceMutation,
                },
            ),
        ]),
    }
}

fn agent(next: &str) -> NodeSpec {
    NodeSpec::Agent(AgentNode {
        agent: AgentId::from("worker"),
        skills: set([SkillId::from("audit")]),
        tools: set([ToolId::from("read")]),
        required_model_capabilities: set([ModelCapability::ToolCalls]),
        max_steps: 4,
        next: NodeId::from(next),
    })
}

fn end() -> NodeSpec {
    NodeSpec::End(EndNode {
        result: ResultSource::LastNode,
    })
}

fn blueprint(nodes: BTreeMap<NodeId, NodeSpec>, entry: &str) -> SchedulerBlueprint {
    SchedulerBlueprint {
        schema: BlueprintSchemaVersion::V1,
        name: BlueprintName::from("test-blueprint"),
        entry: NodeId::from(entry),
        nodes,
        limits: limits(),
        output: OutputContract {
            format: OutputFormat::Markdown,
            include_usage: true,
            include_artifact_refs: true,
        },
    }
}

fn single_agent_blueprint() -> SchedulerBlueprint {
    blueprint(
        BTreeMap::from([
            (NodeId::from("execute"), agent("done")),
            (NodeId::from("done"), end()),
        ]),
        "execute",
    )
}

fn bounded_loop_blueprint() -> SchedulerBlueprint {
    let body = SchedulerGraph {
        entry: NodeId::from("candidate"),
        nodes: BTreeMap::from([
            (NodeId::from("candidate"), agent("body-done")),
            (NodeId::from("body-done"), end()),
        ]),
    };
    blueprint(
        BTreeMap::from([
            (
                NodeId::from("iterate"),
                NodeSpec::Loop(LoopNode {
                    body: Box::new(body),
                    evaluator: EvaluatorId::from("quality"),
                    max_iterations: 3,
                    checkpoint: Some(CapabilityId::from("checkpoint")),
                    on_satisfied: NodeId::from("success"),
                    on_exhausted: NodeId::from("exhausted"),
                }),
            ),
            (NodeId::from("success"), end()),
            (NodeId::from("exhausted"), end()),
        ]),
        "iterate",
    )
}

fn gate_blueprint() -> SchedulerBlueprint {
    blueprint(
        BTreeMap::from([
            (
                NodeId::from("gate"),
                NodeSpec::Gate(GateNode {
                    evaluator: EvaluatorId::from("quality"),
                    on_pass: NodeId::from("passed"),
                    on_fail: NodeId::from("failed"),
                    on_indeterminate: NodeId::from("indeterminate"),
                }),
            ),
            (NodeId::from("passed"), end()),
            (NodeId::from("failed"), end()),
            (NodeId::from("indeterminate"), end()),
        ]),
        "gate",
    )
}

fn validate(
    blueprint: SchedulerBlueprint,
    catalog: &SchedulerCatalog,
    policy: &PolicyEnvelope,
) -> Result<ValidatedBlueprint, BlueprintValidationError> {
    ValidatedBlueprint::new(blueprint, catalog, policy)
}

#[test]
fn validates_bounded_single_agent_blueprint() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let validated = validate(single_agent_blueprint(), &catalog, &policy).expect("valid blueprint");

    assert_eq!(validated.blueprint().nodes.len(), 2);
    assert_eq!(validated.fingerprint().to_string().len(), 64);
}

#[test]
fn canonical_fingerprint_is_stable_and_semantic() {
    let first = single_agent_blueprint();
    let mut second = single_agent_blueprint();
    second.nodes = second.nodes.into_iter().rev().collect();

    let first_fingerprint = BlueprintFingerprint::from_blueprint(&first).expect("fingerprint");
    let second_fingerprint = BlueprintFingerprint::from_blueprint(&second).expect("fingerprint");
    assert_eq!(first_fingerprint, second_fingerprint);

    second.limits.max_tool_calls -= 1;
    let changed = BlueprintFingerprint::from_blueprint(&second).expect("fingerprint");
    assert_ne!(first_fingerprint, changed);
}

#[test]
fn catalog_fingerprint_is_stable() {
    let first = catalog();
    let mut second = catalog();
    second.tools = second.tools.into_iter().rev().collect();
    assert_eq!(
        first.fingerprint().expect("fingerprint"),
        second.fingerprint().expect("fingerprint")
    );
}

#[test]
fn catalog_accepts_natural_language_skill_summaries_and_rejects_invalid_metadata() {
    let mut catalog = catalog();
    let skill = catalog.skills.get_mut(&SkillId::from("audit")).unwrap();
    skill.summary = "A detailed audit procedure. ".repeat(20).trim().to_string();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    validate(single_agent_blueprint(), &catalog, &policy).expect("natural-language summary");

    catalog
        .skills
        .get_mut(&SkillId::from("audit"))
        .unwrap()
        .summary
        .push(' ');
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    assert!(matches!(
        validate(single_agent_blueprint(), &catalog, &policy),
        Err(BlueprintValidationError::InvalidSkillMetadata {
            field: "summary",
            ..
        })
    ));
}

#[test]
fn rejects_zero_and_policy_exceeding_limits() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let mut zero = single_agent_blueprint();
    zero.limits.max_model_calls = 0;
    assert!(matches!(
        validate(zero, &catalog, &policy),
        Err(BlueprintValidationError::ZeroLimit {
            field: "max_model_calls"
        })
    ));

    let mut excessive = single_agent_blueprint();
    excessive.limits.max_parallelism = policy.hard_limits.max_parallelism + 1;
    assert!(matches!(
        validate(excessive, &catalog, &policy),
        Err(BlueprintValidationError::LimitExceeded {
            field: "max_parallelism",
            ..
        })
    ));
}

#[test]
fn rejects_unknown_and_agent_incompatible_skill() {
    let full_catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &full_catalog);
    let mut unknown = single_agent_blueprint();
    let NodeSpec::Agent(node) = unknown.nodes.get_mut(&NodeId::from("execute")).unwrap() else {
        panic!("expected agent node");
    };
    node.skills.insert(SkillId::from("missing"));
    assert!(matches!(
        validate(unknown, &full_catalog, &policy),
        Err(BlueprintValidationError::UnknownSkill { .. })
    ));

    let mut restricted_catalog = catalog();
    restricted_catalog
        .agents
        .get_mut(&AgentId::from("worker"))
        .unwrap()
        .available_skills
        .clear();
    let restricted_policy = PolicyEnvelope::allow_catalog(limits(), &restricted_catalog);
    assert!(matches!(
        validate(
            single_agent_blueprint(),
            &restricted_catalog,
            &restricted_policy
        ),
        Err(BlueprintValidationError::SkillUnavailable { .. })
    ));
}

#[test]
fn rejects_unknown_policy_references_and_denied_capability_effect() {
    let catalog = catalog();
    let mut policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    policy.allowed_tools.insert(ToolId::from("missing"));
    assert!(matches!(
        validate(single_agent_blueprint(), &catalog, &policy),
        Err(BlueprintValidationError::InvalidPolicyReference { kind: "tool", .. })
    ));

    let mut loop_blueprint = bounded_loop_blueprint();
    let NodeSpec::Loop(loop_node) = loop_blueprint
        .nodes
        .get_mut(&NodeId::from("iterate"))
        .expect("loop node")
    else {
        panic!("expected loop node");
    };
    loop_node.checkpoint = Some(CapabilityId::from("checkpoint"));
    let mut policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    policy
        .allowed_effects
        .remove(&EffectClass::WorkspaceMutation);
    assert!(matches!(
        validate(loop_blueprint, &catalog, &policy),
        Err(BlueprintValidationError::CapabilityEffectDenied { .. })
    ));
}

#[test]
fn validates_typed_artifact_output_and_rejects_wrong_capability_kind() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let mut artifact = single_agent_blueprint();
    let NodeSpec::End(end) = artifact.nodes.get_mut(&NodeId::from("done")).unwrap() else {
        panic!("expected end node");
    };
    end.result = ResultSource::Artifact {
        capability: CapabilityId::from("artifacts"),
        name: "result.txt".to_string(),
    };
    validate(artifact.clone(), &catalog, &policy).expect("artifact output");

    let NodeSpec::End(end) = artifact.nodes.get_mut(&NodeId::from("done")).unwrap() else {
        panic!("expected end node");
    };
    end.result = ResultSource::Artifact {
        capability: CapabilityId::from("checkpoint"),
        name: "result.txt".to_string(),
    };
    assert!(matches!(
        validate(artifact, &catalog, &policy),
        Err(BlueprintValidationError::InvalidArtifactCapability { .. })
    ));
}

#[test]
fn policy_can_deny_tool_and_effect() {
    let catalog = catalog();
    let mut with_write = single_agent_blueprint();
    let NodeSpec::Agent(node) = with_write.nodes.get_mut(&NodeId::from("execute")).unwrap() else {
        panic!("expected agent node");
    };
    node.tools.insert(ToolId::from("write"));

    let mut tool_denied = PolicyEnvelope::allow_catalog(limits(), &catalog);
    tool_denied.allowed_tools.remove(&ToolId::from("write"));
    assert!(matches!(
        validate(with_write.clone(), &catalog, &tool_denied),
        Err(BlueprintValidationError::ToolDenied { .. })
    ));

    let mut effect_denied = PolicyEnvelope::allow_catalog(limits(), &catalog);
    effect_denied
        .allowed_effects
        .remove(&EffectClass::WorkspaceMutation);
    assert!(matches!(
        validate(with_write, &catalog, &effect_denied),
        Err(BlueprintValidationError::EffectDenied { .. })
    ));
}

#[test]
fn rejects_ordinary_cycles() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let cycle = blueprint(
        BTreeMap::from([
            (NodeId::from("a"), agent("b")),
            (NodeId::from("b"), agent("a")),
            (NodeId::from("unused-end"), end()),
        ]),
        "a",
    );
    assert!(matches!(
        validate(cycle, &catalog, &policy),
        Err(BlueprintValidationError::OrdinaryCycle { .. })
    ));
}

#[test]
fn validates_structured_bounded_loop() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    validate(bounded_loop_blueprint(), &catalog, &policy).expect("bounded loop should validate");
}

#[test]
fn rejects_unbounded_loop_and_denied_checkpoint() {
    let catalog = catalog();
    let mut policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let body = SchedulerGraph {
        entry: NodeId::from("body-done"),
        nodes: BTreeMap::from([(NodeId::from("body-done"), end())]),
    };
    let mut loop_blueprint = blueprint(
        BTreeMap::from([
            (
                NodeId::from("iterate"),
                NodeSpec::Loop(LoopNode {
                    body: Box::new(body),
                    evaluator: EvaluatorId::from("quality"),
                    max_iterations: 0,
                    checkpoint: Some(CapabilityId::from("checkpoint")),
                    on_satisfied: NodeId::from("done"),
                    on_exhausted: NodeId::from("done"),
                }),
            ),
            (NodeId::from("done"), end()),
        ]),
        "iterate",
    );
    assert!(matches!(
        validate(loop_blueprint.clone(), &catalog, &policy),
        Err(BlueprintValidationError::InvalidLoopIterations { .. })
    ));

    let NodeSpec::Loop(node) = loop_blueprint
        .nodes
        .get_mut(&NodeId::from("iterate"))
        .unwrap()
    else {
        panic!("expected loop node");
    };
    node.max_iterations = 2;
    policy
        .allowed_capabilities
        .remove(&CapabilityId::from("checkpoint"));
    assert!(matches!(
        validate(loop_blueprint, &catalog, &policy),
        Err(BlueprintValidationError::CapabilityDenied { .. })
    ));
}

#[test]
fn validates_parallel_join_and_rejects_disconnected_branch() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let parallel = blueprint(
        BTreeMap::from([
            (
                NodeId::from("fan-out"),
                NodeSpec::Parallel(ParallelNode {
                    branches: vec![NodeId::from("left"), NodeId::from("right")],
                    join: NodeId::from("join"),
                    max_parallelism: 2,
                    failure_mode: ParallelFailureMode::FailFast,
                }),
            ),
            (NodeId::from("left"), agent("join")),
            (NodeId::from("right"), agent("join")),
            (NodeId::from("join"), end()),
        ]),
        "fan-out",
    );
    validate(parallel.clone(), &catalog, &policy).expect("valid parallel graph");

    let mut invalid = parallel;
    invalid
        .nodes
        .insert(NodeId::from("right"), agent("side-end"));
    invalid.nodes.insert(NodeId::from("side-end"), end());
    assert!(matches!(
        validate(invalid, &catalog, &policy),
        Err(BlueprintValidationError::ParallelJoinUnreachable { .. })
    ));
}

#[test]
fn serde_rejects_unknown_blueprint_fields() {
    let mut value = serde_json::to_value(single_agent_blueprint()).expect("serialize");
    value
        .as_object_mut()
        .expect("object")
        .insert("legacyProfile".to_string(), serde_json::json!("sisyphus"));
    let error = serde_json::from_value::<SchedulerBlueprint>(value).expect_err("unknown field");
    assert!(error.to_string().contains("unknown field"));
}

#[derive(Default)]
struct TestModel {
    calls: AtomicU32,
}

#[async_trait]
impl ModelBackend for TestModel {
    async fn invoke(
        &self,
        request: ModelRequest,
        _context: &AgentObservationContext<'_>,
        _observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let handoff = &request.prompt.semi_stable.handoff;
        let has_tool_result = request
            .prompt
            .dynamic
            .history_tail
            .iter()
            .any(|item| matches!(item, ConversationItem::ToolResult { .. }));
        if handoff.goal == "use-tool" && !has_tool_result {
            return Ok(AssistantTurn {
                content: None,
                reasoning: Some("inspect tool input".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call-1".to_string(),
                    tool: ToolId::from("read"),
                    arguments: serde_json::json!({"path": "README.md"}),
                }],
                usage: Usage {
                    input_tokens: 2,
                    output_tokens: 1,
                    ..Usage::default()
                },
                finish_reason: Some("tool-calls".to_string()),
                reasoning_continuation: Some("reasoning-1".to_string()),
            });
        }
        if has_tool_result {
            assert!(matches!(
                request.prompt.dynamic.history_tail.as_slice(),
                [
                    ..,
                    ConversationItem::Assistant { .. },
                    ConversationItem::ToolResult { .. }
                ]
            ));
            assert_eq!(
                request.reasoning_continuation.as_deref(),
                Some("reasoning-1")
            );
        }
        let output = handoff
            .inputs
            .get("scheduler.branch")
            .or_else(|| handoff.inputs.get("scheduler.iteration"))
            .cloned()
            .unwrap_or_else(|| handoff.goal.clone());
        Ok(AssistantTurn {
            content: Some(output),
            reasoning: None,
            tool_calls: Vec::new(),
            usage: Usage {
                input_tokens: 2,
                output_tokens: 3,
                ..Usage::default()
            },
            finish_reason: Some("stop".to_string()),
            reasoning_continuation: None,
        })
    }
}

struct BranchFailingModel;

#[async_trait]
impl ModelBackend for BranchFailingModel {
    async fn invoke(
        &self,
        request: ModelRequest,
        _context: &AgentObservationContext<'_>,
        _observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError> {
        let branch = request
            .prompt
            .semi_stable
            .handoff
            .inputs
            .get("scheduler.branch")
            .cloned()
            .unwrap_or_default();
        if branch == "left" {
            return Err(ModelBackendError::message("left failed"));
        }
        Ok(AssistantTurn {
            content: Some(branch),
            reasoning: None,
            tool_calls: Vec::new(),
            usage: Usage::default(),
            finish_reason: Some("stop".to_string()),
            reasoning_continuation: None,
        })
    }
}

struct TwoToolModel;

#[async_trait]
impl ModelBackend for TwoToolModel {
    async fn invoke(
        &self,
        _request: ModelRequest,
        _context: &AgentObservationContext<'_>,
        _observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError> {
        Ok(AssistantTurn {
            content: None,
            reasoning: None,
            tool_calls: vec![
                ToolCall {
                    id: "call-1".to_string(),
                    tool: ToolId::from("read"),
                    arguments: serde_json::json!({}),
                },
                ToolCall {
                    id: "call-2".to_string(),
                    tool: ToolId::from("read"),
                    arguments: serde_json::json!({}),
                },
            ],
            usage: Usage::default(),
            finish_reason: Some("tool-calls".to_string()),
            reasoning_continuation: None,
        })
    }
}

struct PendingModel;

#[async_trait]
impl ModelBackend for PendingModel {
    async fn invoke(
        &self,
        _request: ModelRequest,
        _context: &AgentObservationContext<'_>,
        _observer: &dyn AgentLoopObserver,
    ) -> Result<AssistantTurn, ModelBackendError> {
        std::future::pending().await
    }
}

#[derive(Default)]
struct TestTools {
    calls: AtomicU32,
}

#[async_trait]
impl ToolBackend for TestTools {
    async fn execute(
        &self,
        _context: &AgentObservationContext<'_>,
        call: &ToolCall,
    ) -> Result<ToolExecution, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ToolExecution {
            output: format!("{}:ok", call.tool.as_str()),
            title: None,
            metadata: None,
            is_error: false,
        })
    }
}

struct PendingTools;

#[async_trait]
impl ToolBackend for PendingTools {
    async fn execute(
        &self,
        _context: &AgentObservationContext<'_>,
        _call: &ToolCall,
    ) -> Result<ToolExecution, String> {
        std::future::pending().await
    }
}

struct TestEvaluator {
    pass_after: u32,
    calls: AtomicU32,
}

#[async_trait]
impl EvaluatorBackend for TestEvaluator {
    async fn evaluate(
        &self,
        _evaluator: &EvaluatorId,
        _candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(EvaluationOutcome {
            evaluation: if call >= self.pass_after {
                Evaluation::Pass
            } else {
                Evaluation::Fail
            },
            usage: Usage::default(),
        })
    }
}

struct StaticEvaluator {
    evaluation: Evaluation,
    usage: Usage,
}

#[async_trait]
impl EvaluatorBackend for StaticEvaluator {
    async fn evaluate(
        &self,
        _evaluator: &EvaluatorId,
        _candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String> {
        Ok(EvaluationOutcome {
            evaluation: self.evaluation,
            usage: self.usage.clone(),
        })
    }
}

struct PendingEvaluator;

#[async_trait]
impl EvaluatorBackend for PendingEvaluator {
    async fn evaluate(
        &self,
        _evaluator: &EvaluatorId,
        _candidate: &NodeResult,
    ) -> Result<EvaluationOutcome, String> {
        std::future::pending().await
    }
}

#[derive(Default)]
struct TestCapabilities {
    checkpoints: AtomicU32,
    restores: AtomicU32,
    cleanups: AtomicU32,
    rollbacks: AtomicU32,
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<ExecutionEvent>>);

impl EventSink for RecordingEvents {
    fn emit(&self, event: ExecutionEvent) {
        self.0.lock().expect("events mutex poisoned").push(event);
    }
}

#[async_trait]
impl CapabilityBackend for TestCapabilities {
    async fn checkpoint(&self, request: &CheckpointRequest) -> Result<CheckpointHandle, String> {
        assert!(!request.workspace_root.is_empty());
        assert!(request.limits.max_files > 0);
        self.checkpoints.fetch_add(1, Ordering::Relaxed);
        Ok(CheckpointHandle {
            capability: request.capability.clone(),
            workspace_root: request.workspace_root.clone(),
            id: format!("checkpoint-{}", request.iteration),
            iteration: request.iteration,
        })
    }

    async fn restore(&self, request: &RestoreRequest) -> Result<(), String> {
        assert!(!request.checkpoint.id.is_empty());
        self.restores.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn store_artifact(&self, request: &ArtifactRequest) -> Result<String, String> {
        Ok(format!("{}/{}", request.capability.as_str(), request.name))
    }

    async fn finalize(&self, disposition: RunDisposition) -> Result<(), String> {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
        if disposition == RunDisposition::Rollback {
            self.rollbacks.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

struct DelayedCapabilities {
    delay: Duration,
    checkpoints_started: AtomicU32,
    checkpoints_finished: AtomicU32,
    artifacts_started: AtomicU32,
    artifacts_finished: AtomicU32,
    dispositions: Mutex<Vec<RunDisposition>>,
}

impl DelayedCapabilities {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            checkpoints_started: AtomicU32::new(0),
            checkpoints_finished: AtomicU32::new(0),
            artifacts_started: AtomicU32::new(0),
            artifacts_finished: AtomicU32::new(0),
            dispositions: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CapabilityBackend for DelayedCapabilities {
    async fn checkpoint(&self, request: &CheckpointRequest) -> Result<CheckpointHandle, String> {
        self.checkpoints_started.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(self.delay).await;
        self.checkpoints_finished.fetch_add(1, Ordering::Relaxed);
        Ok(CheckpointHandle {
            capability: request.capability.clone(),
            workspace_root: request.workspace_root.clone(),
            id: format!("delayed-{}", request.iteration),
            iteration: request.iteration,
        })
    }

    async fn restore(&self, _request: &RestoreRequest) -> Result<(), String> {
        Ok(())
    }

    async fn store_artifact(&self, request: &ArtifactRequest) -> Result<String, String> {
        self.artifacts_started.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(self.delay).await;
        self.artifacts_finished.fetch_add(1, Ordering::Relaxed);
        Ok(format!("{}/{}", request.capability.as_str(), request.name))
    }

    async fn finalize(&self, disposition: RunDisposition) -> Result<(), String> {
        self.dispositions
            .lock()
            .expect("dispositions mutex poisoned")
            .push(disposition);
        Ok(())
    }
}

fn test_engine<'a>(
    model: &'a dyn ModelBackend,
    tools: &'a dyn ToolBackend,
    evaluator: &'a TestEvaluator,
    capabilities: &'a TestCapabilities,
) -> SchedulerEngine<'a> {
    SchedulerEngine::new(
        model,
        tools,
        evaluator,
        capabilities,
        test_catalog(),
        test_policy(),
        "policy-v1",
    )
}

fn test_catalog() -> &'static SchedulerCatalog {
    static CATALOG: OnceLock<SchedulerCatalog> = OnceLock::new();
    CATALOG.get_or_init(catalog)
}

fn test_policy() -> &'static PolicyEnvelope {
    static POLICY: OnceLock<PolicyEnvelope> = OnceLock::new();
    POLICY.get_or_init(|| PolicyEnvelope::allow_catalog(limits(), test_catalog()))
}

fn validated(blueprint: SchedulerBlueprint) -> ValidatedBlueprint {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    validate(blueprint, &catalog, &policy).expect("valid blueprint")
}

fn run_request(goal: &str) -> RunRequest {
    RunRequest {
        handoff: HandoffPacket {
            goal: goal.to_string(),
            ..HandoffPacket::default()
        },
        conversation_seed: Vec::new(),
        workspace_root: "/workspace".to_string(),
        workspace_summary: "test workspace".to_string(),
    }
}

#[tokio::test]
async fn engine_runs_one_agent_loop_with_ordered_tool_history() {
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let outcome = test_engine(&model, &tools, &evaluator, &capabilities)
        .run(
            &validated(single_agent_blueprint()),
            run_request("use-tool"),
            CancellationFlag::default(),
        )
        .await
        .expect("engine run");

    assert_eq!(outcome.result.output.as_deref(), Some("use-tool"));
    assert_eq!(outcome.result.usage.model_calls, 2);
    assert_eq!(outcome.result.usage.tool_calls, 1);
    assert_eq!(outcome.result.usage.input_tokens, 4);
    assert_eq!(outcome.result.usage.output_tokens, 4);
    assert_eq!(model.calls.load(Ordering::Relaxed), 2);
    assert_eq!(tools.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn engine_runs_parallel_branches_once_and_joins_deterministically() {
    let parallel = blueprint(
        BTreeMap::from([
            (
                NodeId::from("fan-out"),
                NodeSpec::Parallel(ParallelNode {
                    branches: vec![NodeId::from("left"), NodeId::from("right")],
                    join: NodeId::from("join"),
                    max_parallelism: 2,
                    failure_mode: ParallelFailureMode::FailFast,
                }),
            ),
            (NodeId::from("left"), agent("join")),
            (NodeId::from("right"), agent("join")),
            (NodeId::from("join"), end()),
        ]),
        "fan-out",
    );
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let outcome = test_engine(&model, &tools, &evaluator, &capabilities)
        .run(
            &validated(parallel),
            run_request("parallel"),
            CancellationFlag::default(),
        )
        .await
        .expect("parallel run");

    assert_eq!(outcome.result.summary, "left\nright");
    assert_eq!(outcome.result.usage.model_calls, 2);
    assert_eq!(outcome.usage.model_calls, 2);
    assert_eq!(outcome.usage.input_tokens, 4);
    assert_eq!(outcome.usage.output_tokens, 6);
    assert_eq!(model.calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn parallel_failure_modes_fail_fast_or_collect_without_double_usage() {
    for mode in [ParallelFailureMode::FailFast, ParallelFailureMode::Collect] {
        let parallel = blueprint(
            BTreeMap::from([
                (
                    NodeId::from("fan-out"),
                    NodeSpec::Parallel(ParallelNode {
                        branches: vec![NodeId::from("left"), NodeId::from("right")],
                        join: NodeId::from("join"),
                        max_parallelism: 2,
                        failure_mode: mode,
                    }),
                ),
                (NodeId::from("left"), agent("join")),
                (NodeId::from("right"), agent("join")),
                (NodeId::from("join"), end()),
            ]),
            "fan-out",
        );
        let model = BranchFailingModel;
        let tools = TestTools::default();
        let evaluator = TestEvaluator {
            pass_after: 1,
            calls: AtomicU32::new(0),
        };
        let capabilities = TestCapabilities::default();
        let result = test_engine(&model, &tools, &evaluator, &capabilities)
            .run(
                &validated(parallel),
                run_request("parallel failure"),
                CancellationFlag::default(),
            )
            .await;
        match mode {
            ParallelFailureMode::FailFast => {
                let error = result.expect_err("fail-fast branch error");
                assert!(matches!(error, EngineError::ParallelBranch { .. }));
            }
            ParallelFailureMode::Collect => {
                let outcome = result.expect("collect branch result");
                assert!(outcome.result.summary.contains("right"));
                assert!(outcome.result.summary.contains("branch left failed"));
                assert_eq!(outcome.usage.model_calls, 2);
            }
        }
        assert_eq!(capabilities.cleanups.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn engine_runs_bounded_loop_until_evaluator_passes() {
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 2,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let events = RecordingEvents::default();
    let outcome = test_engine(&model, &tools, &evaluator, &capabilities)
        .with_events(&events)
        .run(
            &validated(bounded_loop_blueprint()),
            run_request("improve"),
            CancellationFlag::default(),
        )
        .await
        .expect("loop run");

    assert_eq!(outcome.result.output.as_deref(), Some("2"));
    assert_eq!(outcome.usage.model_calls, 2);
    assert_eq!(outcome.usage.input_tokens, 4);
    assert_eq!(outcome.usage.output_tokens, 6);
    assert_eq!(evaluator.calls.load(Ordering::Relaxed), 2);
    assert_eq!(capabilities.checkpoints.load(Ordering::Relaxed), 2);
    assert_eq!(capabilities.restores.load(Ordering::Relaxed), 1);
    let events = events.0.lock().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        ExecutionEvent::NodeStarted { path }
            if path == "root/iterate/iteration-2/candidate"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ExecutionEvent::RunCompleted { usage } if usage.model_calls == 2
    )));
}

#[tokio::test]
async fn engine_rolls_back_every_failed_iteration_before_exhaustion() {
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: u32::MAX,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let outcome = test_engine(&model, &tools, &evaluator, &capabilities)
        .run(
            &validated(bounded_loop_blueprint()),
            run_request("never-pass"),
            CancellationFlag::default(),
        )
        .await
        .expect("bounded exhaustion");

    assert_eq!(outcome.result.output.as_deref(), Some("3"));
    assert_eq!(evaluator.calls.load(Ordering::Relaxed), 3);
    assert_eq!(capabilities.checkpoints.load(Ordering::Relaxed), 3);
    assert_eq!(capabilities.restores.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn cancellation_during_loop_body_restores_checkpoint_before_returning() {
    let model = PendingModel;
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let cancellation = CancellationFlag::default();
    let signal = cancellation.clone();
    let engine = test_engine(&model, &tools, &evaluator, &capabilities);
    let validated = validated(bounded_loop_blueprint());

    let ((), result) = tokio::join!(
        async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            signal.cancel();
        },
        engine.run(&validated, run_request("cancel loop body"), cancellation,),
    );

    assert_eq!(
        result.expect_err("cancelled loop body"),
        EngineError::Agent(AgentLoopError::Cancelled)
    );
    assert_eq!(capabilities.checkpoints.load(Ordering::Relaxed), 1);
    assert_eq!(capabilities.restores.load(Ordering::Relaxed), 1);
    assert_eq!(capabilities.rollbacks.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn checkpoint_finishes_before_cancellation_or_deadline_cleanup() {
    for cancellation_driven in [true, false] {
        let model = TestModel::default();
        let tools = TestTools::default();
        let evaluator = TestEvaluator {
            pass_after: 1,
            calls: AtomicU32::new(0),
        };
        let capabilities = DelayedCapabilities::new(Duration::from_millis(30));
        let cancellation = CancellationFlag::default();
        let signal = cancellation.clone();
        let mut draft = bounded_loop_blueprint();
        if !cancellation_driven {
            draft.limits.max_wall_time_ms = 5;
        }
        let validated = validated(draft);
        let engine = SchedulerEngine::new(
            &model,
            &tools,
            &evaluator,
            &capabilities,
            test_catalog(),
            test_policy(),
            "policy-v1",
        );
        let cancel = async move {
            if cancellation_driven {
                tokio::time::sleep(Duration::from_millis(5)).await;
                signal.cancel();
            }
        };
        let ((), result) = tokio::join!(
            cancel,
            engine.run(&validated, run_request("delayed checkpoint"), cancellation),
        );

        let expected = if cancellation_driven {
            AgentLoopError::Cancelled
        } else {
            AgentLoopError::DeadlineExceeded
        };
        assert_eq!(
            result.expect_err("interrupted checkpoint"),
            EngineError::Agent(expected)
        );
        assert_eq!(capabilities.checkpoints_started.load(Ordering::Relaxed), 1);
        assert_eq!(capabilities.checkpoints_finished.load(Ordering::Relaxed), 1);
        assert_eq!(
            *capabilities
                .dispositions
                .lock()
                .expect("dispositions mutex poisoned"),
            [RunDisposition::Rollback]
        );
    }
}

#[tokio::test]
async fn artifact_finishes_before_cancelled_run_rolls_back() {
    let mut draft = single_agent_blueprint();
    let NodeSpec::End(end) = draft.nodes.get_mut(&NodeId::from("done")).unwrap() else {
        panic!("expected end node");
    };
    end.result = ResultSource::Artifact {
        capability: CapabilityId::from("artifacts"),
        name: "cancelled.txt".to_string(),
    };
    let validated = validated(draft);
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = DelayedCapabilities::new(Duration::from_millis(30));
    let cancellation = CancellationFlag::default();
    let signal = cancellation.clone();
    let engine = SchedulerEngine::new(
        &model,
        &tools,
        &evaluator,
        &capabilities,
        test_catalog(),
        test_policy(),
        "policy-v1",
    );

    let ((), result) = tokio::join!(
        async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            signal.cancel();
        },
        engine.run(&validated, run_request("artifact"), cancellation),
    );

    assert_eq!(
        result.expect_err("cancelled artifact run"),
        EngineError::Agent(AgentLoopError::Cancelled)
    );
    assert_eq!(capabilities.artifacts_started.load(Ordering::Relaxed), 1);
    assert_eq!(capabilities.artifacts_finished.load(Ordering::Relaxed), 1);
    assert_eq!(
        *capabilities
            .dispositions
            .lock()
            .expect("dispositions mutex poisoned"),
        [RunDisposition::Rollback]
    );
}

#[tokio::test]
async fn gate_routes_all_three_states_and_accounts_model_judge_usage_once() {
    for (evaluation, target) in [
        (Evaluation::Pass, "passed"),
        (Evaluation::Fail, "failed"),
        (Evaluation::Indeterminate, "indeterminate"),
    ] {
        let mut catalog = catalog();
        catalog
            .evaluators
            .get_mut(&EvaluatorId::from("quality"))
            .unwrap()
            .kind = EvaluatorKind::ModelJudge;
        let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
        let validated = ValidatedBlueprint::new(gate_blueprint(), &catalog, &policy).unwrap();
        let model = TestModel::default();
        let tools = TestTools::default();
        let evaluator = StaticEvaluator {
            evaluation,
            usage: Usage {
                input_tokens: 7,
                output_tokens: 2,
                reasoning_tokens: 1,
                cache_read_tokens: 3,
                cache_miss_tokens: 4,
                cache_write_tokens: 5,
                ..Usage::default()
            },
        };
        let capabilities = TestCapabilities::default();
        let outcome = SchedulerEngine::new(
            &model,
            &tools,
            &evaluator,
            &capabilities,
            &catalog,
            &policy,
            "policy-v1",
        )
        .run(&validated, run_request("gate"), CancellationFlag::default())
        .await
        .expect("gate run");

        assert!(outcome.node_results.contains_key(&format!("root/{target}")));
        assert_eq!(outcome.usage.model_calls, 1);
        assert_eq!(outcome.usage.input_tokens, 7);
        assert_eq!(outcome.usage.output_tokens, 2);
        assert_eq!(outcome.usage.reasoning_tokens, 1);
        assert_eq!(outcome.usage.cache_read_tokens, 3);
        assert_eq!(outcome.usage.cache_miss_tokens, 4);
        assert_eq!(outcome.usage.cache_write_tokens, 5);
    }
}

#[tokio::test]
async fn cancellation_interrupts_an_inflight_evaluator() {
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let validated = ValidatedBlueprint::new(gate_blueprint(), &catalog, &policy).unwrap();
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = PendingEvaluator;
    let capabilities = TestCapabilities::default();
    let cancellation = CancellationFlag::default();
    let signal = cancellation.clone();
    let engine = SchedulerEngine::new(
        &model,
        &tools,
        &evaluator,
        &capabilities,
        &catalog,
        &policy,
        "policy-v1",
    );
    let ((), result) = tokio::join!(
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            signal.cancel();
        },
        engine.run(&validated, run_request("cancel gate"), cancellation),
    );
    assert_eq!(
        result.expect_err("cancelled evaluator"),
        EngineError::Agent(AgentLoopError::Cancelled)
    );
}

#[tokio::test]
async fn engine_stops_before_work_when_cancelled() {
    assert_ne!(Evaluation::Indeterminate, Evaluation::Pass);
    let model = TestModel::default();
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let cancellation = CancellationFlag::default();
    cancellation.cancel();
    let error = test_engine(&model, &tools, &evaluator, &capabilities)
        .run(
            &validated(single_agent_blueprint()),
            run_request("cancelled"),
            cancellation,
        )
        .await
        .expect_err("cancelled run");
    assert_eq!(error, EngineError::Agent(AgentLoopError::Cancelled));
    assert_eq!(model.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn global_model_and_token_budgets_stop_the_agent_loop() {
    for (model_calls, tokens, expected) in [
        (
            1,
            limits().max_total_tokens,
            AgentLoopError::ModelCallBudgetExceeded,
        ),
        (
            limits().max_model_calls,
            4,
            AgentLoopError::TokenBudgetExceeded,
        ),
    ] {
        let mut draft = single_agent_blueprint();
        draft.limits.max_model_calls = model_calls;
        draft.limits.max_total_tokens = tokens;
        let model = TestModel::default();
        let tools = TestTools::default();
        let evaluator = TestEvaluator {
            pass_after: 1,
            calls: AtomicU32::new(0),
        };
        let capabilities = TestCapabilities::default();
        let error = test_engine(&model, &tools, &evaluator, &capabilities)
            .run(
                &validated(draft),
                run_request("use-tool"),
                CancellationFlag::default(),
            )
            .await
            .expect_err("budget failure");
        assert_eq!(error, EngineError::Agent(expected));
        assert_eq!(capabilities.cleanups.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn global_tool_budget_counts_each_declared_call_once() {
    let mut draft = single_agent_blueprint();
    draft.limits.max_tool_calls = 1;
    let model = TwoToolModel;
    let tools = TestTools::default();
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let capabilities = TestCapabilities::default();
    let error = test_engine(&model, &tools, &evaluator, &capabilities)
        .run(
            &validated(draft),
            run_request("two tools"),
            CancellationFlag::default(),
        )
        .await
        .expect_err("tool budget failure");
    assert_eq!(
        error,
        EngineError::Agent(AgentLoopError::ToolCallBudgetExceeded)
    );
    assert_eq!(tools.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn deadline_interrupts_pending_model_and_tool_work() {
    let evaluator = TestEvaluator {
        pass_after: 1,
        calls: AtomicU32::new(0),
    };
    let pending_model = PendingModel;
    let regular_tools = TestTools::default();
    let regular_model = TestModel::default();
    let pending_tools = PendingTools;
    let cases: [(&dyn ModelBackend, &dyn ToolBackend); 2] = [
        (&pending_model, &regular_tools),
        (&regular_model, &pending_tools),
    ];
    for (model, tools) in cases {
        let mut draft = single_agent_blueprint();
        draft.limits.max_wall_time_ms = 10;
        let capabilities = TestCapabilities::default();
        let error = test_engine(model, tools, &evaluator, &capabilities)
            .run(
                &validated(draft),
                run_request("use-tool"),
                CancellationFlag::default(),
            )
            .await
            .expect_err("deadline failure");
        assert_eq!(error, EngineError::Agent(AgentLoopError::DeadlineExceeded));
    }
}

#[test]
fn prompt_surface_keeps_stable_prefix_across_dynamic_progress() {
    let blueprint = single_agent_blueprint();
    let NodeSpec::Agent(node) = blueprint.nodes.get(&NodeId::from("execute")).unwrap() else {
        panic!("expected agent node");
    };
    let catalog = catalog();
    let policy = PolicyEnvelope::allow_catalog(limits(), &catalog);
    let blueprint_fingerprint = BlueprintFingerprint::from_blueprint(&blueprint).unwrap();
    let catalog_fingerprint = catalog.fingerprint().unwrap();
    let make_input = |progress: &str| PromptSurfaceInput {
        workspace_summary: "workspace".to_string(),
        progress_summary: progress.to_string(),
        handoff: HandoffPacket {
            goal: "audit".to_string(),
            ..HandoffPacket::default()
        },
        history_tail: Arc::new(Vec::new()),
        reasoning_continuation: None,
    };
    let authority = PromptAuthority::new(
        blueprint_fingerprint,
        catalog_fingerprint,
        &catalog,
        &policy,
        "policy-v1",
    );
    let first = build_prompt_surface(&authority, node, make_input("node 1")).unwrap();
    let second = build_prompt_surface(&authority, node, make_input("node 2")).unwrap();

    assert_eq!(first.stable, second.stable);
    assert!(
        String::from_utf8_lossy(&first.stable).contains("Follow the repository audit procedure.")
    );
    assert!(std::sync::Arc::ptr_eq(&first.stable, &second.stable));
    assert_eq!(
        second.fingerprints.compare(Some(&first.fingerprints)),
        CacheDiagnostic::DynamicTailOnly
    );

    let mut changed_node = node.clone();
    changed_node.tools.insert(ToolId::from("write"));
    let changed = build_prompt_surface(&authority, &changed_node, make_input("node 2")).unwrap();
    assert_eq!(first.stable, changed.stable);
    assert_eq!(
        changed.fingerprints.compare(Some(&first.fingerprints)),
        CacheDiagnostic::ToolSurfaceChanged
    );
}

fn template_parameters() -> TemplateParameters {
    TemplateParameters {
        name: BlueprintName::from("generated"),
        primary_agent: AgentId::from("worker"),
        collaborators: vec![AgentId::from("worker"), AgentId::from("worker")],
        skills: set([SkillId::from("audit")]),
        tools: set([ToolId::from("read")]),
        evaluator: Some(EvaluatorId::from("quality")),
        checkpoint: Some(CapabilityId::from("checkpoint")),
        limits: limits(),
        output: OutputContract {
            format: OutputFormat::Markdown,
            include_usage: true,
            include_artifact_refs: true,
        },
    }
}

struct StaticPlanner {
    calls: AtomicU32,
    decision: PlannerDecision,
}

struct CapturingPlanner {
    input: Mutex<Option<PlannerInput>>,
}

#[async_trait]
impl PlannerBackend for CapturingPlanner {
    async fn plan(&self, input: PlannerInput) -> Result<PlannerDecision, String> {
        *self.input.lock().expect("planner input mutex poisoned") = Some(input);
        Ok(PlannerDecision::CreateBlueprint {
            blueprint: single_agent_blueprint(),
        })
    }
}

#[async_trait]
impl PlannerBackend for StaticPlanner {
    async fn plan(&self, _input: PlannerInput) -> Result<PlannerDecision, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.decision.clone())
    }
}

#[tokio::test]
async fn planner_receives_workspace_and_catalog_summary_without_skill_body() {
    let planner = CapturingPlanner {
        input: Mutex::new(None),
    };
    let selector = AutoSelector::new(&planner, test_catalog(), test_policy());
    selector
        .select(SelectionRequest {
            explicit: None,
            locked: None,
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "novel task".to_string(),
            workspace_summary: "stable-workspace-summary".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect("planner selection");

    let input = planner
        .input
        .lock()
        .expect("planner input mutex poisoned")
        .clone()
        .expect("captured planner input");
    assert_eq!(input.workspace_summary, "stable-workspace-summary");
    assert_eq!(input.catalog.skills[0].id, SkillId::from("audit"));
    assert_eq!(
        input.catalog.skills[0].capability_tags,
        ["code-review".to_string()]
    );
    let encoded = serde_json::to_string(&input).unwrap();
    assert!(!encoded.contains("Follow the repository audit procedure."));
}

#[test]
fn every_builtin_template_is_data_validated_by_the_same_validator() {
    for template in [
        TemplateId::Direct,
        TemplateId::Plan,
        TemplateId::Coordinate,
        TemplateId::Verify,
        TemplateId::Autoresearch,
    ] {
        let draft = build_template(template, &template_parameters()).expect("template build");
        ValidatedBlueprint::new(draft, test_catalog(), test_policy())
            .unwrap_or_else(|error| panic!("{template:?} failed validation: {error}"));
    }
}

#[tokio::test]
async fn selector_skips_planner_for_user_lock_and_simple_task() {
    let planner = StaticPlanner {
        calls: AtomicU32::new(0),
        decision: PlannerDecision::CreateBlueprint {
            blueprint: single_agent_blueprint(),
        },
    };
    let selector = AutoSelector::new(&planner, test_catalog(), test_policy());
    let selected = selector
        .select(SelectionRequest {
            explicit: Some(ExplicitSelection::Template {
                id: TemplateId::Direct,
                parameters: template_parameters(),
            }),
            locked: None,
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "ignored".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect("selection");
    assert_eq!(selected.source, SelectionSource::User);
    assert_eq!(selected.blueprint.blueprint().nodes.len(), 2);
    let selected = selector
        .select(SelectionRequest {
            explicit: Some(ExplicitSelection::Blueprint(single_agent_blueprint())),
            locked: None,
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "ignored".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect("explicit blueprint selection");
    assert_eq!(selected.source, SelectionSource::User);
    let locked = ValidatedBlueprint::new(single_agent_blueprint(), test_catalog(), test_policy())
        .expect("session lock blueprint");
    let selected = selector
        .select(SelectionRequest {
            explicit: None,
            locked: Some(locked),
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "continuation".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect("session lock selection");
    assert_eq!(selected.source, SelectionSource::SessionLock);
    let selected = selector
        .select(SelectionRequest {
            explicit: None,
            locked: None,
            task: TaskShape {
                simple: true,
                ..TaskShape::default()
            },
            default_parameters: template_parameters(),
            goal: "simple task".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect("simple heuristic selection");
    assert_eq!(selected.source, SelectionSource::Heuristic);
    assert_eq!(planner.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn selector_validates_ai_generated_blueprint_without_fallback() {
    let mut invalid = single_agent_blueprint();
    invalid.limits.max_model_calls = 0;
    let planner = StaticPlanner {
        calls: AtomicU32::new(0),
        decision: PlannerDecision::CreateBlueprint { blueprint: invalid },
    };
    let selector = AutoSelector::new(&planner, test_catalog(), test_policy());
    let error = selector
        .select(SelectionRequest {
            explicit: None,
            locked: None,
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "novel task".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: BTreeSet::new(),
        })
        .await
        .expect_err("invalid planner output");
    assert!(matches!(
        error,
        SelectionError::Validation(BlueprintValidationError::ZeroLimit { .. })
    ));
    assert_eq!(planner.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn selector_refuses_a_user_rejected_ai_blueprint() {
    let draft = single_agent_blueprint();
    let fingerprint = BlueprintFingerprint::from_blueprint(&draft)
        .unwrap()
        .to_string();
    let planner = StaticPlanner {
        calls: AtomicU32::new(0),
        decision: PlannerDecision::CreateBlueprint { blueprint: draft },
    };
    let selector = AutoSelector::new(&planner, test_catalog(), test_policy());
    let error = selector
        .select(SelectionRequest {
            explicit: None,
            locked: None,
            task: TaskShape::default(),
            default_parameters: template_parameters(),
            goal: "try another topology".to_string(),
            workspace_summary: "workspace".to_string(),
            rejected_blueprint_fingerprints: set([fingerprint.clone()]),
        })
        .await
        .expect_err("rejected AI Blueprint");

    assert_eq!(
        error.to_string(),
        format!("AI planner returned a Blueprint rejected by the user: {fingerprint}")
    );
}
