import {
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { prepareComposerAttachments } from "../lib/composerAttachments";
import {
  attachmentKind,
  attachmentLabel,
  attachmentWorkspacePath,
  clipboardImageFiles,
  droppedFiles,
} from "../lib/composerContext";
import { formatError, type PromptPart } from "../lib/display";
import { useAgendaoStore } from "../store";

export interface ComposerNotice {
  id: number;
  text: string;
  count: number;
}

function composerAttachmentNotice(parts: PromptPart[]) {
  if (parts.length === 1) {
    const part = parts[0];
    const kind = attachmentKind(part);
    const label = attachmentLabel(part);
    if (kind === "image") return `Image ready: ${label}`;
    if (kind === "directory") return `Folder ready: ${label}`;
    if (kind === "text") return `File ready: ${label}`;
    return `Attachment ready: ${label}`;
  }

  const imageCount = parts.filter((part) => attachmentKind(part) === "image").length;
  if (imageCount === parts.length) {
    return `${parts.length} images ready`;
  }
  return `${parts.length} attachments ready`;
}

interface UseComposerAttachmentsOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  reloadWorkspacePreservingSelection: () => void;
  workspaceBasePath: string;
  workspaceDirty: boolean;
}

export function useComposerAttachments({
  apiJson,
  reloadWorkspacePreservingSelection,
  workspaceBasePath,
  workspaceDirty,
}: UseComposerAttachmentsOptions) {
  const setAttachments = useAgendaoStore((s) => s.setAttachments);
  const selectedAttachmentIndex = useAgendaoStore((s) => s.selectedAttachmentIndex);
  const setSelectedAttachmentIndex = useAgendaoStore((s) => s.selectAttachment);
  const setComposerDragActive = useAgendaoStore((s) => s.setComposerDragActive);
  const setBanner = useAgendaoStore((s) => s.setBanner);

  const [composerNotice, setComposerNotice] = useState<ComposerNotice | null>(null);
  const composerNoticeIdRef = useRef(0);

  useEffect(() => {
    if (!composerNotice) return;
    const timeoutId = window.setTimeout(() => {
      setComposerNotice((current) => (current?.id === composerNotice.id ? null : current));
    }, 2400);
    return () => window.clearTimeout(timeoutId);
  }, [composerNotice]);

  const clearComposerNotice = useCallback(() => setComposerNotice(null), []);

  const attachComposerFiles = useCallback(
    async (files: File[], failurePrefix: string) => {
      if (!files.length) return;

      const nextParts = await prepareComposerAttachments(files, {
        workspaceBasePath,
        uploadJson: apiJson,
      }).catch((error) => {
        setComposerNotice(null);
        setBanner(`${failurePrefix}: ${formatError(error)}`);
        return [];
      });

      if (!nextParts.length) return;
      setAttachments((current) => {
        setSelectedAttachmentIndex(current.length + nextParts.length - 1);
        return [...current, ...nextParts];
      });
      const uploadedPaths = nextParts
        .map((part) => attachmentWorkspacePath(part))
        .filter((path): path is string => Boolean(path && path.includes("/.agendao/uploads/")));
      if (uploadedPaths.length && !workspaceDirty) {
        reloadWorkspacePreservingSelection();
      }
      composerNoticeIdRef.current += 1;
      setComposerNotice({
        id: composerNoticeIdRef.current,
        text: composerAttachmentNotice(nextParts),
        count: nextParts.length,
      });
    },
    [
      apiJson,
      reloadWorkspacePreservingSelection,
      setAttachments,
      setBanner,
      setSelectedAttachmentIndex,
      workspaceBasePath,
      workspaceDirty,
    ],
  );

  const handleFileChange = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      await attachComposerFiles(Array.from(event.target.files ?? []), "Attachment failed");
      event.target.value = "";
    },
    [attachComposerFiles],
  );

  const handleComposerPaste = useCallback(
    async (event: ClipboardEvent<HTMLTextAreaElement>) => {
      const itemFiles = clipboardImageFiles(event.clipboardData.items ?? []);
      const files =
        itemFiles.length > 0
          ? itemFiles
          : Array.from(event.clipboardData.files ?? []).filter((file) =>
              file.type.startsWith("image/"),
            );
      if (!files.length) return;
      event.preventDefault();
      await attachComposerFiles(files, "Image paste failed");
    },
    [attachComposerFiles],
  );

  const handleComposerDrop = useCallback(
    async (event: DragEvent<HTMLDivElement>) => {
      event.preventDefault();
      setComposerDragActive(false);
      await attachComposerFiles(droppedFiles(event.dataTransfer), "Drop attach failed");
    },
    [attachComposerFiles, setComposerDragActive],
  );

  const removeAttachmentAt = useCallback(
    (index: number) => {
      setAttachments((current) => current.filter((_, itemIndex) => itemIndex !== index));
      const current = selectedAttachmentIndex;
      if (current === null) {
        setSelectedAttachmentIndex(null);
        return;
      }
      if (current === index) {
        setSelectedAttachmentIndex(null);
        return;
      }
      if (current > index) {
        setSelectedAttachmentIndex(current - 1);
        return;
      }
      setSelectedAttachmentIndex(current);
    },
    [selectedAttachmentIndex, setAttachments, setSelectedAttachmentIndex],
  );

  return {
    attachComposerFiles,
    clearComposerNotice,
    composerNotice,
    handleComposerDrop,
    handleComposerPaste,
    handleFileChange,
    removeAttachmentAt,
  };
}
