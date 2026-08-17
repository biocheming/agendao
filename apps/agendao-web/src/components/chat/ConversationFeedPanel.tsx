import type { RefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { MessageCard } from "./MessageCard";
import { useI18n } from "../../i18n/I18nProvider";
import {
  Conversation,
  ConversationContent,
  ConversationEmptyState,
  ConversationScrollButton,
} from "../ai-elements/conversation";
import { useStickToBottomContext } from "use-stick-to-bottom";
import { Button } from "../ui/button";
import { Shimmer } from "../ai-elements/shimmer";
import { BrainCircuitIcon, ChevronUpIcon, Layers2, LoaderCircleIcon, SparklesIcon, WrenchIcon } from "lucide-react";
import type { FeedMessage } from "../../lib/history";
import { feedStageId, feedToolCallId } from "../../lib/history";
import type { SessionTelemetrySnapshotRecord } from "../../lib/sessionActivity";
import { withSyntheticCompactionMessage } from "../../lib/contextCompaction";
import { useAgendaoStore } from "../../store";

const INITIAL_VISIBLE_MESSAGES = 18;
const LOAD_MORE_MESSAGES_STEP = 16;

interface ConversationFeedPanelProps {
  sessionId: string | null;
  feedRef: RefObject<HTMLDivElement | null>;
  highlightedFeedId: string | null;
  highlightedMessageIds?: Set<string>;
  activeStageId: string | null;
  activeToolCallId: string | null;
  telemetry?: SessionTelemetrySnapshotRecord | null;
  onCopyMessageLink?: (message: FeedMessage) => Promise<void> | void;
  onCopySelectedMessageLink?: () => Promise<void> | void;
  onCopySelectedMessagesMarkdown?: () => Promise<void> | void;
  onEditAndResendMessage?: (message: FeedMessage) => Promise<void> | void;
  onClearSelectedMessages?: () => void;
  onToggleMessageSelected?: (message: FeedMessage) => void;
  onNavigateStage: (stageId: string) => void;
}

// Bridges the StickToBottom scroll container into feedRef so conversation
// jump and the load-earlier scroll anchor can measure/scroll the real
// scroller. Scroll-follow itself is owned by StickToBottom — no forced
// scrollTop writes here (they fight the library and yank users back to the
// bottom while reading history).
function FeedScrollContainerBridge({ feedRef }: { feedRef: RefObject<HTMLDivElement | null> }) {
  const { scrollRef } = useStickToBottomContext();
  useEffect(() => {
    feedRef.current = (scrollRef.current as HTMLDivElement | null) ?? null;
    return () => {
      feedRef.current = null;
    };
  }, [feedRef, scrollRef]);
  return null;
}

function FeedLoadingState() {
  const { t } = useI18n();
  return (
    <div className="roc-panel grid gap-5">
      <div className="flex items-center gap-2 text-muted-foreground">
        <LoaderCircleIcon className="size-4 animate-spin" />
        <span className="text-sm">{t("feed.loadingConversation")}</span>
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
  const { t } = useI18n();
  const hasHiddenMessages = hiddenCount > 0;

  if (!historyLoading && !hasHiddenMessages) return null;

  return (
    <div className="roc-panel flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
      <div className="flex min-w-0 items-start gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-2xl border border-border/45 bg-background/78 text-muted-foreground">
          {historyLoading ? <LoaderCircleIcon className="size-4 animate-spin" /> : <ChevronUpIcon className="size-4" />}
        </div>
        <div className="min-w-0">
          <div className="roc-section-label">{t("feed.history")}</div>
          <p className="mt-1 text-sm leading-6 text-foreground/88">
            {historyLoading
              ? t("feed.loadingEarlier")
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
            {historyLoading
              ? t("feed.loadingEarlier")
              : t("feed.loadEarlier", {
                  count: Math.min(hiddenCount, LOAD_MORE_MESSAGES_STEP),
                })}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export function ConversationFeedPanel({
  sessionId,
  feedRef,
  highlightedFeedId,
  highlightedMessageIds,
  activeStageId,
  activeToolCallId,
  telemetry = null,
  onCopyMessageLink,
  onCopySelectedMessageLink,
  onCopySelectedMessagesMarkdown,
  onEditAndResendMessage,
  onClearSelectedMessages,
  onToggleMessageSelected,
  onNavigateStage,
}: ConversationFeedPanelProps) {
  const { t } = useI18n();
  const historyLoading = useAgendaoStore((s) => s.historyLoading);
  const messages = useAgendaoStore((s) => s.messages);
  const selectedMessageIds = useAgendaoStore((s) => s.selectedMessageIds);
  const streaming = useAgendaoStore((s) => s.streaming);
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

  const timelineMessages = withSyntheticCompactionMessage(messages, {
    sessionId,
    runStatus: telemetry?.runtime?.run_status,
    summary: telemetry?.context_compaction_summary ?? null,
    lifecycle: telemetry?.context_compaction_lifecycle_summary ?? null,
  });
  const safeVisibleCount =
    timelineMessages.length === 0 ? 0 : Math.min(Math.max(visibleCount, 1), timelineMessages.length);
  const hiddenCount = Math.max(0, timelineMessages.length - safeVisibleCount);
  const visibleMessages =
    hiddenCount > 0 ? timelineMessages.slice(-safeVisibleCount) : timelineMessages;

  const handleLoadEarlier = useCallback(() => {
    if (hiddenCount === 0) return;
    revealAnchorHeightRef.current = feedRef.current?.scrollHeight ?? null;
    setVisibleCount((current) => Math.min(timelineMessages.length, current + LOAD_MORE_MESSAGES_STEP));
  }, [feedRef, hiddenCount, timelineMessages.length]);

  return (
    <Conversation className="h-full min-w-0 overflow-x-hidden border-0 bg-transparent">
      <FeedScrollContainerBridge feedRef={feedRef} />
      <ConversationContent className="mx-auto w-full max-w-[88rem] px-4 pb-6 pt-3 md:px-5 md:pb-7 md:pt-3.5">
        {historyLoading && messages.length === 0 ? <FeedLoadingState /> : null}
        {!historyLoading && messages.length === 0 ? (
          <ConversationEmptyState
            className="roc-panel min-h-[22rem] gap-5"
            icon={<SparklesIcon className="size-5" />}
            title={t("feed.emptyTitle")}
            description="Ask for code changes, debugging, or exploration. The feed will turn into a readable execution narrative instead of a raw event log."
          >
            <div className="flex max-w-3xl flex-col items-center gap-5">
              <div className="text-muted-foreground">
                <SparklesIcon className="size-5" />
              </div>
              <div className="space-y-2 text-center">
                <h3 className="text-base font-semibold tracking-tight text-foreground">{t("feed.emptyTitle")}</h3>
                <p className="text-sm leading-6 text-muted-foreground">
                  Ask for code changes, debugging, or exploration. The feed will turn into a readable execution narrative instead of a raw event log.
                </p>
              </div>

              <div className="roc-empty-capability-grid">
                <div className="roc-empty-capability-card">
                  <div className="roc-empty-capability-icon">
                    <BrainCircuitIcon className="size-4.5" />
                  </div>
                  <div className="roc-empty-capability-title">{t("feed.capabilityMultiModel")}</div>
                  <div className="roc-empty-capability-desc">
                    {t("feed.capabilityMultiModelDesc")}
                  </div>
                </div>
                <div className="roc-empty-capability-card">
                  <div className="roc-empty-capability-icon">
                    <WrenchIcon className="size-4.5" />
                  </div>
                  <div className="roc-empty-capability-title">{t("feed.capabilityToolAugmented")}</div>
                  <div className="roc-empty-capability-desc">
                    {t("feed.capabilityToolAugmentedDesc")}
                  </div>
                </div>
                <div className="roc-empty-capability-card">
                  <div className="roc-empty-capability-icon">
                    <Layers2 className="size-4.5" />
                  </div>
                  <div className="roc-empty-capability-title">{t("feed.capabilityContextAware")}</div>
                  <div className="roc-empty-capability-desc">
                    {t("feed.capabilityContextAwareDesc")}
                  </div>
                </div>
              </div>

              <div className="flex flex-wrap items-center justify-center gap-2">
                <span className="roc-empty-suggestion">Refactor a component without changing behavior</span>
                <span className="roc-empty-suggestion">Trace a failing session and explain the root cause</span>
                <span className="roc-empty-suggestion">Compare two implementation options before coding</span>
              </div>
            </div>
          </ConversationEmptyState>
        ) : null}
        {timelineMessages.length > 0 ? (
          <div className="grid min-w-0 gap-4">
            {selectedMessageIds.size > 0 ? (
              <div className="sticky top-2 z-20 flex justify-end">
                <div className="inline-flex max-w-full items-center gap-1 rounded-full border border-border/45 bg-background/88 px-2 py-1 shadow-sm backdrop-blur">
                  <span className="truncate px-1 text-xs text-muted-foreground">
                    {t("app.messageSelected", { count: selectedMessageIds.size })}
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 rounded-full px-2.5 text-xs"
                    onClick={() => void onCopySelectedMessageLink?.()}
                  >
                    {t("app.copySelectedLink")}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 rounded-full px-2.5 text-xs"
                    onClick={() => void onCopySelectedMessagesMarkdown?.()}
                  >
                    {t("app.copyMarkdown")}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-7 rounded-full px-2.5 text-xs"
                    onClick={onClearSelectedMessages}
                  >
                    {t("app.clear")}
                  </Button>
                </div>
              </div>
            ) : null}
            <HistoryBackfillState
              hiddenCount={hiddenCount}
              visibleCount={visibleMessages.length}
              totalCount={timelineMessages.length}
              historyLoading={historyLoading}
              onLoadEarlier={handleLoadEarlier}
            />
            {visibleMessages.map((message) => {
              // Pre-compute per-card booleans so a change of the global
              // active stage/tool-call/aborting id only re-renders the cards
              // whose state actually flips (memo(MessageCard) shallow-compare).
              const stageId = feedStageId(message);
              const toolCallId = feedToolCallId(message);
              return (
                <MessageCard
                  key={message.feedId}
                  message={message}
                  highlighted={highlightedFeedId === message.feedId || Boolean(message.anchorId && highlightedMessageIds?.has(message.anchorId))}
                  selected={Boolean(message.anchorId && selectedMessageIds?.has(message.anchorId))}
                  activeStage={Boolean(activeStageId && stageId === activeStageId)}
                  activeToolCall={Boolean(activeToolCallId && toolCallId === activeToolCallId)}
                  onCopyMessageLink={onCopyMessageLink}
                  onEditAndResend={onEditAndResendMessage}
                  onToggleSelected={onToggleMessageSelected}
                  onNavigateStage={onNavigateStage}
                />
              );
            })}
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
                  <div className="roc-section-label">{t("feed.liveResponse")}</div>
                  <Shimmer as="span" className="text-sm text-foreground/88" duration={1.45}>
                    {t("feed.composingNext")}
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
