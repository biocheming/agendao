import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ActivityEventRecord,
  ExecutionNodeRecord,
  SessionInsightsRecord,
  SessionTelemetrySnapshotRecord,
} from "../lib/sessionActivity";

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
}

const DEFAULT_FILTERS: ActivityFilters = {
  stageId: "",
  executionId: "",
  eventType: "",
};

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

export function useExecutionActivity({
  selectedSessionId,
  apiJson,
  onError,
  onInfo,
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
        setActivityEvents(events);
        setKnownEventTypes((current) =>
          uniqStrings([...current, ...events.map((event) => event.event_type)]).sort(),
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

  useEffect(() => {
    if (!selectedSessionId) {
      resetExecutionActivity();
      return;
    }
    void refreshExecutionActivity(selectedSessionId, activityFilters, activityPage);
  }, [activityFilters, activityPage, refreshExecutionActivity, resetExecutionActivity, selectedSessionId]);

  const executionTopology = telemetry?.topology ?? null;

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
      return telemetry.stages.find((stage) => stage.stage_id === activeStageId) ?? null;
    }
    return (
      telemetry.stages.find((stage) =>
        ["running", "waiting", "retrying", "blocked", "cancelling"].includes(stage.status),
      ) ?? null
    );
  }, [telemetry]);

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
    stageSummaries: telemetry?.stages ?? [],
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
  };
}
