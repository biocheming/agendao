import { useEffect, useMemo, useState } from "react";
import {
  CheckSquare2,
  ChevronDown,
  ChevronRight,
  FolderPlus,
  FolderTree,
  Layers2,
  PanelLeftClose,
  Search,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { SessionTreeNode, WorkspaceSummary } from "@/lib/sidebar";
import { useI18n } from "@/i18n/I18nProvider";

const AGENDAO_LOGO_SRC = `${import.meta.env.BASE_URL}brand/agendao-logo.svg`;

interface SessionSidebarProps {
  workspaces: WorkspaceSummary[];
  currentWorkspacePath: string | null;
  currentWorkspaceLabel: string | null;
  currentWorkspaceRootPath: string | null;
  currentWorkspaceMode: "shared" | "isolated" | null;
  sessionTree: SessionTreeNode[];
  selectedSessionId: string | null;
  deletingSessions?: boolean;
  onCreateProject: (input: { path: string; title?: string }) => void;
  onCreateSession: () => void;
  onDeleteSessions: (sessionIds: string[]) => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onSelectSession: (sessionId: string) => void;
  onHideSidebar: () => void;
}

function flattenSessionIds(nodes: SessionTreeNode[]): string[] {
  return nodes.flatMap((node) => [node.id, ...flattenSessionIds(node.children)]);
}

function workspaceModeLabel(mode: "shared" | "isolated" | null) {
  if (mode === "shared") return "sidebar.shared";
  if (mode === "isolated") return "sidebar.isolated";
  return null;
}

function workspacePathHint(path: string | null, rootPath: string | null) {
  const normalizedPath = path?.trim();
  if (!normalizedPath) return null;
  const normalizedRoot = rootPath?.trim();
  if (!normalizedRoot || normalizedRoot === normalizedPath) return normalizedPath;
  if (normalizedPath.startsWith(`${normalizedRoot}/`)) {
    return normalizedPath.slice(normalizedRoot.length + 1);
  }
  return normalizedPath;
}

function compactPathLabel(path: string | null) {
  const normalizedPath = path?.trim();
  if (!normalizedPath) return null;
  const segments = normalizedPath.split("/").filter(Boolean);
  return segments[segments.length - 1] || normalizedPath;
}

function SessionTreeList({
  nodes,
  selectedSessionId,
  selectionMode,
  selectedIds,
  collapsedIds,
  depth = 0,
  onToggleCollapsed,
  onToggleSelected,
  onSelectSession,
}: {
  nodes: SessionTreeNode[];
  selectedSessionId: string | null;
  selectionMode: boolean;
  selectedIds: Set<string>;
  collapsedIds: Set<string>;
  depth?: number;
  onToggleCollapsed: (sessionId: string) => void;
  onToggleSelected: (sessionId: string) => void;
  onSelectSession: (sessionId: string) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="flex flex-col gap-1">
      {nodes.map((node) => {
        const secondaryLabel =
          node.children.length > 0
            ? t("sidebar.branchCount", { count: node.children.length })
            : depth === 0
              ? t("sidebar.root")
              : t("sidebar.threadDepth", { depth });

        return (
          <div key={node.id} className="flex flex-col gap-1">
            <div className="flex items-start gap-1.25" style={{ paddingLeft: `${depth * 10}px` }}>
              <div className="flex w-4.5 shrink-0 justify-center pt-2">
              {node.children.length > 0 ? (
                <button
                  type="button"
                  className="roc-sidebar-toggle"
                  aria-label={collapsedIds.has(node.id) ? "Expand session" : "Collapse session"}
                  onClick={() => onToggleCollapsed(node.id)}
                >
                  {collapsedIds.has(node.id) ? (
                    <ChevronRight className="h-3 w-3" />
                  ) : (
                    <ChevronDown className="h-3 w-3" />
                  )}
                </button>
              ) : (
                <span className="mt-1.5 h-1 w-1 rounded-full bg-border/80" />
              )}
              </div>

              <button
                type="button"
                data-testid="session-item"
                data-session-id={node.id}
                data-active={node.id === selectedSessionId ? "true" : "false"}
                className="roc-sidebar-item min-w-0 flex-1"
                title={node.title || t("sidebar.noTitle")}
                onClick={() => {
                  if (selectionMode) {
                    onToggleSelected(node.id);
                    return;
                  }
                  onSelectSession(node.id);
                }}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[12.5px] font-medium leading-4.5 tracking-tight text-foreground">
                      {node.title || t("sidebar.noTitle")}
                    </div>
                    <div className="mt-0.5 text-[10px] leading-4 text-muted-foreground">
                      {secondaryLabel}
                    </div>
                  </div>
                  {selectionMode ? (
                    <button
                      type="button"
                      className="roc-sidebar-toggle mt-0.25 shrink-0"
                      aria-label={selectedIds.has(node.id) ? t("sidebar.deselectSession") : t("sidebar.selectSession")}
                      onClick={(event) => {
                        event.stopPropagation();
                        onToggleSelected(node.id);
                      }}
                    >
                      {selectedIds.has(node.id) ? (
                        <CheckSquare2 className="h-3.5 w-3.5" />
                      ) : (
                        <Square className="h-3.5 w-3.5" />
                      )}
                    </button>
                  ) : null}
                </div>
              </button>
            </div>

            {node.children.length > 0 && !collapsedIds.has(node.id) ? (
              <SessionTreeList
                nodes={node.children}
                selectedSessionId={selectedSessionId}
                selectionMode={selectionMode}
                selectedIds={selectedIds}
                collapsedIds={collapsedIds}
                depth={depth + 1}
                onToggleCollapsed={onToggleCollapsed}
                onToggleSelected={onToggleSelected}
                onSelectSession={onSelectSession}
              />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export function SessionSidebar({
  workspaces,
  currentWorkspacePath,
  currentWorkspaceLabel,
  currentWorkspaceRootPath,
  currentWorkspaceMode,
  sessionTree,
  selectedSessionId,
  deletingSessions = false,
  onCreateProject,
  onCreateSession,
  onDeleteSessions,
  onSelectWorkspace,
  onSelectSession,
  onHideSidebar,
}: SessionSidebarProps) {
  const { t } = useI18n();
  const [workspaceQuery, setWorkspaceQuery] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [newProjectPath, setNewProjectPath] = useState("");
  const [newProjectTitle, setNewProjectTitle] = useState("");
  const [collapsedSessionIds, setCollapsedSessionIds] = useState<Set<string>>(new Set());
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const workspaceModeKey = workspaceModeLabel(currentWorkspaceMode);
  const currentWorkspaceHint = workspacePathHint(currentWorkspacePath, currentWorkspaceRootPath);
  const currentWorkspaceShort = compactPathLabel(currentWorkspacePath) || currentWorkspaceLabel;

  const filteredWorkspaces = useMemo(() => {
    const query = workspaceQuery.trim().toLowerCase();
    if (!query) return workspaces;
    return workspaces.filter(
      (workspace) =>
        workspace.label.toLowerCase().includes(query) ||
        workspace.path.toLowerCase().includes(query),
    );
  }, [workspaceQuery, workspaces]);

  const visibleSessionCount = useMemo(() => {
    const walk = (nodes: SessionTreeNode[]): number =>
      nodes.reduce((total, node) => total + 1 + walk(node.children), 0);
    return walk(sessionTree);
  }, [sessionTree]);
  const showProjectsSection = workspaces.length > 1 || workspaceQuery.trim().length > 0;
  const validSessionIds = useMemo(() => new Set(flattenSessionIds(sessionTree)), [sessionTree]);
  const selectedCount = useMemo(
    () => Array.from(selectedSessionIds).filter((id) => validSessionIds.has(id)).length,
    [selectedSessionIds, validSessionIds],
  );

  useEffect(() => {
    setSelectedSessionIds((current) => {
      const next = new Set(Array.from(current).filter((id) => validSessionIds.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [validSessionIds]);

  useEffect(() => {
    if (sessionTree.length > 0) return;
    setSelectionMode(false);
    setSelectedSessionIds(new Set());
    setDeleteConfirmOpen(false);
  }, [sessionTree.length]);

  useEffect(() => {
    if (!deletingSessions) return;
    setDeleteConfirmOpen(false);
    setSelectionMode(false);
    setSelectedSessionIds(new Set());
  }, [deletingSessions]);

  const submitCreateProject = () => {
    const path = newProjectPath.trim();
    if (!path) return;
    onCreateProject({
      path,
      title: newProjectTitle.trim() || undefined,
    });
    setCreateOpen(false);
    setNewProjectPath("");
    setNewProjectTitle("");
  };

  const toggleCollapsed = (sessionId: string) => {
    setCollapsedSessionIds((current) => {
      const next = new Set(current);
      if (next.has(sessionId)) {
        next.delete(sessionId);
      } else {
        next.add(sessionId);
      }
      return next;
    });
  };

  const toggleSelected = (sessionId: string) => {
    setSelectedSessionIds((current) => {
      const next = new Set(current);
      if (next.has(sessionId)) {
        next.delete(sessionId);
      } else {
        next.add(sessionId);
      }
      return next;
    });
  };

  const exitSelectionMode = () => {
    setSelectionMode(false);
    setSelectedSessionIds(new Set());
    setDeleteConfirmOpen(false);
  };

  const startSelectionMode = () => {
    setSelectionMode(true);
    setSelectedSessionIds((current) => {
      if (selectedSessionId && validSessionIds.has(selectedSessionId)) {
        const next = new Set(current);
        next.add(selectedSessionId);
        return next;
      }
      return current;
    });
  };

  const confirmDeleteSelection = () => {
    const ids = Array.from(selectedSessionIds).filter((id) => validSessionIds.has(id));
    if (ids.length === 0) return;
    onDeleteSessions(ids);
  };

  return (
    <aside className="roc-sidebar-shell flex h-full flex-col" data-testid="session-sidebar">
      <div className="flex flex-1 flex-col gap-2.5 overflow-y-auto px-3 py-3">
        <section className="flex flex-col items-start gap-2 px-1 pt-1">
          <div className="flex px-0.5 py-1">
            <img
              src={AGENDAO_LOGO_SRC}
              alt="AgenDao"
              className="h-8 w-auto max-w-[9.5rem] object-contain"
              draggable={false}
            />
          </div>
          <button
            type="button"
            className="inline-flex items-center gap-2 rounded-full border border-border/45 bg-background/68 px-3 py-1.5 text-[11px] font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            title={t("sidebar.hideSidebarTitle")}
            onClick={onHideSidebar}
          >
            <PanelLeftClose className="h-3.5 w-3.5" />
            <span>{t("sidebar.hideSidebar")}</span>
          </button>
        </section>

        <section className="px-1 pt-1 pb-1.5">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-[10px] font-semibold uppercase tracking-[0.22em] text-muted-foreground">
                {t("sidebar.workspace")}
              </p>
              <h2 className="mt-1 text-[15px] font-semibold tracking-tight text-foreground">
                {currentWorkspaceShort || t("sidebar.chooseWorkspace")}
              </h2>
              {currentWorkspaceHint && currentWorkspaceHint !== currentWorkspaceShort ? (
                <p className="mt-0.5 truncate text-[10px] text-muted-foreground">{currentWorkspaceHint}</p>
              ) : null}
            </div>
            {workspaceModeKey ? <span className="roc-badge px-2.5 py-1 text-[10px] font-semibold uppercase tracking-[0.22em]">{t(workspaceModeKey)}</span> : null}
          </div>

          <div className="mt-2.5 grid grid-cols-2 gap-1.5">
            <Button
              variant="ghost"
              size="sm"
              className="roc-action h-8.5 rounded-full justify-start px-3 text-[11px]"
              type="button"
              data-testid="project-new"
              onClick={() => setCreateOpen(true)}
            >
              <FolderPlus className="mr-1.5 h-3.5 w-3.5" />
              {t("sidebar.newProject")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="roc-primary-action h-8.5 rounded-full justify-start px-3 text-[11px] font-semibold"
              type="button"
              data-testid="session-new"
              onClick={onCreateSession}
              disabled={!currentWorkspacePath}
            >
              <Layers2 className="mr-1.5 h-3.5 w-3.5" />
              {t("sidebar.newSession")}
            </Button>
          </div>
        </section>

        {showProjectsSection ? (
          <section className="roc-sidebar-section p-2.5">
            <div className="mb-2 space-y-2 px-0.5">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-muted-foreground">
                    {t("sidebar.projects")}
                  </p>
                </div>
                <span className="roc-sidebar-meta">{filteredWorkspaces.length}</span>
              </div>

              {workspaces.length > 1 ? (
                <div className="relative">
                  <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    value={workspaceQuery}
                    onChange={(event) => setWorkspaceQuery(event.target.value)}
                    placeholder={t("sidebar.searchProjects")}
                    className="h-8 rounded-xl border-border/45 bg-background/72 pl-9 text-[12px]"
                  />
                </div>
              ) : null}
            </div>

            <div className="min-h-0 overflow-y-auto pr-1">
              <div className="flex flex-col gap-2">
                {filteredWorkspaces.length === 0 ? (
                  <div className="rounded-[20px] border border-dashed border-border/45 bg-muted/28 px-3.5 py-4 text-sm text-muted-foreground">
                    {workspaces.length === 0 ? t("sidebar.emptyWorkspaces") : t("sidebar.emptyMatchingWorkspaces")}
                  </div>
                ) : (
                  filteredWorkspaces.map((workspace) => (
                    <button
                      key={workspace.path}
                      type="button"
                      data-active={workspace.path === currentWorkspacePath ? "true" : "false"}
                      className="roc-sidebar-item"
                      title={workspace.path}
                      onClick={() => onSelectWorkspace(workspace.path)}
                    >
                      <div className="flex items-center gap-2.5">
                        <div
                          className="roc-icon-tile"
                          data-emphasis={workspace.path === currentWorkspacePath ? "strong" : undefined}
                        >
                          <FolderTree className="h-3.5 w-3.5 text-primary/80" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <div className="truncate text-[13px] font-medium tracking-tight text-foreground">
                              {workspace.label}
                            </div>
                            <span className="roc-sidebar-meta shrink-0">
                              {workspace.sessionCount}
                            </span>
                          </div>
                          {workspacePathHint(workspace.path, currentWorkspaceRootPath) &&
                          workspacePathHint(workspace.path, currentWorkspaceRootPath) !== workspace.label ? (
                            <div className="mt-0.25 truncate text-[10px] text-muted-foreground">
                              {workspacePathHint(workspace.path, currentWorkspaceRootPath)}
                            </div>
                          ) : null}
                        </div>
                      </div>
                    </button>
                  ))
                )}
              </div>
            </div>
          </section>
        ) : null}

        <section className="roc-sidebar-section flex min-h-0 flex-1 flex-col p-2.5" data-testid="session-list">
          <div className="mb-1.5 px-0.5">
            <div className="flex items-center justify-between gap-2">
              <div>
                <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-muted-foreground">
                  {t("sidebar.sessions")}
                </p>
              </div>
              <div className="flex items-center gap-1.5">
                {selectionMode ? (
                  <>
                    <button
                      type="button"
                      className="roc-sidebar-toggle"
                      title={t("sidebar.cancelSessionSelection")}
                      onClick={exitSelectionMode}
                      aria-label={t("sidebar.cancelSessionSelection")}
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                    <button
                      type="button"
                      className="roc-sidebar-toggle"
                      title={selectedCount > 0 ? t("sidebar.deleteSessionCountTitle", { count: selectedCount }) : t("sidebar.deleteSelectedSessionsHint")}
                      onClick={() => setDeleteConfirmOpen(true)}
                      disabled={selectedCount === 0 || deletingSessions}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="roc-sidebar-toggle"
                    title={t("sidebar.selectSessions")}
                    onClick={startSelectionMode}
                    disabled={visibleSessionCount === 0}
                  >
                    <CheckSquare2 className="h-3.5 w-3.5" />
                  </button>
                )}
                <span className="roc-sidebar-meta">
                  {selectionMode ? t("sidebar.selectedCount", { count: selectedCount }) : visibleSessionCount}
                </span>
              </div>
            </div>
          </div>

          <div className="min-h-0 overflow-y-auto pr-1">
            {sessionTree.length === 0 ? (
              <div className="rounded-[20px] border border-dashed border-border/45 bg-muted/28 px-3.5 py-4 text-sm text-muted-foreground">
                {t("sidebar.emptySessions")}
              </div>
            ) : (
              <SessionTreeList
                nodes={sessionTree}
                selectedSessionId={selectedSessionId}
                selectionMode={selectionMode}
                selectedIds={selectedSessionIds}
                collapsedIds={collapsedSessionIds}
                onToggleCollapsed={toggleCollapsed}
                onToggleSelected={toggleSelected}
                onSelectSession={onSelectSession}
              />
            )}
          </div>
        </section>
      </div>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent className="max-w-md gap-5">
          <DialogHeader>
            <DialogTitle>{t("sidebar.createProject")}</DialogTitle>
            <DialogDescription>
              {t("sidebar.createProjectDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="roc-section py-0">
            <div className="roc-form-field">
              <label htmlFor="project-path" className="roc-form-label">
                {t("sidebar.workspaceFolder")}
              </label>
              <Input
                id="project-path"
                className="h-9 rounded-lg"
                placeholder={t("sidebar.projectPathPlaceholder")}
                value={newProjectPath}
                onChange={(event) => setNewProjectPath(event.target.value)}
              />
            </div>
            <div className="roc-form-field">
              <label htmlFor="project-title" className="roc-form-label">
                {t("sidebar.rootSessionTitle")}
              </label>
              <Input
                id="project-title"
                className="h-9 rounded-lg"
                placeholder={t("sidebar.projectTitlePlaceholder")}
                value={newProjectTitle}
                onChange={(event) => setNewProjectTitle(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)}>
              {t("sidebar.cancel")}
            </Button>
            <Button onClick={submitCreateProject} disabled={!newProjectPath.trim()}>
              {t("sidebar.createProjectPrimary")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteConfirmOpen} onOpenChange={setDeleteConfirmOpen}>
        <DialogContent className="max-w-md gap-5">
          <DialogHeader>
            <DialogTitle>{t("sidebar.deleteSelectedSessions")}</DialogTitle>
            <DialogDescription>
              {selectedCount === 1
                ? t("sidebar.deleteSelectionDescription.one")
                : t("sidebar.deleteSelectionDescription.many", { count: selectedCount })}
              {" "}
              {t("sidebar.deleteSelectionFollowup")}
            </DialogDescription>
          </DialogHeader>
          <div className="roc-section py-0">
            <div className="roc-form-field gap-2">
              <div className="text-sm font-medium text-foreground">
                {t("sidebar.sessionCount", { count: selectedCount })}
              </div>
              <p className="text-sm leading-6 text-muted-foreground">
                {t("sidebar.selectionDescription")}
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDeleteConfirmOpen(false)}
              disabled={deletingSessions}
            >
              {t("sidebar.cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={confirmDeleteSelection}
              disabled={selectedCount === 0 || deletingSessions}
            >
              {deletingSessions ? t("sidebar.deleting") : t("sidebar.deleteAction", { count: selectedCount })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </aside>
  );
}
