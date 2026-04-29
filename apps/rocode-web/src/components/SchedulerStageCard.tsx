import { Button } from "@/components/ui/button";
import type { FeedMessage } from "@/lib/history";
import { humanizeStageEvent, humanizeStageWaitTarget } from "@/lib/stageSignals";
import { cn } from "@/lib/utils";
import {
  ActivityIcon,
  ArrowUpRightIcon,
  ChevronDownIcon,
  GitBranchPlusIcon,
  InfoIcon,
  WorkflowIcon,
} from "lucide-react";
import { useState } from "react";
import { MessageResponse } from "./ai-elements/message";

interface SchedulerStageCardProps {
  message: FeedMessage;
  highlighted?: boolean;
  onNavigateStage: (stageId: string) => void;
  onNavigateChildSession: (
    sessionId: string,
    context?: { stageId?: string | null; toolCallId?: string | null; label?: string | null },
  ) => void;
}

function tokenSummary(message: FeedMessage) {
  return [
    message.prompt_tokens ? `input ${formatCompactTokenCount(message.prompt_tokens)}` : null,
    message.completion_tokens ? `output ${formatCompactTokenCount(message.completion_tokens)}` : null,
    message.reasoning_tokens ? `reasoning ${formatCompactTokenCount(message.reasoning_tokens)}` : null,
    message.cache_read_tokens ? `cache read ${formatCompactTokenCount(message.cache_read_tokens)}` : null,
    message.cache_write_tokens ? `cache write ${formatCompactTokenCount(message.cache_write_tokens)}` : null,
  ].filter(Boolean);
}

function formatCompactTokenCount(value: number) {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  return String(Math.round(value));
}

function compactText(value: unknown) {
  return String(value ?? "").replace(/\s+/g, " ").trim();
}

