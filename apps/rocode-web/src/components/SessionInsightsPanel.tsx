import { memo, useMemo, useState } from "react";
import type { useExecutionActivity } from "../hooks/useExecutionActivity";
import {
  type MemoryDetailResponseRecord,
  memoryRecordIdValue,
} from "../lib/memory";
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "../lib/contextPressure";
import { multimodalCombinedWarnings, multimodalDisplayLabel } from "../lib/multimodal";
import { CompactionContinuityCard } from "./CompactionContinuityCard";

type ExecutionActivityState = ReturnType<typeof useExecutionActivity>;

interface SessionInsightsPanelProps {
  activity: ExecutionActivityState;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
}

function skillBadgeLabel(
  item:
    | { linked_skill_name?: string | null; derived_skill_name?: string | null; title: string }
    | null
    | undefined,
) {
  if (!item) return null;
  return item.linked_skill_name || item.derived_skill_name || null;
}

function formatDateTime(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts).toLocaleString();
}

function formatMoney(value?: number | null) {
  if (typeof value !== "number" || Number.isNaN(value)) return "--";
  return `$${value.toFixed(4)}`;
}

function formatCompactTokenCount(value: number) {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  return String(Math.round(value));
}

function schedulerTraceLabel(kind?: string | null) {
  switch (kind) {
    case "requested_profile":
      return "Requested profile";
    case "command_workflow_override":
      return "Command/workflow";
    case "session_pinned_profile":
      return "Session pinned";
    case "legacy_session_pinned_profile":
      return "Legacy session";
    case "config_default_profile":
      return "Config default";
    case "auto_route":
      return "Auto route";
    case "soft_fallback":
      return "Soft fallback";
    default:
      return kind || "Trace";
  }
}

function formatTrajectoryBand(band?: string | null) {
  if (!band) return "--";
  return band.replaceAll("_", " ");
}

function totalUsageTokens(usage?: {
  input_tokens?: number;
  output_tokens?: number;
  reasoning_tokens?: number;
  cache_read_tokens?: number;
  cache_miss_tokens?: number;
  cache_write_tokens?: number;
} | null) {
  if (!usage) return 0;
  return (
    (usage.input_tokens ?? 0) +
    (usage.output_tokens ?? 0) +
    (usage.reasoning_tokens ?? 0) +
    (usage.cache_read_tokens ?? 0) +
    (usage.cache_miss_tokens ?? 0) +
    (usage.cache_write_tokens ?? 0)
  );
}

