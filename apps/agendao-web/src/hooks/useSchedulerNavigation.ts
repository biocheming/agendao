import type { Dispatch, SetStateAction } from "react";
import { useCallback, useEffect, useMemo } from "react";
import type { SessionRecord } from "../lib/session";
import type { ConversationJumpTarget } from "./useConversationJump";
import type { useExecutionActivity } from "./useExecutionActivity";
import { useAgendaoStore } from "../store";

interface UseSchedulerNavigationOptions {
  sessions: SessionRecord[];
  selectedSessionId: string | null;
  currentSession: SessionRecord | null;
  setSessions: Dispatch<SetStateAction<SessionRecord[]>>;
  setSelectedSessionId: Dispatch<SetStateAction<string | null>>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  setBanner: (message: string) => void;
  executionActivity: ReturnType<typeof useExecutionActivity>;
  jumpToConversationTarget: (target: ConversationJumpTarget) => void;
  queueConversationJumpTarget: (target: ConversationJumpTarget) => void;
}

interface StageNavigationContext extends ConversationJumpTarget {
  sessionId?: string | null;
}

export interface SessionBreadcrumb {
  sessionId: string;
  title: string;
  viaLabel?: string | null;
  viaStageId?: string | null;
  viaToolCallId?: string | null;
}

export interface BreadcrumbProvenance {
  sourceSessionId: string;
  sourceSessionTitle: string;
  label?: string | null;
  stageId?: string | null;
  toolCallId?: string | null;
}

interface StageFocusOptions {
  executionId?: string | null;
  toolCallId?: string | null;
  label?: string | null;
  silent?: boolean;
  sessionId?: string | null;
}

interface AttachedSessionNavigateContext {
  stageId?: string | null;
  toolCallId?: string | null;
  label?: string | null;
}

function normalizeSession(session: SessionRecord): SessionRecord {
  return {
    ...session,
    title: session.title || "(untitled)",
    updated: session.time?.updated ?? session.updated ?? Date.now(),
  };
}

function upsertSession(current: SessionRecord[], incoming: SessionRecord) {
  return [incoming, ...current.filter((session) => session.id !== incoming.id)];
}

