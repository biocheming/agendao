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
import { useI18n } from "@/i18n/I18nProvider";
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

export interface MemoryTabProps {
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
    mutedCardClass,
  } = styles;
  const { t } = useI18n();


  const [memoryTab, setMemoryTab] = useState<"overview" | "records" | "governance" | "retrieval">("overview");

  return (
    <div className="relative grid gap-4" data-testid="settings-memory-root">
      {/* Header + Sub-tabs */}
      <div className="flex items-center justify-between gap-3">
        <h3 className="m-0 text-base font-semibold">{t("settings.memory.title")}</h3>
        <div className="flex gap-1 rounded-lg bg-muted/40 p-1">
          {(["overview", "records", "governance", "retrieval"] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              data-testid={`settings-memory-subtab-${tab}`}
              data-active={memoryTab === tab ? "true" : "false"}
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                memoryTab === tab
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
              onClick={() => setMemoryTab(tab)}
            >
              {t(`settings.memory.subtab.${tab}`)}
            </button>
          ))}
        </div>
      </div>

      {/* Overview Tab */}
      {memoryTab === "overview" ? (
        <div className="grid gap-5">
          <div className="grid gap-2">
            <label htmlFor="settings-memory-search" className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.search")}</label>
            <input
              id="settings-memory-search"
              data-testid="settings-memory-search"
              type="text"
              placeholder={t("settings.memory.searchPlaceholder")}
              value={memorySearchDraft}
              onChange={(event) => onMemorySearchDraftChange(event.target.value)}
            />
          </div>

          <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-muted-foreground">
            <span><strong className="text-foreground">{memoryListResponse?.items.length ?? 0}</strong> {t("settings.memory.stats.records")}</span>
            <span><strong className="text-foreground">{memoryRulePacks?.items?.length ?? 0}</strong> {t("settings.memory.stats.rulePacks")}</span>
            <span><strong className="text-foreground">{memoryRuleHits?.items?.length ?? 0}</strong> {t("settings.memory.stats.hits")}</span>
            <span><strong className="text-foreground">{memoryConsolidationRuns?.items?.length ?? 0}</strong> {t("settings.memory.stats.runs")}</span>
          </div>

          <div className="flex flex-wrap gap-2">
            <button type="button" className={primaryButtonClass} data-testid="settings-memory-refresh" onClick={onLoadMemoryList} disabled={memoryListLoading}>
              {memoryListLoading ? t("settings.memory.refreshing") : t("settings.memory.refreshMemory")}
            </button>
            <button type="button" className={secondaryButtonClass} data-testid="settings-memory-preview" onClick={onLoadMemoryPreview} disabled={memoryPreviewLoading}>
              {memoryPreviewLoading ? t("settings.memory.previewing") : t("settings.memory.previewInjection")}
            </button>
            <button type="button" className={secondaryButtonClass} data-testid="settings-memory-governance-refresh" onClick={onLoadMemoryGovernance} disabled={memoryGovernanceLoading}>
              {memoryGovernanceLoading ? t("settings.memory.refreshing") : t("settings.memory.refreshGovernance")}
            </button>
          </div>

          <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
            <div className="flex items-center gap-4">
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input data-testid="settings-memory-include-candidates" type="checkbox" checked={memoryConsolidateIncludeCandidates} onChange={(event) => onMemoryConsolidateIncludeCandidatesChange(event.target.checked)} />
                {t("settings.memory.includeCandidates")}
              </label>
              <button type="button" className={primaryButtonClass} data-testid="settings-memory-consolidate" onClick={onRunMemoryConsolidation} disabled={memoryConsolidating}>
                {memoryConsolidating ? t("settings.memory.consolidating") : t("settings.memory.runConsolidation")}
              </button>
            </div>
            {memoryConsolidationResult ? (
              <div className="mt-3 text-sm">
                <span className="text-muted-foreground">
                  {t("settings.memory.lastRun", {
                    merged: memoryConsolidationResult.run.merged_count,
                    promoted: memoryConsolidationResult.run.promoted_count,
                    conflicts: memoryConsolidationResult.run.conflict_count,
                  })}
                </span>
              </div>
            ) : null}
          </div>

          <div className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
            <div className="grid gap-2">
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.memory.scope")}</span>
                <span className="ml-2 text-foreground">{selectedSessionId || t("settings.memory.workspaceAuthority")}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.memory.searchFields")}</span>
                <span className="ml-2">{arrayOrEmpty(memoryListResponse?.contract.search_fields).join(", ") || "--"}</span>
              </div>
              <div>
                <span className="text-xs tracking-widest uppercase font-semibold">{t("settings.memory.filters")}</span>
                <span className="ml-2">{arrayOrEmpty(memoryListResponse?.contract.filter_query_parameters).join(", ") || "--"}</span>
              </div>
              <div className="mt-1 text-xs">
                {memoryListResponse?.contract.note ||
                  t("settings.memory.contractNoteFallback")}
              </div>
            </div>
          </div>
        </div>
      ) : null}

      {/* Records Tab */}
      {memoryTab === "records" ? (
        <div className="relative">
          <div className="grid gap-2 max-h-[28rem] overflow-y-auto pr-1" data-testid="settings-memory-records">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.records")}</span>
              <span className="text-xs text-muted-foreground">
                {memoryListLoading ? t("settings.common.loading") : t("settings.memory.recordsCount", { count: memoryListResponse?.items.length ?? 0 })}
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
                    data-testid="settings-memory-record"
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
                        <><span>·</span><span>{t("settings.memory.linkedPrefix", { name: item.linked_skill_name })}</span></>
                      ) : null}
                      {item.derived_skill_name ? (
                        <><span>·</span><span>{t("settings.memory.targetPrefix", { name: item.derived_skill_name })}</span></>
                      ) : null}
                    </div>
                    <strong className="text-sm">{item.title}</strong>
                    <span className="text-xs text-muted-foreground line-clamp-2">{item.summary}</span>
                  </button>
                );
              })
            ) : (
              <div className={mutedCardClass} data-testid="settings-memory-records-empty">{t("settings.memory.recordsEmpty")}</div>
            )}
          </div>

          {selectedMemoryId && memoryDetail && !memoryDetailLoading ? (
            <div className="absolute inset-2 z-10 overflow-hidden rounded-4xl border border-border/60 bg-background/98 shadow-2xl backdrop-blur-sm">
              <div className="flex items-center gap-3 border-b border-border/60 px-4 py-3">
                <button type="button" className="text-xs text-muted-foreground transition-colors hover:text-foreground" onClick={() => onSelectMemoryId("")}>
                  {t("settings.memory.backToRecords")}
                </button>
                <span className="truncate text-xs text-muted-foreground">{memoryRecordIdValue(memoryDetail.record.id)}</span>
              </div>
              <div className="grid max-h-full gap-4 overflow-y-auto px-4 py-4">
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
                      <span>{t("settings.memory.linkedPrefix", { name: memoryDetail.record.linked_skill_name || "--" })}</span>
                      <span>{t("settings.memory.targetPrefix", { name: memoryDetail.record.derived_skill_name || "--" })}</span>
                    </div>
                  ) : null}
                </div>

                <details open>
                  <summary className="cursor-pointer list-none">
                    <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.recordSemantics")}</span>
                  </summary>
                  <div className="mt-2 grid gap-3 sm:grid-cols-3">
                    <div className="grid gap-1">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.triggers")}</span>
                      {arrayOrEmpty(memoryDetail.record.trigger_conditions).length
                        ? arrayOrEmpty(memoryDetail.record.trigger_conditions).map((v) => (<span key={v} className="text-sm">{v}</span>))
                        : <span className="text-sm text-muted-foreground">--</span>}
                    </div>
                    <div className="grid gap-1">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.boundaries")}</span>
                      {arrayOrEmpty(memoryDetail.record.boundaries).length
                        ? arrayOrEmpty(memoryDetail.record.boundaries).map((v) => (<span key={v} className="text-sm">{v}</span>))
                        : <span className="text-sm text-muted-foreground">--</span>}
                    </div>
                    <div className="grid gap-1">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.normalizedFacts")}</span>
                      {arrayOrEmpty(memoryDetail.record.normalized_facts).length
                        ? arrayOrEmpty(memoryDetail.record.normalized_facts).map((v) => (<span key={v} className="text-sm">{v}</span>))
                        : <span className="text-sm text-muted-foreground">--</span>}
                    </div>
                  </div>
                </details>

                <details>
                  <summary className="cursor-pointer list-none">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.validationConflicts")}</span>
                      <span className="text-xs text-muted-foreground">{t("settings.memory.conflictsCount", { count: arrayOrEmpty(memoryConflicts?.conflicts).length })}</span>
                    </div>
                  </summary>
                  <div className="mt-2 grid gap-4 sm:grid-cols-2">
                    <div className="grid gap-2">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.validation")}</span>
                      {memoryValidationReport?.latest ? (
                        <div className="grid gap-1 text-sm">
                          <strong>{memoryValidationReport.latest.status}</strong>
                          <span className="text-muted-foreground">{t("settings.memory.checkedAt", { time: unixTimeLabel(memoryValidationReport.latest.checked_at) })}</span>
                          {arrayOrEmpty(memoryValidationReport.latest.issues).length
                            ? arrayOrEmpty(memoryValidationReport.latest.issues).map((issue) => (<span key={issue}>{issue}</span>))
                            : <span className="text-muted-foreground">{t("settings.memory.noIssues")}</span>}
                        </div>
                      ) : <span className="text-sm text-muted-foreground">{t("settings.memory.noValidationReport")}</span>}
                    </div>
                    <div className="grid gap-2">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.conflicts")}</span>
                      {arrayOrEmpty(memoryConflicts?.conflicts).length ? (
                        <div className="grid gap-2">
                          {arrayOrEmpty(memoryConflicts?.conflicts).map((conflict) => (
                            <div key={conflict.id} className="rounded bg-muted/30 px-3 py-2 text-sm">
                              <strong className="block">{conflict.conflict_kind}</strong>
                              <span className="block text-muted-foreground">{t("settings.memory.otherPrefix", { id: memoryRecordIdValue(conflict.other_record_id) })}</span>
                              <span className="block">{conflict.detail}</span>
                            </div>
                          ))}
                        </div>
                      ) : <span className="text-sm text-muted-foreground">{t("settings.memory.noConflicts")}</span>}
                    </div>
                  </div>
                </details>

                <details>
                  <summary className="cursor-pointer list-none">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t("settings.memory.evidence")}</span>
                      <span className="text-xs text-muted-foreground">{t("settings.memory.refsCount", { count: arrayOrEmpty(memoryDetail.record.evidence_refs).length })}</span>
                    </div>
                  </summary>
                  <div className="mt-2 grid gap-2">
                    {arrayOrEmpty(memoryDetail.record.evidence_refs).length ? (
                      arrayOrEmpty(memoryDetail.record.evidence_refs).map((ref, index) => (
                        <div key={`${ref.session_id ?? "session"}-${index}`} className="rounded bg-muted/30 px-3 py-2 text-sm">
                          <div>{t("settings.memory.evidenceSession", { id: ref.session_id || "--" })}</div>
                          <div>{t("settings.memory.evidenceMessage", { id: ref.message_id || "--" })}</div>
                          <div>{t("settings.memory.evidenceTool", { id: ref.tool_call_id || "--" })}</div>
                          <div>{t("settings.memory.evidenceStage", { id: ref.stage_id || "--" })}</div>
                          {ref.note ? <div className="text-muted-foreground">{t("settings.memory.evidenceNote", { value: ref.note })}</div> : null}
                        </div>
                      ))
                    ) : <span className="text-sm text-muted-foreground">{t("settings.memory.noEvidenceRefs")}</span>}
                  </div>
                </details>
              </div>
            </div>
          ) : null}

          {selectedMemoryId && memoryDetailLoading ? (
            <div className="absolute inset-2 z-10 flex items-center justify-center rounded-4xl border border-border/50 bg-background/85 backdrop-blur-sm">
              <span className="text-sm text-muted-foreground">{t("settings.memory.loadingDetail")}</span>
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Governance Tab */}
      {memoryTab === "governance" ? (
        <div className="grid gap-5">
          <div data-testid="settings-memory-rule-packs">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.rulePacks")}</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? t("settings.common.loading") : t("settings.memory.packsCount", { count: memoryRulePacks?.items?.length ?? 0 })}</span>
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
                            {rule.promotion_target ? <span className="block text-xs text-muted-foreground">{t("settings.memory.promotionTarget", { value: rule.promotion_target })}</span> : null}
                          </div>
                        ))}
                      </div>
                    ) : <span className="mt-1 block text-xs text-muted-foreground">{t("settings.memory.noRules")}</span>}
                  </div>
                ))
              ) : <div className={mutedCardClass} data-testid="settings-memory-rule-packs-empty">{memoryGovernanceLoading ? t("settings.memory.loadingRulePacks") : t("settings.memory.noRulePacks")}</div>}
            </div>
          </div>

          <div data-testid="settings-memory-rule-hits">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.recentRuleHits")}</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? t("settings.common.loading") : t("settings.memory.hitsCount", { count: memoryRuleHits?.items?.length ?? 0 })}</span>
            </div>
            <div className="grid gap-2">
              {arrayOrEmpty(memoryRuleHits?.items).length ? (
                arrayOrEmpty(memoryRuleHits?.items).map((hit) => (
                  <div key={hit.id} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                    <strong className="block text-sm">{hit.hit_kind}</strong>
                    <div className="mt-1 grid gap-0.5 text-sm text-muted-foreground">
                      <span>{t("settings.memory.hitRun", { id: hit.run_id || "--" })}</span>
                      <span>{t("settings.memory.hitRecord", { id: memoryRecordIdValue(hit.memory_id) })}</span>
                      <span>{t("settings.memory.hitPack", { id: hit.rule_pack_id || "--" })}</span>
                      <span>{unixTimeLabel(hit.created_at)}</span>
                      {hit.detail ? <span>{hit.detail}</span> : null}
                    </div>
                  </div>
                ))
              ) : <div className={mutedCardClass} data-testid="settings-memory-rule-hits-empty">{memoryGovernanceLoading ? t("settings.memory.loadingRuleHits") : t("settings.memory.noRuleHits")}</div>}
            </div>
          </div>

          <div data-testid="settings-memory-consolidation-runs">
            <div className="flex items-center justify-between gap-3 mb-2">
              <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.consolidationRuns")}</span>
              <span className="text-xs text-muted-foreground">{memoryGovernanceLoading ? t("settings.common.loading") : t("settings.memory.runsCount", { count: memoryConsolidationRuns?.items?.length ?? 0 })}</span>
            </div>
            <div className="grid gap-2">
              {memoryConsolidationResult ? (
                <div className="border-l-2 border-l-foreground bg-foreground/5 px-4 py-3">
                  <strong className="block text-sm">{t("settings.memory.latestConsolidation")}</strong>
                  <div className="mt-1 grid gap-0.5 text-sm text-muted-foreground">
                    <span>{t("settings.memory.hitRun", { id: memoryConsolidationResult.run.run_id })}</span>
                    <span>{t("settings.memory.consolidationSummary", { merged: memoryConsolidationResult.run.merged_count, promoted: memoryConsolidationResult.run.promoted_count, conflicts: memoryConsolidationResult.run.conflict_count })}</span>
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
                      <span>{t("settings.memory.consolidationSummary", { merged: run.merged_count, promoted: run.promoted_count, conflicts: run.conflict_count })}</span>
                      <span>{t("settings.memory.startedAt", { time: unixTimeLabel(run.started_at) })}</span>
                      <span>{t("settings.memory.finishedAt", { time: run.finished_at ? unixTimeLabel(run.finished_at) : "--" })}</span>
                    </div>
                  </div>
                ))
              ) : <div className={mutedCardClass} data-testid="settings-memory-consolidation-runs-empty">{memoryGovernanceLoading ? t("settings.memory.loadingConsolidationRuns") : t("settings.memory.noConsolidationRuns")}</div>}
            </div>
          </div>
        </div>
      ) : null}

      {/* Retrieval Tab */}
      {memoryTab === "retrieval" ? (
        <div className="grid gap-4">
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.memory.retrievalPreview")}</span>
            <span className="text-xs text-muted-foreground">{memoryPreviewLoading ? t("settings.memory.loadingPreview") : t("settings.memory.recalledCount", { count: arrayOrEmpty(memoryPreview?.packet.items).length })}</span>
          </div>
          <div className={mutedCardClass}>
            {memoryPreview?.contract.note || t("settings.memory.retrievalNoteFallback")}
          </div>
          <div className="grid gap-3" data-testid="settings-memory-retrieval-items">
            {arrayOrEmpty(memoryPreview?.packet.items).length ? (
              arrayOrEmpty(memoryPreview?.packet.items).map((item) => (
                <div key={memoryRecordIdValue(item.card.id)} className="border-l-2 border-l-foreground/10 bg-muted/30 px-4 py-3">
                  <div className="flex items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                    <span>{item.card.kind}</span><span>·</span><span>{item.card.validation_status}</span>
                  </div>
                  <strong className="mt-1 block text-sm">{item.card.title}</strong>
                  <div className="mt-1 text-sm text-muted-foreground">
                    <div>{t("settings.memory.whyRecalled", { value: item.why_recalled })}</div>
                    <div>{t("settings.memory.summaryPrefix", { value: item.card.summary })}</div>
                    {item.evidence_summary ? <div>{t("settings.memory.evidencePrefix", { value: item.evidence_summary })}</div> : null}
                  </div>
                </div>
              ))
            ) : <div className={mutedCardClass} data-testid="settings-memory-retrieval-empty">{t("settings.memory.retrievalEmpty")}</div>}
          </div>
        </div>
      ) : null}
    </div>
  );
}
