import { memo, useMemo, useState } from "react";
import type { useExecutionActivity } from "../../hooks/useExecutionActivity";
import {
  type MemoryDetailResponseRecord,
  memoryRecordIdValue,
} from "../../lib/memory";
import { multimodalCombinedWarnings, multimodalDisplayLabel } from "../../lib/multimodal";
import { useI18n } from "@/i18n/I18nProvider";
import { CompactionContinuityCard } from "../execution/CompactionContinuityCard";
import { SessionBlueprintEditor } from "./SessionBlueprintEditor";

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

function summarizeMetrics(items: Array<[string, string | number | null | undefined]>) {
  return items
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .map(([label, value]) => `${label} ${value}`)
    .join(" · ");
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
  const { t } = useI18n();
  const insights = activity.sessionInsights;
  const telemetry = insights?.telemetry ?? null;
  const runtimeTelemetry = activity.telemetry ?? null;
  const effectivePolicy = insights?.effective_policy ?? null;
  const schedulerPolicy = effectivePolicy?.scheduler ?? null;
  const telemetryUsage = telemetry?.usage ?? null;
  const trajectoryQuality =
    runtimeTelemetry?.tool_trajectory_quality ?? telemetry?.tool_trajectory_quality ?? null;
  const toolResultGovernance =
    runtimeTelemetry?.tool_result_governance ?? telemetry?.tool_result_governance ?? null;
  const memory = insights?.memory ?? null;
  const memorySummary = memory?.summary ?? null;
  const memoryAllowedScopes = memorySummary?.allowed_scopes ?? [];
  const memoryRecentRuleHits = useMemo(() => memorySummary?.recent_rule_hits ?? [], [memorySummary?.recent_rule_hits]);
  const memoryRecentSessionRecords = useMemo(
    () => memory?.recent_session_records ?? [],
    [memory?.recent_session_records],
  );
  const memoryFrozenItems = useMemo(() => memory?.frozen_snapshot?.items ?? [], [memory?.frozen_snapshot?.items]);
  const memoryPrefetchItems = useMemo(
    () => memory?.last_prefetch_packet?.items ?? [],
    [memory?.last_prefetch_packet?.items],
  );
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
      setDetailError(error instanceof Error ? error.message : t("session.unknownError"));
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
  const currentContextTokens = activity.sessionUsage?.context_tokens ?? null;
  const panelActionClass = "roc-action roc-action-pill";
  const compactActionClass = "roc-action roc-action-compact justify-self-start";
  const detailTileClass = "roc-rail-item grid gap-1 bg-card/45";

  return (
    <div className="roc-panel roc-rail-panel min-h-0 p-5">
        <div className="roc-rail-header">
          <div className="roc-rail-headline">
            <p className="roc-section-label">{t("session.runtimeExplain")}</p>
            <h3 className="roc-rail-title">{t("session.panelTitle")}</h3>
            <p className="roc-rail-description">{t("session.panelDescription")}</p>
        </div>
        <button
          className={panelActionClass}
          type="button"
          onClick={() => void activity.refreshExecutionActivity()}
          disabled={activity.activityLoading}
        >
          {activity.activityLoading ? t("session.refreshing") : t("session.refresh")}
        </button>
      </div>

      {!insights ? (
        <div className="roc-rail-empty">
          <div className="roc-section-label">{t("session.emptyLabel")}</div>
          <p className="text-sm font-semibold tracking-tight text-foreground">{t("session.emptyTitle")}</p>
          <p className="text-sm leading-6 text-muted-foreground">
            {t("session.emptyDescription")}
          </p>
        </div>
      ) : (
        <>
          <dl className="roc-structured-dl">
            <div className="roc-structured-row">
              <dt className="roc-structured-key">{t("session.sessionLabel")}</dt>
              <dd className="text-sm text-foreground">{insights.id}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">{t("session.titleLabel")}</dt>
              <dd className="text-sm text-foreground">{insights.title}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">{t("session.directoryLabel")}</dt>
              <dd className="text-sm text-foreground break-all">{insights.directory}</dd>
            </div>
            <div className="roc-structured-row">
              <dt className="roc-structured-key">{t("session.updatedLabel")}</dt>
              <dd className="text-sm text-foreground">{formatDateTime(insights.updated)}</dd>
            </div>
          </dl>

          {telemetry ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">{t("session.runtimeTelemetry")}</p>
                <h4 className="roc-rail-section-title">{t("session.currentRunSnapshot")}</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">{t("session.statusValue", { status: telemetry.last_run_status })}</span>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("session.metric.version"), telemetry.version],
                  trajectoryQuality
                    ? [t("session.metric.trajectory"), `${trajectoryQuality.score} ${formatTrajectoryBand(trajectoryQuality.band)}`]
                    : ["", null],
                ])}
              </p>
              {currentContextTokens ? (
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {t("session.currentLiveContext", { tokens: formatCompactTokenCount(currentContextTokens) })}
                </p>
              ) : null}
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t("session.sessionCumulative", {
                  total: formatCompactTokenCount(totalUsageTokens(telemetryUsage)),
                  input: formatCompactTokenCount(telemetryUsage?.input_tokens ?? 0),
                  output: formatCompactTokenCount(telemetryUsage?.output_tokens ?? 0),
                  reasoning: formatCompactTokenCount(telemetryUsage?.reasoning_tokens ?? 0),
                })}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t("session.cacheUsageLine", {
                  read: formatCompactTokenCount(telemetryUsage?.cache_read_tokens ?? 0),
                  miss: formatCompactTokenCount(telemetryUsage?.cache_miss_tokens ?? 0),
                  write: formatCompactTokenCount(telemetryUsage?.cache_write_tokens ?? 0),
                  cost: formatMoney(telemetryUsage?.total_cost),
                })}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {t("session.updatedAt", { time: formatDateTime(telemetry.updated_at) })}
              </p>
              {trajectoryQuality ? (
                <p className="text-sm text-muted-foreground leading-relaxed">
                  {t("session.trajectoryQuality", {
                    score: trajectoryQuality.score,
                    band: formatTrajectoryBand(trajectoryQuality.band),
                    repaired: trajectoryQuality.repaired_tool_call_count,
                    total: trajectoryQuality.total_tool_calls,
                    errors: trajectoryQuality.error_tool_call_count,
                  })}
                </p>
              ) : null}
              {runtimeTelemetry || telemetry ? (
                <div className={detailTileClass}>
                  <div className="grid gap-1">
                    <p className="roc-section-label">{t("session.toolResultGovernance")}</p>
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {summarizeMetrics([
                        [t("session.metric.single"), toolResultGovernance?.single_result_governed_count ?? 0],
                        [t("session.metric.batch"), toolResultGovernance?.batch_governed_count ?? 0],
                        [t("session.metric.transcriptFallback"), toolResultGovernance?.transcript_fallback_count ?? 0],
                        [t("session.metric.artifact"), toolResultGovernance?.artifact_fallback_count ?? 0],
                      ])}
                    </p>
                  </div>
                  {(toolResultGovernance?.total_original_chars ?? 0) > 0 ? (
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {t("session.governanceChars", {
                        original: (toolResultGovernance?.total_original_chars ?? 0).toLocaleString(),
                        displayed: (toolResultGovernance?.total_displayed_chars ?? 0).toLocaleString(),
                      })}
                    </p>
                  ) : (
                    <p className="text-sm text-muted-foreground leading-relaxed">
                      {t("session.governanceExplanation")}
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
                <p className="roc-section-label">{t("session.effectivePolicy")}</p>
                <h4 className="roc-rail-section-title">{t("session.schedulerSelection")}</h4>
              </div>
              {schedulerPolicy ? (
                <>
                  <p className="text-sm text-muted-foreground leading-relaxed">
                    {summarizeMetrics([
                      [t("session.metric.source"), schedulerPolicy.source],
                      [t("session.metric.applied"), schedulerPolicy.applied ? t("session.yes") : t("session.no")],
                      [t("session.metric.requested"), schedulerPolicy.requested_kind || "--"],
                      [t("session.metric.effective"), schedulerPolicy.blueprint_name || "--"],
                    ])}
                  </p>
                  <div className="grid gap-1 text-sm text-muted-foreground">
                    <p>{schedulerPolicy.blueprint_fingerprint || "--"}</p>
                    <p>{t("session.resolvedAgent", { value: schedulerPolicy.resolved_agent || "--" })}</p>
                  </div>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">{t("session.noSchedulerPolicy")}</p>
              )}
              <div className="flex flex-wrap gap-2">
                <SessionBlueprintEditor
                  sessionId={insights.id}
                  hasBlueprint={Boolean(schedulerPolicy?.blueprint_fingerprint)}
                  apiJson={apiJson}
                  onChanged={activity.refreshExecutionActivity}
                />
              </div>
              {(effectivePolicy.warnings ?? []).length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">{t("session.policyWarnings")}</p>
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
                <p className="roc-section-label">{t("session.multimodalExplain")}</p>
                <h4 className="roc-rail-section-title">{multimodalDisplayLabel(multimodal) || t("session.attachmentBackedInput")}</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="text-xs text-muted-foreground">
                  {summarizeMetrics([
                    [t("session.metric.message"), multimodal.user_message_id],
                    [t("session.metric.attachments"), multimodal.attachment_count],
                  ])}
                </span>
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>{t("session.kinds", { value: (multimodal.kinds ?? []).join(", ") || "--" })}</p>
                <p>{t("session.resolvedModel", { value: multimodal.resolved_model || "--" })}</p>
                <p>{t("session.badges", { value: (multimodal.badges ?? []).join(", ") || "--" })}</p>
                <p>{t("session.hardBlock", { value: multimodal.hard_block ? t("session.yes") : t("session.no") })}</p>
                <p>
                  {t("session.unsupportedParts", {
                    value: (multimodal.unsupported_parts ?? []).join(", ") || t("session.none"),
                  })}
                </p>
                <p>
                  {t("session.recommendedDowngrade", {
                    value: multimodal.recommended_downgrade || t("session.none"),
                  })}
                </p>
                <p>
                  {t("session.transportReplacedParts", {
                    value: (multimodal.transport_replaced_parts ?? []).join(", ") || t("session.none"),
                  })}
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
                  <p className="roc-section-label">{t("session.warningsLabel")}</p>
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
                <p className="roc-section-label">{t("session.memoryExplain")}</p>
                <h4 className="roc-rail-section-title">{t("session.workspaceModeTitle", { mode: insights.memory.summary.workspace_mode })}</h4>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("session.metric.snapshot"), insights.memory.summary.frozen_snapshot_items],
                  [t("session.metric.prefetch"), insights.memory.summary.last_prefetch_items],
                  [t("session.metric.ruleHits"), memoryRecentRuleHits.length],
                  [t("session.metric.warnings"), insights.memory.summary.warning_count],
                ])}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                {summarizeMetrics([
                  [t("session.metric.methodology"), insights.memory.summary.methodology_candidate_count],
                  [t("session.metric.skillTargets"), insights.memory.summary.derived_skill_candidate_count],
                  [t("session.metric.linkedSkills"), insights.memory.summary.linked_skill_count],
                  [t("session.metric.feedbackLessons"), insights.memory.summary.skill_feedback_lesson_count],
                ])}
              </p>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>{t("session.workspaceKey", { value: insights.memory.summary.workspace_key })}</p>
                <p>{t("session.allowedScopes", { value: memoryAllowedScopes.join(", ") || "--" })}</p>
                <p>{t("session.frozenSnapshotGenerated", { value: formatDateTime(insights.memory.summary.frozen_snapshot_generated_at) })}</p>
                <p>{t("session.lastPrefetchGenerated", { value: formatDateTime(insights.memory.summary.last_prefetch_generated_at) })}</p>
                <p>{t("session.lastPrefetchQuery", { value: insights.memory.summary.last_prefetch_query?.trim() || t("session.noQueryCaptured") })}</p>
                <p>
                  {t("session.sessionRecords", {
                    candidate: insights.memory.summary.candidate_count,
                    validated: insights.memory.summary.validated_count,
                    rejected: insights.memory.summary.rejected_count,
                  })}
                </p>
                <p>
                  {t("session.validationPressure", {
                    warnings: insights.memory.summary.warning_count,
                    methodology: insights.memory.summary.methodology_candidate_count,
                    skillTargets: insights.memory.summary.derived_skill_candidate_count,
                  })}
                </p>
                <p>
                  {t("session.retrieval", {
                    runs: insights.memory.summary.retrieval_run_count,
                    hits: insights.memory.summary.retrieval_hit_count,
                    used: insights.memory.summary.retrieval_use_count,
                  })}
                </p>
              </div>
              {skillLinkedRecords.length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">{t("session.skillLinkedRecords")}</p>
                  <div className="grid gap-2 md:grid-cols-2">
                    {skillLinkedRecords.map((item) => (
                      <div
                        key={`skill:${memoryRecordIdValue(item.id)}`}
                        className={detailTileClass}
                      >
                        <div className="flex flex-wrap items-center gap-2">
                          <strong>{item.title}</strong>
                          {skillBadgeLabel(item) ? <span className="text-xs text-muted-foreground">{skillBadgeLabel(item)}</span> : null}
                        </div>
                        <p className="text-xs text-muted-foreground">{item.summary}</p>
                        <button
                          className={compactActionClass}
                          type="button"
                          onClick={() => void loadMemoryDetail(memoryRecordIdValue(item.id))}
                        >
                          {t("session.inspectMemory")}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {insights.memory.summary.latest_consolidation_run ? (
                <div className="grid gap-1 text-sm text-muted-foreground">
                  <p>{t("session.latestConsolidation", { id: insights.memory.summary.latest_consolidation_run.run_id })}</p>
                  <p>
                    {t("session.consolidationSummary", {
                      merged: insights.memory.summary.latest_consolidation_run.merged_count,
                      promoted: insights.memory.summary.latest_consolidation_run.promoted_count,
                      conflicts: insights.memory.summary.latest_consolidation_run.conflict_count,
                    })}
                  </p>
                </div>
              ) : null}
              {memoryRecentRuleHits.length ? (
                <div className="grid gap-2 md:grid-cols-2">
                  {memoryRecentRuleHits.map((hit) => (
                    <div key={hit.id} className={detailTileClass}>
                      <div className="flex flex-wrap items-center gap-2">
                        <strong>{hit.hit_kind}</strong>
                        {hit.memory_id ? <span className="text-xs text-muted-foreground">{memoryRecordIdValue(hit.memory_id)}</span> : null}
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {hit.detail || t("session.noDetailAttached")}
                      </p>
                      {hit.memory_id ? (
                        <button
                          className={compactActionClass}
                          type="button"
                          onClick={() => void loadMemoryDetail(memoryRecordIdValue(hit.memory_id))}
                        >
                          {t("session.inspectMemory")}
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
                  <p>{t("session.frozenSnapshotNote", { value: insights.memory.frozen_snapshot.note || t("session.noNote") })}</p>
                  <p>
                    {t("session.frozenSnapshotScopes", {
                      value: (insights.memory.frozen_snapshot.scopes ?? []).join(", ") || "--",
                    })}
                  </p>
                  {memoryFrozenItems.length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">{t("session.frozenItems")}</p>
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
                              {t("session.inspect")}
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
                  <p>{t("session.prefetchNote", { value: insights.memory.last_prefetch_packet.note || t("session.noNote") })}</p>
                  <p>
                    {t("session.prefetchScopes", {
                      value: (insights.memory.last_prefetch_packet.scopes ?? []).join(", ") || "--",
                    })}
                  </p>
                  <p>{t("session.prefetchRecalledItems", { count: memoryPrefetchItems.length })}</p>
                  {memoryPrefetchItems.length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">{t("session.prefetchItems")}</p>
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
                              {t("session.inspect")}
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
                  <p className="roc-section-label">{t("session.sessionMemoryWrites")}</p>
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
                            {t("session.inspect")}
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
                      <p className="roc-section-label">{t("session.memoryDetail")}</p>
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
                      {t("session.close")}
                    </button>
                  </div>
                  {detailLoading ? (
                    <div className="roc-state-card" data-tone="loading">
                      <p className="text-sm text-muted-foreground">{t("session.loadingMemoryDetail")}</p>
                    </div>
                  ) : detailError ? (
                    <div className="roc-state-card" data-tone="danger">
                      <p className="text-sm text-(--ds-error)">{detailError}</p>
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
                          {t("session.triggers", {
                            value: (selectedMemoryDetail.record.trigger_conditions ?? []).join(" · "),
                          })}
                        </p>
                      ) : null}
                      {(selectedMemoryDetail.record.normalized_facts ?? []).length ? (
                        <p>
                          {t("session.facts", {
                            value: (selectedMemoryDetail.record.normalized_facts ?? [])
                              .slice(0, 4)
                              .join(" · "),
                          })}
                        </p>
                      ) : null}
                    </div>
                  ) : (
                    <div className="roc-state-card" data-tone="muted">
                      <p className="text-sm text-muted-foreground">{t("session.noDetailLoaded")}</p>
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
