import { useState } from "react";
import type { WorkspacePanelTab } from "../components/workspace/WorkspacePanel";

export interface WorkspaceSelectionState {
  panelTab: WorkspacePanelTab;
  selectedPath: string | null;
  selectedType: "file" | "directory";
  pendingSelection: { path: string; type: "file" | "directory" } | null;
  reloadToken: number;
  triggerReload: () => void;
}

export function useWorkspaceSelection(): [
  WorkspaceSelectionState,
  {
    setPanelTab: (t: WorkspacePanelTab) => void;
    setSelectedPath: (p: string | null) => void;
    setSelectedType: (t: "file" | "directory") => void;
    setPendingSelection: (
      s: { path: string; type: "file" | "directory" } | null,
    ) => void;
  },
] {
  const [panelTab, setPanelTab] = useState<WorkspacePanelTab>("files");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedType, setSelectedType] = useState<"file" | "directory">(
    "directory",
  );
  const [pendingSelection, setPendingSelection] = useState<{
    path: string;
    type: "file" | "directory";
  } | null>(null);
  const [reloadToken, setReloadToken] = useState(0);

  return [
    {
      panelTab,
      selectedPath,
      selectedType,
      pendingSelection,
      reloadToken,
      triggerReload: () => setReloadToken((n) => n + 1),
    },
    { setPanelTab, setSelectedPath, setSelectedType, setPendingSelection },
  ];
}