export function useSchedulerNavigation({
  sessions,
  selectedSessionId,
  currentSession,
  setSessions,
  setSelectedSessionId,
  apiJson,
  setBanner,
  executionActivity,
  jumpToConversationTarget,
  queueConversationJumpTarget,
}: UseSchedulerNavigationOptions) {
  const activeStageContext = useAgendaoStore((s) => s.activeStageContext) as StageNavigationContext | null;
  const setActiveStageContext = useAgendaoStore((s) => s.setActiveStageContext);
  const previewStageId = useAgendaoStore((s) => s.previewStageId);
  const setPreviewStageId = useAgendaoStore((s) => s.setPreviewStageId);
  const sessionBreadcrumbs = useAgendaoStore((s) => s.sessionBreadcrumbs);
  const setSessionBreadcrumbs = useAgendaoStore((s) => s.setSessionBreadcrumbs);
  const currentBreadcrumbProvenanceFor = useAgendaoStore((s) => s.currentBreadcrumbProvenanceFor);

  const sessionForId = useCallback(
    (sessionId: string | null | undefined) => {
      if (!sessionId) return null;
      if (currentSession?.id === sessionId) return currentSession;
      return sessions.find((session) => session.id === sessionId) ?? null;
    },
    [currentSession, sessions],
  );

  const breadcrumbForSession = useCallback(
    (sessionId: string, session?: SessionRecord | null): SessionBreadcrumb => ({
      sessionId,
      title: session?.title || sessionForId(sessionId)?.title || "(untitled)",
    }),
    [sessionForId],
  );

  const focusStageInActivity = useCallback(
    (stageId: string, preferredExecutionId?: string | null) => {
      if (!stageId.trim()) return;
      const matchingNode =
        (preferredExecutionId
          ? executionActivity.executionNodes.find((node) => node.id === preferredExecutionId)
          : null) ||
        executionActivity.executionNodes.find((node) => node.stage_id === stageId) ||
        executionActivity.executionNodes.find((node) => node.id === stageId) ||
        null;
      if (matchingNode) {
        executionActivity.setSelectedExecutionId(matchingNode.id);
      }
      executionActivity.patchActivityFilters({ stageId, executionId: "" });
    },
    [executionActivity],
  );

  const currentTrail = useCallback(() => {
    if (!selectedSessionId) return [];
    if (!sessionBreadcrumbs.length) {
      return [breadcrumbForSession(selectedSessionId, currentSession)];
    }
    const selectedIndex = sessionBreadcrumbs.findIndex((crumb) => crumb.sessionId === selectedSessionId);
    if (selectedIndex >= 0) {
      return sessionBreadcrumbs.slice(0, selectedIndex + 1);
    }
    return [breadcrumbForSession(selectedSessionId, currentSession)];
  }, [breadcrumbForSession, currentSession, selectedSessionId, sessionBreadcrumbs]);

  useEffect(() => {
    if (!selectedSessionId) {
      setSessionBreadcrumbs([]);
      return;
    }

    setSessionBreadcrumbs((current) => {
      const session = sessionForId(selectedSessionId);
      if (!current.length) {
        return [breadcrumbForSession(selectedSessionId, session)];
      }

      const index = current.findIndex((crumb) => crumb.sessionId === selectedSessionId);
      if (index >= 0) {
        return current.slice(0, index + 1).map((crumb, crumbIndex) =>
          crumbIndex === index ? { ...crumb, title: session?.title || crumb.title } : crumb,
        );
      }

      return [breadcrumbForSession(selectedSessionId, session)];
    });
  }, [breadcrumbForSession, selectedSessionId, sessionForId, setSessionBreadcrumbs]);

  useEffect(() => {
    setPreviewStageId(null);
  }, [selectedSessionId, setPreviewStageId]);

  useEffect(() => {
    if (!activeStageContext?.stageId || activeStageContext.sessionId !== selectedSessionId) {
      return;
    }
    focusStageInActivity(activeStageContext.stageId, activeStageContext.executionId ?? null);
  }, [
    activeStageContext?.executionId,
    activeStageContext?.sessionId,
    activeStageContext?.stageId,
    focusStageInActivity,
    selectedSessionId,
  ]);

  const focusStage = useCallback(
    (stageId: string, options: StageFocusOptions = {}) => {
      if (!stageId.trim()) return;
      setPreviewStageId(null);
      setActiveStageContext({
        stageId,
        executionId: options.executionId ?? null,
        toolCallId: options.toolCallId ?? null,
        label: options.label ?? stageId,
        sessionId: options.sessionId ?? selectedSessionId ?? null,
      });
      focusStageInActivity(stageId, options.executionId ?? null);
      if (!options.silent) {
        setBanner(`Focused stage ${stageId}`);
      }
    },
    [focusStageInActivity, selectedSessionId, setActiveStageContext, setBanner, setPreviewStageId],
  );

  const previewStage = useCallback((stageId: string | null | undefined) => {
    setPreviewStageId(stageId?.trim() ? stageId : null);
  }, [setPreviewStageId]);

  const navigateToStage = useCallback(
    (stageId: string) => {
      focusStage(stageId);
    },
    [focusStage],
  );

  const navigateToToolCall = useCallback(
    (toolCallId: string, context?: { executionId?: string | null; stageId?: string | null }) => {
      if (!toolCallId.trim()) return;
      if (context?.stageId) {
        focusStage(context.stageId, {
          executionId: context.executionId ?? null,
          toolCallId,
          label: toolCallId,
          silent: true,
        });
      }
      jumpToConversationTarget({
        toolCallId,
        executionId: context?.executionId ?? null,
        stageId: context?.stageId ?? null,
        label: toolCallId,
      });
    },
    [focusStage, jumpToConversationTarget],
  );

  const navigateToAttachedSession = useCallback(
    async (sessionId: string, context?: AttachedSessionNavigateContext) => {
      if (!sessionId.trim()) return;

      let nextSession = sessions.find((session) => session.id === sessionId) ?? null;
      if (!nextSession) {
        try {
          nextSession = normalizeSession(await apiJson<SessionRecord>(`/session/${sessionId}`));
          setSessions((current) => upsertSession(current, nextSession!));
        } catch (error) {
          setBanner(
            `Failed to open attached session ${sessionId}: ${error instanceof Error ? error.message : "Unknown error"}`,
          );
          return;
        }
      }

      const trail = currentTrail();
      const sourceSessionId = selectedSessionId;
      const sourceCrumb = sourceSessionId
        ? trail[trail.length - 1] ?? breadcrumbForSession(sourceSessionId, currentSession)
        : null;
      setSessionBreadcrumbs(
        sourceCrumb
          ? [
              ...trail.slice(0, -1),
              {
                ...sourceCrumb,
                viaLabel:
                  context?.label ||
                  (context?.toolCallId ? `tool ${context.toolCallId}` : null) ||
                  (context?.stageId ? `stage ${context.stageId}` : null) ||
                  `session ${sessionId}`,
                viaStageId: context?.stageId ?? null,
                viaToolCallId: context?.toolCallId ?? null,
              },
              breadcrumbForSession(nextSession.id, nextSession),
            ]
          : [breadcrumbForSession(nextSession.id, nextSession)],
      );
      if (context?.stageId && sourceSessionId) {
        setActiveStageContext({
          stageId: context.stageId,
          toolCallId: context.toolCallId ?? null,
          label: context.label ?? context.stageId,
          sessionId: sourceSessionId,
        });
      }
      setSelectedSessionId(nextSession.id);
      setBanner(`Opened session ${nextSession.title || nextSession.id}`);
    },
    [
      apiJson,
      breadcrumbForSession,
      currentSession,
      currentTrail,
      selectedSessionId,
      sessions,
      setActiveStageContext,
      setBanner,
      setSessionBreadcrumbs,
      setSelectedSessionId,
      setSessions,
    ],
  );

  const navigateToSession = useCallback(
    (sessionId: string) => {
      if (!sessionId.trim()) return;
      setSessionBreadcrumbs([breadcrumbForSession(sessionId, sessionForId(sessionId))]);
      setActiveStageContext(null);
      setSelectedSessionId(sessionId);
    },
    [breadcrumbForSession, sessionForId, setActiveStageContext, setSelectedSessionId, setSessionBreadcrumbs],
  );

  const navigateToBreadcrumb = useCallback(
    (sessionId: string) => {
      const index = sessionBreadcrumbs.findIndex((crumb) => crumb.sessionId === sessionId);
      if (index < 0) return;
      const crumb = sessionBreadcrumbs[index];
      setSessionBreadcrumbs(sessionBreadcrumbs.slice(0, index + 1));
      if (crumb.viaStageId) {
        setActiveStageContext({
          stageId: crumb.viaStageId,
          toolCallId: crumb.viaToolCallId ?? null,
          label: crumb.viaLabel ?? crumb.viaStageId,
          sessionId,
        });
        queueConversationJumpTarget({
          stageId: crumb.viaStageId,
          toolCallId: crumb.viaToolCallId ?? null,
          label: crumb.viaLabel ?? crumb.viaStageId,
        });
      } else {
        setActiveStageContext(null);
      }
      setSelectedSessionId(sessionId);
    },
    [queueConversationJumpTarget, sessionBreadcrumbs, setActiveStageContext, setSelectedSessionId, setSessionBreadcrumbs],
  );

  const restoreActiveStage = useCallback(() => {
    if (!activeStageContext?.stageId || activeStageContext.sessionId !== selectedSessionId) {
      return;
    }
    focusStageInActivity(activeStageContext.stageId, activeStageContext.executionId ?? null);
  }, [activeStageContext, focusStageInActivity, selectedSessionId]);

  const syncStageContext = useCallback(
    (context: StageNavigationContext | null) => {
      if (!context?.stageId) return;
      focusStage(context.stageId, {
        executionId: context.executionId ?? null,
        toolCallId: context.toolCallId ?? null,
        label: context.label ?? context.stageId,
        silent: true,
        sessionId: context.sessionId ?? selectedSessionId ?? null,
      });
    },
    [focusStage, selectedSessionId],
  );

  const currentBreadcrumbProvenance = useMemo(
    () => currentBreadcrumbProvenanceFor(selectedSessionId),
    [currentBreadcrumbProvenanceFor, selectedSessionId],
  );

  const navigateToProvenanceSession = useCallback(() => {
    if (!currentBreadcrumbProvenance) return;
    navigateToBreadcrumb(currentBreadcrumbProvenance.sourceSessionId);
  }, [currentBreadcrumbProvenance, navigateToBreadcrumb]);

  const navigateToProvenanceStage = useCallback(() => {
    if (!currentBreadcrumbProvenance?.stageId) return;
    setActiveStageContext({
      stageId: currentBreadcrumbProvenance.stageId,
      toolCallId: currentBreadcrumbProvenance.toolCallId ?? null,
      label: currentBreadcrumbProvenance.label ?? currentBreadcrumbProvenance.stageId,
      sessionId: currentBreadcrumbProvenance.sourceSessionId,
    });
    queueConversationJumpTarget({
      stageId: currentBreadcrumbProvenance.stageId,
      toolCallId: currentBreadcrumbProvenance.toolCallId ?? null,
      label: currentBreadcrumbProvenance.label ?? currentBreadcrumbProvenance.stageId,
    });
    setSelectedSessionId(currentBreadcrumbProvenance.sourceSessionId);
  }, [currentBreadcrumbProvenance, queueConversationJumpTarget, setActiveStageContext, setSelectedSessionId]);

  const navigateToProvenanceToolCall = useCallback(() => {
    if (!currentBreadcrumbProvenance?.toolCallId) return;
    setActiveStageContext({
      stageId: currentBreadcrumbProvenance.stageId ?? null,
      toolCallId: currentBreadcrumbProvenance.toolCallId,
      label: currentBreadcrumbProvenance.label ?? currentBreadcrumbProvenance.toolCallId,
      sessionId: currentBreadcrumbProvenance.sourceSessionId,
    });
    queueConversationJumpTarget({
      stageId: currentBreadcrumbProvenance.stageId ?? null,
      toolCallId: currentBreadcrumbProvenance.toolCallId,
      label: currentBreadcrumbProvenance.label ?? currentBreadcrumbProvenance.toolCallId,
    });
    setSelectedSessionId(currentBreadcrumbProvenance.sourceSessionId);
  }, [currentBreadcrumbProvenance, queueConversationJumpTarget, setActiveStageContext, setSelectedSessionId]);

  return {
    activeStageId: activeStageContext?.sessionId === selectedSessionId ? activeStageContext.stageId ?? null : null,
    activeToolCallId:
      activeStageContext?.sessionId === selectedSessionId ? activeStageContext.toolCallId ?? null : null,
    previewStageId,
    currentBreadcrumbProvenance,
    sessionBreadcrumbs,
    previewStage,
    navigateToStage,
    navigateToToolCall,
    navigateToAttachedSession,
    navigateToSession,
    navigateToBreadcrumb,
    navigateToProvenanceSession,
    navigateToProvenanceStage,
    navigateToProvenanceToolCall,
    restoreActiveStage,
    syncStageContext,
  };
}
