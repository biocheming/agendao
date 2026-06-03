"use client";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  LoaderCircleIcon,
  CheckCircleIcon,
  XCircleIcon,
  ClockIcon,
  XIcon,
  RefreshCwIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

interface TaskSummary {
  id: string;
  agent_name: string;
  status: string;
  step: number | null;
  max_steps: number | null;
  prompt: string;
  started_at: number;
  elapsed_seconds: number;
}

interface TaskDetail extends TaskSummary {
  finished_at: number | null;
  output_tail: string[];
}

interface TaskPanelProps {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onError?: (message: string) => void;
  className?: string;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

function StatusIcon({ status }: { status: string }) {
  if (status === "completed") {
    return <CheckCircleIcon className="size-4 text-green-500" />;
  }
  if (status === "failed") {
    return <XCircleIcon className="size-4 text-red-500" />;
  }
  if (status === "running") {
    return <LoaderCircleIcon className="size-4 text-blue-500 animate-spin" />;
  }
  if (status === "cancelled") {
    return <XCircleIcon className="size-4 text-yellow-500" />;
  }
  return <ClockIcon className="size-4 text-muted-foreground" />;
}

function TaskCard({
  task,
  onCancel,
  onRefresh,
}: {
  task: TaskSummary;
  onCancel?: (id: string) => void;
  onRefresh?: (id: string) => void;
}) {
  const [elapsed, setElapsed] = useState(task.elapsed_seconds);

  useEffect(() => {
    if (task.status !== "running" && task.status !== "pending") {
      return;
    }
    const interval = setInterval(() => {
      setElapsed((prev) => prev + 1);
    }, 1000);
    return () => clearInterval(interval);
  }, [task.status]);

  return (
    <div
      className={cn(
        "rounded-xl border bg-card/80 p-4 grid gap-3",
        task.status === "running" && "border-blue-500/30",
        task.status === "completed" && "border-green-500/20",
        task.status === "failed" && "border-red-500/30",
        task.status === "cancelled" && "border-yellow-500/20"
      )}
    >
      {/* Header */}
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <StatusIcon status={task.status} />
          <span className="font-medium text-sm truncate">{task.agent_name}</span>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {task.status === "running" && onCancel && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7"
              onClick={() => onCancel(task.id)}
            >
              <XIcon className="size-3" />
            </Button>
          )}
          {onRefresh && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7"
              onClick={() => onRefresh(task.id)}
            >
              <RefreshCwIcon className="size-3" />
            </Button>
          )}
        </div>
      </div>

      {/* Prompt preview */}
      <p className="text-xs text-muted-foreground line-clamp-2">{task.prompt}</p>

      {/* Status line */}
      <div className="flex items-center justify-between text-xs">
        <div className="flex items-center gap-2">
          {task.step !== null && task.max_steps !== null ? (
            <span className="text-muted-foreground">
              Step {task.step + 1} / {task.max_steps}
            </span>
          ) : null}
          <span
            className={cn(
              "px-2 py-0.5 rounded-full text-xs",
              task.status === "running" && "bg-blue-500/10 text-blue-600 dark:text-blue-400",
              task.status === "completed" && "bg-green-500/10 text-green-600 dark:text-green-400",
              task.status === "failed" && "bg-red-500/10 text-red-600 dark:text-red-400",
              task.status === "cancelled" && "bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
              task.status === "pending" && "bg-muted text-muted-foreground"
            )}
          >
            {task.status}
          </span>
        </div>
        <span className="text-muted-foreground font-mono">{formatDuration(elapsed)}</span>
      </div>

      {/* Progress bar for running tasks */}
      {task.status === "running" && task.step !== null && task.max_steps !== null && (
        <div className="h-1 bg-muted rounded-full overflow-hidden">
          <div
            className="h-full bg-blue-500 transition-all duration-300"
            style={{ width: `${((task.step + 1) / task.max_steps) * 100}%` }}
          />
        </div>
      )}
    </div>
  );
}

export function TaskPanel({
  apiJson,
  onError,
  className,
}: TaskPanelProps) {
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  const loadTasks = useCallback(async () => {
    try {
      const data = await apiJson<TaskSummary[]>("/task");
      setTasks(data);
    } catch (err) {
      onError?.(`Failed to load tasks: ${err}`);
    } finally {
      setLoading(false);
    }
  }, [apiJson, onError]);

  useEffect(() => {
    loadTasks();
    // Poll for updates every 3 seconds
    const interval = setInterval(loadTasks, 3000);
    return () => clearInterval(interval);
  }, [loadTasks]);

  const handleCancel = useCallback(
    async (id: string) => {
      try {
        await apiJson(`/task/${id}`, { method: "DELETE" });
        setTasks((prev) =>
          prev.map((t) => (t.id === id ? { ...t, status: "cancelled" } : t))
        );
      } catch (err) {
        onError?.(`Failed to cancel task: ${err}`);
      }
    },
    [apiJson, onError]
  );

  const runningTasks = tasks.filter((t) => t.status === "running" || t.status === "pending");
  const completedTasks = tasks.filter(
    (t) => t.status === "completed" || t.status === "failed" || t.status === "cancelled"
  );

  return (
    <div className={cn("flex flex-col h-full overflow-hidden", className)}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border bg-muted/30">
        <div className="flex items-center gap-2">
          <h3 className="font-medium text-sm">Tasks</h3>
          {runningTasks.length > 0 && (
            <span className="px-2 py-0.5 rounded-full bg-blue-500/10 text-blue-600 dark:text-blue-400 text-xs">
              {runningTasks.length} active
            </span>
          )}
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          className="size-7"
          onClick={() => {
            setRefreshing(true);
            void loadTasks().then(() => setRefreshing(false));
          }}
          disabled={refreshing}
        >
          <RefreshCwIcon className={cn("size-4", refreshing && "animate-spin")} />
        </Button>
      </div>

      {/* Task list */}
      <div className="flex-1 overflow-auto p-3 space-y-3">
        {loading ? (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            <LoaderCircleIcon className="size-5 animate-spin mr-2" />
            Loading tasks...
          </div>
        ) : tasks.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-muted-foreground text-sm">
            <ClockIcon className="size-8 opacity-30 mb-2" />
            <p>No active tasks</p>
            <p className="text-xs mt-1">Tasks will appear here when agents are running</p>
          </div>
        ) : (
          <>
            {/* Running/Pending tasks first */}
            {runningTasks.map((task) => (
              <TaskCard
                key={task.id}
                task={task}
                onCancel={handleCancel}
              />
            ))}

            {/* Completed tasks */}
            {completedTasks.length > 0 && runningTasks.length > 0 && (
              <div className="pt-2">
                <p className="text-xs text-muted-foreground mb-2 uppercase tracking-wider">
                  Recent ({completedTasks.length})
                </p>
              </div>
            )}
            {completedTasks.map((task) => (
              <TaskCard
                key={task.id}
                task={task}
                onRefresh={loadTasks}
              />
            ))}
          </>
        )}
      </div>
    </div>
  );
}
