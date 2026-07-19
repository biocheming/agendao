import { useEffect, useState } from "react";
import { useI18n } from "../../i18n/I18nProvider";
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

function summarizeMetrics(items: Array<[string, string | number | null | undefined]>) {
  return items
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([label, value]) => `${label} ${value}`)
    .join(" · ");
}

function currentContextEstimate(activity: ExecutionActivityState) {
  const activeStage = activity.activeStageSummary;
  const activeStageContext = activeStage && isLiveStageStatus(activeStage.status)
    ? activeStage.context_tokens ?? activeStage.estimated_context_tokens
    : null;
  return currentContextTokensFromSources(activity.sessionUsage?.context_tokens, activeStageContext);
}

type TranslateFn = (key: string, params?: Record<string, string | number>) => string;

function formatRepairKindSummary(
  counts: SessionToolRepairTelemetrySummaryRecord["event_kinds"] | undefined,
  t: TranslateFn,
) {
  if (!counts?.length) return t("execution.noRepairKinds");
  return counts
    .slice(0, 3)
    .map((count) => `${count.key} ${count.count}`)
    .join(" · ");
}

function formatRepairToolSummary(
  tools: SessionToolRepairTelemetrySummaryRecord["tools"] | undefined,
  t: TranslateFn,
) {
  if (!tools?.length) return t("execution.noRepairedTools");
  return tools
    .slice(0, 3)
    .map((tool) => {
      const parts = [`${tool.tool_name} ${tool.repaired_call_count}/${tool.call_count}`];
      if (tool.error_call_count > 0) {
        parts.push(`${t("execution.metric.err")} ${tool.error_call_count}`);
      }
      if (tool.repair_event_count > 0) {
        parts.push(`${t("execution.metric.events")} ${tool.repair_event_count}`);
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
      return "bg-(--ds-ok)/10 text-(--ds-ok)";
    case "error":
      return "bg-(--ds-error)/10 text-(--ds-error)";
    case "start":
    case "running":
      return "bg-(--ds-info)/10 text-(--ds-info)";
    default:
      return "bg-(--ds-fire)/10 text-(--ds-fire)";
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

function liveExecutionPreviewLabel(kind: string | null | undefined, t: TranslateFn) {
  switch (kind) {
    case "diff":
      return t("execution.previewKind.preview");
    case "code":
      return t("execution.previewKind.output");
    default:
      return t("execution.previewKind.detail");
  }
}

function runTailToneClass(tone: ExecutionActivityState["runTailSummary"]["tone"]) {
  switch (tone) {
    case "success":
      return "bg-(--ds-ok)/10 text-(--ds-ok)";
    case "danger":
      return "bg-(--ds-error)/10 text-(--ds-error)";
    case "warning":
      return "bg-(--ds-warn)/10 text-(--ds-warn)";
    case "info":
      return "bg-(--ds-info)/10 text-(--ds-info)";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function eventWindowLabel(page: number, count: number, pageSize: number, t: TranslateFn) {
  if (count === 0) return t("execution.eventWindow", { page, range: "0" });
  const start = (page - 1) * pageSize + 1;
  const end = start + count - 1;
  return t("execution.eventWindow", { page, range: `${start}-${end}` });
}

function stageStatusTone(status: ExecutionActivityState["stageSummaries"][number]["status"]) {
  switch (status) {
    case "running":
      return "bg-(--ds-info)/10 text-(--ds-info)";
    case "waiting":
    case "blocked":
    case "retrying":
      return "bg-(--ds-warn)/10 text-(--ds-warn)";
    case "done":
      return "bg-(--ds-ok)/10 text-(--ds-ok)";
    case "cancelled":
    case "cancelling":
      return "bg-(--ds-error)/10 text-(--ds-error)";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function stageSummaryMeta(stage: ExecutionActivityState["stageSummaries"][number], t: TranslateFn) {
  const parts: string[] = [];
  if (typeof stage.index === "number" && typeof stage.total === "number") {
    parts.push(`${stage.index}/${stage.total}`);
  }
  if (typeof stage.step === "number" && typeof stage.step_total === "number") {
    parts.push(`${t("execution.metric.step")} ${stage.step}/${stage.step_total}`);
  }
  if (stage.waiting_on) {
    parts.push(`${t("execution.metric.waiting")} ${humanizeStageWaitTarget(stage.waiting_on) ?? stage.waiting_on}`);
  }
  if (typeof stage.retry_attempt === "number") {
    parts.push(`${t("execution.metric.retry")} ${stage.retry_attempt}`);
  }
  if (stage.active_agent_count > 0) {
    parts.push(`${t("execution.metric.agents")} ${stage.active_agent_count}`);
  }
  if (stage.active_tool_count > 0) {
    parts.push(`${t("execution.metric.tools")} ${stage.active_tool_count}`);
  }
  if (stage.attached_session_count > 0) {
    parts.push(`${t("execution.metric.attached")} ${stage.attached_session_count}`);
  }
  if (typeof stage.skill_tree_budget === "number") {
    parts.push(
      `${t("execution.metric.budget")} ${stage.skill_tree_budget}${stage.skill_tree_truncated ? t("execution.truncatedSuffix") : ""}`,
    );
  }
  if (typeof stage.context_tokens === "number") {
    parts.push(`${t("execution.metric.ctx")} ${formatCompactTokenCount(stage.context_tokens)}`);
  } else if (typeof stage.estimated_context_tokens === "number") {
    parts.push(`${t("execution.metric.ctx")} ${formatCompactTokenCount(stage.estimated_context_tokens)}`);
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
  const { t } = useI18n();
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
          <span className={cn("w-2.5 h-2.5 rounded-full shrink-0", node.status === "done" ? "bg-(--ds-ok)" : node.status === "running" ? "bg-(--ds-info) animate-pulse" : node.status === "waiting" ? "bg-(--ds-fire)" : "bg-muted-foreground/40")} />
          <span className="text-xs text-muted-foreground font-mono">{node.kind}</span>
          <strong>{node.label || node.id}</strong>
        </button>
        {jumpTarget ? (
          <button
            className="roc-rail-link"
            type="button"
            onClick={() => onJumpToConversation(jumpTarget)}
          >
            {t("execution.jump")}
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
  const { t } = useI18n();
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
    const previewLabel = liveExecutionPreviewLabel(entry.preview?.kind, t);
    return (
      <div key={key} className="roc-rail-item grid gap-1 bg-card/45">
        <div className="roc-rail-meta-list items-center">
          <span className="text-xs text-muted-foreground">{toolKindLabel(entry.kind)}</span>
          <strong>{entry.label}</strong>
          <span className={cn("roc-badge px-3 py-1 text-xs", liveExecutionTone(entry.status))}>
            {entry.status}
          </span>
          {entry.stageId ? (
            <button
              type="button"
              className="text-xs text-muted-foreground transition-colors hover:text-primary"
              onClick={() => onNavigateStage(entry.stageId!)}
            >
              {t("composer.provenanceStage", { id: entry.stageId })}
            </button>
          ) : null}
          {entry.toolCallId ? (
            <button
              type="button"
              className="text-xs text-muted-foreground transition-colors hover:text-primary"
              onClick={() =>
                onNavigateToolCall(entry.toolCallId!, {
                  stageId: entry.stageId,
                  executionId: null,
                })
              }
            >
              {t("composer.provenanceTool", { id: entry.toolCallId })}
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
              <p className="text-[11px] text-muted-foreground">{t("execution.previewTruncated")}</p>
            ) : null}
          </div>
        ) : null}
        <p className="text-xs text-muted-foreground">
          {t("session.updatedAt", { time: formatTs(entry.updatedAt) })}
        </p>
      </div>
    );
  };

  return (
    <div className="roc-panel roc-rail-panel p-5">
      <div className="roc-rail-header">
        <div className="roc-rail-headline">
          <p className="roc-section-label">{t("execution.section.scheduler")}</p>
          <h3 className="roc-rail-title">{t("execution.panelTitle")}</h3>
          <p className="roc-rail-description">{t("execution.panelDescription")}</p>
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
          {activity.activityLoading ? t("execution.refreshing") : t("execution.refresh")}
        </button>
      </div>

      {activity.executionTopology ? (
        <>
          <div className={sideSectionClass}>
            <p className="roc-section-label">{t("execution.section.runTail")}</p>
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
          <p className="text-sm text-muted-foreground leading-relaxed">
            {t("execution.topology")}{" "}
            {summarizeMetrics([
              [t("execution.metric.active"), activity.executionTopology.active_count],
              [t("execution.metric.running"), activity.executionTopology.running_count],
              [t("execution.metric.waiting"), activity.executionTopology.waiting_count],
              [t("execution.metric.retry"), activity.executionTopology.retry_count ?? 0],
              [t("execution.metric.cancelling"), activity.executionTopology.cancelling_count ?? 0],
              [t("execution.metric.done"), activity.executionTopology.done_count],
            ])}
          </p>
          <p className="text-sm text-muted-foreground leading-relaxed">
            {t("session.updatedAt", { time: formatTs(activity.executionTopology.updated_at ?? undefined) })}
          </p>
          {activity.sessionUsage ? (
            <div className="grid gap-3 md:grid-cols-2">
              <div className={sideSectionClass}>
                <p className="roc-section-label">{t("execution.section.sessionCumulative")}</p>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {t("execution.tokens")}{" "}
                  {summarizeMetrics([
                    [t("execution.metric.input"), formatCompactTokenCount(activity.sessionUsage.input_tokens)],
                    [t("execution.metric.output"), formatCompactTokenCount(activity.sessionUsage.output_tokens)],
                    [t("execution.metric.reasoning"), formatCompactTokenCount(activity.sessionUsage.reasoning_tokens)],
                  ])}
                </p>
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {t("execution.cache")}{" "}
                  {summarizeMetrics([
                    [t("execution.metric.read"), formatCompactTokenCount(activity.sessionUsage.cache_read_tokens)],
                    [t("execution.metric.miss"), formatCompactTokenCount(activity.sessionUsage.cache_miss_tokens)],
                    [t("execution.metric.write"), formatCompactTokenCount(activity.sessionUsage.cache_write_tokens)],
                  ])}
                </p>
                {contextEstimate ? (
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    {t("session.currentLiveContext", { tokens: formatCompactTokenCount(contextEstimate) })}
                  </p>
                ) : null}
                <p className="text-sm text-muted-foreground leading-relaxed">{t("execution.totalCost", { value: formatMoney(activity.sessionUsage.total_cost) })}</p>
              </div>
              <div className={sideSectionClass}>
                <p className="roc-section-label">{t("execution.section.activeStage")}</p>
                {activity.activeStageSummary ? (
                  <>
                    <div className="roc-rail-meta-list items-center">
                      <strong>{activity.activeStageSummary.stage_name}</strong>
                      <span className="roc-badge px-3 py-1 text-xs">{activity.activeStageSummary.status}</span>
                      {activity.sessionRuntime?.active_stage_count ? (
                        <span className="text-xs text-muted-foreground">{t("execution.metric.active")} {activity.sessionRuntime.active_stage_count}</span>
                      ) : null}
                    </div>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {summarizeMetrics([
                        typeof activity.activeStageSummary.prompt_tokens === "number"
                          ? [t("execution.metric.in"), formatCompactTokenCount(activity.activeStageSummary.prompt_tokens)]
                          : ["", null],
                        typeof activity.activeStageSummary.completion_tokens === "number"
                          ? [t("execution.metric.out"), formatCompactTokenCount(activity.activeStageSummary.completion_tokens)]
                          : ["", null],
                        typeof activity.activeStageSummary.reasoning_tokens === "number"
                          ? [t("execution.metric.reasoning"), formatCompactTokenCount(activity.activeStageSummary.reasoning_tokens)]
                          : ["", null],
                        typeof activity.activeStageSummary.skill_tree_budget === "number"
                          ? [t("execution.metric.budget"), activity.activeStageSummary.skill_tree_budget]
                          : ["", null],
                      ])}
                    </p>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {activity.activeStageSummary.waiting_on
                        ? t("execution.waitingFor", {
                            value:
                              humanizeStageWaitTarget(activity.activeStageSummary.waiting_on) ??
                              activity.activeStageSummary.waiting_on,
                          })
                        : humanizeStageEvent(activity.activeStageSummary.last_event) || t("execution.noActiveWaitSignal")}
                    </p>
                    {activity.activeStageSummary.activity ? (
                      <p className="text-sm text-muted-foreground leading-relaxed">
                        {t("execution.activityPrefix", { value: activity.activeStageSummary.activity.replace(/\n+/g, " · ") })}
                      </p>
                    ) : null}
                    {activity.activeStageSummary.skill_tree_truncated ? (
                      <p className="text-sm text-(--ds-warn) leading-relaxed">
                        {t("execution.skillTreeTruncated")}{activity.activeStageSummary.skill_tree_truncation_strategy
                          ? t("execution.skillTreeTruncatedVia", { strategy: activity.activeStageSummary.skill_tree_truncation_strategy })
                          : ""}
                      </p>
                    ) : null}
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground leading-relaxed">{t("execution.noActiveStageSummary")}</p>
                )}
              </div>
            </div>
          ) : null}
          {currentLiveExecutions.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.currentToolsSkills")}</p>
              <div className="grid gap-2">
                {currentLiveExecutions.map((entry) => renderLiveExecutionCard(entry, entry.id))}
              </div>
            </div>
          ) : null}
          {recentLiveExecutionOutcomes.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.recentToolOutcomes")}</p>
              <div className="grid gap-2">
                {recentLiveExecutionOutcomes.map((entry) =>
                  renderLiveExecutionCard(entry, `recent-${entry.id}`),
                )}
              </div>
            </div>
          ) : null}
          {recentTerminalStages.length ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.recentStageOutcomes")}</p>
              <div className="grid gap-2">
                {recentTerminalStages.map((stage) => {
                  const meta = stageSummaryMeta(stage, t);
                  return (
                    <div key={`terminal-${stage.stage_id}`} className="roc-rail-item grid gap-1 bg-card/45">
                      <div className="roc-rail-meta-list items-center">
                        <strong>{stage.stage_name}</strong>
                        <span className={cn("roc-badge px-3 py-1 text-xs", stageStatusTone(stage.status))}>
                          {stage.status}
                        </span>
                        <button
                          type="button"
                          className="text-xs text-muted-foreground transition-colors hover:text-primary"
                          onClick={() => onNavigateStage(stage.stage_id)}
                        >
                          {t("composer.provenanceStage", { id: stage.stage_id })}
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
                  <p className="roc-section-label">{t("execution.section.toolRepair")}</p>
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    {t("execution.toolRepairDescription")}{" "}
                    {summarizeMetrics([
                      [
                        t("execution.metric.repaired"),
                        `${sessionToolRepairSummary.repaired_tool_call_count}/${sessionToolRepairSummary.total_tool_calls}`,
                      ],
                      [t("execution.metric.errors"), sessionToolRepairSummary.error_tool_call_count],
                      [t("execution.metric.events"), sessionToolRepairSummary.repair_event_count],
                    ])}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    {t("execution.repairKindsLabel")} {formatRepairKindSummary(sessionToolRepairSummary.event_kinds, t)}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    {t("execution.repairToolsLabel")} {formatRepairToolSummary(sessionToolRepairSummary.tools, t)}
                  </p>
                </div>
              ) : null}
              {modelToolRepairSummary ? (
                <div className={sideSectionClass}>
                  <p className="roc-section-label">{t("execution.section.modelRepairBaseline")}</p>
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    {t("execution.modelRepairBaselineFor", {
                      provider: modelToolRepairSummary.provider_id,
                      model: modelToolRepairSummary.model_id,
                    })}{" "}
                    {summarizeMetrics([
                      [t("execution.metric.sessions"), modelToolRepairSummary.session_count],
                      [t("execution.metric.repairedSessions"), modelToolRepairSummary.repaired_session_count],
                    ])}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    {t("execution.modelRepairCalls", {
                      repaired: modelToolRepairSummary.repaired_tool_call_count,
                      total: modelToolRepairSummary.total_tool_calls,
                      errors: modelToolRepairSummary.error_tool_call_count,
                      events: modelToolRepairSummary.repair_event_count,
                    })}
                  </p>
                  <p className="text-xs text-muted-foreground leading-relaxed">
                    {t("execution.repairKindsLabel")} {formatRepairKindSummary(modelToolRepairSummary.event_kinds, t)}
                  </p>
                </div>
              ) : null}
            </div>
          ) : null}
          {trajectoryQuality ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.trajectoryQuality")}</p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t("execution.trajectoryScore", {
                  score: trajectoryQuality.score,
                  band: formatTrajectoryBand(trajectoryQuality.band),
                  repaired: trajectoryQuality.repaired_tool_call_count,
                  total: trajectoryQuality.total_tool_calls,
                  errors: trajectoryQuality.error_tool_call_count,
                })}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t("execution.trajectorySanitizer", {
                  sanitizer: trajectoryQuality.sanitizer_event_count,
                  strictFail: trajectoryQuality.strict_would_fail_count,
                  provider: trajectoryQuality.provider_diagnostic_count,
                })}
              </p>
            </div>
          ) : null}
          {(activity.telemetry?.pending_permission_count ?? 0) > 0
            || (activity.telemetry?.granted_by_turn_count ?? 0) > 0
            || (activity.telemetry?.granted_by_session_count ?? 0) > 0
            || (activity.telemetry?.last_permission_miss_count ?? 0) > 0 ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.permissionAuthority")}</p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("execution.metric.turn"), activity.telemetry?.granted_by_turn_count ?? 0],
                  [t("execution.metric.session"), activity.telemetry?.granted_by_session_count ?? 0],
                  [t("execution.metric.pending"), activity.telemetry?.pending_permission_count ?? 0],
                  [t("execution.metric.misses"), activity.telemetry?.last_permission_miss_count ?? 0],
                ])}
              </p>
              {activity.telemetry?.last_permission_matcher_kind ? (
                <p className="text-xs text-muted-foreground">
                  {t("execution.lastGrant", { value: activity.telemetry.last_permission_matcher_kind })}
                </p>
              ) : null}
            </div>
          ) : null}
          {activity.telemetry?.runtime_protocol ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.runtimeProtocol")}</p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("execution.metric.ingress"), activity.telemetry.runtime_protocol.prompt_ingress],
                  [t("execution.metric.steering"), activity.telemetry.runtime_protocol.steering.pending_count],
                  [t("execution.metric.interrupt"), activity.telemetry.runtime_protocol.interrupt.phase],
                ])}
              </p>
              {activity.telemetry.runtime_protocol.permission.pending ? (
                <p className="text-xs text-muted-foreground">
                  {t("execution.permissionPendingId", { id: activity.telemetry.runtime_protocol.permission.pending_permission_id ?? "" })}
                  {activity.telemetry.runtime_protocol.permission.pending_tool
                    ? ` · ${activity.telemetry.runtime_protocol.permission.pending_tool}`
                    : ""}
                </p>
              ) : null}
              {activity.telemetry.runtime_protocol.steering.last_latency_ms != null ? (
                <p className="text-xs text-muted-foreground">
                  {t("execution.steeringLatency", { ms: activity.telemetry.runtime_protocol.steering.last_latency_ms })}
                </p>
              ) : null}
              {activity.telemetry.runtime_protocol.permission.last_pending_duration_ms != null ? (
                <p className="text-xs text-muted-foreground">
                  {t("execution.permissionPendingDuration", { ms: activity.telemetry.runtime_protocol.permission.last_pending_duration_ms })}
                </p>
              ) : null}
            </div>
          ) : null}
          {activity.telemetry?.event_bus_telemetry ? (
            <div className={sideSectionClass}>
              <p className="roc-section-label">{t("execution.section.eventBus")}</p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("execution.metric.sends"), activity.telemetry.event_bus_telemetry.send_count],
                  [t("execution.metric.noReceiver"), activity.telemetry.event_bus_telemetry.send_error_count],
                  [t("execution.metric.maxReceivers"), activity.telemetry.event_bus_telemetry.max_receivers],
                ])}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("execution.eventBusLast", {
                  send: activity.telemetry.event_bus_telemetry.last_send_at_ms || 0,
                  error: activity.telemetry.event_bus_telemetry.last_send_error_at_ms || 0,
                })}
              </p>
            </div>
          ) : null}
          {contextClosure ? (
            <div className={sideSectionClass}>
              <div className="roc-rail-section-header">
                <div className="roc-rail-section-copy">
                  <p className="roc-section-label">{t("execution.section.contextClosure")}</p>
                  <h4 className="roc-rail-section-title">{t("execution.readOnlyAcceptanceContract")}</h4>
                </div>
                <p className="roc-rail-section-note">{t("execution.authorityBackedSnapshot")}</p>
              </div>
              <div className="grid gap-3 xl:grid-cols-2">
                <ReadOnlyDiagnosticCard
                  title={t("execution.closureCard.prefix")}
                  statusLabel={contextClosurePrefixStatusLabel(contextClosure.prefix_stability)}
                  statusTone={
                    contextClosure.prefix_stability.prefix_change_detected ? "warn" : "good"
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    {t("execution.prefixBasis", {
                      messages: contextClosure.prefix_stability.api_view_messages,
                      trimmed: contextClosure.prefix_stability.trimmed_model_visible_messages,
                    })}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {contextClosure.prefix_stability.explanation || t("execution.noPrefixExplanation")}
                  </p>
                </ReadOnlyDiagnosticCard>

                <ReadOnlyDiagnosticCard
                  title={t("execution.closureCard.boundary")}
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
                    {t("execution.boundaryDetail", {
                      phase: contextClosure.compaction_boundary.phase || "--",
                      trigger: contextClosure.compaction_boundary.trigger || "--",
                      reason: contextClosure.compaction_boundary.reason || "--",
                    })}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("execution.boundaryRequest", {
                      request: typeof contextClosure.compaction_boundary.request_pressure_percent === "number"
                        ? `${contextClosure.compaction_boundary.request_pressure_percent}%`
                        : "--",
                      live: typeof contextClosure.compaction_boundary.live_pressure_percent === "number"
                        ? `${contextClosure.compaction_boundary.live_pressure_percent}%`
                        : "--",
                      attempted: contextClosure.compaction_boundary.compaction_attempted ? t("session.yes") : t("session.no"),
                      succeeded: contextClosure.compaction_boundary.compaction_succeeded ? t("session.yes") : t("session.no"),
                      blocking: contextClosure.compaction_boundary.blocking ? t("session.yes") : t("session.no"),
                    })}
                  </p>
                </ReadOnlyDiagnosticCard>

                {compactionContinuity ? (
                  <CompactionContinuityCard
                    continuity={compactionContinuity}
                    title={t("execution.closureCard.continuity")}
                    className="roc-rail-item bg-card/45 p-4"
                  />
                ) : null}

                <ReadOnlyDiagnosticCard
                  title={t("execution.closureCard.cache")}
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
                    {t("execution.cacheSource", {
                      source: contextClosureExplainabilitySourceLabel(
                        contextClosure.cache_explainability.source,
                      ),
                      severity: contextClosureSeverityLabel(
                        contextClosure.cache_explainability.severity,
                      ),
                    })}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {contextClosure.cache_explainability.explanation ||
                      t("execution.noCacheNote")}
                  </p>
                  {promptSurfaceEvidence?.changed_fields?.length ? (
                    <p className="text-xs text-muted-foreground">
                      {t("execution.evidencePromptSurface", { fields: promptSurfaceEvidence.changed_fields.join(", ") })}
                    </p>
                  ) : null}
                  {typeof promptSurfaceEvidence?.stable_prefix_change === "boolean" ? (
                    <p className="text-xs text-muted-foreground">
                      {t("execution.prefixState", {
                        state: promptSurfaceEvidence.stable_prefix_change
                          ? t("execution.prefixState.changed")
                          : t("execution.prefixState.held"),
                      })}
                    </p>
                  ) : null}
                  {promptSurfaceEvidence?.dynamic_overlay_reasons?.length ? (
                    <p className="text-xs text-muted-foreground">
                      {t("execution.overlayReasons", { reasons: promptSurfaceEvidence.dynamic_overlay_reasons.join(" · ") })}
                    </p>
                  ) : null}
                </ReadOnlyDiagnosticCard>

                <ReadOnlyDiagnosticCard
                  title={t("execution.closureCard.isolation")}
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
                    {t("execution.isolationUsage", {
                      attached: contextClosure.child_history_isolation.attached_subtree_session_count,
                      subtree: formatCompactTokenCount(
                        contextClosure.child_history_isolation.attached_subtree_cumulative_tokens,
                      ),
                      owner: typeof contextClosure.child_history_isolation.owner_live_context_tokens === "number"
                        ? formatCompactTokenCount(
                            contextClosure.child_history_isolation.owner_live_context_tokens,
                          )
                        : "--",
                    })}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("execution.isolationScope", {
                      ownerLocal: contextClosure.child_history_isolation.owner_local_live_prefix ? t("session.yes") : t("session.no"),
                      workflow: formatCompactTokenCount(
                        contextClosure.child_history_isolation.workflow_cumulative_tokens,
                      ),
                    })}
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
                  <p className="roc-section-label">{t("execution.section.memoryRuntime")}</p>
                  <h4 className="roc-rail-section-title">{t("execution.workspaceExplain", { mode: sessionMemory.workspace_mode })}</h4>
                </div>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("execution.metric.snapshot"), sessionMemory.frozen_snapshot_items],
                  [t("execution.metric.prefetch"), sessionMemory.last_prefetch_items],
                  [t("execution.metric.ruleHits"), sessionMemoryRecentRuleHits.length],
                  [
                    t("execution.metric.sessionWrites"),
                    sessionMemory.candidate_count + sessionMemory.validated_count + sessionMemory.rejected_count,
                  ],
                ])}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("execution.metric.warnings"), sessionMemory.warning_count],
                  [t("execution.metric.methodology"), sessionMemory.methodology_candidate_count],
                  [t("execution.metric.skillTargets"), sessionMemory.derived_skill_candidate_count],
                  [t("execution.metric.linkedSkills"), sessionMemory.linked_skill_count],
                ])}
                {sessionMemory.latest_consolidation_run
                  ? t("execution.consolidationSuffix", { id: sessionMemory.latest_consolidation_run.run_id })
                  : ""}
              </p>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>{t("session.workspaceKey", { value: sessionMemory.workspace_key })}</p>
                <p>{t("session.frozenSnapshotGenerated", { value: formatDateTime(sessionMemory.frozen_snapshot_generated_at ?? undefined) })}</p>
                <p>{t("session.lastPrefetchGenerated", { value: formatDateTime(sessionMemory.last_prefetch_generated_at ?? undefined) })}</p>
                <p>
                  {t("session.lastPrefetchQuery", { value: sessionMemory.last_prefetch_query?.trim() || t("session.noQueryCaptured") })}
                </p>
                <p>
                  {t("execution.sessionMemoryRecords", {
                    candidate: sessionMemory.candidate_count,
                    validated: sessionMemory.validated_count,
                    rejected: sessionMemory.rejected_count,
                  })}
                </p>
                <p>
                  {t("session.validationPressure", {
                    warnings: sessionMemory.warning_count,
                    methodology: sessionMemory.methodology_candidate_count,
                    skillTargets: sessionMemory.derived_skill_candidate_count,
                  })}
                </p>
                <p>
                  {t("execution.skillLinkage", {
                    linked: sessionMemory.linked_skill_count,
                    lessons: sessionMemory.skill_feedback_lesson_count,
                  })}
                </p>
                <p>
                  {t("session.retrieval", {
                    runs: sessionMemory.retrieval_run_count,
                    hits: sessionMemory.retrieval_hit_count,
                    used: sessionMemory.retrieval_use_count,
                  })}
                </p>
              </div>
              {recentSkillRecords.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">{t("execution.section.recentSkillLinkedMemory")}</p>
                  <div className="grid gap-1 text-sm text-muted-foreground">
                    {recentSkillRecords.slice(0, 4).map((item) => (
                      <p key={memoryRecordIdValue(item.id)}>
                        {item.linked_skill_name || item.derived_skill_name}: {item.title}
                      </p>
                    ))}
                  </div>
                </div>
              ) : null}
              {sessionMemory.latest_consolidation_run ? (
                <div className="grid gap-1 text-sm text-muted-foreground">
                  <p>
                    {t("execution.latestConsolidationFinished", { time: formatDateTime(sessionMemory.latest_consolidation_run.finished_at ?? sessionMemory.latest_consolidation_run.started_at) })}
                  </p>
                  <p>
                    {t("session.consolidationSummary", {
                      merged: sessionMemory.latest_consolidation_run.merged_count,
                      promoted: sessionMemory.latest_consolidation_run.promoted_count,
                      conflicts: sessionMemory.latest_consolidation_run.conflict_count,
                    })}
                  </p>
                </div>
              ) : (
                <p className="text-sm text-muted-foreground leading-relaxed">{t("execution.noConsolidationRun")}</p>
              )}
              {sessionMemoryRecentRuleHits.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">{t("execution.section.recentRuleHits")}</p>
                  <div className="grid gap-2 md:grid-cols-2">
                    {sessionMemoryRecentRuleHits.map((hit) => (
                      <div key={hit.id} className={sideItemCardClass}>
                        <div className="flex flex-wrap items-center gap-2">
                          <strong>{hit.hit_kind}</strong>
                          {hit.memory_id ? <span className="text-xs text-muted-foreground">{memoryRecordIdValue(hit.memory_id)}</span> : null}
                        </div>
                        <p className="text-xs text-muted-foreground">
                          {hit.detail || t("session.noDetailAttached")}
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
          <div className="roc-section-label">{t("execution.section.scheduler")}</div>
          <p className="text-sm font-semibold tracking-tight text-foreground">{t("execution.noSchedulerTopology")}</p>
        </div>
      )}

      {activity.stageSummaries.length ? (
        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">{t("execution.section.stageSummaries")}</p>
              <h4 className="roc-rail-section-title">{t("execution.stagesCount", { count: activity.stageSummaries.length })}</h4>
            </div>
            <p className="roc-rail-section-note">
              {t("execution.authorityBackedSnapshot")}
            </p>
          </div>
          <div className="grid gap-3 xl:grid-cols-2">
            {activity.stageSummaries.map((stage) => {
              const meta = stageSummaryMeta(stage, t);
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
                        {t("execution.open")}
                      </button>
                      <button
                        className={compactActionButtonClass}
                        type="button"
                        onClick={() => activity.patchActivityFilters({ stageId: stage.stage_id })}
                      >
                        {t("execution.filterEvents")}
                      </button>
                      {stage.status === "running" ? (
                        <button
                          className={compactActionButtonClass}
                          type="button"
                          data-testid={`stage-abort-${stage.stage_id}`}
                          disabled={activity.stageAbortingId === stage.stage_id}
                          onClick={() => void activity.abortSchedulerStage(stage.stage_id)}
                        >
                          {activity.stageAbortingId === stage.stage_id ? t("execution.aborting") : t("execution.abort")}
                        </button>
                      ) : null}
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
                    {typeof stage.prompt_tokens === "number" ? <span>{t("execution.metric.in")} {formatCompactTokenCount(stage.prompt_tokens)}</span> : null}
                    {typeof stage.completion_tokens === "number" ? <span>{t("execution.metric.out")} {formatCompactTokenCount(stage.completion_tokens)}</span> : null}
                    {typeof stage.reasoning_tokens === "number" ? <span>{t("execution.metric.reasoning")} {formatCompactTokenCount(stage.reasoning_tokens)}</span> : null}
                    {typeof stage.cache_read_tokens === "number" ? <span>{t("execution.metric.cacheRead")} {formatCompactTokenCount(stage.cache_read_tokens)}</span> : null}
                    {typeof stage.cache_miss_tokens === "number" ? <span>{t("execution.metric.cacheMiss")} {formatCompactTokenCount(stage.cache_miss_tokens)}</span> : null}
                    {typeof stage.cache_write_tokens === "number" ? <span>{t("execution.metric.cacheWrite")} {formatCompactTokenCount(stage.cache_write_tokens)}</span> : null}
                  </div>
                  {stage.last_event || stage.focus || stage.activity ? (
                    <div className="grid gap-1 text-xs text-muted-foreground">
                      {stage.last_event ? <p>{t("execution.lastEvent", { value: humanizeStageEvent(stage.last_event) || stage.last_event })}</p> : null}
                      {stage.focus ? <p>{t("execution.focus", { value: stage.focus })}</p> : null}
                      {stage.activity ? <p>{t("execution.activityPrefix", { value: stage.activity.replace(/\n+/g, " · ") })}</p> : null}
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
          <span className={formLabelClass}>{t("execution.stageLabel")}</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.stageId}
            onChange={(event) => activity.patchActivityFilters({ stageId: event.target.value })}
          >
            <option value="">{t("execution.filter.allStages")}</option>
            {activity.stageOptions.map((stageId) => (
              <option key={stageId} value={stageId}>
                {stageId}
              </option>
            ))}
          </select>
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("execution.executionLabel")}</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.executionId}
            onChange={(event) => activity.patchActivityFilters({ executionId: event.target.value })}
          >
            <option value="">{t("execution.filter.allExecutions")}</option>
            {activity.executionNodes.map((node) => (
              <option key={node.id} value={node.id}>
                {node.label || node.id}
              </option>
            ))}
          </select>
        </label>
        <label className={formFieldClass}>
          <span className={formLabelClass}>{t("execution.filter.eventType")}</span>
          <select
            className={formSelectClass}
            value={activity.activityFilters.eventType}
            onChange={(event) => activity.patchActivityFilters({ eventType: event.target.value })}
          >
            <option value="">{t("execution.filter.allEvents")}</option>
            {activity.knownEventTypes.map((eventType) => (
              <option key={eventType} value={eventType}>
                {eventType}
              </option>
            ))}
          </select>
        </label>
        <button className={actionButtonClass} type="button" onClick={activity.clearActivityFilters}>
          {t("app.clear")}
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
            <div className="roc-section-label">{t("execution.executionLabel")}</div>
            <p className="text-sm font-semibold tracking-tight text-foreground">{t("execution.noExecutionTopology")}</p>
          </div>
        )}
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">{t("execution.executionLabel")}</p>
              <h4 className="roc-rail-section-title">{activity.selectedExecution?.label || t("execution.selectExecutionNode")}</h4>
            </div>
            <div className="flex flex-wrap gap-2">
              {executionJump ? (
                <button
                  className={actionButtonClass}
                  type="button"
                  onClick={() => onJumpToConversation(executionJump)}
                >
                  {t("execution.jumpToMessage")}
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
                    ? t("execution.cancelling")
                    : t("execution.cancel")}
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
                        <dt className="roc-structured-key">{t("execution.field.id")}</dt>
                        <dd className="text-sm text-foreground">{selected.id}</dd>
                      </div>
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">{t("execution.field.status")}</dt>
                        <dd className="text-sm text-foreground">{selected.status}</dd>
                      </div>
                      <div className="roc-structured-row">
                        <dt className="roc-structured-key">{t("execution.stageLabel")}</dt>
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
                        <dt className="roc-structured-key">{t("session.updatedLabel")}</dt>
                        <dd className="text-sm text-foreground">{formatTs(selected.updated_at)}</dd>
                      </div>
                    </dl>
                    <div className="flex flex-wrap gap-2">
                      <button
                        className={actionButtonClass}
                        type="button"
                        onClick={() => activity.patchActivityFilters({ executionId: selected.id || "" })}
                      >
                        {t("execution.filterEventsToExecution")}
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
                          {t("execution.filterEventsToStage")}
                        </button>
                      ) : null}
                    </div>
                    <StructuredDataView
                      value={selected.metadata}
                      emptyLabel={t("execution.noExecutionMetadata")}
                    />
                  </>
                );
              })()}
            </>
          ) : (
            <div className="roc-rail-empty">
              <div className="roc-section-label">{t("execution.executionLabel")}</div>
              <p className="text-sm font-semibold tracking-tight text-foreground">{t("execution.chooseNodePrompt")}</p>
            </div>
          )}
        </div>

        <div className={sideSectionClass}>
          <div className="roc-rail-section-header">
            <div className="roc-rail-section-copy">
              <p className="roc-section-label">{t("execution.activityLabel")}</p>
              <h4 className="roc-rail-section-title">{activity.selectedEvent?.event_type || t("execution.recentEvents")}</h4>
            </div>
            {selectedEventJump ? (
              <button
                className={actionButtonClass}
                type="button"
                onClick={() => onJumpToConversation(selectedEventJump)}
              >
                {t("execution.jumpToProvenance")}
              </button>
            ) : null}
          </div>
          {activity.selectedEvent ? (
            <dl className="roc-structured-dl">
              {activity.selectedEvent.stage_id ? (
                <div className="roc-structured-row">
                  <dt className="roc-structured-key">{t("execution.stageLabel")}</dt>
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
                  <dt className="roc-structured-key">{t("execution.attachedSession")}</dt>
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
                  <dt className="roc-structured-key">{t("execution.toolCall")}</dt>
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
                      {event.stage_id ? <span>{t("composer.provenanceStage", { id: event.stage_id })}</span> : null}
                      {event.execution_id ? <span>{t("execution.execLabel", { id: event.execution_id })}</span> : null}
                    </div>
                  ) : null}
                </button>
              ))
            ) : (
              <div className="roc-rail-empty">
                <div className="roc-section-label">{t("execution.activityLabel")}</div>
                <p className="text-sm font-semibold tracking-tight text-foreground">{t("execution.noActivityEvents")}</p>
              </div>
            )}
          </div>
          <div className="roc-rail-section grid gap-3 px-3 py-2">
            <p className="text-xs text-muted-foreground">
              {eventWindowLabel(
                activity.activityPage,
                activity.activityEvents.length,
                activity.activityPageSize,
                t,
              )}{" "}
              {t("execution.limitSuffix", { value: activity.activityPageSize })}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasPreviousPage}
                onClick={activity.firstActivityPage}
              >
                {t("execution.pager.first")}
              </button>
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasPreviousPage}
                onClick={activity.previousActivityPage}
              >
                {t("execution.pager.prev")}
              </button>
              <label className="flex items-center gap-2">
                <span className={formLabelClass}>{t("execution.pager.page")}</span>
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
                {t("execution.pager.go")}
              </button>
              <button
                className={compactActionButtonClass}
                type="button"
                disabled={!activity.activityHasNextPage}
                onClick={activity.nextActivityPage}
              >
                {t("execution.pager.next")}
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
                    {t("execution.filterToExecution")}
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
                    {t("execution.filterToStage")}
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
                    {t("execution.openAttachedSession")}
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
                    {t("execution.openToolCall")}
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
                emptyLabel={t("execution.noStructuredPayload")}
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
