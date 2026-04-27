use super::{SchedulerPresetKind, SchedulerProfilePlan, SchedulerStageKind};
use crate::skill_tree::SkillTreeRequestPlan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewMode {
    Normal,
    Skip,
}

/// First-layer decision: should the request enter multi-stage orchestration at all?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteMode {
    /// Skip all subsequent stages; Route's own output is the final response.
    Direct,
    /// Proceed with preset selection and multi-stage orchestration.
    Orchestrate,
}

/// When mode = Direct, what kind of direct response is it?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirectKind {
    /// A complete reply (greeting, knowledge answer, etc.)
    Reply,
    /// A clarifying question back to the user.
    Clarify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    /// First layer: does this request need orchestration?
    pub mode: RouteMode,

    /// When mode = Direct, what kind of direct response (reply vs clarify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_kind: Option<DirectKind>,

    /// When mode = Direct, the actual response content to return to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_response: Option<String>,

    /// Second layer (only when mode = Orchestrate): which preset to use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_plan_stage: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_mode: Option<ReviewMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_append: Option<String>,

    #[serde(default)]
    pub rationale_summary: String,
}

fn route_preset_description(kind: SchedulerPresetKind) -> &'static str {
    match kind {
        SchedulerPresetKind::Sisyphus => {
            "OMO-style delegation-first with 5-type intent classification and aggressive delegation"
        }
        SchedulerPresetKind::Prometheus => {
            "OMO-style planner-only workflow with interview-first clarification, planning review, and handoff instead of code execution"
        }
        SchedulerPresetKind::Atlas => {
            "OMO-style todo-list-driven parallel coordination with task completion tracking"
        }
        SchedulerPresetKind::Hephaestus => {
            "OMO-style autonomous deep worker with 5-phase internal loop (explore/plan/decide/execute/verify)"
        }
        SchedulerPresetKind::Verifier => {
            "costlier verifier workflow for explicit multi-candidate comparison using score-job evidence and selected-candidate finalization"
        }
    }
}

