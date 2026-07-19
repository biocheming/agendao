import { useCallback, useEffect, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
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
import { memoryRecordIdValue } from "@/lib/memory";
import { useI18n } from "@/i18n/I18nProvider";
import type { SettingsTabId } from "../shared";
import { arrayOrEmpty, formatError } from "../shared";

export interface MemorySettingsState {
  memorySearchDraft: string;
  setMemorySearchDraft: Dispatch<SetStateAction<string>>;
  memoryListLoading: boolean;
  setMemoryListLoading: Dispatch<SetStateAction<boolean>>;
  memoryListResponse: MemoryListResponseRecord | null;
  setMemoryListResponse: Dispatch<SetStateAction<MemoryListResponseRecord | null>>;
  selectedMemoryId: string | null;
  setSelectedMemoryId: Dispatch<SetStateAction<string | null>>;
  memoryDetailLoading: boolean;
  setMemoryDetailLoading: Dispatch<SetStateAction<boolean>>;
  memoryDetail: MemoryDetailResponseRecord | null;
  setMemoryDetail: Dispatch<SetStateAction<MemoryDetailResponseRecord | null>>;
  memoryValidationReport: MemoryValidationReportResponseRecord | null;
  setMemoryValidationReport: Dispatch<
    SetStateAction<MemoryValidationReportResponseRecord | null>
  >;
  memoryConflicts: MemoryConflictResponseRecord | null;
  setMemoryConflicts: Dispatch<SetStateAction<MemoryConflictResponseRecord | null>>;
  memoryPreviewLoading: boolean;
  setMemoryPreviewLoading: Dispatch<SetStateAction<boolean>>;
  memoryPreview: MemoryRetrievalPreviewResponseRecord | null;
  setMemoryPreview: Dispatch<SetStateAction<MemoryRetrievalPreviewResponseRecord | null>>;
  memoryRulePacks: MemoryRulePackListResponseRecord | null;
  setMemoryRulePacks: Dispatch<SetStateAction<MemoryRulePackListResponseRecord | null>>;
  memoryRuleHits: MemoryRuleHitListResponseRecord | null;
  setMemoryRuleHits: Dispatch<SetStateAction<MemoryRuleHitListResponseRecord | null>>;
  memoryConsolidationRuns: MemoryConsolidationRunListResponseRecord | null;
  setMemoryConsolidationRuns: Dispatch<
    SetStateAction<MemoryConsolidationRunListResponseRecord | null>
  >;
  memoryConsolidationResult: MemoryConsolidationResponseRecord | null;
  setMemoryConsolidationResult: Dispatch<
    SetStateAction<MemoryConsolidationResponseRecord | null>
  >;
  memoryGovernanceLoading: boolean;
  setMemoryGovernanceLoading: Dispatch<SetStateAction<boolean>>;
  memoryConsolidating: boolean;
  setMemoryConsolidating: Dispatch<SetStateAction<boolean>>;
  memoryConsolidateIncludeCandidates: boolean;
  setMemoryConsolidateIncludeCandidates: Dispatch<SetStateAction<boolean>>;
}

export function useMemorySettingsState(): MemorySettingsState {
  const [memorySearchDraft, setMemorySearchDraft] = useState("");
  const [memoryListLoading, setMemoryListLoading] = useState(false);
  const [memoryListResponse, setMemoryListResponse] = useState<MemoryListResponseRecord | null>(null);
  const [selectedMemoryId, setSelectedMemoryId] = useState<string | null>(null);
  const [memoryDetailLoading, setMemoryDetailLoading] = useState(false);
  const [memoryDetail, setMemoryDetail] = useState<MemoryDetailResponseRecord | null>(null);
  const [memoryValidationReport, setMemoryValidationReport] =
    useState<MemoryValidationReportResponseRecord | null>(null);
  const [memoryConflicts, setMemoryConflicts] = useState<MemoryConflictResponseRecord | null>(null);
  const [memoryPreviewLoading, setMemoryPreviewLoading] = useState(false);
  const [memoryPreview, setMemoryPreview] = useState<MemoryRetrievalPreviewResponseRecord | null>(null);
  const [memoryRulePacks, setMemoryRulePacks] = useState<MemoryRulePackListResponseRecord | null>(null);
  const [memoryRuleHits, setMemoryRuleHits] = useState<MemoryRuleHitListResponseRecord | null>(null);
  const [memoryConsolidationRuns, setMemoryConsolidationRuns] =
    useState<MemoryConsolidationRunListResponseRecord | null>(null);
  const [memoryConsolidationResult, setMemoryConsolidationResult] =
    useState<MemoryConsolidationResponseRecord | null>(null);
  const [memoryGovernanceLoading, setMemoryGovernanceLoading] = useState(false);
  const [memoryConsolidating, setMemoryConsolidating] = useState(false);
  const [memoryConsolidateIncludeCandidates, setMemoryConsolidateIncludeCandidates] =
    useState(false);

  return {
    memorySearchDraft,
    setMemorySearchDraft,
    memoryListLoading,
    setMemoryListLoading,
    memoryListResponse,
    setMemoryListResponse,
    selectedMemoryId,
    setSelectedMemoryId,
    memoryDetailLoading,
    setMemoryDetailLoading,
    memoryDetail,
    setMemoryDetail,
    memoryValidationReport,
    setMemoryValidationReport,
    memoryConflicts,
    setMemoryConflicts,
    memoryPreviewLoading,
    setMemoryPreviewLoading,
    memoryPreview,
    setMemoryPreview,
    memoryRulePacks,
    setMemoryRulePacks,
    memoryRuleHits,
    setMemoryRuleHits,
    memoryConsolidationRuns,
    setMemoryConsolidationRuns,
    memoryConsolidationResult,
    setMemoryConsolidationResult,
    memoryGovernanceLoading,
    setMemoryGovernanceLoading,
    memoryConsolidating,
    setMemoryConsolidating,
    memoryConsolidateIncludeCandidates,
    setMemoryConsolidateIncludeCandidates,
  };
}

export interface MemorySettingsActionsDeps {
  activeTab: SettingsTabId;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onBanner: (message: string) => void;
  setFeedback: Dispatch<SetStateAction<string | null>>;
  selectedSessionId: string | null;
  memorySearchDraft: string;
  setMemoryListLoading: Dispatch<SetStateAction<boolean>>;
  setMemoryListResponse: Dispatch<SetStateAction<MemoryListResponseRecord | null>>;
  selectedMemoryId: string | null;
  setSelectedMemoryId: Dispatch<SetStateAction<string | null>>;
  setMemoryDetailLoading: Dispatch<SetStateAction<boolean>>;
  setMemoryDetail: Dispatch<SetStateAction<MemoryDetailResponseRecord | null>>;
  setMemoryValidationReport: Dispatch<
    SetStateAction<MemoryValidationReportResponseRecord | null>
  >;
  setMemoryConflicts: Dispatch<SetStateAction<MemoryConflictResponseRecord | null>>;
  setMemoryPreviewLoading: Dispatch<SetStateAction<boolean>>;
  setMemoryPreview: Dispatch<SetStateAction<MemoryRetrievalPreviewResponseRecord | null>>;
  setMemoryRulePacks: Dispatch<SetStateAction<MemoryRulePackListResponseRecord | null>>;
  setMemoryRuleHits: Dispatch<SetStateAction<MemoryRuleHitListResponseRecord | null>>;
  setMemoryConsolidationRuns: Dispatch<
    SetStateAction<MemoryConsolidationRunListResponseRecord | null>
  >;
  setMemoryConsolidationResult: Dispatch<
    SetStateAction<MemoryConsolidationResponseRecord | null>
  >;
  setMemoryGovernanceLoading: Dispatch<SetStateAction<boolean>>;
  setMemoryConsolidating: Dispatch<SetStateAction<boolean>>;
  memoryConsolidateIncludeCandidates: boolean;
}

export interface MemorySettingsActions {
  loadMemoryList: () => Promise<void>;
  loadMemoryPreview: () => Promise<void>;
  loadMemoryGovernance: () => Promise<void>;
  runMemoryConsolidation: () => Promise<void>;
}

export function useMemorySettingsActions({
  activeTab,
  apiJson,
  onBanner,
  setFeedback,
  selectedSessionId,
  memorySearchDraft,
  setMemoryListLoading,
  setMemoryListResponse,
  selectedMemoryId,
  setSelectedMemoryId,
  setMemoryDetailLoading,
  setMemoryDetail,
  setMemoryValidationReport,
  setMemoryConflicts,
  setMemoryPreviewLoading,
  setMemoryPreview,
  setMemoryRulePacks,
  setMemoryRuleHits,
  setMemoryConsolidationRuns,
  setMemoryConsolidationResult,
  setMemoryGovernanceLoading,
  setMemoryConsolidating,
  memoryConsolidateIncludeCandidates,
}: MemorySettingsActionsDeps): MemorySettingsActions {
  const { t } = useI18n();
  const loadMemoryList = useCallback(async () => {
    setMemoryListLoading(true);
    try {
      const params = new URLSearchParams();
      if (memorySearchDraft.trim()) {
        params.set("search", memorySearchDraft.trim());
      }
      params.set("limit", "60");
      if (selectedSessionId) {
        params.set("source_session_id", selectedSessionId);
      }
      const route = memorySearchDraft.trim() ? "/memory/search" : "/memory/list";
      const path = `${route}${params.toString() ? `?${params.toString()}` : ""}`;
      const response = await apiJson<MemoryListResponseRecord>(path);
      response.items = arrayOrEmpty(response.items);
      if (response.contract) {
        response.contract.search_fields = arrayOrEmpty(response.contract.search_fields);
        response.contract.filter_query_parameters = arrayOrEmpty(
          response.contract.filter_query_parameters,
        );
        response.contract.non_search_fields = arrayOrEmpty(response.contract.non_search_fields);
      }
      setMemoryListResponse(response);
      setSelectedMemoryId((current) => {
        if (
          current &&
          response.items.some((item) => memoryRecordIdValue(item.id) === current)
        ) {
          return current;
        }
        return response.items[0] ? memoryRecordIdValue(response.items[0].id) : null;
      });
    } catch (error) {
      const message = t("settings.feedback.memoryListLoadFailed", { error: formatError(error) });
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryListLoading(false);
    }
  }, [
    apiJson,
    memorySearchDraft,
    onBanner,
    selectedSessionId,
    setFeedback,
    setMemoryListLoading,
    setMemoryListResponse,
    setSelectedMemoryId,
    t,
  ]);

  const loadMemoryPreview = useCallback(async () => {
    setMemoryPreviewLoading(true);
    try {
      const params = new URLSearchParams();
      if (memorySearchDraft.trim()) {
        params.set("query", memorySearchDraft.trim());
      }
      params.set("limit", "6");
      if (selectedSessionId) {
        params.set("session_id", selectedSessionId);
      }
      const path = `/memory/retrieval-preview?${params.toString()}`;
      const response = await apiJson<MemoryRetrievalPreviewResponseRecord>(path);
      response.packet.items = arrayOrEmpty(response.packet.items);
      response.packet.scopes = arrayOrEmpty(response.packet.scopes);
      if (response.contract) {
        response.contract.search_fields = arrayOrEmpty(response.contract.search_fields);
        response.contract.filter_query_parameters = arrayOrEmpty(
          response.contract.filter_query_parameters,
        );
        response.contract.non_search_fields = arrayOrEmpty(response.contract.non_search_fields);
      }
      setMemoryPreview(response);
    } catch (error) {
      const message = t("settings.feedback.memoryPreviewLoadFailed", { error: formatError(error) });
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryPreviewLoading(false);
    }
  }, [
    apiJson,
    memorySearchDraft,
    onBanner,
    selectedSessionId,
    setFeedback,
    setMemoryPreview,
    setMemoryPreviewLoading,
    t,
  ]);

  const loadMemoryGovernance = useCallback(async () => {
    setMemoryGovernanceLoading(true);
    try {
      const [rulePacks, ruleHits, runs] = await Promise.all([
        apiJson<MemoryRulePackListResponseRecord>("/memory/rule-packs"),
        apiJson<MemoryRuleHitListResponseRecord>("/memory/rule-hits?limit=30"),
        apiJson<MemoryConsolidationRunListResponseRecord>("/memory/consolidation/runs?limit=20"),
      ]);
      rulePacks.items = arrayOrEmpty(rulePacks.items).map((pack) => ({
        ...pack,
        rules: arrayOrEmpty(pack.rules),
      }));
      ruleHits.items = arrayOrEmpty(ruleHits.items);
      runs.items = arrayOrEmpty(runs.items);
      setMemoryRulePacks(rulePacks);
      setMemoryRuleHits(ruleHits);
      setMemoryConsolidationRuns(runs);
    } catch (error) {
      const message = t("settings.feedback.memoryGovernanceLoadFailed", { error: formatError(error) });
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryGovernanceLoading(false);
    }
  }, [
    apiJson,
    onBanner,
    setFeedback,
    setMemoryConsolidationRuns,
    setMemoryGovernanceLoading,
    setMemoryRuleHits,
    setMemoryRulePacks,
    t,
  ]);

  const runMemoryConsolidation = useCallback(async () => {
    setMemoryConsolidating(true);
    try {
      const response = await apiJson<MemoryConsolidationResponseRecord>("/memory/consolidate", {
        method: "POST",
        body: JSON.stringify({
          include_candidates: memoryConsolidateIncludeCandidates,
        }),
      });
      response.merged_record_ids = arrayOrEmpty(response.merged_record_ids);
      response.promoted_record_ids = arrayOrEmpty(response.promoted_record_ids);
      response.archived_record_ids = arrayOrEmpty(response.archived_record_ids);
      response.reflection_notes = arrayOrEmpty(response.reflection_notes);
      response.rule_hits = arrayOrEmpty(response.rule_hits);
      setMemoryConsolidationResult(response);
      await loadMemoryGovernance();
      await loadMemoryList();
      if (selectedMemoryId) {
        setSelectedMemoryId(selectedMemoryId);
      }
    } catch (error) {
      const message = t("settings.feedback.memoryConsolidationFailed", { error: formatError(error) });
      setFeedback(message);
      onBanner(message);
    } finally {
      setMemoryConsolidating(false);
    }
  }, [
    apiJson,
    loadMemoryGovernance,
    loadMemoryList,
    memoryConsolidateIncludeCandidates,
    onBanner,
    selectedMemoryId,
    setFeedback,
    setMemoryConsolidating,
    setMemoryConsolidationResult,
    setSelectedMemoryId,
    t,
  ]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryList();
  }, [activeTab, loadMemoryList]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryPreview();
  }, [activeTab, loadMemoryPreview]);

  useEffect(() => {
    if (activeTab !== "memory") {
      return;
    }
    void loadMemoryGovernance();
  }, [activeTab, loadMemoryGovernance]);

  useEffect(() => {
    if (activeTab !== "memory" || !selectedMemoryId) {
      setMemoryDetail(null);
      setMemoryValidationReport(null);
      setMemoryConflicts(null);
      setMemoryDetailLoading(false);
      return;
    }

    let cancelled = false;
    setMemoryDetailLoading(true);

    void (async () => {
      try {
        const [detail, validation, conflicts] = await Promise.all([
          apiJson<MemoryDetailResponseRecord>(`/memory/${encodeURIComponent(selectedMemoryId)}`),
          apiJson<MemoryValidationReportResponseRecord>(
            `/memory/${encodeURIComponent(selectedMemoryId)}/validation-report`,
          ),
          apiJson<MemoryConflictResponseRecord>(
            `/memory/${encodeURIComponent(selectedMemoryId)}/conflicts`,
          ),
        ]);
        if (cancelled) return;
        detail.record.trigger_conditions = arrayOrEmpty(detail.record.trigger_conditions);
        detail.record.boundaries = arrayOrEmpty(detail.record.boundaries);
        detail.record.normalized_facts = arrayOrEmpty(detail.record.normalized_facts);
        detail.record.evidence_refs = arrayOrEmpty(detail.record.evidence_refs);
        if (validation.latest) {
          validation.latest.issues = arrayOrEmpty(validation.latest.issues);
        }
        conflicts.conflicts = arrayOrEmpty(conflicts.conflicts);
        setMemoryDetail(detail);
        setMemoryValidationReport(validation);
        setMemoryConflicts(conflicts);
      } catch (error) {
        if (cancelled) return;
        const message = t("settings.feedback.memoryDetailLoadFailed", { error: formatError(error) });
        setFeedback(message);
        onBanner(message);
        setMemoryDetail(null);
        setMemoryValidationReport(null);
        setMemoryConflicts(null);
      } finally {
        if (!cancelled) {
          setMemoryDetailLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    activeTab,
    apiJson,
    onBanner,
    selectedMemoryId,
    setFeedback,
    setMemoryConflicts,
    setMemoryDetail,
    setMemoryDetailLoading,
    setMemoryValidationReport,
    t,
  ]);

  return {
    loadMemoryList,
    loadMemoryPreview,
    loadMemoryGovernance,
    runMemoryConsolidation,
  };
}