function sectionSummary(value: unknown, maxLength = 140) {
  const text = compactText(value);
  if (!text) return null;
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1)}…`;
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
      // Fall back to raw text.
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

function StructuredValue({ value }: { value: unknown }) {
  const display = normalizeValue(value);
  if (!display.text) return null;

  if (display.structured) {
    return <pre className="roc-structured-value roc-structured-copy">{display.text}</pre>;
  }

  return <p className="roc-structured-copy text-sm leading-6 whitespace-pre-wrap text-foreground">{display.text}</p>;
}

function MarkdownSectionValue({ value }: { value: unknown }) {
  const text = String(value ?? "").trim();
  if (!text) return null;

  if (text.startsWith("{") || text.startsWith("[")) {
    try {
      return <pre className="roc-structured-value roc-structured-copy">{JSON.stringify(JSON.parse(text), null, 2)}</pre>;
    } catch {
      // Fall through to markdown rendering.
    }
  }

  return (
    <MessageResponse className="roc-markdown-flow roc-message-body size-full [&_p]:text-foreground/92">
      {text}
    </MessageResponse>
  );
}

function classifyFact(value: unknown) {
  const display = normalizeValue(value);
  const inline =
    !display.structured &&
    display.text.length > 0 &&
    display.text.length <= 48 &&
    !display.text.includes(",") &&
    !display.text.includes(":");
  return { display, inline };
}

function DisclosurePanel({
  icon,
  label,
  title,
  summary,
  defaultOpen = false,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  title: string;
  summary?: string | null;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <section className="roc-detail-card">
      <button
        type="button"
        className="roc-detail-trigger"
        onClick={() => setOpen((value) => !value)}
      >
        <div className="roc-detail-icon">{icon}</div>
        <div className="min-w-0 flex-1">
          <div className="roc-section-label">{label}</div>
          <div className="roc-detail-title">{title}</div>
          {summary ? <p className="roc-detail-summary line-clamp-2">{summary}</p> : null}
        </div>
        <ChevronDownIcon
          className={cn(
            "mt-1 ml-auto size-4 shrink-0 text-muted-foreground transition-transform duration-200",
            open && "rotate-180",
          )}
        />
      </button>
      <div
        className={cn(
          "overflow-hidden transition-all duration-200",
          open ? "max-h-[2600px]" : "max-h-0",
        )}
      >
        <div className={cn(open ? "roc-detail-body" : "pt-0")}>{children}</div>
      </div>
    </section>
  );
}

export function SchedulerStageCard({
  message,
  highlighted = false,
  onNavigateStage,
  onNavigateChildSession,
}: SchedulerStageCardProps) {
  const chips = [
    message.profile,
    message.status,
    message.stage_index && message.stage_total
      ? `${message.stage_index}/${message.stage_total}`
      : null,
    typeof message.step === "number" ? `step ${message.step}` : null,
  ].filter(Boolean);
  const tokens = tokenSummary(message);
  const stageTitle = message.title || message.stage || "Scheduler Stage";
  const waitingLabel = humanizeStageWaitTarget(message.waiting_on);
  const lastEventLabel = humanizeStageEvent(message.last_event);
  const stageSummary =
    compactText(message.focus) ||
    compactText(lastEventLabel) ||
    compactText(message.text) ||
    null;

  const decisionInlineFields = message.decision?.fields
    ?.map((field) => ({
      label: field.label ?? "Field",
      ...classifyFact(field.value ?? ""),
    }))
    .filter((field) => field.inline) ?? [];
  const decisionBlockFields = message.decision?.fields
    ?.map((field) => ({
      label: field.label ?? "Field",
      ...classifyFact(field.value ?? ""),
    }))
    .filter((field) => !field.inline) ?? [];

  return (
    <article
      className={cn(
        "roc-message-card grid gap-4 p-5",
        highlighted && "border-primary/35 bg-accent/34",
      )}
      data-testid="scheduler-stage-card"
      data-feed-id={message.feedId}
      data-stage-id={message.stage_id}
      data-child-session-id={message.child_session_id}
    >
      <div className="roc-message-meta-row">
        <div className="roc-message-meta-group">
          <span className="roc-section-label">Scheduler Stage</span>
          {(message.role ?? "assistant") !== "assistant" ? (
            <span className="roc-badge">{message.role}</span>
          ) : null}
        </div>
        {chips.length ? (
          <div className="roc-message-meta-group">
            {chips.map((chip, index) => (
              <span key={`${message.feedId}-chip-${index}`} className="roc-badge">
                {chip}
              </span>
            ))}
          </div>
        ) : null}
      </div>

      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-3">
            <div className="roc-icon-tile size-10" data-emphasis="strong">
              <WorkflowIcon className="size-4" />
            </div>
            <div className="min-w-0 flex-1">
              <h3 className="roc-message-title mb-0 text-lg">
                {stageTitle}
              </h3>
              {stageSummary ? (
                <p className="roc-detail-summary">{stageSummary}</p>
              ) : null}
            </div>
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {message.stage_id ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="roc-primary-action roc-action-pill"
              data-testid="scheduler-stage-open-stage"
              onClick={() => onNavigateStage(message.stage_id!)}
            >
              <ArrowUpRightIcon className="size-3.5" />
              stage {message.stage_id}
            </Button>
          ) : null}
          {message.child_session_id ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="roc-action roc-action-compact gap-1.5 px-3"
              data-testid="scheduler-stage-open-child"
              onClick={() =>
                onNavigateChildSession(message.child_session_id!, {
                  stageId: message.stage_id ?? null,
                  toolCallId: message.tool_call_id ?? null,
                  label: message.title || message.stage || message.stage_id || message.child_session_id,
                })
              }
            >
              <GitBranchPlusIcon className="size-3.5" />
              child {message.child_session_id}
            </Button>
          ) : null}
        </div>
      </div>

      {(waitingLabel || lastEventLabel) ? (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 border-t border-border/20 pt-3 text-xs text-muted-foreground">
          {waitingLabel ? (
            <div className="inline-flex min-w-0 items-baseline gap-2">
              <span className="roc-section-label">Waiting</span>
              <span className="text-sm font-medium text-foreground/78">{waitingLabel}</span>
            </div>
          ) : null}
          {lastEventLabel ? (
            <div className="inline-flex min-w-0 items-baseline gap-2">
              <span className="roc-section-label">Last Event</span>
              <span className="text-sm text-foreground/72">{lastEventLabel}</span>
            </div>
          ) : null}
        </div>
      ) : null}

      {tokens.length ? (
        <div className="flex flex-wrap gap-2">
          {tokens.map((token) => (
            <span key={`${message.feedId}-${token}`} className="roc-badge">
              {token}
            </span>
          ))}
        </div>
      ) : null}

      {message.decision ? (
        <section className="roc-detail-card">
          <div className="flex items-start gap-3">
            <div className="roc-detail-icon">
              <ActivityIcon className="size-4" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="roc-section-label">Decision</div>
              <div className="roc-detail-title">
                {message.decision.title || "Stage decision"}
              </div>
            </div>
          </div>
          <div className="roc-detail-body">
          {message.decision.fields?.length ? (
            <div className="grid gap-2.5">
              {decisionInlineFields.length ? (
                <div className="flex flex-wrap gap-2">
                  {decisionInlineFields.map((field, index) => (
                    <span key={`${message.feedId}-decision-inline-${index}`} className="roc-inline-fact">
                      <span className="roc-inline-fact-label">{field.label}</span>
                      <span className="roc-inline-fact-value">{field.display.text}</span>
                    </span>
                  ))}
                </div>
              ) : null}
              {decisionBlockFields.length ? (
                <dl className="roc-structured-dl">
                  {decisionBlockFields.map((field, index) => (
                    <div key={`${message.feedId}-decision-field-${index}`} className="roc-structured-row">
                      <dt className="roc-structured-key">{field.label}</dt>
                      <dd className="m-0">
                        <StructuredValue value={field.display.text} />
                      </dd>
                    </div>
                  ))}
                </dl>
              ) : null}
            </div>
          ) : null}
          {message.decision.sections?.map((section, index) => (
            (() => {
              const title = section.title || `Section ${index + 1}`;
              const isResponseSection = title.trim().toLowerCase() === "response";

              if (isResponseSection) {
                return (
                  <DisclosurePanel
                    key={`${message.feedId}-decision-section-${index}`}
                    icon={<InfoIcon className="size-4" />}
                    label="Response"
                    title={title}
                    summary={sectionSummary(section.body || "")}
                    defaultOpen={false}
                  >
                    <MarkdownSectionValue value={section.body || ""} />
                  </DisclosurePanel>
                );
              }

              return (
                <div key={`${message.feedId}-decision-section-${index}`} className="roc-detail-card">
                  <div className="roc-section-label">{title}</div>
                  <div className="roc-detail-body">
                    <MarkdownSectionValue value={section.body || ""} />
                  </div>
                </div>
              );
            })()
          ))}
          </div>
        </section>
      ) : null}

      {message.activity ? (
        <DisclosurePanel
          icon={<ActivityIcon className="size-4" />}
          label="Trace"
          title="Activity trace"
          summary="Collapsed raw activity log for this stage."
          defaultOpen={false}
        >
          <StructuredValue value={message.activity} />
        </DisclosurePanel>
      ) : null}

      {(message.active_skills?.length || message.active_agents?.length || message.active_categories?.length) ? (
        <div className="grid gap-2">
          {message.active_skills?.length ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className="roc-section-label">Skills</span>
              {message.active_skills.map((skill) => (
                <span key={`${message.feedId}-skill-${skill}`} className="roc-badge">
                  {skill}
                </span>
              ))}
            </div>
          ) : null}
          {message.active_agents?.length ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className="roc-section-label">Agents</span>
              {message.active_agents.map((agent) => (
                <span key={`${message.feedId}-agent-${agent}`} className="roc-badge">
                  {agent}
                </span>
              ))}
            </div>
          ) : null}
          {message.active_categories?.length ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className="roc-section-label">Categories</span>
              {message.active_categories.map((category) => (
                <span key={`${message.feedId}-category-${category}`} className="roc-badge">
                  {category}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}
