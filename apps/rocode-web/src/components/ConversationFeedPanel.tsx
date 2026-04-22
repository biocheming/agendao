import type { RefObject } from "react";
import { useEffect, useRef, useState } from "react";
import { MessageCard } from "./MessageCard";
import {
  Conversation,
  ConversationContent,
  ConversationEmptyState,
  ConversationScrollButton,
} from "./ai-elements/conversation";
import { Button } from "./ui/button";
import { Shimmer } from "./ai-elements/shimmer";
import { ChevronUpIcon, LoaderCircleIcon, SparklesIcon } from "lucide-react";
import type { FeedMessage } from "../lib/history";

const INITIAL_VISIBLE_MESSAGES = 18;
const LOAD_MORE_MESSAGES_STEP = 16;

interface ConversationFeedPanelProps {
  sessionId: string | null;
  feedRef: RefObject<HTMLDivElement | null>;
  historyLoading: boolean;
  messages: FeedMessage[];
  highlightedFeedId: string | null;
  activeStageId: string | null;
  activeToolCallId: string | null;
  streaming?: boolean;
  onNavigateStage: (stageId: string) => void;
  onNavigateChildSession: (
    sessionId: string,
    context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null },
  ) => void;
}

function FeedLoadingState() {
  return (
    <div className="roc-panel grid gap-5">
      <div className="flex items-center gap-2 text-muted-foreground">
        <LoaderCircleIcon className="size-4 animate-spin" />
        <span className="text-sm">Loading conversation…</span>
      </div>
      <div className="grid gap-4">
        <div className="grid gap-3">
          <div className="roc-skeleton-line h-4 w-24" />
          <div className="roc-skeleton-panel h-16 w-full" />
        </div>
        <div className="ml-auto grid w-[74%] gap-3">
          <div className="roc-skeleton-line h-4 w-20" />
          <div className="roc-skeleton-panel h-20 w-full" />
        </div>
        <div className="grid w-[88%] gap-3">
          <div className="roc-skeleton-line h-4 w-28" />
          <div className="roc-skeleton-panel h-14 w-full" />
        </div>
      </div>
    </div>
  );
}

