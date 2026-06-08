"use client";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  GitBranchIcon,
  PlusIcon,
  Trash2Icon,
  RefreshCwIcon,
  LoaderCircleIcon,
  XCircleIcon,
  FolderIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { apiJson } from "../../lib/api";

interface WorktreeInfo {
  path: string;
  branch: string;
  head: string;
}

interface WorktreePanelProps {
  className?: string;
}

export function WorktreePanel({ className }: WorktreePanelProps) {
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [newBranchName, setNewBranchName] = useState("");
  const [newPath, setNewPath] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadWorktrees = useCallback(async () => {
    try {
      const data = await apiJson<WorktreeInfo[]>("/worktree");
      setWorktrees(data);
      setError(null);
    } catch (err) {
      setError(`Failed to load worktrees: ${err}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadWorktrees();
  }, [loadWorktrees]);

  const handleCreate = useCallback(async () => {
    setCreating(true);
    try {
      await apiJson<WorktreeInfo>("/worktree", {
        method: "POST",
        body: JSON.stringify({
          branch: newBranchName || null,
          path: newPath || null,
        }),
      });
      setCreateOpen(false);
      setNewBranchName("");
      setNewPath("");
      void loadWorktrees();
    } catch (err) {
      setError(`Failed to create worktree: ${err}`);
    } finally {
      setCreating(false);
    }
  }, [newBranchName, newPath, loadWorktrees]);

  const handleDelete = useCallback(
    async (path: string) => {
      if (!confirm(`Remove worktree at ${path}?`)) return;
      try {
        await apiJson("/worktree", {
          method: "DELETE",
          body: JSON.stringify({ path, force: false }),
        });
        void loadWorktrees();
      } catch (err) {
        setError(`Failed to remove worktree: ${err}`);
      }
    },
    [loadWorktrees]
  );

  return (
    <div className={cn("roc-panel roc-rail-panel h-full overflow-hidden p-5", className)} data-testid="worktree-panel">
      <div className="roc-rail-header">
        <div className="roc-rail-headline">
          <p className="roc-section-label">Workspace</p>
          <div className="flex items-center gap-2">
            <GitBranchIcon className="size-4 text-muted-foreground" />
            <h3 className="roc-rail-title">Worktrees</h3>
            <span className="roc-badge px-3 py-1.5 text-xs">{worktrees.length}</span>
          </div>
          <p className="roc-rail-description">Branch sandboxes for parallel experiments without disturbing the main repository.</p>
        </div>
        <div className="roc-rail-toolbar">
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            data-testid="worktree-refresh"
            onClick={() => {
              setRefreshing(true);
              void loadWorktrees().then(() => setRefreshing(false));
            }}
            disabled={refreshing}
          >
            <RefreshCwIcon className={cn("size-4", refreshing && "animate-spin")} />
          </Button>
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger asChild>
              <Button variant="ghost" size="icon-sm" className="size-7" data-testid="worktree-create-open">
                <PlusIcon className="size-4" />
              </Button>
            </DialogTrigger>
            <DialogContent className="max-w-md gap-5" data-testid="worktree-create-dialog">
              <DialogHeader>
                <DialogTitle>Create Worktree</DialogTitle>
                <DialogDescription>
                  Create a Git worktree for branch-specific experiments without touching the main checkout.
                </DialogDescription>
              </DialogHeader>
              <div className="roc-section py-0">
                <div className="roc-form-field">
                  <label htmlFor="branch" className="roc-form-label">
                    Branch name (optional)
                  </label>
                  <Input
                    id="branch"
                    data-testid="worktree-branch-input"
                    className="h-9 rounded-lg"
                    placeholder="feature/experiment"
                    value={newBranchName}
                    onChange={(e) => setNewBranchName(e.target.value)}
                  />
                </div>
                <div className="roc-form-field">
                  <label htmlFor="path" className="roc-form-label">
                    Path (optional)
                  </label>
                  <Input
                    id="path"
                    data-testid="worktree-path-input"
                    className="h-9 rounded-lg"
                    placeholder="repo-feature-1"
                    value={newPath}
                    onChange={(e) => setNewPath(e.target.value)}
                  />
                </div>
              </div>
              <DialogFooter>
                <Button variant="outline" onClick={() => setCreateOpen(false)}>
                  Cancel
                </Button>
                <Button data-testid="worktree-create-submit" onClick={() => void handleCreate()} disabled={creating}>
                  {creating ? (
                    <>
                      <LoaderCircleIcon className="size-4 animate-spin mr-2" />
                      Creating...
                    </>
                  ) : (
                    <>
                      <PlusIcon className="size-4 mr-2" />
                      Create
                    </>
                  )}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      {error && (
        <div className="roc-state-card" data-tone="danger" data-testid="worktree-error">
          <div className="flex items-start gap-3">
            <div className="roc-status-orb shrink-0" data-tone="danger">
              <XCircleIcon className="size-4" />
            </div>
            <div className="min-w-0">
              <div className="roc-section-label">Worktree Error</div>
              <p className="mt-1 text-sm leading-6 text-foreground/88">{error}</p>
            </div>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-auto space-y-3 pr-1">
        {loading ? (
          <div className="roc-state-card flex items-center gap-3" data-tone="loading" data-testid="worktree-loading">
            <div className="roc-status-orb shrink-0" data-tone="loading">
              <LoaderCircleIcon className="size-4 animate-spin" />
            </div>
            <div className="min-w-0">
              <div className="roc-section-label">Loading</div>
              <p className="mt-1 text-sm leading-6 text-foreground/86">Loading worktrees and branch sandboxes…</p>
            </div>
          </div>
        ) : worktrees.length === 0 ? (
          <div className="roc-rail-empty py-8" data-tone="muted" data-testid="worktree-empty">
            <div className="roc-status-orb">
              <GitBranchIcon className="size-4" />
            </div>
            <div className="space-y-1">
              <p className="text-sm font-semibold tracking-tight text-foreground">No worktrees yet</p>
              <p className="text-sm leading-6 text-muted-foreground">Create a worktree to branch safely without disturbing the main repository.</p>
            </div>
          </div>
        ) : (
          worktrees.map((wt) => (
            <div
              key={wt.path}
              className="roc-rail-section roc-surface-interactive"
              data-testid="worktree-item"
            >
              <div className="roc-rail-section-header">
                <div className="flex items-center gap-2 min-w-0">
                  <FolderIcon className="size-4 flex-shrink-0 text-muted-foreground" />
                  <div className="roc-rail-section-copy">
                    <span className="roc-rail-section-title truncate">{wt.branch}</span>
                    <p className="roc-rail-section-note font-mono truncate" title={wt.path}>
                      {wt.path}
                    </p>
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="size-7 text-muted-foreground hover:text-destructive"
                  data-testid="worktree-delete"
                  onClick={() => void handleDelete(wt.path)}
                >
                  <Trash2Icon className="size-4" />
                </Button>
              </div>
              <div className="roc-rail-meta-list">
                <span className="roc-badge px-3 py-1.5 text-xs">sandbox</span>
                <span className="font-mono px-1.5 py-0.5 rounded bg-muted">
                  {wt.head.slice(0, 7)}
                </span>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
