import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { OutputBlock, ToolOutputBlock } from "../lib/history";
import { useAgendaoStore } from "../store";
import {
  canonicalLiveExecutionStatus,
  type LiveExecutionEntry,
} from "../lib/liveExecutionState";
import type {
  ExecutionNodeRecord,
  SessionInsightsRecord,
  SessionTelemetrySnapshotRecord,
} from "../lib/sessionActivity";
import { buildRunTailSummary, type RunTailSummary } from "../lib/runTailSummary";
import { toolActivityLabel } from "../lib/toolLabels";
import { type OutputField, type OutputPreview } from "../lib/history";
import {
  toolDisplayFields,
  toolDisplayPreview,
  toolDisplayRawLabelKey,
  toolDisplaySummary,
  toolExecutionKind,
} from "../lib/toolPresentation";
import { toolIdFromPartKey } from "../lib/liveIdentity";
import { isOptimisticSessionId } from "../lib/session";

interface UseExecutionActivityOptions {
  selectedSessionId: string | null;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onError: (message: string) => void;
  onInfo: (message: string) => void;
  awaitingUser?: boolean;
  pendingPermission?: boolean;
}

const LIVE_EXECUTION_LIMIT = 8;

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}

async function loadExecutionActivityData(
  selectedSessionId: string,
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>,
) {
  const [telemetry, insights] = await Promise.all([
    apiJson<SessionTelemetrySnapshotRecord>(`/session/${selectedSessionId}/telemetry`),
    apiJson<SessionInsightsRecord>(`/session/${selectedSessionId}/insights`),
  ]);
  return { telemetry, insights };
}

function flattenExecutionNodes(nodes: ExecutionNodeRecord[]): ExecutionNodeRecord[] {
  return nodes.flatMap((node) => [node, ...flattenExecutionNodes(node.children ?? [])]);
}

