import type { RefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

export interface FeedJumpMessage {
  feedId: string;
  kind?: string;
  id?: string;
  stage_id?: string;
}

export interface ConversationJumpTarget {
  executionId?: string | null;
  toolCallId?: string | null;
  stageId?: string | null;
  label?: string | null;
}

interface UseConversationJumpOptions {
  messages: FeedJumpMessage[];
  feedRef: RefObject<HTMLDivElement | null>;
  onMiss: (message: string) => void;
}

function toolCallIdFromExecutionId(executionId?: string | null) {
  if (!executionId) return null;
  return executionId.startsWith("tool_call:") ? executionId.slice("tool_call:".length) : executionId;
}

function findConversationTarget(messages: FeedJumpMessage[], target: ConversationJumpTarget) {
  const toolCallId = target.toolCallId || toolCallIdFromExecutionId(target.executionId);
  if (toolCallId) {
    const toolMessage = [...messages]
      .reverse()
      .find((message) => message.kind === "tool" && message.id === toolCallId);
    if (toolMessage) return toolMessage;
  }

  if (target.stageId) {
    const stageMessage = [...messages]
      .reverse()
      .find((message) => message.stage_id === target.stageId);
    if (stageMessage) return stageMessage;
  }

  if (target.executionId) {
    const executionMessage = [...messages]
      .reverse()
      .find((message) => message.id === target.executionId);
    if (executionMessage) return executionMessage;
  }

  return null;
}

export function useConversationJump({ messages, feedRef, onMiss }: UseConversationJumpOptions) {
  const [highlightedFeedId, setHighlightedFeedId] = useState<string | null>(null);
  const [pendingTarget, setPendingTarget] = useState<ConversationJumpTarget | null>(null);
  const highlightTimerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (highlightTimerRef.current !== null) {
        window.clearTimeout(highlightTimerRef.current);
      }
    },
    [],
  );

  const jumpToTarget = useCallback(
    (target: ConversationJumpTarget, options?: { suppressMiss?: boolean }) => {
      const message = findConversationTarget(messages, target);
      if (!message) {
        if (!options?.suppressMiss) {
          onMiss(`No matching conversation block found for ${target.label || "this provenance target"}.`);
        }
        return false;
      }

      const element = feedRef.current?.querySelector<HTMLElement>(
        `[data-feed-id="${CSS.escape(message.feedId)}"]`,
      );
      if (!element) {
        if (!options?.suppressMiss) {
          onMiss(`Conversation block ${target.label || message.feedId} is not mounted right now.`);
        }
        return false;
      }

      element.scrollIntoView({ behavior: "smooth", block: "center" });
      setHighlightedFeedId(message.feedId);
      if (highlightTimerRef.current !== null) {
        window.clearTimeout(highlightTimerRef.current);
      }
      highlightTimerRef.current = window.setTimeout(() => {
        setHighlightedFeedId((current) => (current === message.feedId ? null : current));
      }, 2200);
      return true;
    },
    [feedRef, messages, onMiss],
  );

  useEffect(() => {
    if (!pendingTarget) return;
    if (jumpToTarget(pendingTarget, { suppressMiss: true })) {
      setPendingTarget(null);
    }
  }, [jumpToTarget, pendingTarget]);

  const jumpToConversationTarget = useCallback(
    (target: ConversationJumpTarget) => {
      setPendingTarget(null);
      jumpToTarget(target);
    },
    [jumpToTarget],
  );

  const queueConversationJumpTarget = useCallback((target: ConversationJumpTarget) => {
    setPendingTarget(target);
  }, []);

  const jumpOrQueueConversationTarget = useCallback(
    (target: ConversationJumpTarget) => {
      setPendingTarget(null);
      if (!jumpToTarget(target, { suppressMiss: true })) {
        setPendingTarget(target);
      }
    },
    [jumpToTarget],
  );

  return {
    highlightedFeedId,
    jumpToConversationTarget,
    jumpOrQueueConversationTarget,
    queueConversationJumpTarget,
  };
}