function HistoryBackfillState({
  hiddenCount,
  visibleCount,
  totalCount,
  historyLoading,
  onLoadEarlier,
}: {
  hiddenCount: number;
  visibleCount: number;
  totalCount: number;
  historyLoading: boolean;
  onLoadEarlier: () => void;
}) {
  const hasHiddenMessages = hiddenCount > 0;

  if (!historyLoading && !hasHiddenMessages) return null;

  return (
    <div className="roc-panel flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-2xl border border-border/45 bg-background/78 text-muted-foreground">
          {historyLoading ? <LoaderCircleIcon className="size-4 animate-spin" /> : <ChevronUpIcon className="size-4" />}
        </div>
        <div className="min-w-0">
          <div className="roc-section-label">History</div>
          <p className="mt-1 text-sm leading-6 text-foreground/88">
            {historyLoading
              ? "Loading earlier turns and stitching them back into the timeline."
              : `Showing the latest ${visibleCount} turns first so the current narrative stays readable.`}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {totalCount > 0 ? (
          <span className="roc-badge">
            {visibleCount} / {totalCount} in view
          </span>
        ) : null}
        {hasHiddenMessages ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="rounded-full px-4"
            disabled={historyLoading}
            onClick={onLoadEarlier}
          >
            {historyLoading ? "Loading earlier…" : `Load ${Math.min(hiddenCount, LOAD_MORE_MESSAGES_STEP)} earlier`}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export function ConversationFeedPanel({
  sessionId,
  feedRef,
  historyLoading,
  messages,
  highlightedFeedId,
  activeStageId,
  activeToolCallId,
  streaming = false,
  onNavigateStage,
  onNavigateChildSession,
}: ConversationFeedPanelProps) {
  const [visibleCount, setVisibleCount] = useState(0);
  const revealAnchorHeightRef = useRef<number | null>(null);
  const previousMessageCountRef = useRef(0);

  useEffect(() => {
    setVisibleCount(0);
    previousMessageCountRef.current = 0;
    revealAnchorHeightRef.current = null;
  }, [sessionId]);

  useEffect(() => {
    if (messages.length === 0) {
      setVisibleCount(0);
      previousMessageCountRef.current = 0;
      return;
    }

    const previousCount = previousMessageCountRef.current;
    previousMessageCountRef.current = messages.length;

    setVisibleCount((current) => {
      if (current === 0) return Math.min(messages.length, INITIAL_VISIBLE_MESSAGES);
      if (messages.length < current) return Math.min(messages.length, current);

      const appended = messages.length - previousCount;
      const wasShowingTail = current >= previousCount - 2;
      if (appended > 0 && wasShowingTail) return messages.length;

      return current;
    });
  }, [messages.length]);

  useEffect(() => {
    if (revealAnchorHeightRef.current === null || !feedRef.current) return;
    const previousHeight = revealAnchorHeightRef.current;
    revealAnchorHeightRef.current = null;
    feedRef.current.scrollTop += feedRef.current.scrollHeight - previousHeight;
  }, [feedRef, visibleCount]);

  // Sync feedRef to the Conversation scroll container
  useEffect(() => {
    if (!feedRef.current) return;
    feedRef.current.scrollTop = feedRef.current.scrollHeight;
  }, [feedRef, messages]);

  const safeVisibleCount = messages.length === 0 ? 0 : Math.min(Math.max(visibleCount, 1), messages.length);
  const hiddenCount = Math.max(0, messages.length - safeVisibleCount);
  const visibleMessages = hiddenCount > 0 ? messages.slice(-safeVisibleCount) : messages;

  const handleLoadEarlier = () => {
    if (hiddenCount === 0) return;
    revealAnchorHeightRef.current = feedRef.current?.scrollHeight ?? null;
    setVisibleCount((current) => Math.min(messages.length, current + LOAD_MORE_MESSAGES_STEP));
  };

  return (
    <Conversation className="h-full min-w-0 overflow-x-hidden border-0 bg-transparent">
      <ConversationContent className="mx-auto w-full max-w-[76rem] px-4 pb-6 pt-3 md:px-5 md:pb-7 md:pt-3.5">
        {historyLoading && messages.length === 0 ? <FeedLoadingState /> : null}
        {!historyLoading && messages.length === 0 ? (
          <ConversationEmptyState
            className="roc-panel min-h-[22rem] gap-4"
            icon={<SparklesIcon className="size-5" />}
            title="Conversation starts here"
            description="Ask for code changes, debugging, or exploration. The feed will turn into a readable execution narrative instead of a raw event log."
          >
            <div className="flex max-w-2xl flex-col items-center gap-4">
              <div className="text-muted-foreground">
                <SparklesIcon className="size-5" />
              </div>
              <div className="space-y-2 text-center">
                <h3 className="text-base font-semibold tracking-tight text-foreground">Conversation starts here</h3>
                <p className="text-sm leading-6 text-muted-foreground">
                  Ask for code changes, debugging, or exploration. The feed will turn into a readable execution narrative instead of a raw event log.
                </p>
              </div>
              <div className="flex flex-wrap items-center justify-center gap-2">
                <span className="roc-empty-suggestion">Refactor a component without changing behavior</span>
                <span className="roc-empty-suggestion">Trace a failing session and explain the root cause</span>
                <span className="roc-empty-suggestion">Compare two implementation options before coding</span>
              </div>
            </div>
          </ConversationEmptyState>
        ) : null}
        {messages.length > 0 ? (
          <div className="grid min-w-0 gap-4">
            <HistoryBackfillState
              hiddenCount={hiddenCount}
              visibleCount={visibleMessages.length}
              totalCount={messages.length}
              historyLoading={historyLoading}
              onLoadEarlier={handleLoadEarlier}
            />
            {visibleMessages.map((message) => (
              <MessageCard
                key={message.feedId}
                message={message}
                highlighted={highlightedFeedId === message.feedId}
                activeStageId={activeStageId}
                activeToolCallId={activeToolCallId}
                onNavigateStage={onNavigateStage}
                onNavigateChildSession={onNavigateChildSession}
              />
            ))}
            {streaming ? (
              <div className="roc-panel flex items-center gap-3 px-3.5 py-2.5">
                <div className="flex size-9 items-center justify-center rounded-2xl border border-border/45 bg-background/78">
                  <div className="flex items-center gap-1.5">
                    <span className="roc-streaming-dot" />
                    <span className="roc-streaming-dot" />
                    <span className="roc-streaming-dot" />
                  </div>
                </div>
                <div className="min-w-0">
                  <div className="roc-section-label">Live Response</div>
                  <Shimmer as="span" className="text-sm text-foreground/88" duration={1.45}>
                    ROCode is composing the next visible block…
                  </Shimmer>
                </div>
              </div>
            ) : null}
          </div>
        ) : null}
      </ConversationContent>
      <ConversationScrollButton />
    </Conversation>
  );
}
