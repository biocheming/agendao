import { useCallback, useEffect, useMemo } from "react";
import type { SessionRecord } from "../lib/session";
import type { ConversationJumpTarget } from "./useConversationJump";
import type { useExecutionActivity } from "./useExecutionActivity";
import { useAgendaoStore } from "../store";

interface UseSchedulerNavigationOptions {
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

export function useSchedulerNavigation({
  executionActivity,
  jumpToConversationTarget,
  queueConversationJumpTarget,
}: UseSchedulerNavigationOptions) {
  const sessions = useAgendaoStore((s) => s.sessions);
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const setSelectedSessionId = useAgendaoStore((s) => s.setSelectedSessionId);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const activeStageContext = useAgendaoStore((s) => s.activeStageContext) as StageNavigationContext | null;
  const setActiveStageContext = useAgendaoStore((s) => s.setActiveStageContext);
  const previewStageId = useAgendaoStore((s) => s.previewStageId);
  const setPreviewStageId = useAgendaoStore((s) => s.setPreviewStageId);
  const sessionBreadcrumbs = useAgendaoStore((s) => s.sessionBreadcrumbs);
  const setSessionBreadcrumbs = useAgendaoStore((s) => s.setSessionBreadcrumbs);
  const currentBreadcrumbProvenanceFor = useAgendaoStore((s) => s.currentBreadcrumbProvenanceFor);
  const currentSession = useMemo(
    () => sessions.find((session) => session.id === selectedSessionId) ?? null,
    [selectedSessionId, sessions],
  );

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

  // Depend on the stable members, not the whole executionActivity return
  // object (recreated every App render) — otherwise navigateToStage changes
  // identity on every render and defeats memo(MessageCard).
  const executionNodes = executionActivity.executionNodes;
  const setSelectedExecutionId = executionActivity.setSelectedExecutionId;
  const focusStageInActivity = useCallback(
    (stageId: string, preferredExecutionId?: string | null) => {
      if (!stageId.trim()) return;
      const matchingNode =
        (preferredExecutionId
          ? executionNodes.find((node) => node.id === preferredExecutionId)
          : null) ||
        executionNodes.find((node) => node.stage_id === stageId) ||
        executionNodes.find((node) => node.id === stageId) ||
        null;
      if (matchingNode) {
        setSelectedExecutionId(matchingNode.id);
      }
    },
    [executionNodes, setSelectedExecutionId],
  );

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
    navigateToSession,
    navigateToBreadcrumb,
    navigateToProvenanceSession,
    navigateToProvenanceStage,
    navigateToProvenanceToolCall,
    restoreActiveStage,
    syncStageContext,
  };
}
