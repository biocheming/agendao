import { cn } from "@/lib/utils";
import {
  FolderIcon,
  FolderOpenIcon,
  FileIcon,
  FileTextIcon,
  FileCodeIcon,
  FileJsonIcon,
  FileImageIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import type { FileTreeNodeRecord } from "@/lib/workspace";

interface WorkspaceTreeNodeProps {
  node: FileTreeNodeRecord;
  depth?: number;
  isLast?: boolean;
  parentLines?: boolean[];
  selectedPath: string | null;
  linkedPath?: string | null;
  linkedLabel?: string | null;
  linkedStageId?: string | null;
  onSelectNode: (node: FileTreeNodeRecord) => void;
  onPreviewStage?: (stageId: string | null) => void;
}

function nodeLinked(node: FileTreeNodeRecord, linkedPath?: string | null) {
  if (!linkedPath) return false;
  if (node.path === linkedPath) return true;
  return node.type === "directory" && linkedPath.startsWith(`${node.path}/`);
}

function fileIcon(name: string): ReactNode {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";

  if (["json", "jsonl"].includes(ext))
    return <FileJsonIcon className="size-3.5 text-amber-600/80" />;
  if (["ts", "tsx", "js", "jsx", "py", "rs", "go", "java", "c", "cpp", "h"].includes(ext))
    return <FileCodeIcon className="size-3.5 text-violet-500/80" />;
  if (["md", "mdx", "txt", "log"].includes(ext))
    return <FileTextIcon className="size-3.5 text-emerald-600/80" />;
  if (["png", "jpg", "jpeg", "gif", "svg", "webp"].includes(ext))
    return <FileImageIcon className="size-3.5 text-rose-500/80" />;

  return <FileIcon className="size-3.5 text-muted-foreground/60" />;
}

export function WorkspaceTreeNode({
  node,
  depth = 0,
  isLast = true,
  parentLines = [],
  selectedPath,
  linkedPath = null,
  linkedLabel = null,
  linkedStageId = null,
  onSelectNode,
  onPreviewStage,
}: WorkspaceTreeNodeProps) {
  const linked = nodeLinked(node, linkedPath);
  const children = node.children ?? [];
  const hasChildren = children.length > 0;

  // Build the prefix with ASCII tree lines
  const prefix = parentLines.map((showLine) => (showLine ? "│" : " ")).join(" ");
  const connector = isLast ? "└" : "├";

  if (node.type === "directory") {
    return (
      <div className="grid">
        <button
          type="button"
          data-testid="workspace-node"
          data-path={node.path}
          data-node-type={node.type}
          className={cn(
            "group w-full min-h-[28px] border border-transparent bg-transparent text-left flex items-center gap-1.5 text-xs font-medium text-muted-foreground/80 cursor-pointer hover:bg-muted/30 transition-colors",
            selectedPath === node.path && "bg-primary/5 border-primary/10",
            linked && "bg-amber-500/5",
          )}
          style={{ paddingLeft: `${depth > 0 ? 4 : 0}px` }}
          onClick={() => onSelectNode(node)}
          onMouseEnter={() => linked && linkedStageId ? onPreviewStage?.(linkedStageId) : undefined}
          onMouseLeave={() => linked ? onPreviewStage?.(null) : undefined}
          title={node.path}
        >
          {/* Tree lines + connector */}
          {depth > 0 && (
            <span className="font-mono text-[10px] text-muted-foreground/30 select-none">
              {prefix} {connector}─
            </span>
          )}

          {/* Folder icon */}
          <FolderOpenIcon className="size-3.5 text-primary/70" />

          {/* Directory name */}
          <span className="truncate">{node.name}</span>

          {/* Linked badge */}
          {selectedPath === node.path && linkedLabel ? (
            <span className="ml-auto flex items-center gap-1 rounded-sm border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700/90">
              <span className="size-1.5 rounded-full bg-amber-500/70" />
              {linkedLabel}
            </span>
          ) : null}
        </button>

        {/* Children */}
        {hasChildren && (
          <div className="grid">
            {children.map((child, idx) => (
              <WorkspaceTreeNode
                key={child.path}
                node={child}
                depth={depth + 1}
                isLast={idx === children.length - 1}
                parentLines={[...parentLines, !isLast]}
                selectedPath={selectedPath}
                linkedPath={linkedPath}
                linkedLabel={linkedLabel}
                linkedStageId={linkedStageId}
                onSelectNode={onSelectNode}
                onPreviewStage={onPreviewStage}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  // File node
  return (
    <button
      type="button"
      data-testid="workspace-node"
      data-path={node.path}
      data-node-type={node.type}
      className={cn(
        "group w-full min-h-[28px] border border-transparent bg-transparent text-left flex items-center gap-1.5 text-xs text-muted-foreground/70 cursor-pointer hover:bg-muted/30 hover:text-foreground/90 transition-colors",
        selectedPath === node.path && "bg-primary/5 border-primary/10 text-foreground",
        linked && "bg-amber-500/5",
      )}
      style={{ paddingLeft: `${depth > 0 ? 4 : 0}px` }}
      onClick={() => onSelectNode(node)}
      onMouseEnter={() => linked && linkedStageId ? onPreviewStage?.(linkedStageId) : undefined}
      onMouseLeave={() => linked ? onPreviewStage?.(null) : undefined}
      title={node.path}
    >
      {/* Tree lines + connector */}
      {depth > 0 && (
        <span className="font-mono text-[10px] text-muted-foreground/30 select-none">
          {prefix} {connector}─
        </span>
      )}

      {/* File icon based on extension */}
      {fileIcon(node.name)}

      {/* File name */}
      <span className="truncate">{node.name}</span>

      {/* Linked badge */}
      {selectedPath === node.path && linkedLabel ? (
        <span className="ml-auto flex items-center gap-1 rounded-sm border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700/90">
          <span className="size-1.5 rounded-full bg-amber-500/70" />
          {linkedLabel}
        </span>
      ) : null}
    </button>
  );
}
