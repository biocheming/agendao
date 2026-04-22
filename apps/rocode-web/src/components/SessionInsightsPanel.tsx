import { useMemo, useState } from "react";
import type { useExecutionActivity } from "../hooks/useExecutionActivity";
import {
  type MemoryDetailResponseRecord,
  memoryRecordIdValue,
} from "../lib/memory";
import { multimodalCombinedWarnings, multimodalDisplayLabel } from "../lib/multimodal";

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

export function SessionInsightsPanel({ activity, apiJson }: SessionInsightsPanelProps) {
  const insights = activity.sessionInsights;
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
    insights?.memory?.summary.recent_rule_hits.forEach((hit) => {
      const memoryId = memoryRecordIdValue(hit.memory_id);
      if (memoryId) ids.add(memoryId);
    });
    (insights?.memory?.frozen_snapshot?.items ?? []).forEach((item) =>
      ids.add(memoryRecordIdValue(item.card.id)),
    );
    (insights?.memory?.last_prefetch_packet?.items ?? []).forEach((item) =>
      ids.add(memoryRecordIdValue(item.card.id)),
    );
    insights?.memory?.recent_session_records.forEach((item) =>
      ids.add(memoryRecordIdValue(item.id)),
    );
    return ids;
  }, [insights]);
  const skillLinkedRecords = useMemo(
    () =>
      insights?.memory?.recent_session_records.filter(
        (item) => item.linked_skill_name || item.derived_skill_name,
      ) ?? [],
    [insights],
  );
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
          <div className="roc-section-label">Runtime Explain</div>
          <p className="text-sm font-semibold tracking-tight text-foreground">No session insights loaded yet.</p>
          <p className="text-sm leading-6 text-muted-foreground">
            Refresh activity after a session run to inspect telemetry, memory, and multimodal runtime detail.
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

          {insights.telemetry ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Persisted Telemetry</p>
                <h4 className="roc-rail-section-title">Stored Run Snapshot</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">version {insights.telemetry.version}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">status {insights.telemetry.last_run_status}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">stages {insights.telemetry.stage_summaries.length}</span>
              </div>
              <p className="text-sm text-muted-foreground leading-relaxed">
                Input {insights.telemetry.usage.input_tokens} · output {insights.telemetry.usage.output_tokens} · reasoning {insights.telemetry.usage.reasoning_tokens} · cost {formatMoney(insights.telemetry.usage.total_cost)}
              </p>
              <p className="text-sm text-muted-foreground leading-relaxed">
                Updated {formatDateTime(insights.telemetry.updated_at)}
              </p>
            </div>
          ) : null}

          {insights.multimodal ? (
            <div className="roc-rail-section">
              <div className="roc-rail-section-copy">
                <p className="roc-section-label">Multimodal Explain</p>
                <h4 className="roc-rail-section-title">{multimodalDisplayLabel(insights.multimodal) || "Attachment-backed input"}</h4>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">message {insights.multimodal.user_message_id}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">attachments {insights.multimodal.attachment_count}</span>
                {insights.multimodal.kinds.map((kind) => (
                  <span key={`kind:${kind}`} className="roc-badge px-3 py-1.5 text-xs">
                    {kind}
                  </span>
                ))}
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>Resolved model: {insights.multimodal.resolved_model || "--"}</p>
                <p>Badges: {insights.multimodal.badges.join(", ") || "--"}</p>
                <p>Hard block: {insights.multimodal.hard_block ? "yes" : "no"}</p>
                <p>
                  Unsupported parts:{" "}
                  {insights.multimodal.unsupported_parts.join(", ") || "none"}
                </p>
                <p>
                  Recommended downgrade:{" "}
                  {insights.multimodal.recommended_downgrade || "none"}
                </p>
                <p>
                  Transport replaced parts:{" "}
                  {insights.multimodal.transport_replaced_parts.join(", ") || "none"}
                </p>
              </div>
              {insights.multimodal.attachments.length ? (
                <div className="grid gap-2 md:grid-cols-2">
                  {insights.multimodal.attachments.map((attachment) => (
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
              {multimodalCombinedWarnings(insights.multimodal).length ? (
                <div className="grid gap-2">
                  <p className="roc-section-label">Warnings</p>
                  {multimodalCombinedWarnings(insights.multimodal).map((warning, index) => (
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
                <span className="roc-badge px-3 py-1.5 text-xs">rule hits {insights.memory.summary.recent_rule_hits.length}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">warnings {insights.memory.summary.warning_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">methodology {insights.memory.summary.methodology_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">skill targets {insights.memory.summary.derived_skill_candidate_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">linked skills {insights.memory.summary.linked_skill_count}</span>
                <span className="roc-badge px-3 py-1.5 text-xs">feedback lessons {insights.memory.summary.skill_feedback_lesson_count}</span>
              </div>
              <div className="grid gap-1 text-sm text-muted-foreground">
                <p>Workspace key: {insights.memory.summary.workspace_key}</p>
                <p>Allowed scopes: {insights.memory.summary.allowed_scopes.join(", ") || "--"}</p>
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
              {insights.memory.summary.recent_rule_hits.length ? (
                <div className="grid gap-2 md:grid-cols-2">
                  {insights.memory.summary.recent_rule_hits.map((hit) => (
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
                  {(insights.memory.frozen_snapshot.items ?? []).length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">Frozen Items</p>
                      {(insights.memory.frozen_snapshot.items ?? []).map((item) => (
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
                  <p>Prefetch recalled items: {(insights.memory.last_prefetch_packet.items ?? []).length}</p>
                  {(insights.memory.last_prefetch_packet.items ?? []).length ? (
                    <div className="grid gap-2">
                      <p className="roc-section-label">Prefetch Items</p>
                      {(insights.memory.last_prefetch_packet.items ?? []).map((item) => (
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
              {insights.memory.recent_session_records.length ? (
                <div className="grid gap-2 text-sm text-muted-foreground">
                  <p className="roc-section-label">Session Memory Writes</p>
                  <div className="grid gap-2 md:grid-cols-2">
                    {insights.memory.recent_session_records.map((record) => (
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
}
