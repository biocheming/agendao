"use client";

import { useCallback, useEffect, useState } from "react";
import type { useExecutionActivity } from "../hooks/useExecutionActivity";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  ArrowRightIcon,
  CheckCircleIcon,
  CheckIcon,
  ChevronRightIcon,
  ClockIcon,
  CopyIcon,
  LoaderCircleIcon,
  MessageSquareIcon,
  PlayIcon,
  WrenchIcon,
  XCircleIcon,
} from "lucide-react";

type ExecutionActivityState = ReturnType<typeof useExecutionActivity>;

interface ProvenanceTimelineProps {
  sessionId: string;
  activity: ExecutionActivityState;
  onNavigateStage?: (stageId: string) => void;
  className?: string;
}

function formatTimestamp(ts: number): string {
  const date = new Date(ts);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);

  if (diffMins < 1) return "just now";
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffMins < 1440) return `${Math.floor(diffMins / 60)}h ago`;
  return date.toLocaleDateString();
}

function eventWindowLabel(page: number, count: number, pageSize: number) {
  if (count === 0) return `page ${page} · items 0`;
  const start = (page - 1) * pageSize + 1;
  const end = start + count - 1;
  return `page ${page} · items ${start}-${end}`;
}

function getEventIcon(eventType: string) {
  if (eventType.includes("start") || eventType.includes("begin")) {
    return <PlayIcon className="size-3.5" />;
  }
  if (eventType.includes("complete") || eventType.includes("done")) {
    return <CheckCircleIcon className="size-3.5 text-green-500" />;
  }
  if (eventType.includes("fail") || eventType.includes("error")) {
    return <XCircleIcon className="size-3.5 text-red-500" />;
  }
  if (eventType.includes("message") || eventType.includes("user")) {
    return <MessageSquareIcon className="size-3.5" />;
  }
  if (eventType.includes("tool") || eventType.includes("call")) {
    return <WrenchIcon className="size-3.5" />;
  }
  return <ClockIcon className="size-3.5" />;
}

