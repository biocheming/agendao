import type {
  MemoryConsolidationResponseRecord,
  MemoryConsolidationRunListResponseRecord,
  MemoryConflictResponseRecord,
  MemoryDetailResponseRecord,
  MemoryListResponseRecord,
  MemoryRetrievalPreviewResponseRecord,
  MemoryRuleHitListResponseRecord,
  MemoryRulePackListResponseRecord,
  MemoryValidationReportResponseRecord,
} from "@/lib/memory";
import { useState } from "react";
import { memoryRecordIdValue } from "@/lib/memory";
import { cn } from "@/lib/utils";

interface MemoryTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  summaryCardClass: string;
  sectionCardClass: string;
  mutedCardClass: string;
  insetCardClass: string;
  disclosureCardClass: string;
}

interface MemoryTabProps {
  selectedSessionId: string | null;
  styles: MemoryTabStyles;
  memorySearchDraft: string;
  onMemorySearchDraftChange: (value: string) => void;
  memoryListLoading: boolean;
  onLoadMemoryList: () => void;
  memoryPreviewLoading: boolean;
  onLoadMemoryPreview: () => void;
  memoryGovernanceLoading: boolean;
  onLoadMemoryGovernance: () => void;
  memoryConsolidateIncludeCandidates: boolean;
  onMemoryConsolidateIncludeCandidatesChange: (value: boolean) => void;
  memoryConsolidating: boolean;
  onRunMemoryConsolidation: () => void;
  memoryListResponse: MemoryListResponseRecord | null;
  selectedMemoryId: string | null;
  selectedMemoryCardIdLabel: string | null;
  onSelectMemoryId: (value: string) => void;
  memoryDetailLoading: boolean;
  memoryDetail: MemoryDetailResponseRecord | null;
  memoryValidationReport: MemoryValidationReportResponseRecord | null;
  memoryConflicts: MemoryConflictResponseRecord | null;
  memoryPreview: MemoryRetrievalPreviewResponseRecord | null;
  memoryRulePacks: MemoryRulePackListResponseRecord | null;
  memoryRuleHits: MemoryRuleHitListResponseRecord | null;
  memoryConsolidationRuns: MemoryConsolidationRunListResponseRecord | null;
  memoryConsolidationResult: MemoryConsolidationResponseRecord | null;
}

function arrayOrEmpty<T>(value: T[] | null | undefined): T[] {
  return Array.isArray(value) ? value : [];
}

function unixTimeLabel(value?: number | null): string {
  if (!value) return "--";
  try {
    const timestamp = value > 1_000_000_000_000 ? value : value * 1000;
    return new Date(timestamp).toLocaleString();
  } catch {
    return String(value);
  }
}

