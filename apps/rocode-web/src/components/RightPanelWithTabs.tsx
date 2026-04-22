"use client";

import { cn } from "@/lib/utils";
import type { useExecutionActivity } from "../hooks/useExecutionActivity";
import {
  ClockIcon,
  GitBranchIcon,
  ListTodoIcon,
} from "lucide-react";
import { useState } from "react";
import { TaskPanel } from "./TaskPanel";
import { WorktreePanel } from "./WorktreePanel";
import { ProvenanceTimeline } from "./ProvenanceTimeline";

type TabId = "tasks" | "worktrees" | "provenance";

const tabs: { id: TabId; label: string; icon: React.ReactNode }[] = [
  { id: "tasks", label: "Tasks", icon: <ListTodoIcon className="size-4" /> },
  { id: "worktrees", label: "Worktrees", icon: <GitBranchIcon className="size-4" /> },
  { id: "provenance", label: "Provenance", icon: <ClockIcon className="size-4" /> },
];

interface RightPanelWithTabsProps {
  sessionId?: string;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onBanner: (message: string | null) => void;
  executionActivity: ReturnType<typeof useExecutionActivity>;
  onNavigateStage?: (stageId: string) => void;
}

export function RightPanelWithTabs({
  sessionId,
  apiJson,
  onBanner,
  executionActivity,
  onNavigateStage,
}: RightPanelWithTabsProps) {
  const [activeTab, setActiveTab] = useState<TabId>("tasks");

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Tab bar */}
      <div className="flex items-center border-b border-border bg-muted/20">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            className={cn(
              "flex items-center gap-1.5 px-3 py-2.5 text-sm border-b-2 transition-colors",
              activeTab === tab.id
                ? "border-primary text-foreground bg-background"
                : "border-transparent text-muted-foreground hover:text-foreground hover:bg-muted/50"
            )}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.icon}
            <span className="hidden sm:inline">{tab.label}</span>
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {activeTab === "tasks" && (
          <TaskPanel apiJson={apiJson} onError={onBanner} className="h-full" />
        )}
        {activeTab === "worktrees" && (
          <WorktreePanel className="h-full" />
        )}
        {activeTab === "provenance" && sessionId && (
          <ProvenanceTimeline
            sessionId={sessionId}
            activity={executionActivity}
            onNavigateStage={onNavigateStage}
            className="h-full"
          />
        )}
        {activeTab === "provenance" && !sessionId && (
          <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
            Select a session to view provenance
          </div>
        )}
      </div>
    </div>
  );
}