fn route_preset_list(presets: &[SchedulerPresetKind]) -> String {
    presets
        .iter()
        .map(|preset| {
            format!(
                "- {}: {}.",
                preset.as_str(),
                route_preset_description(*preset)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn route_preset_schema_values() -> String {
    SchedulerPresetKind::all()
        .iter()
        .map(|preset| preset.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn route_system_prompt() -> String {
    let recommended_presets = route_preset_list(SchedulerPresetKind::router_recommended_presets());
    let preset_schema_values = route_preset_schema_values();

    format!(
        r#"You are ROCode's request router.

Your job: classify the incoming request once and return a bounded routing decision.
You make TWO decisions in order. Never skip the first.

## Decision 1: Should this request enter multi-stage orchestration?

- Greeting, thanks, social chat → mode: "direct", direct_kind: "reply"
- Pure concept question (not about this repo) → mode: "direct", direct_kind: "reply"
- Ambiguous request that needs clarification before any work → mode: "direct", direct_kind: "clarify"
- Concrete coding task, bug fix, refactoring → mode: "orchestrate"
- Questions about this repo that require reading code → mode: "orchestrate"

When mode = "direct", write your full response in direct_response and stop.
When mode = "orchestrate", proceed to Decision 2.

## Decision 2: How to orchestrate? (only when mode = "orchestrate")

Recommended public presets:
{recommended_presets}

Rubric:
- Prefer sisyphus when the request is execution-oriented, concrete enough to act on now, and the main value comes from immediate delegation plus verification rather than interview-first planning.
- Choose sisyphus when a single-loop delegation-first executor should classify intent, assess the local codebase, fan out explore/librarian work in parallel, and then implement or delegate bounded tasks.
- Do not prefer sisyphus when the user primarily needs a reviewed planning handoff, architecture interview, or explicit planning-only workflow before any execution.
- Prefer prometheus when the right outcome is an interview-first, planner-only workflow that ends in a reviewed plan handoff instead of code execution.
- Choose prometheus especially when requirements, scope boundaries, guardrails, test strategy, or acceptance criteria are not yet locked and should be clarified before execution.
- Choose prometheus when read-only repo inspection or planning-oriented research should happen before committing to an execution path, or when the user is explicitly asking for a plan, architecture, migration strategy, or work breakdown.
- Do not prefer prometheus when the request is already concrete, execution-ready, and the main value comes from immediate delegated implementation rather than reviewed planning.
- Prefer atlas when the task is coordination-heavy: a real task list or work plan, multiple worker rounds, explicit task tracking, parallel waves, and QA-style verification of each delegated item.
- Do not prefer atlas when the main value comes from a single executor acting end-to-end without worker coordination, or when the user mainly wants a planning-only handoff.
- Prefer hephaestus when a single autonomous deep worker should act immediately, keep orchestration overhead low, make the change end-to-end, and verify the result without interview-first planning.
- Do not prefer hephaestus when the task primarily needs multi-worker coordination, explicit task-ledger management, or a reviewed planning handoff before execution.
- Set insert_plan_stage=true only when an extra planning step should be inserted before delegation or execution; for prometheus, the workflow is already planning-first, so this is usually unnecessary.
- Set review_mode=skip only when extra review is clearly unnecessary; for prometheus, review should normally remain enabled.
- Use context_append only for short execution-critical context, not for re-explaining the whole task.

You may inspect the repo with read-only tools before deciding.
Never produce chain-of-thought. Return only a JSON object, optionally inside a ```json block.

JSON schema (direct mode):
{{
  "mode": "direct",
  "direct_kind": "reply | clarify",
  "direct_response": "your full response to the user",
  "rationale_summary": "short summary"
}}

JSON schema (orchestrate mode):
{{
  "mode": "orchestrate",
  "preset": "{preset_schema_values} | null",
  "insert_plan_stage": true | false | null,
  "review_mode": "normal | skip | null",
  "context_append": "optional short markdown string or null",
  "rationale_summary": "short summary"
}}"#
    )
}

pub fn parse_route_decision(output: &str) -> Option<RouteDecision> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    for candidate in json_candidates(trimmed) {
        if let Ok(decision) = serde_json::from_str::<RouteDecision>(&candidate) {
            if validate_route_decision(&decision).is_ok() {
                return Some(decision);
            }
        }
    }

    None
}

pub fn validate_route_decision(decision: &RouteDecision) -> Result<(), String> {
    match decision.mode {
        RouteMode::Direct => {
            if decision.direct_kind.is_none() {
                return Err("direct route decisions must include `direct_kind`".to_string());
            }
            let has_response = decision
                .direct_response
                .as_deref()
                .map(str::trim)
                .map(|value| !value.is_empty())
                .unwrap_or(false);
            if !has_response {
                return Err(
                    "direct route decisions must include a non-empty `direct_response`".to_string(),
                );
            }
            if decision.preset.is_some()
                || decision.insert_plan_stage.is_some()
                || decision.review_mode.is_some()
            {
                return Err(
                    "direct route decisions may not include orchestration-only fields".to_string(),
                );
            }
        }
        RouteMode::Orchestrate => {
            if decision.direct_kind.is_some() || decision.direct_response.is_some() {
                return Err(
                    "orchestrate route decisions may not include direct-response fields"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

pub fn apply_route_decision(
    resolved_plan: &mut SchedulerProfilePlan,
    route_stage_index: usize,
    decision: &RouteDecision,
) {
    let prefix = resolved_plan.stages[..=route_stage_index].to_vec();
    let mut suffix = resolved_plan.stages[route_stage_index + 1..].to_vec();

    if let Some(preset_name) = decision.preset.as_deref() {
        if let Ok(kind) = preset_name.parse::<SchedulerPresetKind>() {
            suffix = default_post_route_stages(kind);
            resolved_plan.orchestrator = Some(kind.as_str().to_string());
        }
    }

    if decision.insert_plan_stage == Some(true) && !suffix.contains(&SchedulerStageKind::Plan) {
        let insert_at = suffix
            .iter()
            .position(|stage| {
                matches!(
                    stage,
                    SchedulerStageKind::Delegation | SchedulerStageKind::ExecutionOrchestration
                )
            })
            .unwrap_or(suffix.len());
        suffix.insert(insert_at, SchedulerStageKind::Plan);
    }

    if decision.review_mode == Some(ReviewMode::Skip) {
        suffix.retain(|stage| *stage != SchedulerStageKind::Review);
    }

    resolved_plan.stages = prefix.into_iter().chain(suffix).collect();

    if let Some(context) = decision
        .context_append
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        append_skill_tree_context(&mut resolved_plan.skill_tree, context);
    }
}

fn default_post_route_stages(kind: SchedulerPresetKind) -> Vec<SchedulerStageKind> {
    kind.definition().post_route_stage_kinds()
}

fn append_skill_tree_context(skill_tree: &mut Option<SkillTreeRequestPlan>, context: &str) {
    match skill_tree {
        Some(tree) => tree.append_context(context),
        None => {
            *skill_tree = Some(SkillTreeRequestPlan {
                context_markdown: context.to_string(),
                token_budget: None,
                truncation_strategy: Default::default(),
            });
        }
    }
}

fn json_candidates(output: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    for marker in ["```json", "```JSON", "```"] {
        let mut remaining = output;
        while let Some(start) = remaining.find(marker) {
            let after = &remaining[start + marker.len()..];
            if let Some(end) = after.find("```") {
                let candidate = after[..end].trim();
                if !candidate.is_empty() {
                    candidates.push(candidate.to_string());
                }
                remaining = &after[end + 3..];
            } else {
                break;
            }
        }
    }

    if let Some((start, end)) = find_balanced_json_object(output) {
        let candidate = output[start..end].trim();
        if !candidate.is_empty() {
            candidates.push(candidate.to_string());
        }
    }

    if candidates.is_empty() {
        candidates.push(output.trim().to_string());
    }

    candidates
}

fn find_balanced_json_object(input: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return start.map(|begin| (begin, idx + ch.len_utf8()));
                }
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_plan() -> SchedulerProfilePlan {
        SchedulerProfilePlan::new(vec![
            SchedulerStageKind::RequestAnalysis,
            SchedulerStageKind::Route,
            SchedulerStageKind::Delegation,
            SchedulerStageKind::Review,
            SchedulerStageKind::Synthesis,
        ])
    }

    #[test]
    fn route_prompt_only_recommends_public_omo_presets() {
        let prompt = route_system_prompt();
        assert!(prompt.contains("Recommended public presets:"));
        assert!(prompt.contains("- sisyphus:"));
        assert!(prompt.contains("- prometheus:"));
        assert!(prompt.contains("- atlas:"));
        assert!(prompt.contains("- hephaestus:"));
        assert!(prompt.contains("- verifier:"));
        assert!(prompt.contains("explicit multi-candidate comparison"));
        assert!(!prompt.contains("Prefer jiangziya when"));
        assert!(!prompt.contains("Prefer guiguzi when"));
        assert!(!prompt.contains("Prefer pangu when"));
        assert!(!prompt.contains("Prefer luban when"));
    }

    #[test]
    fn route_prompt_describes_sisyphus_as_single_loop_executor() {
        let prompt = route_system_prompt();
        assert!(prompt.contains("execution-oriented, concrete enough to act on now"));
        assert!(prompt.contains("single-loop delegation-first executor"));
        assert!(prompt.contains("fan out explore/librarian work in parallel"));
        assert!(prompt.contains("reviewed planning handoff"));
    }

    #[test]
    fn route_prompt_describes_atlas_as_coordination_loop() {
        let prompt = route_system_prompt();
        assert!(prompt.contains("coordination-heavy: a real task list or work plan"));
        assert!(prompt.contains("explicit task tracking, parallel waves"));
        assert!(prompt.contains("QA-style verification of each delegated item"));
        assert!(prompt.contains("single executor acting end-to-end"));
    }

    #[test]
    fn route_prompt_describes_hephaestus_as_autonomous_deep_worker() {
        let prompt = route_system_prompt();
        assert!(prompt.contains("single autonomous deep worker should act immediately"));
        assert!(prompt.contains("keep orchestration overhead low"));
        assert!(prompt.contains("verify the result without interview-first planning"));
        assert!(prompt.contains("reviewed planning handoff before execution"));
    }

    #[test]
    fn route_prompt_describes_prometheus_as_interview_first_planner() {
        let prompt = route_system_prompt();
        assert!(prompt.contains("interview-first, planner-only workflow"));
        assert!(prompt.contains("requirements, scope boundaries, guardrails, test strategy, or acceptance criteria are not yet locked"));
        assert!(prompt.contains("for prometheus, the workflow is already planning-first"));
        assert!(prompt.contains("for prometheus, review should normally remain enabled"));
    }

    #[test]
    fn parse_route_output_extracts_json_from_markdown_block() {
        let output = r#"
analysis
```json
{
  "mode": "orchestrate",
  "preset": "prometheus",
  "insert_plan_stage": null,
  "review_mode": "normal",
  "context_append": null,
  "rationale_summary": "needs planning"
}
```
"#;
        let decision = parse_route_decision(output).expect("decision should parse");
        assert_eq!(decision.mode, RouteMode::Orchestrate);
        assert_eq!(decision.preset.as_deref(), Some("prometheus"));
        assert_eq!(decision.review_mode, Some(ReviewMode::Normal));
    }

    #[test]
    fn parse_route_output_handles_plain_json() {
        let output = r#"{"mode":"orchestrate","preset":"sisyphus","rationale_summary":"direct"}"#;
        let decision = parse_route_decision(output).expect("decision should parse");
        assert_eq!(decision.mode, RouteMode::Orchestrate);
        assert_eq!(decision.preset.as_deref(), Some("sisyphus"));
        assert_eq!(decision.rationale_summary, "direct");
    }

    #[test]
    fn parse_route_output_handles_direct_reply() {
        let output = r#"{"mode":"direct","direct_kind":"reply","direct_response":"你好！有什么可以帮你的？","rationale_summary":"greeting"}"#;
        let decision = parse_route_decision(output).expect("decision should parse");
        assert_eq!(decision.mode, RouteMode::Direct);
        assert_eq!(decision.direct_kind, Some(DirectKind::Reply));
        assert!(decision.direct_response.as_ref().unwrap().contains("你好"));
    }

    #[test]
    fn parse_route_output_handles_direct_clarify() {
        let output = r#"{"mode":"direct","direct_kind":"clarify","direct_response":"你是想看整个模块的结构，还是某个具体函数？","rationale_summary":"ambiguous request"}"#;
        let decision = parse_route_decision(output).expect("decision should parse");
        assert_eq!(decision.mode, RouteMode::Direct);
        assert_eq!(decision.direct_kind, Some(DirectKind::Clarify));
    }

    #[test]
    fn parse_route_output_rejects_direct_without_response() {
        let output = r#"{"mode":"direct","direct_kind":"reply","rationale_summary":"greeting"}"#;
        assert!(parse_route_decision(output).is_none());
    }

    #[test]
    fn validate_route_decision_rejects_mixed_direct_and_orchestration_fields() {
        let decision = RouteDecision {
            mode: RouteMode::Direct,
            direct_kind: Some(DirectKind::Reply),
            direct_response: Some("hi".to_string()),
            preset: Some("prometheus".to_string()),
            insert_plan_stage: None,
            review_mode: None,
            context_append: None,
            rationale_summary: "invalid".to_string(),
        };
        assert!(validate_route_decision(&decision).is_err());
    }

    fn orchestrate_decision(
        preset: Option<&str>,
        insert_plan_stage: Option<bool>,
        review_mode: Option<ReviewMode>,
        context_append: Option<&str>,
        rationale: &str,
    ) -> RouteDecision {
        RouteDecision {
            mode: RouteMode::Orchestrate,
            direct_kind: None,
            direct_response: None,
            preset: preset.map(str::to_string),
            insert_plan_stage,
            review_mode,
            context_append: context_append.map(str::to_string),
            rationale_summary: rationale.to_string(),
        }
    }

    #[test]
    fn apply_preset_replaces_stages_after_route() {
        let mut plan = base_plan();
        let decision = orchestrate_decision(Some("prometheus"), None, None, None, "needs plan");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn apply_insert_plan_stage_adds_plan_before_delegation() {
        let mut plan = base_plan();
        let decision = orchestrate_decision(None, Some(true), None, None, "insert plan");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Delegation,
                SchedulerStageKind::Review,
                SchedulerStageKind::Synthesis,
            ]
        );
    }

    #[test]
    fn apply_review_mode_skip_removes_review() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(None, None, Some(ReviewMode::Skip), None, "skip review");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::Delegation,
                SchedulerStageKind::Synthesis,
            ]
        );
    }

    #[test]
    fn apply_context_append_updates_skill_tree() {
        let mut plan = base_plan().with_skill_tree(SkillTreeRequestPlan {
            context_markdown: "base context".to_string(),
            token_budget: None,
            truncation_strategy: Default::default(),
        });
        let decision =
            orchestrate_decision(None, None, None, Some("extra route context"), "append");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.skill_tree
                .as_ref()
                .map(|tree| tree.context_markdown.as_str()),
            Some("base context\n\nextra route context")
        );
    }

    #[test]
    fn apply_context_append_respects_skill_tree_budget() {
        let mut plan = base_plan().with_skill_tree(SkillTreeRequestPlan {
            context_markdown: "AAAAAAAAAAAAAAAAAAAA".to_string(),
            token_budget: Some(10),
            truncation_strategy: crate::skill_tree::SkillTreeTruncationStrategy::Tail,
        });
        let decision =
            orchestrate_decision(None, None, None, Some("BBBBBBBBBBBBBBBBBBBB"), "append");
        apply_route_decision(&mut plan, 1, &decision);

        let context = plan
            .skill_tree
            .as_ref()
            .map(|tree| tree.context_markdown.as_str())
            .expect("skill tree should remain");
        assert!(context.contains("truncated"));
        assert!(context.ends_with("BBBBBBBBBB"));
    }

    #[test]
    fn apply_prometheus_preset_replaces_stages_after_route() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("prometheus"), None, None, None, "planner handoff");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::Interview,
                SchedulerStageKind::Plan,
                SchedulerStageKind::Review,
                SchedulerStageKind::Handoff,
            ]
        );
    }

    #[test]
    fn apply_prometheus_preset_updates_request_scoped_orchestrator() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("prometheus"), None, None, None, "planner handoff");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(plan.orchestrator.as_deref(), Some("prometheus"));
    }

    #[test]
    fn apply_sisyphus_preset_replaces_stages_after_route() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("sisyphus"), None, None, None, "single-loop delegation");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn apply_sisyphus_preset_updates_request_scoped_orchestrator() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("sisyphus"), None, None, None, "single-loop delegation");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(plan.orchestrator.as_deref(), Some("sisyphus"));
    }

    #[test]
    fn apply_atlas_preset_replaces_stages_after_route() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("atlas"), None, None, None, "todo-list coordination");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::ExecutionOrchestration,
                SchedulerStageKind::Synthesis,
            ]
        );
    }

    #[test]
    fn apply_atlas_preset_updates_request_scoped_orchestrator() {
        let mut plan = base_plan();
        let decision =
            orchestrate_decision(Some("atlas"), None, None, None, "todo-list coordination");
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(plan.orchestrator.as_deref(), Some("atlas"));
    }

    #[test]
    fn apply_hephaestus_preset_replaces_stages_after_route() {
        let mut plan = base_plan();
        let decision = orchestrate_decision(
            Some("hephaestus"),
            None,
            None,
            None,
            "autonomous deep worker",
        );
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(
            plan.stages,
            vec![
                SchedulerStageKind::RequestAnalysis,
                SchedulerStageKind::Route,
                SchedulerStageKind::ExecutionOrchestration,
            ]
        );
    }

    #[test]
    fn apply_hephaestus_preset_updates_request_scoped_orchestrator() {
        let mut plan = base_plan();
        let decision = orchestrate_decision(
            Some("hephaestus"),
            None,
            None,
            None,
            "autonomous deep worker",
        );
        apply_route_decision(&mut plan, 1, &decision);
        assert_eq!(plan.orchestrator.as_deref(), Some("hephaestus"));
    }
}