export function MemoryTab({
  selectedSessionId,
  styles,
  memorySearchDraft,
  onMemorySearchDraftChange,
  memoryListLoading,
  onLoadMemoryList,
  memoryPreviewLoading,
  onLoadMemoryPreview,
  memoryGovernanceLoading,
  onLoadMemoryGovernance,
  memoryConsolidateIncludeCandidates,
  onMemoryConsolidateIncludeCandidatesChange,
  memoryConsolidating,
  onRunMemoryConsolidation,
  memoryListResponse,
  selectedMemoryId,
  selectedMemoryCardIdLabel,
  onSelectMemoryId,
  memoryDetailLoading,
  memoryDetail,
  memoryValidationReport,
  memoryConflicts,
  memoryPreview,
  memoryRulePacks,
  memoryRuleHits,
  memoryConsolidationRuns,
  memoryConsolidationResult,
}: MemoryTabProps) {
  const {
    primaryButtonClass,
    secondaryButtonClass,
    summaryCardClass,
    sectionCardClass,
    mutedCardClass,
    insetCardClass,
    disclosureCardClass,
  } = styles;


  const [memoryTab, setMemoryTab] = useState<"overview" | "records" | "governance" | "retrieval">("overview");

  return (
    <div className="relative grid gap-4">
      {/* Header + Sub-tabs */}
      <div className="flex items-center justify-between gap-3">
        <h3 className="m-0 text-base font-semibold">Memory</h3>
        <div className="flex gap-1 rounded-lg bg-muted/40 p-1">
          {(["overview", "records", "governance", "retrieval"] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                memoryTab === tab
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
              onClick={() => setMemoryTab(tab)}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {/* Overview Tab */}
      {memoryTab === "overview" ? (
        <div className="grid gap-5">
          <div className="grid gap-2">
            <label htmlFor="settings-memory-search" className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Search</label>
            <input
              id="settings-memory-search"
              type="text"
              placeholder="Search title, summary, normalized facts"
              value={memorySearchDraft}
              onChange={(event) => onMemorySearchDraftChange(event.target.value)}
            />
          </div>

          <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
            <span><strong className="text-foreground">{memoryListResponse?.items.length ?? 0}</strong> records</span>
            <span><strong className="text-foreground">{memoryRulePacks?.items?.length ?? 0}</strong> rule packs</span>
            <span><strong className="text-foreground">{memoryRuleHits?.items?.length ?? 0}</strong> hits</span>
            <span><strong className="text-foreground">{memoryConsolidationRuns?.items?.length ?? 0}</strong> runs</span>
          </div>

          <div className="flex flex-wrap gap-2">
            <button type="button" className={primaryButtonClass} onClick={onLoadMemoryList} disabled={memoryListLoading}>
              {memoryListLoading ? "Refreshing..." : "Refresh Memory"}
            </button>
            <button type="button" className={secondaryButtonClass} onClick={onLoadMemoryPreview} disabled={memoryPreviewLoading}>
              {memoryPreviewLoading ? "Previewing..." : "Preview Injection"}
            </button>
            <button type="button" className={secondaryButtonClass} onClick={onLoadMemoryGovernance} disabled={memoryGovernanceLoading}>
              {memoryGovernanceLoading ? "Refreshing..." : "Refresh Governance"}
            </button>
          </div>

          <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input type="checkbox" checked={memoryConsolidateIncludeCandidates} onChange={(event) => onMemoryConsolidateIncludeCandidatesChange(event.target.checked)} />
                Include candidates
              </label>
              <button type="button" className={primaryButtonClass} onClick={onRunMemoryConsolidation} disabled={memoryConsolidating}>
                {memoryConsolidating ? "Consolidating..." : "Run Consolidation"}
              </button>
            </div>
            {memoryConsolidationResult ? (
              <div className="mt-3 text-sm">
                <span className="text-muted-foreground">
                  Last run: merged {memoryConsolidationResult.run.merged_count} · promoted{" "}
                  {memoryConsolidationResult.run.promoted_count} · conflicts{" "}
                  {memoryConsolidationResult.run.conflict_count}
                </span>
              </div>
            ) : null}
          </div>

          <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
            <div className="grid gap-2">
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Scope</span>
                <span className="ml-2 text-foreground">{selectedSessionId || "workspace authority"}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Search Fields</span>
                <span className="ml-2">{arrayOrEmpty(memoryListResponse?.contract.search_fields).join(", ") || "--"}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">Filters</span>
                <span className="ml-2">{arrayOrEmpty(memoryListResponse?.contract.filter_query_parameters).join(", ") || "--"}</span>
              </div>
              <div className="mt-1 text-xs">
                {memoryListResponse?.contract.note ||
                  "Read models come from /memory/list, /memory/{id}, /memory/{id}/validation-report, and /memory/{id}/conflicts."}
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {/* Records Tab */}
      {memoryTab === "records" ? (
        <div className="relative">
          <div className="grid gap-2 max-h-[28rem] overflow-y-auto pr-1">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Memory Records</span>
              <span className="text-xs text-muted-foreground">
                {memoryListLoading ? "Loading..." : `${memoryListResponse?.items.length ?? 0} records`}
              </span>
            </div>

            {memoryListResponse?.items.length ? (
              memoryListResponse.items.map((item) => {
                const recordId = memoryRecordIdValue(item.id);
                const active = recordId === selectedMemoryId;
                return (
                  <button
                    key={recordId}
                    type="button"
                    className={cn(
                      "grid gap-1.5 rounded-lg border-l-2 px-4 py-3 text-left transition-colors",
                      active
                        ? "border-l-foreground/40 bg-foreground/5"
                        : "border-l-transparent bg-muted/20 hover:bg-muted/40"
                    )}
                    onClick={() => onSelectMemoryId(recordId)}
                  >
                    <div className="flex flex-wrap items-center gap-1.5 text-xs uppercase tracking-wide text-muted-foreground">
                      <span>{item.kind}</span>
                      <span>·</span>
                      <span>{item.status}</span>
                      <span>·</span>
                      <span>{item.validation_status}</span>
                      {item.linked_skill_name ? (
                        <><span>·</span><span>linked {item.linked_skill_name}</span></>
                      ) : null}
                      {item.derived_skill_name ? (
                        <><span>·</span><span>target {item.derived_skill_name}</span></>
                      ) : null}
                    </div>
                    <strong className="text-sm">{item.title}</strong>
                    <span className="text-xs text-muted-foreground line-clamp-2">{item.summary}</span>
                  </button>
                );
              })
            ) : (
              <div className={mutedCardClass}>No memory records matched this query.</div>
            )}
          </div>

          {selectedMemoryId && memoryDetail && !memoryDetailLoading ? (
            <div className="absolute inset-0 z-10 grid gap-4 overflow-y-auto rounded-lg bg-background p-4 shadow-lg">
              <div className="flex items-center gap-3">
                <button type="button" className="text-xs text-muted-foreground hover:text-foreground transition-colors" onClick={() => onSelectMemoryId("")}>
                  ← Back to records
                </button>
                <span className="text-xs text-muted-foreground">{memoryRecordIdValue(memoryDetail.record.id)}</span>
              </div>

              <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0">
                    <strong className="block text-base text-foreground">{memoryDetail.record.title}</strong>
                    <p className="m-0 mt-1 text-sm leading-relaxed text-muted-foreground">{memoryDetail.record.summary}</p>
                  </div>
                  <div className="flex gap-3 text-xs text-muted-foreground">
                    <span>{memoryDetail.record.kind} / {memoryDetail.record.scope}</span>
                    <span>{memoryDetail.record.status} / {memoryDetail.record.validation_status}</span>
                  </div>
                </div>
                {(memoryDetail.record.linked_skill_name || memoryDetail.record.derived_skill_name) ? (
                  <div className="mt-2 flex gap-4 text-xs text-muted-foreground">
                    <span>linked {memoryDetail.record.linked_skill_name || "--"}</span>
                    <span>target {memoryDetail.record.derived_skill_name || "--"}</span>
                  </div>
                ) : null}
              </div>

              <details open>
                <summary className="cursor-pointer list-none">
                  <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Record Semantics</span>
                </summary>
                <div className="mt-2 grid gap-3 sm:grid-cols-3">
                  <div className="grid gap-1">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Triggers</span>
                    {arrayOrEmpty(memoryDetail.record.trigger_conditions).length
                      ? arrayOrEmpty(memoryDetail.record.trigger_conditions).map((v) => (<span key={v} className="text-sm">{v}</span>))
                      : <span className="text-sm text-muted-foreground">--</span>}
                  </div>
                  <div className="grid gap-1">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Boundaries</span>
                    {arrayOrEmpty(memoryDetail.record.boundaries).length
                      ? arrayOrEmpty(memoryDetail.record.boundaries).map((v) => (<span key={v} className="text-sm">{v}</span>))
                      : <span className="text-sm text-muted-foreground">--</span>}
                  </div>
                  <div className="grid gap-1">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Normalized Facts</span>
                    {arrayOrEmpty(memoryDetail.record.normalized_facts).length
                      ? arrayOrEmpty(memoryDetail.record.normalized_facts).map((v) => (<span key={v} className="text-sm">{v}</span>))
                      : <span className="text-sm text-muted-foreground">--</span>}
                  </div>
                </div>
              </details>

              <details>
                <summary className="cursor-pointer list-none">
                  <div className="flex items-center gap-2">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Validation + Conflicts</span>
                    <span className="text-xs text-muted-foreground">{arrayOrEmpty(memoryConflicts?.conflicts).length} conflict{arrayOrEmpty(memoryConflicts?.conflicts).length === 1 ? "" : "s"}</span>
                  </div>
                </summary>
                <div className="mt-2 grid gap-4 sm:grid-cols-2">
                  <div className="grid gap-2">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Validation</span>
                    {memoryValidationReport?.latest ? (
                      <div className="grid gap-1 text-sm">
                        <strong>{memoryValidationReport.latest.status}</strong>
                        <span className="text-muted-foreground">Checked: {unixTimeLabel(memoryValidationReport.latest.checked_at)}</span>
                        {arrayOrEmpty(memoryValidationReport.latest.issues).length
                          ? arrayOrEmpty(memoryValidationReport.latest.issues).map((issue) => (<span key={issue}>{issue}</span>))
                          : <span className="text-muted-foreground">No issues.</span>}
                      </div>
                    ) : <span className="text-sm text-muted-foreground">No validation report yet.</span>}
                  </div>
                  <div className="grid gap-2">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Conflicts</span>
                    {arrayOrEmpty(memoryConflicts?.conflicts).length ? (
                      <div className="grid gap-2">
                        {arrayOrEmpty(memoryConflicts?.conflicts).map((conflict) => (
                          <div key={conflict.id} className="bg-muted/30 rounded px-3 py-2 text-sm">
                            <strong className="block">{conflict.conflict_kind}</strong>
                            <span className="block text-muted-foreground">Other: {memoryRecordIdValue(conflict.other_record_id)}</span>
                            <span className="block">{conflict.detail}</span>
                          </div>
                        ))}
                      </div>
                    ) : <span className="text-sm text-muted-foreground">No conflicts.</span>}
                  </div>
                </div>
              </details>

              <details>
                <summary className="cursor-pointer list-none">
                  <div className="flex items-center gap-2">
                    <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Evidence</span>
                    <span className="text-xs text-muted-foreground">{arrayOrEmpty(memoryDetail.record.evidence_refs).length} ref{arrayOrEmpty(memoryDetail.record.evidence_refs).length === 1 ? "" : "s"}</span>
                  </div>
                </summary>
                <div className="mt-2 grid gap-2">
                  {arrayOrEmpty(memoryDetail.record.evidence_refs).length ? (
                    arrayOrEmpty(memoryDetail.record.evidence_refs).map((ref, index) => (
                      <div key={`${ref.session_id ?? "session"}-${index}`} className="bg-muted/30 rounded px-3 py-2 text-sm">
                        <div>session: {ref.session_id || "--"}</div>
                        <div>message: {ref.message_id || "--"}</div>
                        <div>tool: {ref.tool_call_id || "--"}</div>
                        <div>stage: {ref.stage_id || "--"}</div>
                        {ref.note ? <div className="text-muted-foreground">note: {ref.note}</div> : null}
                      </div>
                    ))
                  ) : <span className="text-sm text-muted-foreground">No evidence refs.</span>}
                </div>
              </details>
            </div>
          ) : null}

          {selectedMemoryId && memoryDetailLoading ? (
            <div className="absolute inset-0 z-10 flex items-center justify-center rounded-lg bg-background/80">
              <span className="text-sm text-muted-foreground">Loading detail...</span>
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Governance Tab */}
      {memoryTab === "governance" ? (
        <div className="grid gap-5">
          <div>
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Rule Packs</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? "Loading..." : `${memoryRulePacks?.items?.length ?? 0} packs`}</span>
            </div>
            <div className="grid gap-2">
              {memoryRulePacks?.items?.length ? (
                memoryRulePacks.items.map((pack) => (
                  <div key={pack.id} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                    <div className="flex items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                      <span>{pack.rule_pack_kind}</span><span>·</span><span>{pack.version}</span>
                    </div>
                    <strong className="mt-1 block text-sm">{pack.id}</strong>
                    {arrayOrEmpty(pack.rules).length ? (
                      <div className="mt-2 grid gap-1.5">
                        {arrayOrEmpty(pack.rules).map((rule) => (
                          <div key={rule.id} className="bg-muted/40 rounded px-3 py-2 text-sm">
                            <strong className="block">{rule.id}</strong>
                            <span className="block text-muted-foreground">{rule.description}</span>
                            {rule.promotion_target ? <span className="block text-xs text-muted-foreground">promotion target: {rule.promotion_target}</span> : null}
                          </div>
                        ))}
                      </div>
                    ) : <span className="mt-1 block text-xs text-muted-foreground">No rules declared.</span>}
                  </div>
                ))
              ) : <div className={mutedCardClass}>{memoryGovernanceLoading ? "Loading rule packs..." : "No rule packs available."}</div>}
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Recent Rule Hits</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? "Loading..." : `${memoryRuleHits?.items?.length ?? 0} hits`}</span>
            </div>
            <div className="grid gap-2">
              {arrayOrEmpty(memoryRuleHits?.items).length ? (
                arrayOrEmpty(memoryRuleHits?.items).map((hit) => (
                  <div key={hit.id} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                    <strong className="block text-sm">{hit.hit_kind}</strong>
                    <div className="mt-1 grid gap-0.5 text-sm text-muted-foreground">
                      <span>run: {hit.run_id || "--"}</span>
                      <span>record: {memoryRecordIdValue(hit.memory_id)}</span>
                      <span>pack: {hit.rule_pack_id || "--"}</span>
                      <span>{unixTimeLabel(hit.created_at)}</span>
                      {hit.detail ? <span>{hit.detail}</span> : null}
                    </div>
                  </div>
                ))
              ) : <div className={mutedCardClass}>{memoryGovernanceLoading ? "Loading rule hits..." : "No recent rule hits."}</div>}
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Consolidation Runs</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? "Loading..." : `${memoryConsolidationRuns?.items?.length ?? 0} runs`}</span>
            </div>
            <div className="grid gap-2">
              {memoryConsolidationResult ? (
                <div className="border-l-2 border-l-foreground bg-foreground/5 px-4 py-3">
                  <strong className="block text-sm">Latest Consolidation</strong>
                  <div className="mt-1 grid gap-0.5 text-sm text-muted-foreground">
                    <span>run: {memoryConsolidationResult.run.run_id}</span>
                    <span>merged {memoryConsolidationResult.run.merged_count} · promoted {memoryConsolidationResult.run.promoted_count} · conflicts {memoryConsolidationResult.run.conflict_count}</span>
                    {arrayOrEmpty(memoryConsolidationResult.reflection_notes).length ? (
                      <div className="mt-1 grid gap-0.5">{arrayOrEmpty(memoryConsolidationResult.reflection_notes).map((note) => (<span key={note}>{note}</span>))}</div>
                    ) : null}
                  </div>
                </div>
              ) : null}
              {arrayOrEmpty(memoryConsolidationRuns?.items).length ? (
                arrayOrEmpty(memoryConsolidationRuns?.items).map((run) => (
                  <div key={run.run_id} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                    <strong className="block text-sm">{run.run_id}</strong>
                    <div className="mt-1 grid gap-0.5 text-sm text-muted-foreground">
                      <span>merged {run.merged_count} · promoted {run.promoted_count} · conflicts {run.conflict_count}</span>
                      <span>started: {unixTimeLabel(run.started_at)}</span>
                      <span>finished: {run.finished_at ? unixTimeLabel(run.finished_at) : "--"}</span>
                    </div>
                  </div>
                ))
              ) : <div className={mutedCardClass}>{memoryGovernanceLoading ? "Loading consolidation runs..." : "No consolidation runs recorded yet."}</div>}
            </div>
          </div>
        </div>
      ) : null}

      {/* Retrieval Tab */}
      {memoryTab === "retrieval" ? (
        <div className="grid gap-4">
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">Retrieval Preview</span>
            <span className="text-xs text-muted-foreground">{memoryPreviewLoading ? "Loading preview..." : `${arrayOrEmpty(memoryPreview?.packet.items).length} recalled`}</span>
          </div>
          <div className={mutedCardClass}>
            {memoryPreview?.contract.note || "Formal preview of which memory records would be injected into the current turn and why."}
          </div>
          <div className="grid gap-3">
            {arrayOrEmpty(memoryPreview?.packet.items).length ? (
              arrayOrEmpty(memoryPreview?.packet.items).map((item) => (
                <div key={memoryRecordIdValue(item.card.id)} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                  <div className="flex items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                    <span>{item.card.kind}</span><span>·</span><span>{item.card.validation_status}</span>
                  </div>
                  <strong className="mt-1 block text-sm">{item.card.title}</strong>
                  <div className="mt-1 text-sm text-muted-foreground">
                    <div>why: {item.why_recalled}</div>
                    <div>summary: {item.card.summary}</div>
                    {item.evidence_summary ? <div>evidence: {item.evidence_summary}</div> : null}
                  </div>
                </div>
              ))
            ) : <div className={mutedCardClass}>No memory records would be injected for the current search/session scope.</div>}
          </div>
        </div>
      ) : null}
    </div>
  );
}
