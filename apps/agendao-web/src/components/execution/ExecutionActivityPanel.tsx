import { useEffect, useState } from "react";
import type { ConversationJumpTarget } from "../../hooks/useConversationJump";
import type { useExecutionActivity } from "../../hooks/useExecutionActivity";
import { partitionLiveExecutions } from "../../lib/liveExecutionState";
import type {
  ModelToolRepairTelemetrySummaryRecord,
  SessionToolRepairTelemetrySummaryRecord,
} from "../../lib/sessionActivity";
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "../../lib/contextPressure";
import { toolKindLabel } from "../../lib/toolLabels";
import { promptSurfaceEvidenceFromTelemetry } from "../../lib/cacheDiagnostics";
import {
  compactionContinuityFromTelemetry,
  contextClosureBoundaryStatusLabel,
  contextClosureCacheStatusLabel,
  contextClosureContractFromTelemetry,
  contextClosureExplainabilitySourceLabel,
  contextClosureGovernanceStatusLabel,
  contextClosureIsolationStatusLabel,
  contextClosurePrefixStatusLabel,
  contextClosureSeverityLabel,
} from "../../lib/contextClosureDiagnostics";
import { humanizeStageEvent, humanizeStageWaitTarget } from "../../lib/stageSignals";
import { cn } from "@/lib/utils";
import { memoryRecordIdValue } from "../../lib/memory";
import { CompactionContinuityCard } from "./CompactionContinuityCard";
import { ReadOnlyDiagnosticCard } from "./ReadOnlyDiagnosticCard";
import { StructuredDataView } from "./StructuredDataView";
import type { OutputField } from "../../lib/history";

type ExecutionActivityState = ReturnType<typeof useExecutionActivity>;

interface ExecutionActivityPanelProps {
  activity: ExecutionActivityState;
  activeStageId: string | null;
  previewStageId?: string | null;
  onJumpToConversation: (target: ConversationJumpTarget) => void;
  onNavigateStage: (stageId: string) => void;
  onNavigateAttachedSession: (
    sessionId: string,
    context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null },
  ) => void;
  onNavigateToolCall: (
    toolCallId: string,
    context?: { executionId?: string | null; stageId?: string | null },
  ) => void;
}

function formatTs(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts).toLocaleTimeString();
}

function formatMoney(value?: number | null) {
  if (typeof value !== "number" || Number.isNaN(value)) return "--";
  return `$${value.toFixed(4)}`;
}

function formatDateTime(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts).toLocaleString();
}

function formatCompactTokenCount(value: number) {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  return String(Math.round(value));
}

function currentContextEstimate(activity: ExecutionActivityState) {
  const activeStage = activity.activeStageSummary;
  const activeStageContext = activeStage && isLiveStageStatus(activeStage.status)
    ? activeStage.context_tokens ?? activeStage.estimated_context_tokens
    : null;
  return currentContextTokensFromSources(activity.sessionUsage?.context_tokens, activeStageContext);
}

function formatRepairKindSummary(
  counts: SessionToolRepairTelemetrySummaryRecord["event_kinds"] | undefined,
) {
  if (!counts?.length) return "No repair kinds recorded yet.";
  return counts
    .slice(0, 3)
    .map((count) => `${count.key} ${count.count}`)
    .join(" · ");
}

function formatRepairToolSummary(
  tools: SessionToolRepairTelemetrySummaryRecord["tools"] | undefined,
) {
  if (!tools?.length) return "No repaired tools recorded yet.";
  return tools
    .slice(0, 3)
    .map((tool) => {
      const parts = [`${tool.tool_name} ${tool.repaired_call_count}/${tool.call_count}`];
      if (tool.error_call_count > 0) {
        parts.push(`err ${tool.error_call_count}`);
      }
      if (tool.repair_event_count > 0) {
        parts.push(`events ${tool.repair_event_count}`);
      }
      return parts.join(" · ");
    })
    .join(" | ");
}

function formatTrajectoryBand(band?: string | null) {
  if (!band) return "--";
  return band.replaceAll("_", " ");
}

function liveExecutionTone(status: string) {
  switch (status) {
    case "done":
    case "result":
      return "bg-green-500/10 text-green-700 dark:text-green-300";
    case "error":
      return "bg-rose-500/10 text-rose-700 dark:text-rose-300";
    case "start":
    case "running":
      return "bg-blue-500/10 text-blue-700 dark:text-blue-300";
    default:
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300";
  }
}

function liveExecutionFieldSummary(fields: OutputField[]) {
  return fields
    .slice(0, 2)
    .map((field) => {
      const label = field.label?.trim();
      const value = field.value?.trim();
      if (label && value) return `${label}: ${value}`;
      return value || label || "";
    })
    .filter((value) => value.length > 0)
    .join(" · ");
}

function liveExecutionPreviewLabel(kind?: string | null) {
  switch (kind) {
    case "diff":
      return "Preview";
    case "code":
      return "Output";
    default:
      return "Detail";
  }
}

