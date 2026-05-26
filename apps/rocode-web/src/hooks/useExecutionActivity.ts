import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { OutputBlock } from "../lib/history";
import {
  canonicalLiveExecutionStatus,
  type LiveExecutionEntry,
} from "../lib/liveExecutionState";
import type {
  ActivityEventRecord,
  ExecutionNodeRecord,
  SessionInsightsRecord,
  SessionTelemetrySnapshotRecord,
  StageSummaryRecord,
} from "../lib/sessionActivity";
import { isLiveStageStatus } from "../lib/contextPressure";
import { buildRunTailSummary, type RunTailSummary } from "../lib/runTailSummary";
import { isSkillToolName, toolActivityLabel } from "../lib/toolLabels";
import type { OutputField, OutputPreview } from "../lib/history";
import { toolIdFromPartKey } from "../lib/liveIdentity";

export interface ActivityFilters {
  stageId: string;
  executionId: string;
  eventType: string;
}

const ACTIVITY_PAGE_SIZE = 24;

interface UseExecutionActivityOptions {
  selectedSessionId: string | null;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onError: (message: string) => void;
  onInfo: (message: string) => void;
  statusLine?: string;
  latestRuntimeError?: string | null;
  awaitingUser?: boolean;
  pendingPermission?: boolean;
}

const DEFAULT_FILTERS: ActivityFilters = {
  stageId: "",
  executionId: "",
  eventType: "",
};

const LIVE_EXECUTION_LIMIT = 8;

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}

function executionActivityQuery(filters: ActivityFilters, page: number) {
  const search = new URLSearchParams();
  search.set("limit", String(ACTIVITY_PAGE_SIZE));
  search.set("offset", String(Math.max(0, page - 1) * ACTIVITY_PAGE_SIZE));
  if (filters.stageId) search.set("stage_id", filters.stageId);
  if (filters.executionId) search.set("execution_id", filters.executionId);
  if (filters.eventType) search.set("event_type", filters.eventType);
  return search.toString();
}

async function loadExecutionActivityData(
  selectedSessionId: string,
  filters: ActivityFilters,
  page: number,
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>,
) {
  const query = executionActivityQuery(filters, page);
  const [telemetry, insights, events] = await Promise.all([
    apiJson<SessionTelemetrySnapshotRecord>(`/session/${selectedSessionId}/telemetry`),
    apiJson<SessionInsightsRecord>(`/session/${selectedSessionId}/insights`),
    apiJson<ActivityEventRecord[]>(`/session/${selectedSessionId}/events?${query}`),
  ]);
  return { telemetry, insights, events };
}

function flattenExecutionNodes(nodes: ExecutionNodeRecord[]): ExecutionNodeRecord[] {
  return nodes.flatMap((node) => [node, ...flattenExecutionNodes(node.children ?? [])]);
}

function uniqStrings(values: Array<string | null | undefined>) {
  return Array.from(new Set(values.filter((value): value is string => Boolean(value && value.trim()))));
}

function sameActivityFilters(left: ActivityFilters, right: ActivityFilters) {
  return (
    left.stageId === right.stageId &&
    left.executionId === right.executionId &&
    left.eventType === right.eventType
  );
}

