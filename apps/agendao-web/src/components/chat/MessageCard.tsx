"use client";

import { Button } from "@/components/ui/button";
import { StructuredDataView } from "@/components/execution/StructuredDataView";
import { cn } from "@/lib/utils";
import {
  cacheBustSummaryFromMetadata,
  cacheBustSummaryLabel,
  cacheBustSummaryStatusLabel,
} from "@/lib/cacheDiagnostics";
import {
  ActivityIcon,
  BrainCircuitIcon,
  CheckIcon,
  ChevronDownIcon,
  CopyIcon,
  InfoIcon,
  SparklesIcon,
  WrenchIcon,
} from "lucide-react";
import { useCallback, useState } from "react";
import { MessageResponse } from "../ai-elements/message";
import {
  feedAttachedSessionId,
  feedStageId,
  feedToolCallId,
  isMultimodalInfoOutputBlock,
  isReasoningOutputBlock,
  isSchedulerStageOutputBlock,
  isStatusOutputBlock,
  isToolOutputBlock,
  type FeedBlock,
  type FeedMessage,
  type MultimodalInfoOutputBlock,
  type OutputField,
  type StatusOutputBlock,
  type ToolOutputBlock,
} from "../../lib/history";
import { SchedulerStageCard } from "./SchedulerStageCard";
import { toolActivityLabel } from "../../lib/toolLabels";
import { sanitizeAssistantDisplayText } from "../../lib/blockPresentation";
import { compactText, excerptText, normalizeValue } from "../../lib/stagePresentation";
import {
  toolCompatDetail,
  toolDisplayFields,
  toolDisplayPreview,
  toolDisplayRawLabelKey,
  toolDisplaySummary,
} from "../../lib/toolPresentation";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  isSyntheticCompactionMessage,
  syntheticCompactionLines,
} from "../../lib/contextCompaction";

interface MessageCardProps {
  message: FeedMessage;
  highlighted?: boolean;
  selected?: boolean;
  activeStageId?: string | null;
  activeToolCallId?: string | null;
  onCopyMessageLink?: (message: FeedMessage) => Promise<void> | void;
  onToggleSelected?: (message: FeedMessage) => void;
  onNavigateStage: (stageId: string) => void;
  onNavigateAttachedSession: (
    sessionId: string,
    context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null },
  ) => void;
}

function formatClock(ts?: number) {
  if (typeof ts !== "number" || !Number.isFinite(ts) || ts <= 0) return null;
  const normalized = ts > 1_000_000_000_000 ? ts : ts * 1000;
  return new Date(normalized).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function readableSummary(message: FeedMessage) {
  const summary = compactText(message.summary);
  if (!summary) return null;

  const title = compactText(message.title);
  const text = compactText(message.text);
  if (summary === title || summary === text) return null;
  if (text && text.includes(summary)) return null;

  return summary;
}

function attachedSessionLabel(message: FeedMessage): string {
  if (typeof message.title === "string" && message.title.trim()) {
    return message.title;
  }
  if (isSchedulerStageOutputBlock(message) && typeof message.stage === "string" && message.stage.trim()) {
    return message.stage;
  }
  return feedAttachedSessionId(message) ?? "";
}

function joinSummaryParts(parts: Array<string | null | undefined>) {
  return parts.filter(Boolean).join(" · ");
}

function MetaActionButton({
  children,
  onClick,
  className,
}: {
  children: React.ReactNode;
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn("text-xs text-muted-foreground transition-colors hover:text-primary", className)}
    >
      {children}
    </button>
  );
}

function StructuredText({
  value,
  className,
}: {
  value: unknown;
  className?: string;
}) {
  const display = normalizeValue(value);
  if (!display.text) return null;

  if (display.structured) {
    return <pre className={cn("roc-structured-value roc-structured-copy", className)}>{display.text}</pre>;
  }

  return (
    <p className={cn("roc-structured-copy text-sm leading-6 whitespace-pre-wrap text-foreground", className)}>
      {display.text}
    </p>
  );
}

function classifyField(field: OutputField) {
  const label = field.label?.trim() || "Field";
  const display = normalizeValue(field.value ?? "");
  const shortInline =
    !display.structured &&
    display.text.length > 0 &&
    display.text.length <= 42 &&
    !display.text.includes(",") &&
    !display.text.includes(":");
  return { label, display, shortInline };
}