function runTailToneClass(tone: ExecutionActivityState["runTailSummary"]["tone"]) {
  switch (tone) {
    case "success":
      return "bg-green-500/10 text-green-700 dark:text-green-300";
    case "danger":
      return "bg-rose-500/10 text-rose-700 dark:text-rose-300";
    case "warning":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "info":
      return "bg-blue-500/10 text-blue-700 dark:text-blue-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function eventWindowLabel(page: number, count: number, pageSize: number) {
  if (count === 0) return `page ${page} · items 0`;
  const start = (page - 1) * pageSize + 1;
  const end = start + count - 1;
  return `page ${page} · items ${start}-${end}`;
}

function stageStatusTone(status: ExecutionActivityState["stageSummaries"][number]["status"]) {
  switch (status) {
    case "running":
      return "bg-blue-500/10 text-blue-700 dark:text-blue-300";
    case "waiting":
    case "blocked":
    case "retrying":
      return "bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "done":
      return "bg-green-500/10 text-green-700 dark:text-green-300";
    case "cancelled":
    case "cancelling":
      return "bg-rose-500/10 text-rose-700 dark:text-rose-300";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function stageSummaryMeta(stage: ExecutionActivityState["stageSummaries"][number]) {
  const parts: string[] = [];
  if (typeof stage.index === "number" && typeof stage.total === "number") {
    parts.push(`${stage.index}/${stage.total}`);
  }
  if (typeof stage.step === "number" && typeof stage.step_total === "number") {
    parts.push(`step ${stage.step}/${stage.step_total}`);
  }
  if (stage.waiting_on) {
    parts.push(`waiting ${humanizeStageWaitTarget(stage.waiting_on) ?? stage.waiting_on}`);
  }
  if (typeof stage.retry_attempt === "number") {
    parts.push(`retry ${stage.retry_attempt}`);
  }
  if (stage.active_agent_count > 0) {
    parts.push(`agents ${stage.active_agent_count}`);
  }
  if (stage.active_tool_count > 0) {
    parts.push(`tools ${stage.active_tool_count}`);
  }
  if (stage.attached_session_count > 0) {
    parts.push(`attached ${stage.attached_session_count}`);
  }
  if (typeof stage.skill_tree_budget === "number") {
    parts.push(
      `budget ${stage.skill_tree_budget}${stage.skill_tree_truncated ? " truncated" : ""}`,
    );
  }
  if (typeof stage.context_tokens === "number") {
    parts.push(`ctx ${formatCompactTokenCount(stage.context_tokens)}`);
  } else if (typeof stage.estimated_context_tokens === "number") {
    parts.push(`ctx ${formatCompactTokenCount(stage.estimated_context_tokens)}`);
  }
  return parts;
}

function terminalStageSummaries(
  stages: ExecutionActivityState["stageSummaries"],
): ExecutionActivityState["stageSummaries"] {
  return stages
    .filter((stage) => !isLiveStageStatus(stage.status))
    .sort((left, right) => {
      const leftIndex = left.index ?? Number.MAX_SAFE_INTEGER;
      const rightIndex = right.index ?? Number.MAX_SAFE_INTEGER;
      if (leftIndex !== rightIndex) return rightIndex - leftIndex;
      return (right.stage_id ?? "").localeCompare(left.stage_id ?? "");
    })
    .slice(0, 4);
}

function metadataValue(record: Record<string, unknown> | null | undefined, key: string) {
  const value = record?.[key];
  return typeof value === "string" && value.trim() ? value : null;
}

function executionJumpTarget(node: ExecutionActivityState["selectedExecution"]) {
  if (!node) return null;
  const toolCallId = metadataValue(node.metadata, "tool_call_id");
  if (toolCallId) {
    return {
      toolCallId,
      executionId: node.id,
      stageId: node.stage_id,
      label: node.label || toolCallId,
    };
  }
  if (node.stage_id) {
    return {
      stageId: node.stage_id,
      executionId: node.id,
      label: node.label || node.stage_id,
    };
  }
  return null;
}

function eventJumpTarget(event: ExecutionActivityState["selectedEvent"]) {
  if (!event) return null;
  const payload = event.payload ?? {};
  const toolCallId =
    (typeof payload.tool_call_id === "string" && payload.tool_call_id) ||
    (typeof payload.id === "string" && payload.id.startsWith("call_") ? payload.id : null);
  return {
    toolCallId,
    executionId: event.execution_id ?? null,
    stageId: event.stage_id ?? null,
    label: event.event_type || "event",
  };
}

function eventAttachedSessionId(event: ExecutionActivityState["selectedEvent"]) {
  if (!event) return null;
  const payload = event.payload ?? {};
  return typeof payload.attached_session_id === "string" && payload.attached_session_id
    ? payload.attached_session_id
    : null;
}

function ExecutionNodeTree({
  node,
  selectedExecutionId,
  activeStageId,
  previewStageId = null,
  onSelectExecution,
  onJumpToConversation,
}: {
  node: ExecutionActivityState["executionNodes"][number];
  selectedExecutionId: string | null;
  activeStageId: string | null;
  previewStageId?: string | null;
  onSelectExecution: (id: string) => void;
  onJumpToConversation: (target: ConversationJumpTarget) => void;
}) {
  const jumpTarget = executionJumpTarget(node);
  const stageClass =
    selectedExecutionId === node.id
      ? "active"
      : previewStageId && node.stage_id === previewStageId
        ? "stage-preview"
        : activeStageId && node.stage_id === activeStageId
          ? "stage-active"
          : "";

  return (
    <div className="pl-3 border-l-2 border-border/50">
      <div className="flex items-center gap-2">
        <button
          data-active={stageClass === "active" ? "true" : "false"}
          data-preview={stageClass === "stage-preview" ? "true" : stageClass === "stage-active" ? "true" : "false"}
          className={cn("roc-rail-item flex w-full items-center gap-2 text-sm", stageClass === "active" && "font-semibold")}
          type="button"
          onClick={() => onSelectExecution(node.id)}
        >
          <span className={cn("w-2.5 h-2.5 rounded-full shrink-0", node.status === "done" ? "bg-green-500" : node.status === "running" ? "bg-blue-500 animate-pulse" : node.status === "waiting" ? "bg-amber-400" : "bg-muted-foreground/40")} />
          <span className="text-xs text-muted-foreground font-mono">{node.kind}</span>
          <strong>{node.label || node.id}</strong>
        </button>
        {jumpTarget ? (
          <button
            className="roc-rail-link"
            type="button"
            onClick={() => onJumpToConversation(jumpTarget)}
          >
            Jump
          </button>
        ) : null}
      </div>
      {node.recent_event || node.waiting_on ? (
        <div className="text-xs text-muted-foreground pl-7 leading-relaxed">{node.recent_event || node.waiting_on}</div>
      ) : null}
      {node.children?.length ? (
        <div className="ml-3">
          {node.children.map((child) => (
            <ExecutionNodeTree
              key={child.id}
              node={child}
              selectedExecutionId={selectedExecutionId}
              activeStageId={activeStageId}
              previewStageId={previewStageId}
              onSelectExecution={onSelectExecution}
              onJumpToConversation={onJumpToConversation}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function ExecutionActivityPanel({
  activity,
  activeStageId,
  previewStageId = null,
  onJumpToConversation,
  onNavigateStage,
  onNavigateAttachedSession,
  onNavigateToolCall,
}: ExecutionActivityPanelProps) {
  const [pageDraft, setPageDraft] = useState(String(activity.activityPage));
  const contextEstimate = currentContextEstimate(activity);
  const executionJump = executionJumpTarget(activity.selectedExecution);
  const selectedEventJump = eventJumpTarget(activity.selectedEvent);
  const selectedEventAttachedSessionId = eventAttachedSessionId(activity.selectedEvent);
  const canCancelSelectedExecution =
    Boolean(activity.selectedExecution) &&
    activity.selectedExecution?.status !== "done" &&
    activity.executionCancellingId !== activity.selectedExecution?.id;

  useEffect(() => {
    setPageDraft(String(activity.activityPage));
  }, [activity.activityPage]);

  const actionButtonClass = "roc-action roc-action-pill";
  const compactActionButtonClass = "roc-action roc-action-compact";
  const sideSectionClass = "roc-rail-section";
  const sideItemCardClass = "roc-rail-item grid gap-1 bg-card/45";
  const formFieldClass = "roc-form-field";
  const formLabelClass = "roc-form-label";
  const formSelectClass = "roc-form-select";
  const formInputClass = "roc-form-control";
  const sessionMemory = activity.sessionMemory;
  const sessionMemoryRecentRuleHits = sessionMemory?.recent_rule_hits ?? [];
  const insightRecentSessionRecords = activity.sessionInsights?.memory?.recent_session_records ?? [];
  const executionRoots = activity.executionTopology?.roots ?? [];
  const recentSkillRecords =
    insightRecentSessionRecords.filter(
      (item) => item.linked_skill_name || item.derived_skill_name,
    );
  const liveExecutions = activity.liveExecutions ?? [];
  const partitionedLiveExecutions = partitionLiveExecutions(liveExecutions, {
    currentLimit: 4,
    recentLimit: 6,
  });
  const currentLiveExecutions = partitionedLiveExecutions.current;
  const recentLiveExecutionOutcomes = partitionedLiveExecutions.recent;
  const recentTerminalStages = terminalStageSummaries(activity.stageSummaries);
  const runTail = activity.runTailSummary;
  const contextClosure = contextClosureContractFromTelemetry(activity.telemetry);
  const compactionContinuity = compactionContinuityFromTelemetry(activity.telemetry);
  const promptSurfaceEvidence = promptSurfaceEvidenceFromTelemetry(activity.telemetry);
  const sessionToolRepairSummary = activity.telemetry?.tool_repair_summary ?? null;
  const modelToolRepairSummary: ModelToolRepairTelemetrySummaryRecord | null =
    activity.telemetry?.model_tool_repair_summary ?? null;
  const trajectoryQuality = activity.telemetry?.tool_trajectory_quality ?? null;

  const renderLiveExecutionCard = (entry: typeof liveExecutions[number], key: string) => {
    const fieldSummary = liveExecutionFieldSummary(entry.fields);
    const previewText = entry.preview?.text?.trim() || null;
    const previewLabel = liveExecutionPreviewLabel(entry.preview?.kind);
    return (
      <div key={key} className="roc-rail-item grid gap-1 bg-card/45">
        <div className="roc-rail-meta-list items-center">
          <span className="roc-badge px-3 py-1 text-xs">{toolKindLabel(entry.kind)}</span>
          <strong>{entry.label}</strong>
          <span className={cn("roc-badge px-3 py-1 text-xs", liveExecutionTone(entry.status))}>
            {entry.status}
          </span>
          {entry.stageId ? (
            <button
              type="button"
              className="roc-badge px-3 py-1 text-xs"
              onClick={() => onNavigateStage(entry.stageId!)}
            >
              stage {entry.stageId}
            </button>
          ) : null}
          {entry.toolCallId ? (
            <button
              type="button"
              className="roc-badge px-3 py-1 text-xs"
              onClick={() =>
                onNavigateToolCall(entry.toolCallId!, {
                  stageId: entry.stageId,
                  executionId: null,
                })
              }
            >
              tool {entry.toolCallId}
            </button>
          ) : null}
        </div>
        {entry.summary ? (
          <p className="text-sm text-muted-foreground leading-relaxed">{entry.summary}</p>
        ) : null}
        {!entry.summary && fieldSummary ? (
          <p className="text-sm text-muted-foreground leading-relaxed">{fieldSummary}</p>
        ) : null}
        {entry.fields.length ? (
          <dl className="grid gap-1 text-xs text-muted-foreground">
            {entry.fields.map((field, index) => (
              <div key={`${entry.id}-field-${index}`} className="grid gap-0.5">
                {field.label ? <dt className="font-medium text-foreground/80">{field.label}</dt> : null}
                {field.value ? <dd className="m-0 whitespace-pre-wrap break-words">{field.value}</dd> : null}
              </div>
            ))}
          </dl>
        ) : null}
        {previewText ? (
          <div className="grid gap-1">
            <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground/80">
              {previewLabel}
            </p>
            <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-md bg-muted/50 p-2 text-xs leading-relaxed text-muted-foreground">
              {previewText}
            </pre>
            {entry.preview?.truncated ? (
              <p className="text-[11px] text-muted-foreground">Preview truncated.</p>
            ) : null}
          </div>
        ) : null}
        <p className="text-xs text-muted-foreground">
          Updated {formatTs(entry.updatedAt)}
        </p>
      </div>
    );
  };

  return (
    <div className="roc-panel roc-rail-panel p-5">
      <div className="roc-rail-header">
        <div className="roc-rail-headline">
          <p className="roc-section-label">Scheduler</p>
          <h3 className="roc-rail-title">Execution + Activity</h3>
          <p className="roc-rail-description">Authority-backed topology, stage runtime, and recent event flow for the current session.</p>
        </div>
        <button
          className={actionButtonClass}
          type="button"
          onClick={() =>
            void activity.refreshExecutionActivity(
              undefined,
              activity.activityFilters,
              activity.activityPage,
            )
          }
          disabled={activity.activityLoading}
        >
          {activity.activityLoading ? "Refreshing..." : "Refresh"}
        </button>
      </div>

      {activity.executionTopology ? (
        <>
          <div className={sideSectionClass}>
            <p className="roc-section-label">Run Tail</p>
            <div className="roc-rail-item grid gap-2 bg-card/45">
              <div className="roc-rail-meta-list items-center">
                <strong>{runTail.title}</strong>
                <span className={cn("roc-badge px-3 py-1 text-xs", runTailToneClass(runTail.tone))}>
                  {runTail.status}
                </span>
              </div>
              {runTail.detail ? (
                <p className="text-sm text-muted-foreground leading-relaxed">{runTail.detail}</p>
              ) : null}
            </div>
          </div>
          <div className="roc-rail-meta-list">
            <span className="roc-badge px-3 py-1.5 text-xs">active {activity.executionTopology.active_count}</span>
            <span className="roc-badge px-3 py-1.5 text-xs">running {activity.executionTopology.running_count}</span>
            <span className="roc-badge px-3 py-1.5 text-xs">waiting {activity.executionTopology.waiting_count}</span>
            <span className="roc-badge px-3 py-1.5 text-xs">retry {activity.executionTopology.retry_count ?? 0}</span>
            <span className="roc-badge px-3 py-1.5 text-xs">cancelling {activity.executionTopology.cancelling_count ?? 0}</span>
            <span className="roc-badge px-3 py-1.5 text-xs">done {activity.executionTopology.done_count}</span>
          </div>
          <p className="text-sm text-muted-foreground leading-relaxed">
            Updated {formatTs(activity.executionTopology.updated_at ?? undefined)}
          </p>
          {activity.sessionUsage ? (
            <div className="grid gap-3 md:grid-cols-2">
              <div className={sideSectionClass}>
                <p className="roc-section-label">Session Cumulative</p>
                <div className="roc-rail-meta-list">
                  <span className="roc-badge px-3 py-1.5 text-xs">input {formatCompactTokenCount(activity.sessionUsage.input_tokens)}</span>
                  <span className="roc-badge px-3 py-1.5 text-xs">output {formatCompactTokenCount(activity.sessionUsage.output_tokens)}</span>
                  <span className="roc-badge px-3 py-1.5 text-xs">reasoning {formatCompactTokenCount(activity.sessionUsage.reasoning_tokens)}</span>
                  <span className="roc-badge px-3 py-1.5 text-xs">cache read {formatCompactTokenCount(activity.sessionUsage.cache_read_tokens)}</span>
                  <span className="roc-badge px-3 py-1.5 text-xs">cache miss {formatCompactTokenCount(activity.sessionUsage.cache_miss_tokens)}</span>
                  <span className="roc-badge px-3 py-1.5 text-xs">cache write {formatCompactTokenCount(activity.sessionUsage.cache_write_tokens)}</span>
                </div>
                {contextEstimate ? (
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    Current live context {formatCompactTokenCount(contextEstimate)}
                  </p>
                ) : null}
                <p className="text-sm text-muted-foreground leading-relaxed">Total cost {formatMoney(activity.sessionUsage.total_cost)}</p>
              </div>
              <div className={sideSectionClass}>
                <p className="roc-section-label">Active Stage</p>
                {activity.activeStageSummary ? (
                  <>
                    <div className="roc-rail-meta-list items-center">
                      <strong>{activity.activeStageSummary.stage_name}</strong>
                      <span className="roc-badge px-3 py-1 text-xs">{activity.activeStageSummary.status}</span>
                      {activity.sessionRuntime?.active_stage_count ? (
                        <span className="roc-badge px-3 py-1 text-xs">active {activity.sessionRuntime.active_stage_count}</span>
                      ) : null}
                    </div>
                    <div className="roc-rail-meta-list">
                      {typeof activity.activeStageSummary.prompt_tokens === "number" ? (
                        <span className="roc-badge px-3 py-1 text-xs">in {formatCompactTokenCount(activity.activeStageSummary.prompt_tokens)}</span>
                      ) : null}
                      {typeof activity.activeStageSummary.completion_tokens === "number" ? (
                        <span className="roc-badge px-3 py-1 text-xs">out {formatCompactTokenCount(activity.activeStageSummary.completion_tokens)}</span>
                      ) : null}
                      {typeof activity.activeStageSummary.reasoning_tokens === "number" ? (
                        <span className="roc-badge px-3 py-1 text-xs">reasoning {formatCompactTokenCount(activity.activeStageSummary.reasoning_tokens)}</span>
                      ) : null}
                      {typeof activity.activeStageSummary.skill_tree_budget === "number" ? (
                        <span className="roc-badge px-3 py-1 text-xs">budget {activity.activeStageSummary.skill_tree_budget}</span>
                      ) : null}
                    </div>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {activity.activeStageSummary.waiting_on
                        ? `Waiting for ${
                            humanizeStageWaitTarget(activity.activeStageSummary.waiting_on) ??
                            activity.activeStageSummary.waiting_on
                          }`
                        : humanizeStageEvent(activity.activeStageSummary.last_event) || "No active wait signal"}
                    </p>
                    {activity.activeStageSummary.activity ? (
                      <p className="text-sm text-muted-foreground leading-relaxed">
                        Activity: {activity.activeStageSummary.activity.replace(/\n+/g, " · ")}
                      </p>
                    ) : null}
                    {activity.activeStageSummary.skill_tree_truncated ? (
                      <p className="text-sm text-amber-700 dark:text-amber-300 leading-relaxed">
                        Skill tree truncated{activity.activeStageSummary.skill_tree_truncation_strategy
                          ? ` via ${activity.activeStageSummary.skill_tree_truncation_strategy}`
                          : ""}
                      </p>
                    ) : null}
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground leading-relaxed">No active stage summary in telemetry.</p>
                )}
              </div>
            </div>
          ) : null}
          {currentLiveExecutions.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Current Tools + Skills</p>
              <div className="grid gap-2">
                {currentLiveExecutions.map((entry) => renderLiveExecutionCard(entry, entry.id))}
              </div>
            </div>
          ) : null}
          {recentLiveExecutionOutcomes.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Recent Tool Outcomes</p>
              <div className="grid gap-2">
                {recentLiveExecutionOutcomes.map((entry) =>
                  renderLiveExecutionCard(entry, `recent-${entry.id}`),
                )}
              </div>
            </div>
          ) : null}
          {recentTerminalStages.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Recent Stage Outcomes</p>
              <div className="grid gap-2">
                {recentTerminalStages.map((stage) => {
                  const meta = stageSummaryMeta(stage);
                  return (
                    <div key={`terminal-${stage.stage_id}`} className="roc-rail-item grid gap-1 bg-card/45">
                      <div className="roc-rail-meta-list items-center">
                        <strong>{stage.stage_name}</strong>
                        <span className={cn("roc-badge px-3 py-1 text-xs", stageStatusTone(stage.status))}>
                          {stage.status}
                        </span>
                        <button
                          type="button"
                          className="roc-badge px-3 py-1 text-xs"
                          onClick={() => onNavigateStage(stage.stage_id)}
                        >
                          stage {stage.stage_id}
                        </button>
                      </div>
                      {meta.length ? (
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {meta.join(" · ")}
                        </p>
                      ) : null}
                      {stage.last_event ? (
                        <p className="text-sm text-muted-foreground leading-relaxed">
                          {humanizeStageEvent(stage.last_event) || stage.last_event}
                        </p>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}
          {sessionToolRepairSummary || modelToolRepairSummary ? (
            <div className="grid gap-3 md:grid-cols-2">
              {sessionToolRepairSummary ? (
                <div className={sideSectionClass}>
                  <p className="roc-section-label">Tool Repair</p>
                  <div className="roc-rail-meta-list">
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      repaired {sessionToolRepairSummary.repaired_tool_call_count}/
                      {sessionToolRepairSummary.total_tool_calls}
                    </span>
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      errors {sessionToolRepairSummary.error_tool_call_count}
                    </span>
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      events {sessionToolRepairSummary.repair_event_count}
                    </span>
                  </div>
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    Session-local repair activity for finalized tool calls.
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    Kinds {formatRepairKindSummary(sessionToolRepairSummary.event_kinds)}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    Tools {formatRepairToolSummary(sessionToolRepairSummary.tools)}
                  </p>
                </div>
              ) : null}
              {modelToolRepairSummary ? (
                <div className={sideSectionClass}>
                  <p className="roc-section-label">Model Repair Baseline</p>
                  <div className="roc-rail-meta-list">
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      {modelToolRepairSummary.provider_id}/{modelToolRepairSummary.model_id}
                    </span>
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      sessions {modelToolRepairSummary.session_count}
                    </span>
                    <span className="roc-badge px-3 py-1.5 text-xs">
                      repaired sessions {modelToolRepairSummary.repaired_session_count}
                    </span>
                  </div>
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    Cross-session baseline for the same provider/model pair.
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    Calls {modelToolRepairSummary.repaired_tool_call_count}/
                    {modelToolRepairSummary.total_tool_calls} repaired · errors{" "}
                    {modelToolRepairSummary.error_tool_call_count} · events{" "}
                    {modelToolRepairSummary.repair_event_count}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    Kinds {formatRepairKindSummary(modelToolRepairSummary.event_kinds)}
                  </p>
                </div>
              ) : null}
            </div>
          ) : null}
          {trajectoryQuality ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Trajectory Quality</p>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">score {trajectoryQuality.score}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">{formatTrajectoryBand(trajectoryQuality.band)}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  repaired {trajectoryQuality.repaired_tool_call_count}/{trajectoryQuality.total_tool_calls}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  errors {trajectoryQuality.error_tool_call_count}
                </span>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                Sanitizer {trajectoryQuality.sanitizer_event_count} · strict-fail {trajectoryQuality.strict_would_fail_count} · provider {trajectoryQuality.provider_diagnostic_count}
              </p>
            </div>
          ) : null}
          {(activity.telemetry?.pending_permission_count ?? 0) > 0
            || (activity.telemetry?.granted_by_turn_count ?? 0) > 0
            || (activity.telemetry?.granted_by_session_count ?? 0) > 0
            || (activity.telemetry?.last_permission_miss_count ?? 0) > 0 ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Permission Authority</p>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">
                  turn {activity.telemetry?.granted_by_turn_count ?? 0}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  session {activity.telemetry?.granted_by_session_count ?? 0}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  pending {activity.telemetry?.pending_permission_count ?? 0}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  misses {activity.telemetry?.last_permission_miss_count ?? 0}
                </span>
              </div>
              {activity.telemetry?.last_permission_matcher_kind ? (
                <p className="text-xs text-muted-foreground">
                  Last grant: {activity.telemetry.last_permission_matcher_kind}
                </p>
              ) : null}
            </div>
          ) : null}
          {activity.telemetry?.runtime_protocol ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Runtime Protocol</p>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">
                  ingress {activity.telemetry.runtime_protocol.prompt_ingress}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  steering {activity.telemetry.runtime_protocol.steering.pending_count}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  interrupt {activity.telemetry.runtime_protocol.interrupt.phase}
                </span>
              </div>
              {activity.telemetry.runtime_protocol.permission.pending ? (
                <p className="text-xs text-muted-foreground">
                  Permission {activity.telemetry.runtime_protocol.permission.pending_permission_id}
                  {activity.telemetry.runtime_protocol.permission.pending_tool
                    ? ` · ${activity.telemetry.runtime_protocol.permission.pending_tool}`
                    : ""}
                </p>
              ) : null}
              {activity.telemetry.runtime_protocol.steering.last_latency_ms != null ? (
                <p className="text-xs text-muted-foreground">
                  Steering latency {activity.telemetry.runtime_protocol.steering.last_latency_ms}ms
                </p>
              ) : null}
              {activity.telemetry.runtime_protocol.permission.last_pending_duration_ms != null ? (
                <p className="text-xs text-muted-foreground">
                  Permission pending {activity.telemetry.runtime_protocol.permission.last_pending_duration_ms}ms
                </p>
              ) : null}
            </div>
          ) : null}
          {activity.telemetry?.event_bus_telemetry ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">Event Bus</p>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">
                  sends {activity.telemetry.event_bus_telemetry.send_count}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  no-receiver {activity.telemetry.event_bus_telemetry.send_error_count}
                </span>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  max receivers {activity.telemetry.event_bus_telemetry.max_receivers}
                </span>
              </div>
              <p className="text-xs text-muted-foreground">
                Last send {activity.telemetry.event_bus_telemetry.last_send_at_ms || 0} · last error{" "}
                {activity.telemetry.event_bus_telemetry.last_send_error_at_ms || 0}
              </p>
            </div>
          ) : null}
          {contextClosure ? (
            <div className={sideSectionClass}>
              <div className="roc-rail-section-header">
                <div className="roc-rail-section-copy">
                  <p className="roc-section-label">Context Closure</p>
                  <h4 className="roc-rail-section-title">Read-only Acceptance Contract</h4>
                </div>
                <p className="roc-rail-section-note">Authority-backed telemetry snapshot</p>
              </div>
              <div className="grid gap-3 xl:grid-cols-2">
                <ReadOnlyDiagnosticCard
                  title="Prefix"
                  statusLabel={contextClosurePrefixStatusLabel(contextClosure.prefix_stability)}
                  statusTone={
                    contextClosure.prefix_stability.prefix_change_detected ? "warn" : "good"
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    Basis API view · {contextClosure.prefix_stability.api_view_messages} messages · trimmed {contextClosure.prefix_stability.trimmed_model_visible_messages}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {contextClosure.prefix_stability.explanation || "No prefix instability explanation recorded."}
                  </p>
                </ReadOnlyDiagnosticCard>

                <ReadOnlyDiagnosticCard
                  title="Boundary"
                  statusLabel={contextClosureBoundaryStatusLabel(contextClosure.compaction_boundary)}
                  statusTone={
                    contextClosure.compaction_boundary.blocking
                      ? "critical"
                      : contextClosure.compaction_boundary.boundary_recorded
                        ? "warn"
                        : "neutral"
                  }
                  badges={
                    contextClosure.compaction_boundary.governance_status
                      ? [
                          contextClosureGovernanceStatusLabel(
                            contextClosure.compaction_boundary.governance_status,
                          ),
                        ]
                      : []
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    Detail {contextClosure.compaction_boundary.phase || "--"} · {contextClosure.compaction_boundary.trigger || "--"} · {contextClosure.compaction_boundary.reason || "--"}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Request {typeof contextClosure.compaction_boundary.request_pressure_percent === "number"
                      ? `${contextClosure.compaction_boundary.request_pressure_percent}%`
                      : "--"} · live {typeof contextClosure.compaction_boundary.live_pressure_percent === "number"
                      ? `${contextClosure.compaction_boundary.live_pressure_percent}%`
                      : "--"} · attempted {contextClosure.compaction_boundary.compaction_attempted ? "yes" : "no"} · succeeded {contextClosure.compaction_boundary.compaction_succeeded ? "yes" : "no"} · blocking {contextClosure.compaction_boundary.blocking ? "yes" : "no"}
                  </p>
                </ReadOnlyDiagnosticCard>

                {compactionContinuity ? (
                  <CompactionContinuityCard
                    continuity={compactionContinuity}
                    title="Continuity"
                    className="roc-rail-item bg-card/45 p-4"
                  />
                ) : null}

                <ReadOnlyDiagnosticCard
                  title="Cache"
                  statusLabel={contextClosureCacheStatusLabel(
                    contextClosure.cache_explainability,
                  )}
                  statusTone={
                    !contextClosure.cache_explainability.issue_present
                      ? "good"
                      : contextClosure.cache_explainability.explained
                        ? "warn"
                        : "critical"
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    Source {contextClosureExplainabilitySourceLabel(
                      contextClosure.cache_explainability.source,
                    )} · severity {contextClosureSeverityLabel(
                      contextClosure.cache_explainability.severity,
                    )}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {contextClosure.cache_explainability.explanation ||
                      "No cache explainability note recorded."}
                  </p>
                  {promptSurfaceEvidence?.changed_fields?.length ? (
                    <p className="text-xs text-muted-foreground">
                      Evidence prompt surface {promptSurfaceEvidence.changed_fields.join(", ")}
                    </p>
                  ) : null}
                </ReadOnlyDiagnosticCard>

                <ReadOnlyDiagnosticCard
                  title="Isolation"
                  statusLabel={contextClosureIsolationStatusLabel(
                    contextClosure.child_history_isolation,
                  )}
                  statusTone={
                    contextClosure.child_history_isolation.child_history_in_live_prefix_detected
                      ? "critical"
                      : contextClosure.child_history_isolation.owner_local_live_prefix
                        ? "good"
                        : "warn"
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    Usage attached subtree {contextClosure.child_history_isolation.attached_subtree_session_count} · subtree cumulative {formatCompactTokenCount(
                      contextClosure.child_history_isolation.attached_subtree_cumulative_tokens,
                    )} · owner live {typeof contextClosure.child_history_isolation.owner_live_context_tokens === "number"
                      ? formatCompactTokenCount(
                          contextClosure.child_history_isolation.owner_live_context_tokens,
                        )
                      : "--"}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Scope owner-local live prefix {contextClosure.child_history_isolation.owner_local_live_prefix ? "yes" : "no"} · workflow cumulative {formatCompactTokenCount(
                      contextClosure.child_history_isolation.workflow_cumulative_tokens,
                    )}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {contextClosure.child_history_isolation.explanation}
                  </p>
                </ReadOnlyDiagnosticCard>
              </div>
            </div>
          ) : null}
          {sessionMemory ? (
            <div className={sideSectionClass}>
              <div className="roc-rail-section-header">
                <div className="roc-rail-section-copy">
                  <p className="roc-section-label">Memory Runtime</p>
                  <h4 className="roc-rail-section-title">{sessionMemory.workspace_mode} workspace explain</h4>
                </div>
                <span className="roc-badge px-3 py-1.5 text-xs">
                  snapshot {sessionMemory.frozen_snapshot_items}
                </span>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">prefetch {sessionMemory.last_prefetch_items}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">rule hits {sessionMemoryRecentRuleHits.length}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">session writes {sessionMemory.candidate_count + sessionMemory.validated_count + sessionMemory.rejected_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">warnings {sessionMemory.warning_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">methodology {sessionMemory.methodology_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">skill targets {sessionMemory.derived_skill_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">linked skills {sessionMemory.linked_skill_count}</span>
                {sessionMemory.latest_consolidation_run ? (
                  <span className="roc-badge px-3 py-1.5 text-xs">
                    consolidation {sessionMemory.latest_consolidation_run.run_id}
                  </span>
                ) : null}
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>Workspace key: {sessionMemory.workspace_key}</p>
                <p>Frozen snapshot generated: {formatDateTime(sessionMemory.frozen_snapshot_generated_at ?? undefined)}</p>
                <p>Last prefetch generated: {formatDateTime(sessionMemory.last_prefetch_generated_at ?? undefined)}</p>
                <p>
                  Last prefetch query: {sessionMemory.last_prefetch_query?.trim() || "No query captured"}
                </p>
                <p>
                  Session memory records: candidate {sessionMemory.candidate_count} · validated {sessionMemory.validated_count} · rejected {sessionMemory.rejected_count}
                </p>
                <p>
                  Validation pressure: warnings {sessionMemory.warning_count} · methodology {sessionMemory.methodology_candidate_count} · skill targets {sessionMemory.derived_skill_candidate_count}
                </p>
                <p>
                  Skill linkage: linked {sessionMemory.linked_skill_count} · feedback lessons {sessionMemory.skill_feedback_lesson_count}
                </p>
                <p>
                  Retrieval: runs {sessionMemory.retrieval_run_count} · hits {sessionMemory.retrieval_hit_count} · used {sessionMemory.retrieval_use_count}
                </p>
              </div>
              {recentSkillRecords.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Recent Skill-Linked Memory</p>
                  <div className="roc-rail-meta-list">
                    {recentSkillRecords.slice(0, 4).map((item) => (
                      <span key={memoryRecordIdValue(item.id)} className="roc-badge px-3 py-1.5 text-xs">
                        {item.linked_skill_name || item.derived_skill_name}: {item.title}
                      </span>
                    ))}
                  </div>
                </div>
              ) : null}
              {sessionMemory.latest_consolidation_run ? (
                <div className="grid gap-1 text-sm text-muted-foreground">
                  <p>
                    Latest consolidation finished {formatDateTime(sessionMemory.latest_consolidation_run.finished_at ?? sessionMemory.latest_consolidation_run.started_at)}
                  </p>
                  <p>
                    Merged {sessionMemory.latest_consolidation_run.merged_count} · promoted {sessionMemory.latest_consolidation_run.promoted_count} · conflicts {sessionMemory.latest_consolidation_run.conflict_count}
                  </p>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground leading-relaxed">No consolidation run has been recorded for this workspace yet.</p>
              )}
              {sessionMemoryRecentRuleHits.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Recent Rule Hits</p>
                  <div className="grid gap-2 md:grid-cols-2">
                    {sessionMemoryRecentRuleHits.map((hit) => (
                      <div key={hit.id} className={sideItemCardClass}>
                        <div className="flex flex-wrap items-center gap-2">
                          <strong>{hit.hit_kind}</strong>
                          {hit.memory_id ? (
                            <span className="roc-badge px-2.5 py-1 text-xs">{memoryRecordIdValue(hit.memory_id)}</span>
                          ) : null}
                        </div>
                        <p className="text-xs text-muted-foreground">
                          {hit.detail || "No detail attached"}
                        </p>
                        <p className="text-xs text-muted-foreground">
                          {formatDateTime(hit.created_at)}
                        </p>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}
        </>
      ) : (
        <div className="roc-rail-empty">
          <div className="roc-section-label">Scheduler</div>
          <p className="text-sm font-semibold tracking-tight text-foreground">No scheduler topology loaded yet.</p>
        </div>
      )}

      {activity.stageSummaries.length ? (
        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">Stage Summaries</p>
              <h4 className="roc-rail-section-title">{activity.stageSummaries.length} stages</h4>
            </div>
            <p className="roc-rail-section-note">
              Authority-backed telemetry snapshot
            </p>
          </div>
          <div className="grid gap-3 xl:grid-cols-2">
            {activity.stageSummaries.map((stage) => {
              const meta = stageSummaryMeta(stage);
              const isHighlighted =
                stage.stage_id === activity.sessionRuntime?.active_stage_id ||
                stage.stage_id === previewStageId;
              return (
                <div
                  key={stage.stage_id}
                  data-active={isHighlighted ? "true" : "false"}
                  data-preview={previewStageId === stage.stage_id ? "true" : "false"}
                  className="roc-rail-item grid gap-3 bg-card/45 p-4"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <strong className="truncate">{stage.stage_name}</strong>
                        <span
                          className={cn(
                            "rounded-full px-2.5 py-1 text-xs font-medium",
                            stageStatusTone(stage.status),
                          )}
                        >
                          {stage.status}
                        </span>
                      </div>
                      <p className="text-xs text-muted-foreground font-mono mt-1">
                        {stage.stage_id}
                      </p>
                    </div>
                    <div className="flex flex-wrap gap-2 shrink-0">
                      <button
                        className={compactActionButtonClass}
                        type="button"
                        onClick={() => onNavigateStage(stage.stage_id)}
                      >
                        Open
                      </button>
                      <button
                        className={compactActionButtonClass}
                        type="button"
                        onClick={() => activity.patchActivityFilters({ stageId: stage.stage_id })}
                      >
                        Filter Events
                      </button>
                    </div>
                  </div>
                  {meta.length ? (
                    <div className="flex flex-wrap gap-2">
                      {meta.map((item) => (
                        <span
                          key={`${stage.stage_id}:${item}`}
                          className="roc-badge px-2.5 py-1 text-xs"
                        >
                          {item}
                        </span>
                      ))}
                    </div>
                  ) : null}
                  <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                    {typeof stage.prompt_tokens === "number" ? <span>in {formatCompactTokenCount(stage.prompt_tokens)}</span> : null}
                    {typeof stage.completion_tokens === "number" ? <span>out {formatCompactTokenCount(stage.completion_tokens)}</span> : null}
                    {typeof stage.reasoning_tokens === "number" ? <span>reasoning {formatCompactTokenCount(stage.reasoning_tokens)}</span> : null}
                    {typeof stage.cache_read_tokens === "number" ? <span>cache read {formatCompactTokenCount(stage.cache_read_tokens)}</span> : null}
                    {typeof stage.cache_miss_tokens === "number" ? <span>cache miss {formatCompactTokenCount(stage.cache_miss_tokens)}</span> : null}
                    {typeof stage.cache_write_tokens === "number" ? <span>cache write {formatCompactTokenCount(stage.cache_write_tokens)}</span> : null}
                  </div>
                  {stage.last_event || stage.focus || stage.activity ? (
                    <div className="grid gap-1 text-xs text-muted-foreground">
                      {stage.last_event ? <p>Last event: {humanizeStageEvent(stage.last_event) || stage.last_event}</p> : null}
                      {stage.focus ? <p>Focus: {stage.focus}</p> : null}
                      {stage.activity ? <p>Activity: {stage.activity.replace(/\n+/g, " · ")}</p> : null}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-[repeat(3,minmax(0,1fr))_auto] md:items-end">
        <label className={formFieldClass}>
          <span className={formLabelClass}>Stage</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.stageId}
            onChange={(event) => activity.patchActivityFilters({ stageId: event.target.value })}
          >
            <option value="">all stages</option>
            {activity.stageOptions.map((stageId) => (
              <option key={stageId} value={stageId}>
                {stageId}
              </option>
            ))}
          </select>
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>Execution</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.executionId}
            onChange={(event) => activity.patchActivityFilters({ executionId: event.target.value })}
          >
            <option value="">all executions</option>
            {activity.executionNodes.map((node) => (
              <option key={node.id} value={node.id}>
                {node.label || node.id}
              </option>
            ))}
          </select>
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>Event Type</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.eventType}
            onChange={(event) => activity.patchActivityFilters({ eventType: event.target.value })}
          >
            <option value="">all events</option>
            {activity.knownEventTypes.map((eventType) => (
              <option key={eventType} value={eventType}>
                {eventType}
              </option>
            ))}
          </select>
        </label>
        <button className={actionButtonClass} type="button" onClick={activity.clearActivityFilters}>
          Clear
        </button>
      </div>

      <div className="max-h-64 overflow-auto flex flex-col gap-1">
        {executionRoots.length ? (
          executionRoots.map((node) => (
            <ExecutionNodeTree
              key={node.id}
              node={node}
              selectedExecutionId={activity.selectedExecutionId}
              activeStageId={activeStageId}
              previewStageId={previewStageId}
              onSelectExecution={activity.setSelectedExecutionId}
              onJumpToConversation={onJumpToConversation}
            />
          ))
        ) : (
          <div className="roc-rail-empty">
            <div className="roc-section-label">Execution</div>
            <p className="text-sm font-semibold tracking-tight text-foreground">No active execution topology for this session.</p>
          </div>
        )}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">Execution</p>
              <h4 className="roc-rail-section-title">{activity.selectedExecution?.label || "Select an execution node"}</h4>
            </div>
            <div className="flex flex-wrap gap-2">
              {executionJump ? (
                <button
                  className={actionButtonClass}
                  type="button"
                  onClick={() => onJumpToConversation(executionJump)}
                >
                  Jump to Message
                </button>
              ) : null}
              {activity.selectedExecution ? (
                <button
                  className={actionButtonClass}
                  type="button"
                  disabled={!canCancelSelectedExecution}
                  onClick={() => void activity.cancelExecution(activity.selectedExecution!.id || undefined)}
                >
                  {activity.executionCancellingId === activity.selectedExecution!.id
                    ? "Cancelling..."
                    : "Cancel"}
                </button>
              ) : null}
            </div>
          </div>
          {activity.selectedExecution ? (
            <>
              {(() => {
                const selected = activity.selectedExecution;
                return (
                  <>
                    <dl className="roc-structured-dl">
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">ID</dt>
                        <dd className="text-sm text-foreground">{selected.id}</dd>
                      </div>
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">Status</dt>
                        <dd className="text-sm text-foreground">{selected.status}</dd>
                      </div>
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">Stage</dt>
                        <dd className="text-sm text-foreground">
                          {selected.stage_id ? (
                            <button
                              className="roc-rail-link"
                              type="button"
                              onClick={() => onNavigateStage(selected.stage_id || "")}
                            >
                              {selected.stage_id}
                            </button>
                          ) : (
                            "--"
                          )}
                        </dd>
                      </div>
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">Updated</dt>
                        <dd className="text-sm text-foreground">{formatTs(selected.updated_at)}</dd>
                      </div>
                    </dl>
                    <div className="flex flex-wrap gap-2">
                      <button
                        className={actionButtonClass}
                        type="button"
                        onClick={() => activity.patchActivityFilters({ executionId: selected.id || "" })}
                      >
                        Filter Events to Execution
                      </button>
                      {selected.stage_id ? (
                        <button
                          className={actionButtonClass}
                          type="button"
                          onClick={() =>
                            activity.patchActivityFilters({
                              stageId: selected.stage_id || "",
                            })
                          }
                        >
                          Filter Events to Stage
                        </button>
                      ) : null}
                    </div>
                    <StructuredDataView
                      value={selected.metadata}
                      emptyLabel="No execution metadata for this node."
                    />
                  </>
                );
              })()}
            </>
          ) : (
            <div className="roc-rail-empty">
              <div className="roc-section-label">Execution</div>
              <p className="text-sm font-semibold tracking-tight text-foreground">Choose a node to inspect its metadata and provenance.</p>
            </div>
          )}
        </div>

        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">Activity</p>
              <h4 className="roc-rail-section-title">{activity.selectedEvent?.event_type || "Recent events"}</h4>
            </div>
            {selectedEventJump ? (
              <button
                className={actionButtonClass}
                type="button"
                onClick={() => onJumpToConversation(selectedEventJump)}
              >
                Jump to Provenance
              </button>
            ) : null}
          </div>
          {activity.selectedEvent ? (
            <dl className="roc-structured-dl">
              {activity.selectedEvent.stage_id ? (
                <div className="roc-structured-row">
                  <dt className="roc-structured-key">Stage</dt>
                  <dd className="text-sm text-foreground">
                    <button
                      className="roc-rail-link"
                      type="button"
                      onClick={() => onNavigateStage(activity.selectedEvent?.stage_id || "")}
                    >
                      {activity.selectedEvent.stage_id}
                    </button>
                  </dd>
                </div>
              ) : null}
              {selectedEventAttachedSessionId ? (
                <div className="roc-structured-row">
                  <dt className="roc-structured-key">Attached Session</dt>
                  <dd className="text-sm text-foreground">
                    <button
                      className="roc-rail-link"
                      type="button"
                      onClick={() =>
                        onNavigateAttachedSession(selectedEventAttachedSessionId, {
                          stageId: activity.selectedEvent?.stage_id ?? null,
                          toolCallId: selectedEventJump?.toolCallId ?? null,
                          label: activity.selectedEvent?.event_type || selectedEventAttachedSessionId,
                        })
                      }
                    >
                      {selectedEventAttachedSessionId}
                    </button>
                  </dd>
                </div>
              ) : null}
              {selectedEventJump?.toolCallId ? (
                <div className="roc-structured-row">
                  <dt className="roc-structured-key">Tool Call</dt>
                  <dd className="text-sm text-foreground">
                    <button
                      className="roc-rail-link"
                      type="button"
                      onClick={() =>
                        onNavigateToolCall(selectedEventJump.toolCallId!, {
                          executionId: selectedEventJump.executionId,
                          stageId: selectedEventJump.stageId,
                        })
                      }
                    >
                      {selectedEventJump.toolCallId}
                    </button>
                  </dd>
                </div>
              ) : null}
            </dl>
          ) : null}
          <div className="max-h-64 overflow-auto flex flex-col gap-1">
            {activity.activityEvents.length ? (
              activity.activityEvents.map((event, index) => (
                <button
                  key={event.event_id || `${event.ts || "event"}:${event.event_type || index}`}
                  data-active={activity.selectedEventId === event.event_id ? "true" : "false"}
                  data-preview={previewStageId && event.stage_id === previewStageId ? "true" : "false"}
                  className={cn("roc-rail-item flex w-full flex-col gap-1 text-sm", activity.selectedEventId === event.event_id && "font-semibold")}
                  type="button"
                  onClick={() => activity.setSelectedEventId(event.event_id || null)}
                >
                  <div className="flex items-center justify-between gap-2">
                    <strong>{event.event_type || "event"}</strong>
                    <span>{formatTs(event.ts)}</span>
                  </div>
                  {event.summary ? <p>{event.summary}</p> : null}
                  {event.stage_id || event.execution_id ? (
                    <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                      {event.stage_id ? <span>stage {event.stage_id}</span> : null}
                      {event.execution_id ? <span>exec {event.execution_id}</span> : null}
                    </div>
                  ) : null}
                </button>
              ))
            ) : (
              <div className="roc-rail-empty">
                <div className="roc-section-label">Activity</div>
                <p className="text-sm font-semibold tracking-tight text-foreground">No recent activity events for this filter.</p>
              </div>
            )}
          </div>
          <div className="roc-rail-section grid gap-3 px-3 py-2">
            <p className="text-xs text-muted-foreground">
              {eventWindowLabel(
                activity.activityPage,
                activity.activityEvents.length,
                activity.activityPageSize,
              )}{" "}
              · limit {activity.activityPageSize}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasPreviousPage}
                onClick={activity.firstActivityPage}
              >
                First
              </button>
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasPreviousPage}
                onClick={activity.previousActivityPage}
              >
                Prev
              </button>
              <label className="flex items-center gap-2">
                <span className={formLabelClass}>Page</span>
                <input
                  className={`${formInputClass} h-8 w-20 px-2.5 py-1.5`}
                  type="number"
                  min={1}
                  step={1}
                  value={pageDraft}
                  onChange={(event) => setPageDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      const page = Number.parseInt(pageDraft, 10);
                      activity.goToActivityPage(Number.isFinite(page) ? page : 1);
                    }
                  }}
                />
              </label>
              <button
                className={compactActionButtonClass}
                type="button"
                onClick={() => {
                  const page = Number.parseInt(pageDraft, 10);
                  activity.goToActivityPage(Number.isFinite(page) ? page : 1);
                }}
              >
                Go
              </button>
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasNextPage}
                onClick={activity.nextActivityPage}
              >
                Next
              </button>
            </div>
          </div>
          {activity.selectedEvent ? (
            <>
              <div className="flex flex-wrap gap-2">
                {activity.selectedEvent.execution_id ? (
                  <button
                    className={actionButtonClass}
                    type="button"
                    onClick={() =>
                      activity.patchActivityFilters({ executionId: activity.selectedEvent?.execution_id || "" })
                    }
                  >
                    Filter to Execution
                  </button>
                ) : null}
                {activity.selectedEvent.stage_id ? (
                  <button
                    className={actionButtonClass}
                    type="button"
                    onClick={() =>
                      activity.patchActivityFilters({ stageId: activity.selectedEvent?.stage_id || "" })
                    }
                  >
                    Filter to Stage
                  </button>
                ) : null}
                {selectedEventAttachedSessionId ? (
                  <button
                    className={actionButtonClass}
                    type="button"
                    onClick={() =>
                      onNavigateAttachedSession(selectedEventAttachedSessionId, {
                        stageId: activity.selectedEvent?.stage_id ?? null,
                        toolCallId: selectedEventJump?.toolCallId ?? null,
                        label: activity.selectedEvent?.event_type || selectedEventAttachedSessionId,
                      })
                    }
                  >
                    Open Attached Session
                  </button>
                ) : null}
                {selectedEventJump?.toolCallId ? (
                  <button
                    className={actionButtonClass}
                    type="button"
                    onClick={() =>
                      onNavigateToolCall(selectedEventJump.toolCallId!, {
                        executionId: selectedEventJump.executionId,
                        stageId: selectedEventJump.stageId,
                      })
                    }
                  >
                    Open Tool Call
                  </button>
                ) : null}
              </div>
              <StructuredDataView
                value={{
                  scope: activity.selectedEvent.scope,
                  stage_id: activity.selectedEvent.stage_id,
                  attached_session_id: selectedEventAttachedSessionId,
                  execution_id: activity.selectedEvent.execution_id,
                  tool_call_id: selectedEventJump?.toolCallId ?? null,
                  payload: activity.selectedEvent.payload,
                }}
                emptyLabel="No structured payload for this event."
                onNavigateKeyValue={(key, value) => {
                  if (key === "stage_id") onNavigateStage(value);
                  if (key === "attached_session_id") {
                    onNavigateAttachedSession(value, {
                      stageId: activity.selectedEvent?.stage_id ?? null,
                      toolCallId: selectedEventJump?.toolCallId ?? null,
                      label: activity.selectedEvent?.event_type || value,
                    });
                  }
                  if (key === "tool_call_id") {
                    onNavigateToolCall(value, {
                      executionId: selectedEventJump?.executionId,
                      stageId: selectedEventJump?.stageId,
                    });
                  }
                }}
              />
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