function metadataString(metadata: Record<string, unknown> | null | undefined, key: string) {
  const value = metadata?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function stableToolCallIdFromBlock(block: OutputBlock): string | null {
  if (typeof block.tool_call_id === "string" && block.tool_call_id.trim()) {
    return block.tool_call_id.trim();
  }
  const wireLegacyBlockId = block.live_identity?.legacy_block_id?.trim();
  if (wireLegacyBlockId) return wireLegacyBlockId;
  return toolIdFromPartKey(block.live_identity?.part_key) ?? null;
}

function liveExecutionKind(block: OutputBlock): LiveExecutionEntry["kind"] {
  return isSkillToolName(block.name ?? block.title ?? "") ? "skill" : "tool";
}

function liveExecutionStatus(block: OutputBlock): LiveExecutionEntry["status"] {
  const partKind = block.live_identity?.part_kind;
  if (partKind === "tool_call") {
    return "running";
  }
  if (partKind === "tool_result") {
    return canonicalLiveExecutionStatus(block.phase === "error" ? "error" : "done");
  }
  return canonicalLiveExecutionStatus(block.phase);
}

function liveExecutionSummary(block: OutputBlock): string | null {
  // Execution activity prefers the shared display contract. Raw detail/text is
  // only a compatibility fallback for older tool payloads.
  const hasDisplayContract = Boolean(
    block.display?.summary?.trim() ||
    block.display?.fields?.length ||
    block.display?.preview?.text?.trim(),
  );
  const candidate = [
    block.display?.summary,
    block.summary,
    metadataString(block.metadata, "skill_name"),
    !hasDisplayContract ? block.detail : null,
    !hasDisplayContract ? block.text : null,
  ].find((value) => typeof value === "string" && value.trim().length > 0);
  return typeof candidate === "string" ? candidate.trim() : null;
}

function liveExecutionFields(block: OutputBlock): OutputField[] {
  return Array.isArray(block.display?.fields) ? block.display.fields : [];
}

function liveExecutionPreview(block: OutputBlock): OutputPreview | null {
  const displayPreview = block.display?.preview;
  const hasDisplayContract = Boolean(
    block.display?.summary?.trim() ||
    block.display?.fields?.length ||
    displayPreview?.text?.trim(),
  );
  if (displayPreview?.text?.trim()) {
    return {
      kind: displayPreview.kind?.trim() || "text",
      text: displayPreview.text.trim(),
      truncated: Boolean(displayPreview.truncated),
    };
  }
  if (!hasDisplayContract && block.preview?.trim()) {
    return {
      kind: "text",
      text: block.preview.trim(),
      truncated: false,
    };
  }
  return null;
}

function liveExecutionStageId(block: OutputBlock): string | null {
  if (typeof block.stage_id === "string" && block.stage_id.trim()) {
    return block.stage_id.trim();
  }
  return metadataString(block.metadata, "stage_id");
}

function stageSummaryFromOutputBlock(block: OutputBlock): StageSummaryRecord | null {
  if (block.kind !== "scheduler_stage" || !block.stage_id || !block.stage) {
    return null;
  }

  return {
    stage_id: block.stage_id,
    stage_name: block.stage,
    index: block.stage_index ?? null,
    total: block.stage_total ?? null,
    step: block.step ?? null,
    step_total: null,
    status: block.status ?? "running",
    prompt_tokens: block.prompt_tokens ?? null,
    context_tokens: block.prompt_tokens ?? null,
    completion_tokens: block.completion_tokens ?? null,
    reasoning_tokens: block.reasoning_tokens ?? null,
    cache_read_tokens: block.cache_read_tokens ?? null,
    cache_miss_tokens: block.cache_miss_tokens ?? null,
    cache_write_tokens: block.cache_write_tokens ?? null,
    focus: block.focus ?? null,
    last_event: block.last_event ?? null,
    waiting_on: block.waiting_on ?? null,
    activity: block.activity ?? null,
    estimated_context_tokens:
      typeof block.prompt_tokens === "number" ? block.prompt_tokens : null,
    skill_tree_budget: null,
    skill_tree_truncation_strategy: null,
    skill_tree_truncated: null,
    retry_attempt: null,
    active_agent_count: Array.isArray(block.active_agents) ? block.active_agents.length : 0,
    active_tool_count: 0,
    attached_session_count: block.attached_session_id ? 1 : 0,
    primary_attached_session_id: block.attached_session_id ?? null,
  };
}

export function useExecutionActivity({
  selectedSessionId,
  apiJson,
  onError,
  onInfo,
  statusLine = "ready",
  latestRuntimeError = null,
  awaitingUser = false,
  pendingPermission = false,
}: UseExecutionActivityOptions) {
  const [telemetry, setTelemetry] = useState<SessionTelemetrySnapshotRecord | null>(null);
  const [insights, setInsights] = useState<SessionInsightsRecord | null>(null);
  const [activityEvents, setActivityEvents] = useState<ActivityEventRecord[]>([]);
  const [activityLoading, setActivityLoading] = useState(false);
  const [activityFilters, setActivityFilters] = useState<ActivityFilters>(DEFAULT_FILTERS);
  const [activityPage, setActivityPage] = useState(1);
  const [selectedExecutionId, setSelectedExecutionId] = useState<string | null>(null);
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);
  const [knownEventTypes, setKnownEventTypes] = useState<string[]>([]);
  const [executionCancellingId, setExecutionCancellingId] = useState<string | null>(null);
  const [liveExecutions, setLiveExecutions] = useState<LiveExecutionEntry[]>([]);
  const sessionRef = useRef<string | null>(selectedSessionId);
  const previousSessionRef = useRef<string | null>(selectedSessionId);
  const filtersRef = useRef<ActivityFilters>(DEFAULT_FILTERS);
  const pageRef = useRef(1);

  useEffect(() => {
    sessionRef.current = selectedSessionId;
  }, [selectedSessionId]);

  useEffect(() => {
    if (previousSessionRef.current === selectedSessionId) return;
    previousSessionRef.current = selectedSessionId;
    setTelemetry(null);
    setInsights(null);
    setActivityEvents([]);
    setActivityFilters(DEFAULT_FILTERS);
    setActivityPage(1);
    setSelectedExecutionId(null);
    setSelectedEventId(null);
    setKnownEventTypes([]);
    setLiveExecutions([]);
  }, [selectedSessionId]);

  useEffect(() => {
    filtersRef.current = activityFilters;
  }, [activityFilters]);

  useEffect(() => {
    pageRef.current = activityPage;
  }, [activityPage]);

  const resetExecutionActivity = useCallback(() => {
    setTelemetry(null);
    setInsights(null);
    setActivityEvents([]);
    setActivityLoading(false);
    setActivityFilters(DEFAULT_FILTERS);
    setActivityPage(1);
    setSelectedExecutionId(null);
    setSelectedEventId(null);
    setKnownEventTypes([]);
    setExecutionCancellingId(null);
    setLiveExecutions([]);
  }, []);

  const refreshExecutionActivity = useCallback(
    async (sessionId = sessionRef.current, filters = filtersRef.current, page = pageRef.current) => {
      if (!sessionId) {
        resetExecutionActivity();
        return;
      }

      setActivityLoading(true);
      try {
        const { telemetry, insights, events } = await loadExecutionActivityData(
          sessionId,
          filters,
          page,
          apiJson,
        );
        if (sessionRef.current !== sessionId) return;
        setTelemetry(telemetry);
        setInsights(insights);
        const safeEvents = Array.isArray(events) ? events : [];
        setActivityEvents(safeEvents);
        setKnownEventTypes((current) =>
          uniqStrings([...current, ...safeEvents.map((event) => event.event_type)]).sort(),
        );
      } catch (error) {
        if (sessionRef.current === sessionId) {
          onError(`Failed to load execution activity: ${formatError(error)}`);
        }
      } finally {
        if (sessionRef.current === sessionId) {
          setActivityLoading(false);
        }
      }
    },
    [apiJson, onError, resetExecutionActivity],
  );

  const applySchedulerStageOutputBlock = useCallback((block: OutputBlock, sessionId = sessionRef.current) => {
    if (!sessionId) return;
    const summary = stageSummaryFromOutputBlock(block);
    if (!summary) return;

    setTelemetry((current) => {
      if (!current) return current;
      if (current.runtime?.session_id !== sessionId) return current;

      const nextStages = [...(Array.isArray(current.stages) ? current.stages : [])];
      const existingIndex = nextStages.findIndex((stage) => stage.stage_id === summary.stage_id);
      if (existingIndex >= 0) {
        nextStages[existingIndex] = summary;
      } else {
        nextStages.push(summary);
        nextStages.sort((left, right) => {
          const leftIndex = left.index ?? Number.MAX_SAFE_INTEGER;
          const rightIndex = right.index ?? Number.MAX_SAFE_INTEGER;
          if (leftIndex !== rightIndex) return leftIndex - rightIndex;
          return left.stage_id.localeCompare(right.stage_id);
        });
      }

      return {
        ...current,
        stages: nextStages,
      };
    });
  }, []);

  const applyLiveExecutionOutputBlock = useCallback((block: OutputBlock, sessionId = sessionRef.current) => {
    if (!sessionId || block.kind !== "tool") return;

    const label = toolActivityLabel(block.name ?? block.title ?? "tool");
    const toolCallId = stableToolCallIdFromBlock(block);
    const stageId = liveExecutionStageId(block);
    const id = toolCallId ?? `${label}:${stageId ?? "root"}`;
    const next: LiveExecutionEntry = {
      id,
      label,
      status: liveExecutionStatus(block),
      kind: liveExecutionKind(block),
      summary: liveExecutionSummary(block),
      fields: liveExecutionFields(block),
      preview: liveExecutionPreview(block),
      toolCallId,
      stageId,
      updatedAt: Date.now(),
    };

    setLiveExecutions((current) => {
      const filtered = current.filter((entry) => entry.id !== id);
      return [next, ...filtered]
        .sort((left, right) => right.updatedAt - left.updatedAt)
        .slice(0, LIVE_EXECUTION_LIMIT);
    });
  }, []);

  useEffect(() => {
    if (!selectedSessionId) {
      resetExecutionActivity();
      return;
    }
    void refreshExecutionActivity(selectedSessionId, activityFilters, activityPage);
  }, [activityFilters, activityPage, refreshExecutionActivity, resetExecutionActivity, selectedSessionId]);

  const telemetryStages = Array.isArray(telemetry?.stages) ? telemetry.stages : [];
  const executionTopology = telemetry?.topology
    ? {
        ...telemetry.topology,
        roots: Array.isArray(telemetry.topology.roots) ? telemetry.topology.roots : [],
      }
    : null;

  const executionNodes = useMemo(
    () => flattenExecutionNodes(executionTopology?.roots ?? []),
    [executionTopology?.roots],
  );

  const selectedExecution = useMemo(
    () => executionNodes.find((node) => node.id === selectedExecutionId) ?? null,
    [executionNodes, selectedExecutionId],
  );

  const selectedEvent = useMemo(
    () => activityEvents.find((event) => event.event_id === selectedEventId) ?? null,
    [activityEvents, selectedEventId],
  );

  const activeStageSummary = useMemo(() => {
    if (!telemetry) return null;
    const activeStageId = telemetry.runtime.active_stage_id;
    if (activeStageId) {
      return telemetryStages.find((stage) => stage.stage_id === activeStageId) ?? null;
    }
    return telemetryStages.find((stage) => isLiveStageStatus(stage.status)) ?? null;
  }, [telemetry, telemetryStages]);

  const runTailSummary = useMemo<RunTailSummary>(() => {
    return buildRunTailSummary({
      statusLine,
      runtimeStatus: telemetry?.runtime?.run_status,
      latestRuntimeError,
      awaitingUser,
      pendingPermission,
      usage: telemetry?.usage,
      activeStageName: activeStageSummary?.stage_name,
    });
  }, [
    activeStageSummary?.stage_name,
    awaitingUser,
    latestRuntimeError,
    pendingPermission,
    telemetry?.runtime?.run_status,
    statusLine,
    telemetry?.usage,
  ]);

  const stageOptions = useMemo(
    () =>
      uniqStrings([
        ...executionNodes.map((node) => node.stage_id),
        ...activityEvents.map((event) => event.stage_id ?? undefined),
        activityFilters.stageId,
        selectedExecution?.stage_id,
        selectedEvent?.stage_id ?? undefined,
      ]).sort(),
    [activityEvents, activityFilters.stageId, executionNodes, selectedEvent?.stage_id, selectedExecution?.stage_id],
  );

  useEffect(() => {
    if (selectedExecutionId && !executionNodes.some((node) => node.id === selectedExecutionId)) {
      setSelectedExecutionId(null);
    }
  }, [executionNodes, selectedExecutionId]);

  useEffect(() => {
    if (selectedEventId && !activityEvents.some((event) => event.event_id === selectedEventId)) {
      setSelectedEventId(null);
    }
  }, [activityEvents, selectedEventId]);

  const patchActivityFilters = useCallback((patch: Partial<ActivityFilters>) => {
    setSelectedEventId(null);
    setActivityPage(1);
    setActivityFilters((current) => {
      const next = { ...current, ...patch };
      return sameActivityFilters(current, next) ? current : next;
    });
  }, []);

  const clearActivityFilters = useCallback(() => {
    setSelectedEventId(null);
    setActivityPage(1);
    setActivityFilters((current) =>
      sameActivityFilters(current, DEFAULT_FILTERS) ? current : DEFAULT_FILTERS,
    );
  }, []);

  const goToActivityPage = useCallback((page: number) => {
    setSelectedEventId(null);
    setActivityPage((current) => {
      const next = Math.max(1, Math.trunc(page) || 1);
      return current === next ? current : next;
    });
  }, []);

  const nextActivityPage = useCallback(() => {
    setSelectedEventId(null);
    setActivityPage((current) => current + 1);
  }, []);

  const previousActivityPage = useCallback(() => {
    setSelectedEventId(null);
    setActivityPage((current) => Math.max(1, current - 1));
  }, []);

  const firstActivityPage = useCallback(() => {
    setSelectedEventId(null);
    setActivityPage(1);
  }, []);

  const cancelExecution = useCallback(
    async (executionId = selectedExecutionId, sessionId = sessionRef.current) => {
      if (!sessionId || !executionId) return;
      setExecutionCancellingId(executionId);
      try {
        const response = await apiJson<{ cancelled?: boolean; error?: string }>(
          `/session/${sessionId}/executions/${encodeURIComponent(executionId)}/cancel`,
          { method: "POST" },
        );
        if (!response.cancelled) {
          throw new Error(response.error || "execution not found");
        }
        onInfo(`Cancelling ${executionId}`);
        await refreshExecutionActivity(sessionId, filtersRef.current, pageRef.current);
      } catch (error) {
        onError(`Failed to cancel execution: ${formatError(error)}`);
      } finally {
        setExecutionCancellingId((current) => (current === executionId ? null : current));
      }
    },
    [apiJson, onError, onInfo, refreshExecutionActivity, selectedExecutionId],
  );

  return {
    telemetry,
    sessionInsights: insights,
    sessionRuntime: telemetry?.runtime ?? null,
    sessionUsage: telemetry?.usage ?? null,
    sessionMemory: telemetry?.memory ?? null,
    stageSummaries: telemetryStages,
    activeStageSummary,
    executionTopology,
    activityEvents,
    activityLoading,
    activityFilters,
    activityPage,
    activityPageSize: ACTIVITY_PAGE_SIZE,
    activityHasPreviousPage: activityPage > 1,
    activityHasNextPage: activityEvents.length >= ACTIVITY_PAGE_SIZE,
    knownEventTypes,
    stageOptions,
    executionNodes,
    selectedExecutionId,
    selectedExecution,
    executionCancellingId,
    selectedEventId,
    selectedEvent,
    setSelectedExecutionId,
    setSelectedEventId,
    patchActivityFilters,
    clearActivityFilters,
    goToActivityPage,
    nextActivityPage,
    previousActivityPage,
    firstActivityPage,
    cancelExecution,
    refreshExecutionActivity,
    applySchedulerStageOutputBlock,
    applyLiveExecutionOutputBlock,
    liveExecutions,
    runTailSummary,
  };
}