function FieldList({ fields }: { fields?: OutputField[] }) {
  if (!fields?.length) return null;

  const inlineFields = fields
    .map(classifyField)
    .filter((field) => field.shortInline);
  const blockFields = fields
    .map(classifyField)
    .filter((field) => !field.shortInline);

  return (
    <div className="roc-structured-stack">
      {inlineFields.length ? (
        <div className="roc-structured-inline-list">
          {inlineFields.map((field, index) => (
            <span key={`${field.label}-inline-${index}`} className="roc-inline-fact">
              <span className="roc-inline-fact-label">{field.label}</span>
              <span className="roc-inline-fact-value">{field.display.text}</span>
            </span>
          ))}
        </div>
      ) : null}
      {blockFields.length ? (
        <dl className="roc-structured-dl">
          {blockFields.map((field, index) => (
            <div key={`${field.label}-${index}`} className="roc-structured-row">
              <dt className="roc-structured-key">{field.label}</dt>
              <dd className="m-0">
                <StructuredText value={field.display.text} />
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

function DisclosureCard({
  icon,
  label,
  title,
  summary,
  defaultExpanded = false,
  tone = "default",
  blockMeta,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  title: string;
  summary?: string | null;
  defaultExpanded?: boolean;
  tone?: "default" | "danger";
  blockMeta?: {
    kind: string;
    feedId?: string;
    blockId?: string;
    anchorId?: string;
  };
  children: React.ReactNode;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <section
      className="roc-detail-card"
      data-testid="transcript-block"
      data-kind={blockMeta?.kind}
      data-feed-id={blockMeta?.feedId}
      data-block-id={blockMeta?.blockId}
      data-message-anchor={blockMeta?.anchorId}
      data-expanded={expanded ? "true" : "false"}
      data-tone={tone === "danger" ? "danger" : undefined}
    >
      <button
        type="button"
        className="roc-detail-trigger"
        onClick={() => setExpanded((value) => !value)}
      >
        <div className="roc-detail-icon">{icon}</div>
        <div className="min-w-0 flex-1">
          <div className="roc-section-label">{label}</div>
          <div className="roc-detail-title">{title}</div>
          {summary ? (
            <p className={cn("roc-detail-summary", expanded ? "line-clamp-2" : "line-clamp-1")}>{summary}</p>
          ) : null}
        </div>
        <ChevronDownIcon
          className={cn(
            "mt-0.5 size-4 shrink-0 text-muted-foreground transition-transform duration-200",
            expanded && "rotate-180",
          )}
        />
      </button>

      <div
        className={cn(
          "overflow-hidden transition-all duration-200",
          expanded ? "max-h-[2400px]" : "max-h-0",
        )}
      >
        <div className={cn(expanded ? "roc-detail-body" : "pt-0")}>{children}</div>
      </div>
    </section>
  );
}

function ReasoningBlock({ message }: { message: FeedBlock<"reasoning"> }) {
  const text = message.text;
  return (
    <DisclosureCard
      icon={<BrainCircuitIcon className="size-4" />}
      label="THINKING"
      title="Thinking"
      summary={excerptText(text, 132) || "Collapsed by default so the visible response keeps its reading pace."}
      blockMeta={{
        kind: message.kind,
        feedId: message.feedId,
        blockId: message.id,
        anchorId: message.anchorId,
      }}
    >
      <StructuredText value={text} className="text-muted-foreground" />
    </DisclosureCard>
  );
}

function StatusBlock({ message }: { message: StatusOutputBlock }) {
  if (isSyntheticCompactionMessage(message as FeedMessage)) {
    const { statusLine, detailLine } = syntheticCompactionLines(message as FeedBlock<"status">);
    return (
      <section className="roc-detail-card" data-tone="warning" data-kind="compaction">
        <div className="flex items-start gap-2.5">
          <div className="roc-detail-icon text-amber-600 dark:text-amber-300">
            <ActivityIcon className="size-4" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="roc-section-label">Context</div>
            <div className="roc-detail-title">{message.title?.trim() || "Compacting conversation"}</div>
            <div className="mt-2 flex items-center gap-1.5" aria-hidden="true">
              <span className="roc-streaming-dot" />
              <span className="roc-streaming-dot" />
              <span className="roc-streaming-dot" />
            </div>
            {statusLine ? <p className="mt-2 text-sm leading-6 text-foreground/88">{statusLine}</p> : null}
            {detailLine ? <p className="mt-1 text-sm leading-6 text-muted-foreground">{detailLine}</p> : null}
          </div>
        </div>
      </section>
    );
  }
  const isError = message.tone === "error";
  const title = message.title?.trim() || (isError ? "Runtime error" : "System update");
  const summary = message.summary?.trim() || excerptText(message.text, 120) || null;
  const hasDetail = Boolean(message.text?.trim() || message.fields?.length);

  if (!hasDetail) {
    return (
      <section className="roc-detail-card" data-tone={isError ? "danger" : undefined}>
        <div className="flex items-start gap-2.5">
          <div
            className={cn(
              "roc-detail-icon",
              isError ? "border-destructive/20 text-destructive" : "text-muted-foreground",
            )}
          >
            <ActivityIcon className="size-4" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="roc-section-label">{isError ? "Error" : "Status"}</div>
            <div className={cn("roc-detail-title", isError && "text-destructive")}>{title}</div>
            {summary ? (
              <p className={cn("roc-detail-summary", isError && "text-destructive/80")}>{summary}</p>
            ) : null}
          </div>
        </div>
      </section>
    );
  }

  return (
    <DisclosureCard
      icon={<ActivityIcon className="size-4" />}
      label={isError ? "Error" : "Status"}
      title={title}
      summary={summary}
      defaultExpanded={isError}
      tone={isError ? "danger" : "default"}
    >
      {message.text?.trim() ? (
        <p className={cn("text-sm leading-6 whitespace-pre-wrap", isError ? "text-destructive" : "text-foreground/88")}>
          {message.text}
        </p>
      ) : null}
      {message.fields?.length ? <FieldList fields={message.fields} /> : null}
    </DisclosureCard>
  );
}

function InfoBlock({ message }: { message: MultimodalInfoOutputBlock }) {
  const title = message.title?.trim() || "Context note";
  const summary = message.summary?.trim() || excerptText(message.text, 120) || null;
  const hasDetail = Boolean(message.text?.trim() || message.fields?.length);

  if (!hasDetail) {
    return (
      <section className="roc-detail-card">
        <div className="flex items-start gap-2.5">
          <div className="roc-detail-icon">
            <InfoIcon className="size-4" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="roc-section-label">Context</div>
            <div className="roc-detail-title">{title}</div>
            {summary ? <p className="roc-detail-summary">{summary}</p> : null}
          </div>
        </div>
      </section>
    );
  }

  return (
    <DisclosureCard
      icon={<InfoIcon className="size-4" />}
      label="Context"
      title={title}
      summary={summary}
    >
      {message.text?.trim() ? (
        <StructuredText value={message.text} className="text-muted-foreground" />
      ) : null}
      {message.fields?.length ? <FieldList fields={message.fields} /> : null}
    </DisclosureCard>
  );
}

function ToolBlock({ message, active }: { message: ToolOutputBlock; active: boolean }) {
  // Phase W2/W5: prefer live_identity for live tool cards, then fall back to
  // the persisted history marker injected by buildFeedFromHistory() so history
  // rebuild preserves TOOL RUNNING / TOOL RESULT semantics.
  const metadataPartKind =
    typeof message.metadata?.agendao_web_history_part_kind === "string"
      ? message.metadata.agendao_web_history_part_kind
      : null;
  const partKind = message.live_identity?.part_kind ?? metadataPartKind;
  const isRunning = partKind === "tool_call";
  const isResult = partKind === "tool_result";
  const label = isRunning ? "TOOL RUNNING" : isResult ? "TOOL RESULT" : "TOOL";
  const iconColor = isRunning ? "text-amber-500" : isResult ? "text-emerald-500" : "";

  // P2-3: tool presentation is centralized in lib/toolPresentation.ts
  const summary = toolDisplaySummary(message);
  const rawLabelKey = toolDisplayRawLabelKey(message);
  const toolTitle = toolActivityLabel(rawLabelKey);
  const fields = toolDisplayFields(message);
  const { previewText, previewKind, previewTruncated } = toolDisplayPreview(message);

  const compatDetail = toolCompatDetail(message);
  const hasStructuredObject =
    message.structured !== null &&
    message.structured !== undefined &&
    typeof message.structured === "object";

  return (
    <DisclosureCard
      icon={<WrenchIcon className={cn("size-4", iconColor)} />}
      label={label}
      title={toolTitle}
      summary={summary}
      defaultExpanded={active}
      blockMeta={{
        kind: message.kind,
        feedId:
          typeof (message as FeedMessage).feedId === "string"
            ? (message as FeedMessage).feedId
            : undefined,
        blockId: message.id,
        anchorId:
          typeof (message as FeedMessage).anchorId === "string"
            ? (message as FeedMessage).anchorId
            : undefined,
      }}
    >
      <div className="grid gap-2.5">
        {message.tool_call_id || message.stage_id ? (
          <p className="text-sm text-muted-foreground">
            {joinSummaryParts([
              message.tool_call_id ? `tool ${message.tool_call_id}` : null,
              message.stage_id ? `stage ${message.stage_id}` : null,
            ])}
          </p>
        ) : null}
        {fields?.length ? <FieldList fields={fields} /> : null}
        {compatDetail ? (
          <StructuredText value={compatDetail} className="text-muted-foreground" />
        ) : null}
        {previewText ? (
          <div className="grid gap-1.5">
            <div className="roc-section-label">
              {previewKind === "diff" ? "Preview" : previewKind === "code" ? "Output" : "Detail"}
            </div>
            <StructuredText value={previewText} className="text-muted-foreground" />
            {previewTruncated ? (
              <p className="text-[11px] leading-5 text-muted-foreground">Preview truncated.</p>
            ) : null}
          </div>
        ) : null}
        {hasStructuredObject ? (
          <div className="grid gap-1.5">
            <div className="roc-section-label">Structured</div>
            <StructuredDataView value={message.structured} emptyLabel="No structured tool detail." />
          </div>
        ) : null}
      </div>
    </DisclosureCard>
  );
}

export function MessageCard({
  message,
  highlighted = false,
  selected = false,
  activeStageId = null,
  activeToolCallId = null,
  onCopyMessageLink,
  onToggleSelected,
  onNavigateStage,
  onNavigateAttachedSession,
}: MessageCardProps) {
  const [copied, setCopied] = useState(false);
  const displayText = sanitizeAssistantDisplayText(message.text ?? "", message.kind, message.role);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(displayText);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [displayText]);

  if (isSchedulerStageOutputBlock(message)) {
    return (
      <SchedulerStageCard
        message={message}
        highlighted={highlighted || Boolean(activeStageId && message.stage_id === activeStageId)}
        onNavigateStage={onNavigateStage}
        onNavigateAttachedSession={onNavigateAttachedSession}
      />
    );
  }

  if (isReasoningOutputBlock(message)) {
    if (!message.text.trim()) return null;
    return <ReasoningBlock message={message} />;
  }

  if (isStatusOutputBlock(message)) {
    return <StatusBlock message={message} />;
  }

  if (isToolOutputBlock(message)) {
    return (
      <ToolBlock
        message={message}
        active={Boolean(activeToolCallId && message.tool_call_id === activeToolCallId)}
      />
    );
  }

  if (isMultimodalInfoOutputBlock(message)) {
    return <InfoBlock message={message} />;
  }

  const role = message.role ?? "assistant";
  const isUser = role === "user";
  const roleLabel = isUser ? "USER" : "ASSIST";
  const clock = formatClock(message.ts);
  const summary = readableSummary(message);
  const stageId = feedStageId(message);
  const toolCallId = feedToolCallId(message);
  const attachedSessionId = feedAttachedSessionId(message);
  const cacheSummary = cacheBustSummaryFromMetadata(message.metadata);
  const cacheDiagnosticLabel = cacheBustSummaryStatusLabel(cacheSummary);
  const cacheDiagnosticDetail = cacheBustSummaryLabel(cacheSummary);
  const metaSummary = joinSummaryParts([
    clock,
    cacheDiagnosticLabel ? `cache ${cacheDiagnosticLabel}` : null,
    toolCallId ? `tool ${toolCallId}` : null,
  ]);
  const active =
    Boolean(activeStageId && stageId === activeStageId) ||
    Boolean(activeToolCallId && toolCallId === activeToolCallId);

  return (
    <article
      className={cn("grid min-w-0 gap-1", isUser && "justify-items-end")}
      data-testid="message-card"
      data-feed-id={message.feedId}
      data-message-anchor={message.anchorId}
      data-block-id={message.id}
      data-stage-id={stageId}
      data-kind={message.kind}
    >
      <div className={cn("w-full", isUser ? "max-w-[82%]" : "max-w-full")}>
        <section
          className="roc-message-card p-3 md:p-4"
          data-tone={isUser ? "user" : "assistant"}
          data-highlighted={highlighted ? "true" : "false"}
          data-active={active ? "true" : "false"}
        >
          <div className="roc-message-meta-row">
            <div className="roc-message-meta-group">
              {onToggleSelected && message.anchorId ? (
                <input
                  type="checkbox"
                  className="h-3.5 w-3.5 accent-current"
                  checked={selected}
                  aria-label="Select message"
                  onChange={() => onToggleSelected(message)}
                />
              ) : null}
              <span className="roc-section-label">{roleLabel}</span>
              {metaSummary ? (
                <span
                  className={cn(
                    "text-xs",
                    cacheDiagnosticLabel
                      ? "text-amber-700 dark:text-amber-300"
                      : "text-muted-foreground",
                  )}
                  title={cacheDiagnosticDetail || metaSummary}
                >
                  {metaSummary}
                </span>
              ) : null}
            </div>
            {stageId || toolCallId ? (
              <div className="roc-message-meta-group">
                {stageId ? (
                  <MetaActionButton onClick={() => onNavigateStage(stageId)}>
                    stage {stageId}
                  </MetaActionButton>
                ) : null}
              </div>
            ) : null}
          </div>

          {message.title?.trim() && message.title.trim() !== displayText.trim() ? (
            <div className="roc-message-title">
              {message.title.trim()}
            </div>
          ) : null}

          {displayText ? (
            <MessageResponse
              className={cn(
                "roc-markdown-flow roc-message-body size-full",
                isUser ? "[&_p]:text-foreground" : "[&_p]:text-foreground/92",
              )}
            >
              {displayText}
            </MessageResponse>
          ) : null}

          {message.fields?.length ? (
            <div className="mt-4">
              <FieldList fields={message.fields} />
            </div>
          ) : null}

          {!isUser && displayText ? (
            <div className="roc-message-footer">
              <div className="min-w-0 flex-1">
                {summary ? <p className="roc-message-summary">{summary}</p> : null}
              </div>
              <TooltipProvider>
                {onCopyMessageLink && message.anchorId ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="roc-action roc-action-compact h-7 w-7 rounded-full text-muted-foreground hover:text-foreground"
                        title="Copy message link"
                        onClick={() => void onCopyMessageLink(message)}
                      >
                        <InfoIcon className="size-3.5" />
                        <span className="sr-only">Copy message link</span>
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent side="top">Copy message link</TooltipContent>
                  </Tooltip>
                ) : null}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="roc-action roc-action-compact h-7 w-7 rounded-full text-muted-foreground hover:text-foreground"
                      title={copied ? "Copied" : "Copy message"}
                      onClick={handleCopy}
                    >
                      {copied ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}
                      <span className="sr-only">{copied ? "Copied" : "Copy message"}</span>
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent side="top">
                    {copied ? "Copied" : "Copy message"}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </div>
          ) : null}
        </section>
      </div>
      {!isUser && attachedSessionId ? (
        <div className="pl-1">
          <MetaActionButton
            className="roc-action roc-action-pill justify-center px-3.5 py-1.5 text-xs text-foreground no-underline"
            onClick={() =>
              onNavigateAttachedSession(attachedSessionId, {
                stageId: stageId ?? null,
                toolCallId: toolCallId ?? null,
                label: attachedSessionLabel(message),
              })
            }
          >
            <SparklesIcon className="mr-1 size-3.5" />
            Open attached session {attachedSessionLabel(message)}
          </MetaActionButton>
        </div>
      ) : null}
    </article>
  );
}
