"use client";

import { Button } from "@/components/ui/button";
import { StructuredDataView } from "@/components/StructuredDataView";
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
import { MessageResponse } from "./ai-elements/message";
import type { FeedMessage, OutputBlock, OutputField } from "../lib/history";
import { SchedulerStageCard } from "./SchedulerStageCard";
import { isSkillToolName, toolActivityLabel } from "../lib/toolLabels";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

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

function compactText(value?: string | null) {
  return value?.replace(/\s+/g, " ").trim() ?? "";
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

function excerptText(value?: string | null, maxLength = 120) {
  const text = compactText(value);
  if (!text) return null;
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1)}…`;
}

function looksLikeStructuredEnvelope(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value as Record<string, unknown>);
  if (keys.length === 0) return false;
  const knownEnvelopeKeys = [
    "kind",
    "phase",
    "role",
    "text",
    "parts",
    "metadata",
    "output_block",
    "display",
    "structured",
    "summary",
    "fields",
    "tool_call_id",
    "stage_id",
    "choices",
    "usage",
    "object",
    "created",
    "model",
    "candidates",
    "output",
  ];
  return keys.some((key) => knownEnvelopeKeys.includes(key));
}

function stripTrailingStructuredJson(text: string) {
  const trimmed = text.trimEnd();
  const candidateStarts = [trimmed.lastIndexOf("\n\n{"), trimmed.lastIndexOf("\n{"), trimmed.lastIndexOf("\n\n[")];

  for (const startIndex of candidateStarts) {
    if (startIndex < 0) continue;
    const candidate = trimmed.slice(startIndex).trimStart();
    if (!(candidate.startsWith("{") || candidate.startsWith("["))) continue;
    try {
      const parsed = JSON.parse(candidate);
      if (!looksLikeStructuredEnvelope(parsed)) continue;
      const prefix = trimmed.slice(0, startIndex).trimEnd();
      if (!prefix) continue;
      return prefix;
    } catch {
      continue;
    }
  }

  return trimmed;
}

function sanitizeDisplayedMessageText(message: FeedMessage) {
  const raw = message.text?.trimEnd() ?? "";
  if (!raw) return raw;
  if ((message.role ?? "assistant") !== "assistant") return raw;
  return stripTrailingStructuredJson(raw);
}

function normalizeValue(value: unknown) {
  const text = String(value ?? "").trim();
  if (!text) return { structured: false, text: "" };

  const candidate = text.startsWith("{") || text.startsWith("[");
  if (candidate) {
    try {
      return {
        structured: true,
        text: JSON.stringify(JSON.parse(text), null, 2),
      };
    } catch {
      // Keep original text when JSON parsing fails.
    }
  }

  return {
    structured:
      text.includes("\n") ||
      text.length > 140 ||
      text.includes("{") ||
      text.includes("["),
    text,
  };
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
      className={cn("roc-badge transition-colors hover:border-primary/35 hover:text-primary", className)}
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

function ReasoningBlock({ message }: { message: FeedMessage }) {
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

function StatusBlock({ message }: { message: OutputBlock }) {
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

function InfoBlock({ message }: { message: OutputBlock }) {
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

function ToolBlock({ message, active }: { message: OutputBlock; active: boolean }) {
  // Phase W2/W5: prefer live_identity for live tool cards, then fall back to
  // the persisted history marker injected by buildFeedFromHistory() so history
  // rebuild preserves TOOL RUNNING / TOOL RESULT semantics.
  const metadataPartKind =
    typeof message.metadata?.rocode_web_history_part_kind === "string"
      ? message.metadata.rocode_web_history_part_kind
      : null;
  const partKind = message.live_identity?.part_kind ?? metadataPartKind;
  const isRunning = partKind === "tool_call";
  const isResult = partKind === "tool_result";
  const label = isRunning ? "TOOL RUNNING" : isResult ? "TOOL RESULT" : "TOOL";
  const iconColor = isRunning ? "text-amber-500" : isResult ? "text-emerald-500" : "";
  const displaySummary = message.display?.summary?.trim() || null;
  const displayFields = message.display?.fields?.length ? message.display.fields : undefined;
  const displayPreviewText = message.display?.preview?.text?.trim() || null;
  const hasDisplayContract = Boolean(displaySummary || displayFields?.length || displayPreviewText);
  const summary =
    displaySummary ||
    message.summary?.trim() ||
    (!hasDisplayContract ? message.detail?.trim() || message.text?.trim() || null : null) ||
    null;
  const rawHeader = message.display?.header?.trim() || null;
  const rawTitle = message.title?.trim() || null;
  const rawName = message.name?.trim() || null;
  const skillLike = isSkillToolName(rawName ?? rawTitle ?? rawHeader ?? "");
  const toolTitle = rawHeader && !/^[A-Za-z0-9._:-]+$/.test(rawHeader)
    ? rawHeader
    : toolActivityLabel(rawHeader ?? rawTitle ?? rawName ?? message.kind);
  const fields = displayFields ?? message.fields;
  const previewText = displayPreviewText || (!hasDisplayContract ? message.preview?.trim() || null : null);
  const previewKind = message.display?.preview?.kind?.trim() || null;
  const previewTruncated = Boolean(message.display?.preview?.truncated);
  const compatDetailFallback =
    !hasDisplayContract && !fields?.length ? message.detail?.trim() || null : null;
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
          <div className="roc-message-meta-group">
            {message.tool_call_id ? <span className="roc-badge">tool {message.tool_call_id}</span> : null}
            {message.stage_id ? <span className="roc-badge">stage {message.stage_id}</span> : null}
          </div>
        ) : null}
        {fields?.length ? <FieldList fields={fields} /> : null}
        {compatDetailFallback ? (
          <StructuredText value={compatDetailFallback} className="text-muted-foreground" />
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
  const displayText = sanitizeDisplayedMessageText(message);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(displayText);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [displayText]);

  if (message.kind === "scheduler_stage") {
    return (
      <SchedulerStageCard
        message={message}
        highlighted={highlighted || Boolean(activeStageId && message.stage_id === activeStageId)}
        onNavigateStage={onNavigateStage}
        onNavigateAttachedSession={onNavigateAttachedSession}
      />
    );
  }

  if (message.kind === "reasoning") {
    if (!message.text.trim()) return null;
    return <ReasoningBlock message={message} />;
  }

  if (message.kind === "status") {
    return <StatusBlock message={message} />;
  }

  if (message.kind === "tool") {
    return (
      <ToolBlock
        message={message}
        active={Boolean(activeToolCallId && message.tool_call_id === activeToolCallId)}
      />
    );
  }

  if (message.kind === "multimodal_info") {
    return <InfoBlock message={message} />;
  }

  const role = message.role ?? "assistant";
  const isUser = role === "user";
  const roleLabel = isUser ? "USER" : "ASSIST";
  const clock = formatClock(message.ts);
  const summary = readableSummary(message);
  const cacheSummary = cacheBustSummaryFromMetadata(message.metadata);
  const cacheDiagnosticLabel = cacheBustSummaryStatusLabel(cacheSummary);
  const cacheDiagnosticDetail = cacheBustSummaryLabel(cacheSummary);
  const active =
    Boolean(activeStageId && message.stage_id === activeStageId) ||
    Boolean(activeToolCallId && message.tool_call_id === activeToolCallId);

  return (
    <article
      className={cn("grid min-w-0 gap-1", isUser && "justify-items-end")}
      data-testid="message-card"
      data-feed-id={message.feedId}
      data-message-anchor={message.anchorId}
      data-block-id={message.id}
      data-stage-id={message.stage_id}
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
              {clock ? <span className="roc-badge">{clock}</span> : null}
              {cacheDiagnosticLabel ? (
                <span
                  className="roc-badge border-amber-500/30 text-amber-700 dark:text-amber-300"
                  title={cacheDiagnosticDetail || "Prompt cache diagnostic"}
                >
                  cache {cacheDiagnosticLabel}
                </span>
              ) : null}
            </div>
            {message.stage_id || message.tool_call_id ? (
              <div className="roc-message-meta-group">
                {message.stage_id ? (
                  <MetaActionButton onClick={() => onNavigateStage(message.stage_id!)}>
                    stage {message.stage_id}
                  </MetaActionButton>
                ) : null}
                {message.tool_call_id ? <span className="roc-badge">tool {message.tool_call_id}</span> : null}
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
      {!isUser && message.attached_session_id ? (
        <div className="pl-1">
          <MetaActionButton
            className="roc-action roc-action-pill justify-center px-3.5 py-1.5 text-xs text-foreground no-underline"
            onClick={() =>
              onNavigateAttachedSession(message.attached_session_id!, {
                stageId: message.stage_id ?? null,
                toolCallId: message.tool_call_id ?? null,
                label: message.title || message.stage || message.attached_session_id,
              })
            }
          >
            <SparklesIcon className="mr-1 size-3.5" />
            Open attached session {message.title || message.attached_session_id}
          </MetaActionButton>
        </div>
      ) : null}
    </article>
  );
}