function metadataString(metadata: Record<string, unknown> | null | undefined, key: string) {
  const value = metadata?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function stableToolCallIdFromBlock(block: ToolOutputBlock): string | null {
  if (typeof block.tool_call_id === "string" && block.tool_call_id.trim()) {
    return block.tool_call_id.trim();
  }
  return toolIdFromPartKey(block.live_identity?.part_key) ?? null;
}

// P2-3: Tool display helpers consolidated in lib/toolPresentation.ts
function liveExecutionKind(block: ToolOutputBlock): LiveExecutionEntry["kind"] {
  return toolExecutionKind(block);
}

function liveExecutionStatus(block: ToolOutputBlock): LiveExecutionEntry["status"] {
  const partKind = block.live_identity?.part_kind;
  if (partKind === "tool_call") {
    return "running";
  }
  if (partKind === "tool_result") {
    return canonicalLiveExecutionStatus(block.phase === "error" ? "error" : "done");
  }
  return canonicalLiveExecutionStatus(block.phase);
}

function liveExecutionSummary(block: ToolOutputBlock): string | null {
  // P2-3: Delegates to shared toolDisplaySummary which handles
  // display.summary → summary → direct detail chain.
  return toolDisplaySummary(block);
}

function liveExecutionFields(block: ToolOutputBlock): OutputField[] {
  return toolDisplayFields(block) ?? [];
}

function liveExecutionPreview(block: ToolOutputBlock): OutputPreview | null {
  const { previewText, previewKind, previewTruncated } = toolDisplayPreview(block);
  if (!previewText) return null;
  return {
    kind: previewKind || "text",
    text: previewText,
    truncated: previewTruncated,
  };
}

function liveExecutionStageId(block: ToolOutputBlock): string | null {
  if (typeof block.stage_id === "string" && block.stage_id.trim()) {
    return block.stage_id.trim();
  }
  return metadataString(block.metadata, "stage_id");
}

export function useExecutionActivity({
  selectedSessionId,
  apiJson,
  onError,
  onInfo,
  awaitingUser = false,
  pendingPermission = false,
}: UseExecutionActivityOptions) {
  // Streaming-high-frequency store fields subscribed here (leaf hook) instead
  // of App, so per-SSE-event status changes don't re-render the whole tree.
  const statusLine = useAgendaoStore((s) => s.statusLine);
  const latestRuntimeError = useAgendaoStore((s) => s.latestRuntimeError);
  const [telemetry, setTelemetry] = useState<SessionTelemetrySnapshotRecord | null>(null);
  const [insights, setInsights] = useState<SessionInsightsRecord | null>(null);
  const [activityLoading, setActivityLoading] = useState(false);
  const [selectedExecutionId, setSelectedExecutionId] = useState<string | null>(null);
  const [executionCancellingId, setExecutionCancellingId] = useState<string | null>(null);
  const [liveExecutions, setLiveExecutions] = useState<LiveExecutionEntry[]>([]);
  const sessionRef = useRef<string | null>(selectedSessionId);
  const previousSessionRef = useRef<string | null>(selectedSessionId);

  useEffect(() => {
    sessionRef.current = selectedSessionId;
  }, [selectedSessionId]);

  useEffect(() => {
    if (previousSessionRef.current === selectedSessionId) return;
    previousSessionRef.current = selectedSessionId;
    setTelemetry(null);
    setInsights(null);
    setSelectedExecutionId(null);
    setLiveExecutions([]);
  }, [selectedSessionId]);

  const resetExecutionActivity = useCallback(() => {
    setTelemetry(null);
    setInsights(null);
    setActivityLoading(false);
    setSelectedExecutionId(null);
    setExecutionCancellingId(null);
    setLiveExecutions([]);
  }, []);

  const refreshExecutionActivity = useCallback(
    async (sessionId = sessionRef.current) => {
      if (!sessionId) {
        resetExecutionActivity();
        return;
      }

      setActivityLoading(true);
      try {
        const { telemetry, insights } = await loadExecutionActivityData(sessionId, apiJson);
        if (sessionRef.current !== sessionId) return;
        setTelemetry(telemetry);
        setInsights(insights);
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

  const applyLiveExecutionOutputBlock = useCallback((block: OutputBlock, sessionId = sessionRef.current) => {
    if (!sessionId || block.kind !== "tool") return;

    const label = toolActivityLabel(toolDisplayRawLabelKey(block));
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
    if (isOptimisticSessionId(selectedSessionId)) {
      resetExecutionActivity();
      return;
    }
    const timer = window.setTimeout(() => {
      void refreshExecutionActivity(selectedSessionId);
    }, 220);
    return () => window.clearTimeout(timer);
  }, [refreshExecutionActivity, resetExecutionActivity, selectedSessionId]);
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

  const activeExecution = useMemo(
    () => [...executionNodes].reverse().find((node) => node.status !== "done") ?? null,
    [executionNodes],
  );

  const runTailSummary = useMemo<RunTailSummary>(() => {
    return buildRunTailSummary({
      statusLine,
      runtimeStatus: telemetry?.runtime?.run_status,
      latestRuntimeError,
      awaitingUser,
      pendingPermission,
      usage: telemetry?.usage,
      activeStageName: activeExecution?.label,
    });
  }, [
    activeExecution?.label,
    awaitingUser,
    latestRuntimeError,
    pendingPermission,
    telemetry?.runtime?.run_status,
    statusLine,
    telemetry?.usage,
  ]);

  useEffect(() => {
    if (selectedExecutionId && !executionNodes.some((node) => node.id === selectedExecutionId)) {
      setSelectedExecutionId(null);
    }
  }, [executionNodes, selectedExecutionId]);

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
        await refreshExecutionActivity(sessionId);
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
    activeExecution,
    executionTopology,
    activityLoading,
    executionNodes,
    selectedExecutionId,
    selectedExecution,
    executionCancellingId,
    setSelectedExecutionId,
    cancelExecution,
    refreshExecutionActivity,
    applyLiveExecutionOutputBlock,
    liveExecutions,
    runTailSummary,
  };
}