// P2-3: side panel uses selector-local reads via memo().
export const SessionInsightsPanel = memo(function SessionInsightsPanel({
  activity,
  apiJson,
}: SessionInsightsPanelProps) {
  const insights = activity.sessionInsights;
  const telemetry = insights?.telemetry ?? null;
  const runtimeTelemetry = activity.telemetry ?? null;
  const effectivePolicy = insights?.effective_policy ?? null;
  const schedulerPolicy = effectivePolicy?.scheduler ?? null;
  const telemetryUsage = telemetry?.usage ?? null;
  const telemetryStages = telemetry?.stage_summaries ?? [];
  const trajectoryQuality =
    runtimeTelemetry?.tool_trajectory_quality ?? telemetry?.tool_trajectory_quality ?? null;
  const toolResultGovernance =
    runtimeTelemetry?.tool_result_governance ?? telemetry?.tool_result_governance ?? null;
  const memory = insights?.memory ?? null;
  const memorySummary = memory?.summary ?? null;
  const memoryAllowedScopes = memorySummary?.allowed_scopes ?? [];
  const memoryRecentRuleHits = memorySummary?.recent_rule_hits ?? [];
  const memoryRecentSessionRecords = memory?.recent_session_records ?? [];
  const memoryFrozenItems = memory?.frozen_snapshot?.items ?? [];
  const memoryPrefetchItems = memory?.last_prefetch_packet?.items ?? [];
  const multimodal = insights?.multimodal ?? null;
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);
  const [selectedMemoryDetail, setSelectedMemoryDetail] = useState<MemoryDetailResponseRecord | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  const loadMemoryDetail = async (recordId: string) => {
    setSelectedMemoryId(recordId);
    setDetailLoading(true);
    setDetailError(null);
    try {
      const detail = await apiJson<MemoryDetailResponseRecord>(`/memory/${encodeURIComponent(recordId)}`);
      setSelectedMemoryDetail(detail);
    } catch (error) {
      setSelectedMemoryDetail(null);
      setDetailError(error instanceof Error ? error.message : "Unknown error");
    } finally {
      setDetailLoading(false);
    }
  };

  const insightMemoryIds = useMemo(() => {
    const ids = new Set<string>();
    memoryRecentRuleHits.forEach((hit) => {
      const memoryId = memoryRecordIdValue(hit.memory_id);
      if (memoryId) ids.add(memoryId);
    });
    memoryFrozenItems.forEach((item) =>
      ids.add(memoryRecordIdValue(item.card.id)),
    );
    memoryPrefetchItems.forEach((item) =>
      ids.add(memoryRecordIdValue(item.card.id)),
    );
    memoryRecentSessionRecords.forEach((item) =>
      ids.add(memoryRecordIdValue(item.id)),
    );
    return ids;
  }, [memoryFrozenItems, memoryPrefetchItems, memoryRecentRuleHits, memoryRecentSessionRecords]);
  const skillLinkedRecords = useMemo(
    () =>
      memoryRecentSessionRecords.filter(
        (item) => item.linked_skill_name || item.derived_skill_name,
      ),
    [memoryRecentSessionRecords],
  );
  const currentContextTokens = useMemo(() => {
    const activeStage = activity.activeStageSummary;
    const activeStageContext = activeStage && isLiveStageStatus(activeStage.status)
      ? activeStage.context_tokens ?? activeStage.estimated_context_tokens
      : null;
    return currentContextTokensFromSources(activity.sessionUsage?.context_tokens, activeStageContext);
  }, [activity.activeStageSummary, activity.sessionUsage?.context_tokens]);
  const panelActionClass = "roc-action roc-action-pill";
  const compactActionClass = "roc-action roc-action-compact justify-self-start";
  const detailTileClass = "roc-rail-item grid gap-1 bg-card/45";

  return (
    <div className="roc-panel roc-rail-panel min-h-0 p-5">
        <div className="roc-rail-header">
          <div className="roc-rail-headline">
            <p className="roc-section-label">Runtime Explain</p>
            <h3 className="roc-rail-title">Session Insights</h3>
            <p className="roc-rail-description">Persisted telemetry, multimodal runtime, and memory traces for the current session.</p>
        </div>
        <button
          className={panelActionClass}
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

      {!insights ? (
        <div className="roc-rail-empty">
          <div className="roc-section-label">Insights</div>
          <p className="text-sm font-semibold tracking-tight text-foreground">No session insights yet.</p>
          <p className="text-sm leading-6 text-muted-foreground">
            Run a prompt or press Refresh after activity is recorded. This tab surfaces memory hits,
            multimodal attachments, and live context telemetry rather than duplicating the file preview.
          </p>
        </div>
      ) : (
        <>
          <dl className="roc-structured-dl">
            <div className="roc-structured-row">
              <dt className="roc-structured-key">Session</dt>
              <dd className="text-sm text-foreground">{insights.id}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">Title</dt>
              <dd className="text-sm text-foreground">{insights.title}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">Directory</dt>
              <dd className="text-sm text-foreground break-all">{insights.directory}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">Updated</dt>
              <dd className="text-sm text-foreground">{formatDateTime(insights.updated)}</dd>
            </div>
          </dl>

          {telemetry ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Runtime Telemetry</p>
                <h4 className="roc-rail-section-title">Current Run Snapshot</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">version {telemetry.version}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">status {telemetry.last_run_status}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">stages {telemetryStages.length}</span>
                {trajectoryQuality ? (
                  <span className="roc-badge px-3 py-1.5 text-xs">
                    trajectory {trajectoryQuality.score} {formatTrajectoryBand(trajectoryQuality.band)}
                  </span>
                ) : null}
              </div>
              {currentContextTokens ? (
                <p className="text-sm text-muted-foreground leading-relaxed">
                  Current live context {formatCompactTokenCount(currentContextTokens)}
                </p>
              ) : null}
              <p className="text-sm text-muted-foreground leading-relaxed">
                Session cumulative {formatCompactTokenCount(totalUsageTokens(telemetryUsage))} total · input {formatCompactTokenCount(telemetryUsage?.input_tokens ?? 0)} · output {formatCompactTokenCount(telemetryUsage?.output_tokens ?? 0)} · reasoning {formatCompactTokenCount(telemetryUsage?.reasoning_tokens ?? 0)}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                Cache read {formatCompactTokenCount(telemetryUsage?.cache_read_tokens ?? 0)} · cache miss {formatCompactTokenCount(telemetryUsage?.cache_miss_tokens ?? 0)} · cache write {formatCompactTokenCount(telemetryUsage?.cache_write_tokens ?? 0)} · cost {formatMoney(telemetryUsage?.total_cost)}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                Updated {formatDateTime(telemetry.updated_at)}
              </p>
              {trajectoryQuality ? (
                <p className="text-sm text-muted-foreground leading-relaxed">
                  Trajectory quality {trajectoryQuality.score} · {formatTrajectoryBand(trajectoryQuality.band)} · repaired {trajectoryQuality.repaired_tool_call_count}/{trajectoryQuality.total_tool_calls} · errors {trajectoryQuality.error_tool_call_count}
                </p>
              ) : null}
              {runtimeTelemetry || telemetry ? (
                <div className={detailTileClass}>
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="roc-section-label">Tool Result Governance</p>
                    <span className="roc-badge px-2.5 py-1 text-xs">
                      single {toolResultGovernance?.single_result_governed_count ?? 0}
                    </span>
                    <span className="roc-badge px-2.5 py-1 text-xs">
                      batch {toolResultGovernance?.batch_governed_count ?? 0}
                    </span>
                    <span className="roc-badge px-2.5 py-1 text-xs">
                      transcript fallback {toolResultGovernance?.transcript_fallback_count ?? 0}
                    </span>
                    <span className="roc-badge px-2.5 py-1 text-xs">
                      artifact {toolResultGovernance?.artifact_fallback_count ?? 0}
                    </span>
                  </div>
                  {(toolResultGovernance?.total_original_chars ?? 0) > 0 ? (
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      Chars: {(toolResultGovernance?.total_original_chars ?? 0).toLocaleString()} original → {(toolResultGovernance?.total_displayed_chars ?? 0).toLocaleString()} displayed. Full results are artifact-backed — fetched on demand, not held in UI store.
                    </p>
                  ) : (
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      Shows how many finalized tool results were governed into preview/artifact form before entering transcript and replay surfaces.
                    </p>
                  )}
                </div>
              ) : null}
              {telemetry.compaction_continuity ? (
                <div className="grid gap-2 md:grid-cols-2">
                  <CompactionContinuityCard
                    continuity={telemetry.compaction_continuity}
                    className={detailTileClass}
                  />
                </div>
              ) : null}
            </div>
          ) : null}

          {effectivePolicy ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Effective Policy</p>
                <h4 className="roc-rail-section-title">Scheduler Selection</h4>
              </div>
              {schedulerPolicy ? (
                <>
                  <div className="roc-rail-meta-list">
                    <span className="roc-badge px-3 py-1.5 text-xs">source {schedulerPolicy.source}</span>
                    <span className="roc-badge px-3 py-1.5 text-xs">applied {schedulerPolicy.applied ? "yes" : "no"}</span>
                    <span className="roc-badge px-3 py-1.5 text-xs">requested {schedulerPolicy.requested_profile || "--"}</span>
                    <span className="roc-badge px-3 py-1.5 text-xs">effective {schedulerPolicy.effective_profile || "--"}</span>
                  </div>
                  <div className="grid gap-1 text-sm text-muted-foreground">
                    <p>Mode kind: {schedulerPolicy.mode_kind || "--"}</p>
                    <p>Root agent: {schedulerPolicy.root_agent || "--"}</p>
                    <p>Resolved agent: {schedulerPolicy.resolved_agent || "--"}</p>
                  </div>
                  {(schedulerPolicy.selection_trace ?? []).length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">Selection Trace</p>
                      {(schedulerPolicy.selection_trace ?? []).map((step, index) => (
                        <div
                          key={`scheduler-trace:${index}:${step.kind}:${step.profile ?? "--"}`}
                          className={detailTileClass}
                        >
                          <div className="flex flex-wrap items-center gap-2">
                            <strong>{schedulerTraceLabel(step.kind)}</strong>
                            {step.profile ? (
                              <span className="roc-badge px-2.5 py-1 text-xs">{step.profile}</span>
                            ) : null}
                            <span className="text-xs text-muted-foreground">
                              applied {step.applied ? "yes" : "no"}
                            </span>
                          </div>
                          {step.detail ? (
                            <p className="text-xs text-muted-foreground">{step.detail}</p>
                          ) : null}
                        </div>
                      ))}
                    </div>
                  ) : null}
                  {schedulerPolicy.warning ? (
                    <div className="roc-rail-item bg-card/45 text-sm text-muted-foreground">
                      {schedulerPolicy.warning}
                    </div>
                  ) : null}
                </>
              ) : (
                <p className="text-sm text-muted-foreground">No scheduler policy is currently active.</p>
              )}
              {(effectivePolicy.warnings ?? []).length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Policy Warnings</p>
                  {(effectivePolicy.warnings ?? []).map((warning, index) => (
                    <div key={`effective-policy-warning:${index}`} className="roc-rail-item bg-card/45 text-sm text-muted-foreground">
                      {warning}
                    </div>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}

          {multimodal ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Multimodal Explain</p>
                <h4 className="roc-rail-section-title">{multimodalDisplayLabel(multimodal) || "Attachment-backed input"}</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">message {multimodal.user_message_id}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">attachments {multimodal.attachment_count}</span>
                {(multimodal.kinds ?? []).map((kind) => (
                  <span key={`kind:${kind}`} className="roc-badge px-3 py-1.5 text-xs">
                    {kind}
                  </span>
                ))}
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>Resolved model: {multimodal.resolved_model || "--"}</p>
                <p>Badges: {(multimodal.badges ?? []).join(", ") || "--"}</p>
                <p>Hard block: {multimodal.hard_block ? "yes" : "no"}</p>
                <p>
                  Unsupported parts:{" "}
                  {(multimodal.unsupported_parts ?? []).join(", ") || "none"}
                </p>
                <p>
                  Recommended downgrade:{" "}
                  {multimodal.recommended_downgrade || "none"}
                </p>
                <p>
                  Transport replaced parts:{" "}
                  {(multimodal.transport_replaced_parts ?? []).join(", ") || "none"}
                </p>
              </div>
              {(multimodal.attachments ?? []).length ? (
                <div className="grid gap-2 md:grid-cols-2">
                  {(multimodal.attachments ?? []).map((attachment) => (
                    <div
                      key={`multimodal:${attachment.filename}:${attachment.mime}`}
                      className={detailTileClass}
                    >
                      <strong>{attachment.filename}</strong>
                      <p className="text-xs text-muted-foreground">{attachment.mime}</p>
                    </div>
                  ))}
                </div>
              ) : null}
              {multimodalCombinedWarnings(multimodal).length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Warnings</p>
                  {multimodalCombinedWarnings(multimodal).map((warning, index) => (
                    <div key={`multimodal-warning:${index}`} className="roc-rail-item bg-card/45 text-sm text-muted-foreground">
                      {warning}
                    </div>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}

          {insights.memory ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Memory Explain</p>
                <h4 className="roc-rail-section-title">{insights.memory.summary.workspace_mode} workspace</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">snapshot {insights.memory.summary.frozen_snapshot_items}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">prefetch {insights.memory.summary.last_prefetch_items}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">rule hits {memoryRecentRuleHits.length}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">warnings {insights.memory.summary.warning_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">methodology {insights.memory.summary.methodology_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">skill targets {insights.memory.summary.derived_skill_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">linked skills {insights.memory.summary.linked_skill_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">feedback lessons {insights.memory.summary.skill_feedback_lesson_count}</span>
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>Workspace key: {insights.memory.summary.workspace_key}</p>
                <p>Allowed scopes: {memoryAllowedScopes.join(", ") || "--"}</p>
                <p>Frozen snapshot generated: {formatDateTime(insights.memory.summary.frozen_snapshot_generated_at)}</p>
                <p>Last prefetch generated: {formatDateTime(insights.memory.summary.last_prefetch_generated_at)}</p>
                <p>Last prefetch query: {insights.memory.summary.last_prefetch_query?.trim() || "No query captured"}</p>
                <p>
                  Session records: candidate {insights.memory.summary.candidate_count} · validated {insights.memory.summary.validated_count} · rejected {insights.memory.summary.rejected_count}
                </p>
                <p>
                  Validation pressure: warnings {insights.memory.summary.warning_count} · methodology {insights.memory.summary.methodology_candidate_count} · skill targets {insights.memory.summary.derived_skill_candidate_count}
                </p>
                <p>
                  Retrieval: runs {insights.memory.summary.retrieval_run_count} · hits {insights.memory.summary.retrieval_hit_count} · used {insights.memory.summary.retrieval_use_count}
                </p>
              </div>
              {skillLinkedRecords.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Skill-Linked Recent Records</p>
                  <div className="grid gap-2 md:grid-cols-2">
                    {skillLinkedRecords.map((item) => (
                      <div
                        key={`skill:${memoryRecordIdValue(item.id)}`}
                        className={detailTileClass}
                      >
                        <div className="flex flex-wrap items-center gap-2">
                          <strong>{item.title}</strong>
                          {skillBadgeLabel(item) ? (
                            <span className="roc-badge px-2.5 py-1 text-xs">{skillBadgeLabel(item)}</span>
                          ) : null}
                        </div>
                        <p className="text-xs text-muted-foreground">{item.summary}</p>
                        <button
                          className={compactActionClass}
                          type="button"
                          onClick={() => void loadMemoryDetail(memoryRecordIdValue(item.id))}
                        >
                          Inspect Memory
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {insights.memory.summary.latest_consolidation_run ? (
                <div className="grid gap-1 text-sm text-muted-foreground">
                  <p>Latest consolidation: {insights.memory.summary.latest_consolidation_run.run_id}</p>
                  <p>
                    Merged {insights.memory.summary.latest_consolidation_run.merged_count} · promoted {insights.memory.summary.latest_consolidation_run.promoted_count} · conflicts {insights.memory.summary.latest_consolidation_run.conflict_count}
                  </p>
                </div>
              ) : null}
              {memoryRecentRuleHits.length ? (
                <div className="grid gap-2 md:grid-cols-2">
                  {memoryRecentRuleHits.map((hit) => (
                    <div key={hit.id} className={detailTileClass}>
                      <div className="flex flex-wrap items-center gap-2">
                        <strong>{hit.hit_kind}</strong>
                        {hit.memory_id ? (
                          <span className="roc-badge px-2.5 py-1 text-xs">
                            {memoryRecordIdValue(hit.memory_id)}
                          </span>
                        ) : null}
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {hit.detail || "No detail attached"}
                      </p>
                      {hit.memory_id ? (
                        <button
                          className={compactActionClass}
                          type="button"
                          onClick={() => void loadMemoryDetail(memoryRecordIdValue(hit.memory_id))}
                        >
                          Inspect Memory
                        </button>
                      ) : null}
                      <p className="text-xs text-muted-foreground">
                        {formatDateTime(hit.created_at)}
                      </p>
                    </div>
                  ))}
                </div>
              ) : null}
              {insights.memory.frozen_snapshot ? (
                <div className="grid gap-2 text-sm text-muted-foreground">
                  <p>Frozen snapshot note: {insights.memory.frozen_snapshot.note || "No note"}</p>
                  <p>
                    Frozen snapshot scopes: {(insights.memory.frozen_snapshot.scopes ?? []).join(", ") || "--"}
                  </p>
                  {memoryFrozenItems.length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">Frozen Items</p>
                      {memoryFrozenItems.map((item) => (
                        <div
                          key={`frozen:${memoryRecordIdValue(item.card.id)}`}
                          className={detailTileClass}
                        >
                          <div className="flex items-start justify-between gap-3">
                            <div>
                              <strong>{item.card.title}</strong>
                              <p className="text-xs text-muted-foreground">
                                {memoryRecordIdValue(item.card.id)}
                              </p>
                            </div>
                            <button
                              className="roc-action roc-action-compact"
                              type="button"
                              onClick={() => void loadMemoryDetail(memoryRecordIdValue(item.card.id))}
                            >
                              Inspect
                            </button>
                          </div>
                          <p className="text-xs text-muted-foreground">{item.why_recalled}</p>
                          <p className="text-xs text-muted-foreground">{item.card.summary}</p>
                        </div>
                      ))}
                    </div>
                  ) : null}
                </div>
              ) : null}
              {insights.memory.last_prefetch_packet ? (
                <div className="grid gap-2 text-sm text-muted-foreground">
                  <p>Prefetch note: {insights.memory.last_prefetch_packet.note || "No note"}</p>
                  <p>
                    Prefetch scopes: {(insights.memory.last_prefetch_packet.scopes ?? []).join(", ") || "--"}
                  </p>
                  <p>Prefetch recalled items: {memoryPrefetchItems.length}</p>
                  {memoryPrefetchItems.length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">Prefetch Items</p>
                      {memoryPrefetchItems.map((item) => (
                        <div
                          key={`prefetch:${memoryRecordIdValue(item.card.id)}`}
                          className={detailTileClass}
                        >
                          <div className="flex items-start justify-between gap-3">
                            <div>
                              <strong>{item.card.title}</strong>
                              <p className="text-xs text-muted-foreground">
                                {memoryRecordIdValue(item.card.id)}
                              </p>
                            </div>
                            <button
                              className="roc-action roc-action-compact"
                              type="button"
                              onClick={() => void loadMemoryDetail(memoryRecordIdValue(item.card.id))}
                            >
                              Inspect
                            </button>
                          </div>
                          <p className="text-xs text-muted-foreground">{item.why_recalled}</p>
                          <p className="text-xs text-muted-foreground">{item.card.summary}</p>
                        </div>
                      ))}
                    </div>
                  ) : null}
                </div>
              ) : null}
              {memoryRecentSessionRecords.length ? (
                <div className="grid gap-2 text-sm text-muted-foreground">
                  <p className="roc-section-label">Session Memory Writes</p>
                  <div className="grid gap-2">
                    {memoryRecentSessionRecords.map((record) => (
                      <div
                        key={`session:${memoryRecordIdValue(record.id)}`}
                        className={detailTileClass}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <strong>{record.title}</strong>
                            <p className="text-xs text-muted-foreground">
                              {memoryRecordIdValue(record.id)}
                            </p>
                          </div>
                          <button
                            className="roc-action roc-action-compact"
                            type="button"
                            onClick={() => void loadMemoryDetail(memoryRecordIdValue(record.id))}
                          >
                            Inspect
                          </button>
                        </div>
                        <p className="text-xs text-muted-foreground">
                          {record.kind} · {record.status} · {record.validation_status}
                        </p>
                        <p className="text-xs text-muted-foreground">{record.summary}</p>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {selectedMemoryId && insightMemoryIds.has(selectedMemoryId) ? (
                <div className="roc-rail-section bg-background/70">
                  <div className="roc-rail-section-header">
                    <div className="roc-rail-section-copy">
                      <p className="roc-section-label">Memory Detail</p>
                      <h4 className="roc-rail-section-title">{selectedMemoryId}</h4>
                    </div>
                    <button
                      className="roc-action roc-action-compact"
                      type="button"
                      onClick={() => {
                        setSelectedMemoryId(null);
                        setSelectedMemoryDetail(null);
                        setDetailError(null);
                      }}
                    >
                      Close
                    </button>
                  </div>
                  {detailLoading ? (
                    <div className="roc-state-card" data-tone="loading">
                      <p className="text-sm text-muted-foreground">Loading memory detail...</p>
                    </div>
                  ) : detailError ? (
                    <div className="roc-state-card" data-tone="danger">
                      <p className="text-sm text-rose-700 dark:text-rose-300">{detailError}</p>
                    </div>
                  ) : selectedMemoryDetail ? (
                    <div className="grid gap-1 text-sm text-muted-foreground">
                      <p>
                        <strong className="text-foreground">{selectedMemoryDetail.record.title}</strong>
                      </p>
                      <p>{selectedMemoryDetail.record.summary}</p>
                      <p>
                        {selectedMemoryDetail.record.kind} · {selectedMemoryDetail.record.scope} · {selectedMemoryDetail.record.status} · {selectedMemoryDetail.record.validation_status}
                      </p>
                      {(selectedMemoryDetail.record.trigger_conditions ?? []).length ? (
                        <p>
                          Triggers: {(selectedMemoryDetail.record.trigger_conditions ?? []).join(" · ")}
                        </p>
                      ) : null}
                      {(selectedMemoryDetail.record.normalized_facts ?? []).length ? (
                        <p>
                          Facts: {(selectedMemoryDetail.record.normalized_facts ?? [])
                            .slice(0, 4)
                            .join(" · ")}
                        </p>
                      ) : null}
                    </div>
                  ) : (
                    <div className="roc-state-card" data-tone="muted">
                      <p className="text-sm text-muted-foreground">No detail loaded.</p>
                    </div>
                  )}
                </div>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
});