function getEventLabel(eventType: string): string {
  return eventType
    .replace(/^execution\./, "")
    .replace(/^stage\./, "")
    .replace(/\./g, " ")
    .replace(/_/g, " ")
    .split(" ")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function EventCard({
  event,
  selected,
  onSelect,
  onNavigateStage,
}: {
  event: ExecutionActivityState["activityEvents"][number];
  selected: boolean;
  onSelect: (eventId: string | null) => void;
  onNavigateStage?: (stageId: string) => void;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(JSON.stringify(event, null, 2));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [event]);

  return (
    <button
      className="relative pl-6 pb-4 text-left w-full border-0 bg-transparent"
      type="button"
      onClick={() => onSelect(event.event_id || null)}
    >
      <div className="absolute left-[9px] top-2 bottom-0 w-px bg-border" />
      <div className="absolute left-0 top-1.5 w-5 h-5 rounded-full bg-background border-2 border-primary flex items-center justify-center">
        <div className={cn("w-2 h-2 rounded-full", selected ? "bg-primary" : "bg-primary/60")} />
      </div>

      <div
        className={cn(
          "rounded-lg border p-3 ml-4 transition-colors",
          selected ? "bg-primary/10 border-primary/40" : "bg-card/80 hover:bg-card",
        )}
      >
        <div className="flex items-center justify-between gap-2 mb-2">
          <div className="flex items-center gap-2 min-w-0">
            <span className="text-muted-foreground shrink-0">
              {getEventIcon(event.event_type || "event")}
            </span>
            <span className="font-medium text-sm truncate">
              {getEventLabel(event.event_type || "event")}
            </span>
          </div>
          <div className="flex items-center gap-1 shrink-0">
            {event.ts ? (
              <span className="text-xs text-muted-foreground">
                {formatTimestamp(event.ts)}
              </span>
            ) : null}
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-6"
              onClick={(copyEvent) => {
                copyEvent.stopPropagation();
                void handleCopy();
              }}
            >
              {copied ? <CheckIcon className="size-3" /> : <CopyIcon className="size-3" />}
            </Button>
          </div>
        </div>

        <div className="flex flex-wrap gap-2 text-xs">
          {event.stage_id ? (
            <button
              className="flex items-center gap-1 px-2 py-1 rounded-full bg-primary/10 text-primary hover:bg-primary/20 transition-colors"
              type="button"
              onClick={(clickEvent) => {
                clickEvent.stopPropagation();
                onNavigateStage?.(event.stage_id || "");
              }}
              title="Navigate to stage"
            >
              <span className="font-mono">{event.stage_id.slice(0, 8)}</span>
              <ChevronRightIcon className="size-3" />
            </button>
          ) : null}
          {event.execution_id ? (
            <span className="flex items-center gap-1 px-2 py-1 rounded-full bg-muted text-muted-foreground font-mono">
              {event.execution_id.slice(0, 8)}
            </span>
          ) : null}
          {event.event_type ? (
            <span className="px-2 py-1 rounded-full bg-muted text-muted-foreground">
              {event.event_type}
            </span>
          ) : null}
        </div>

        {event.scope ? (
          <div className="mt-2">
            <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
              {event.scope}
            </span>
          </div>
        ) : null}

        {event.summary ? (
          <div className="mt-2 text-xs text-muted-foreground">{event.summary}</div>
        ) : null}

        {event.payload && Object.keys(event.payload).length > 0 ? (
          <div className="mt-2 text-xs text-muted-foreground break-all">
            {JSON.stringify(event.payload).slice(0, 160)}
            {JSON.stringify(event.payload).length > 160 ? "..." : ""}
          </div>
        ) : null}
      </div>
    </button>
  );
}

export function ProvenanceTimeline({
  sessionId,
  activity,
  onNavigateStage,
  className,
}: ProvenanceTimelineProps) {
  const [pageDraft, setPageDraft] = useState(String(activity.activityPage));

  useEffect(() => {
    setPageDraft(String(activity.activityPage));
  }, [activity.activityPage]);

  const handleExport = useCallback(() => {
    const markdown = activity.activityEvents
      .map((event) => {
        const ts = event.ts ? new Date(event.ts).toISOString() : "";
        return `- **[${getEventLabel(event.event_type || "event")}]** ${ts}\n  - stage: \`${event.stage_id || "N/A"}\`\n  - execution: \`${event.execution_id || "N/A"}\``;
      })
      .join("\n");

    const content = `# Session Provenance\n\nSession: ${sessionId}\nExported: ${new Date().toISOString()}\n\n## Events\n\n${markdown}`;

    const blob = new Blob([content], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `provenance-${sessionId.slice(0, 8)}.md`;
    link.click();
    URL.revokeObjectURL(url);
  }, [activity.activityEvents, sessionId]);

  return (
    <div className={cn("flex flex-col h-full overflow-hidden", className)}>
      <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-muted/30">
        <div className="flex items-center gap-2 min-w-0">
          <ClockIcon className="size-4 shrink-0" />
          <h3 className="font-medium text-sm">Provenance</h3>
          <span className="text-xs text-muted-foreground truncate">
            ({activity.activityEvents.length})
          </span>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            onClick={() =>
              void activity.refreshExecutionActivity(
                undefined,
                activity.activityFilters,
                activity.activityPage,
              )
            }
            disabled={activity.activityLoading}
          >
            <LoaderCircleIcon
              className={cn("size-4", activity.activityLoading && "animate-spin")}
            />
          </Button>
          {activity.activityEvents.length > 0 ? (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7"
              onClick={handleExport}
              title="Export as Markdown"
            >
              <ArrowRightIcon className="size-4" />
            </Button>
          ) : null}
        </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-[1fr_1fr_1fr_auto] gap-3 p-3 border-b border-border bg-background/50">
        <label className="grid gap-1 text-xs text-muted-foreground">
          <span>Stage</span>
          <select
            className="h-9 rounded-md border border-input bg-transparent px-2 text-sm text-foreground"
            value={activity.activityFilters.stageId}
            onChange={(event) => activity.patchActivityFilters({ stageId: event.target.value })}
          >
            <option value="">all stages</option>
            {activity.stageOptions.map((stageId) => (
              <option key={stageId} value={stageId}>
                {stageId}
              </option>
            ))}
          </select>
        </label>
        <label className="grid gap-1 text-xs text-muted-foreground">
          <span>Execution</span>
          <select
            className="h-9 rounded-md border border-input bg-transparent px-2 text-sm text-foreground"
            value={activity.activityFilters.executionId}
            onChange={(event) =>
              activity.patchActivityFilters({ executionId: event.target.value })
            }
          >
            <option value="">all executions</option>
            {activity.executionNodes.map((node) => (
              <option key={node.id} value={node.id}>
                {node.label || node.id}
              </option>
            ))}
          </select>
        </label>
        <label className="grid gap-1 text-xs text-muted-foreground">
          <span>Event Type</span>
          <select
            className="h-9 rounded-md border border-input bg-transparent px-2 text-sm text-foreground"
            value={activity.activityFilters.eventType}
            onChange={(event) => activity.patchActivityFilters({ eventType: event.target.value })}
          >
            <option value="">all events</option>
            {activity.knownEventTypes.map((eventType) => (
              <option key={eventType} value={eventType}>
                {eventType}
              </option>
            ))}
          </select>
        </label>
        <div className="flex items-end">
          <Button
            variant="outline"
            className="w-full md:w-auto"
            onClick={activity.clearActivityFilters}
          >
            Clear
          </Button>
        </div>
      </div>

      <div className="flex-1 overflow-auto p-3">
        {activity.activityLoading && activity.activityEvents.length === 0 ? (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            <LoaderCircleIcon className="size-5 animate-spin mr-2" />
            Loading events...
          </div>
        ) : activity.activityEvents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-muted-foreground text-sm">
            <ClockIcon className="size-8 opacity-30 mb-2" />
            <p>No events recorded</p>
            <p className="text-xs mt-1">Events will appear as the session runs</p>
          </div>
        ) : (
          <div className="space-y-0">
            {activity.activityEvents.map((event, idx) => (
              <EventCard
                key={event.event_id || `${event.ts || "event"}:${event.event_type || idx}`}
                event={event}
                selected={activity.selectedEventId === event.event_id}
                onSelect={activity.setSelectedEventId}
                onNavigateStage={onNavigateStage}
              />
            ))}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border bg-background/60 px-3 py-2">
        <p className="text-xs text-muted-foreground">
          {eventWindowLabel(
            activity.activityPage,
            activity.activityEvents.length,
            activity.activityPageSize,
          )}{" "}
          · limit {activity.activityPageSize}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={!activity.activityHasPreviousPage}
            onClick={activity.firstActivityPage}
          >
            First
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={!activity.activityHasPreviousPage}
            onClick={activity.previousActivityPage}
          >
            Prev
          </Button>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <span>Page</span>
            <input
              className="h-8 w-16 rounded-md border border-input bg-transparent px-2 py-1 text-sm text-foreground"
              type="number"
              min={1}
              step={1}
              value={pageDraft}
              onChange={(event) => setPageDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  const page = Number.parseInt(pageDraft, 10);
                  activity.goToActivityPage(Number.isFinite(page) ? page : 1);
                }
              }}
            />
          </label>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              const page = Number.parseInt(pageDraft, 10);
              activity.goToActivityPage(Number.isFinite(page) ? page : 1);
            }}
          >
            Go
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={!activity.activityHasNextPage}
            onClick={activity.nextActivityPage}
          >
            Next
          </Button>
        </div>
      </div>
    </div>
  );
}
