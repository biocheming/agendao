"use client";

import { cn } from "@/lib/utils";
import { apiUrl } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  XIcon,
  FileIcon,
  ImageIcon,
  FileCodeIcon,
  FileTextIcon,
  DownloadIcon,
  PencilIcon,
  CheckIcon,
} from "lucide-react";
import { useCallback, useMemo, useState, useRef, useEffect } from "react";

// File type detection
function getFileType(filename: string): "image" | "code" | "text" | "binary" | "pdf" {
  const ext = filename.split(".").pop()?.toLowerCase() || "";

  const imageExts = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "avif"];
  const codeExts = [
    "ts", "tsx", "js", "jsx", "py", "rs", "go", "java", "c", "cpp", "h", "hpp",
    "rb", "php", "swift", "kt", "scala", "cs", "vue", "svelte",
    "json", "yaml", "yml", "toml", "xml", "html", "css", "scss", "sass", "less",
    "md", "mdx", "sh", "bash", "zsh", "fish", "ps1",
    "sql", "graphql", "proto", "dart", "lua", "r", "vim",
  ];
  const pdfExts = ["pdf"];

  if (imageExts.includes(ext)) return "image";
  if (pdfExts.includes(ext)) return "pdf";
  if (codeExts.includes(ext)) return "code";
  return "text";
}

function getFileIcon(filename: string, type: string) {
  switch (type) {
    case "image":
      return <ImageIcon className="size-4" />;
    case "code":
      return <FileCodeIcon className="size-4" />;
    case "pdf":
      return <FileTextIcon className="size-4" />;
    default:
      return <FileIcon className="size-4" />;
  }
}

export interface OpenFileTab {
  path: string;
  name: string;
  type: "file";
  content: string;
  mimeType?: string;
  dirty?: boolean;
}

interface FilePreviewPanelProps {
  openTabs: OpenFileTab[];
  activeTabPath: string | null;
  onTabChange: (path: string) => void;
  onTabClose: (path: string) => void;
  onContentChange?: (path: string, content: string) => void;
  onSave?: (path: string) => void;
  onDownload?: (path: string) => void;
  readOnly?: boolean;
  className?: string;
  fileLoading?: boolean;
  fileSaving?: boolean;
}

export function FilePreviewPanel({
  openTabs,
  activeTabPath,
  onTabChange,
  onTabClose,
  onContentChange,
  onSave,
  onDownload,
  readOnly = false,
  className,
  fileLoading = false,
  fileSaving = false,
}: FilePreviewPanelProps) {
  const activeTab = openTabs.find((tab) => tab.path === activeTabPath);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        if (activeTab && activeTab.dirty && onSave) {
          onSave(activeTab.path);
        }
      }
    },
    [activeTab, onSave]
  );

  if (openTabs.length === 0) {
    return (
      <div
        className={cn(
          "flex flex-col items-center justify-center h-full text-muted-foreground gap-3",
          className
        )}
      >
        <FileIcon className="size-12 opacity-30" />
        <div className="text-center">
          <h3 className="font-medium">No files open</h3>
          <p className="text-sm">Select a file from the tree to preview it</p>
        </div>
      </div>
    );
  }

  return (
    <div className={cn("flex flex-col h-full overflow-hidden", className)}>
      {/* Tab bar */}
      <div className="flex items-center gap-0 border-b border-border bg-muted/30 overflow-x-auto">
        {openTabs.map((tab) => {
          const fileType = getFileType(tab.name);
          const isActive = tab.path === activeTabPath;

          return (
            <div
              key={tab.path}
              className={cn(
                "group flex items-center gap-2 px-3 py-2 border-r border-border cursor-pointer",
                "hover:bg-muted/50 transition-colors min-w-0",
                isActive && "bg-background border-b-2 border-b-primary"
              )}
              onClick={() => onTabChange(tab.path)}
            >
              {getFileIcon(tab.name, fileType)}
              <span className="truncate text-sm max-w-[120px]">{tab.name}</span>
              {tab.dirty && (
                <span className="size-2 rounded-full bg-amber-500 flex-shrink-0" />
              )}
              <button
                className={cn(
                  "size-5 rounded flex items-center justify-center",
                  "opacity-0 group-hover:opacity-100 hover:bg-muted transition-opacity",
                  "flex-shrink-0"
                )}
                onClick={(e) => {
                  e.stopPropagation();
                  onTabClose(tab.path);
                }}
              >
                <XIcon className="size-3" />
              </button>
            </div>
          );
        })}
      </div>

      {/* Active tab content */}
      {activeTab ? (
        <FilePreviewContent
          tab={activeTab}
          onContentChange={onContentChange}
          onSave={onSave}
          onDownload={onDownload}
          readOnly={readOnly}
          fileLoading={fileLoading}
          fileSaving={fileSaving}
          onKeyDown={handleKeyDown}
          textareaRef={textareaRef}
        />
      ) : null}
    </div>
  );
}

function FilePreviewContent({
  tab,
  onContentChange,
  onSave,
  onDownload,
  readOnly,
  fileLoading,
  fileSaving,
  onKeyDown,
  textareaRef,
}: {
  tab: OpenFileTab;
  onContentChange?: (path: string, content: string) => void;
  onSave?: (path: string) => void;
  onDownload?: (path: string) => void;
  readOnly: boolean;
  fileLoading: boolean;
  fileSaving: boolean;
  onKeyDown: (e: React.KeyboardEvent) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const fileType = getFileType(tab.name);
  const [localContent, setLocalContent] = useState(tab.content);

  useEffect(() => {
    setLocalContent(tab.content);
  }, [tab.content]);

  const handleContentChange = useCallback(
    (value: string) => {
      setLocalContent(value);
      onContentChange?.(tab.path, value);
    },
    [tab.path, onContentChange]
  );

  const handleSave = useCallback(() => {
    onSave?.(tab.path);
  }, [tab.path, onSave]);

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border bg-muted/20">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="truncate max-w-[300px]" title={tab.path}>
            {tab.path}
          </span>
          {tab.dirty && (
            <span className="text-amber-600 dark:text-amber-400">Unsaved</span>
          )}
        </div>
        <div className="flex items-center gap-1">
          {fileType === "code" && !readOnly && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5"
              onClick={handleSave}
              disabled={!tab.dirty || fileSaving}
            >
              {fileSaving ? (
                <CheckIcon className="size-3.5" />
              ) : (
                <PencilIcon className="size-3.5" />
              )}
              {fileSaving ? "Saving..." : "Save"}
            </Button>
          )}
          {onDownload && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5"
              onClick={() => onDownload(tab.path)}
            >
              <DownloadIcon className="size-3.5" />
              Download
            </Button>
          )}
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-auto">
        {fileLoading ? (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            Loading...
          </div>
        ) : fileType === "image" ? (
          <ImagePreview path={tab.path} content={tab.content} mimeType={tab.mimeType} />
        ) : fileType === "pdf" ? (
          <PDFPreview path={tab.path} />
        ) : (
          <CodePreview
            content={localContent}
            filename={tab.name}
            readOnly={readOnly}
            onChange={handleContentChange}
            onKeyDown={onKeyDown}
            textareaRef={textareaRef}
          />
        )}
      </div>
    </div>
  );
}

function ImagePreview({
  path,
  content,
  mimeType,
}: {
  path: string;
  content: string;
  mimeType?: string;
}) {
  // content is base64 data URL
  const src = content.startsWith("data:") ? content : `data:${mimeType || "image/png"};base64,${content}`;

  return (
    <div className="flex items-center justify-center h-full p-4 bg-muted/10">
      <img
        src={src}
        alt={path.split("/").pop()}
        className="max-w-full max-h-full object-contain rounded-lg shadow-lg"
      />
    </div>
  );
}

function PDFPreview({ path }: { path: string }) {
  // Use iframe for PDF viewing
  return (
    <div className="h-full w-full">
      <iframe
        src={apiUrl(`/file/raw?path=${encodeURIComponent(path)}`)}
        className="w-full h-full border-0"
        title={`PDF: ${path}`}
      />
    </div>
  );
}

function CodePreview({
  content,
  filename,
  readOnly,
  onChange,
  onKeyDown,
  textareaRef,
}: {
  content: string;
  filename: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const ext = filename.split(".").pop()?.toLowerCase() || "";
  const lineCount = content.split("\n").length;

  return (
    <div className="relative h-full">
      {/* Line numbers */}
      <div
        className="absolute left-0 top-0 bottom-0 w-12 bg-muted/20 border-r border-border overflow-hidden select-none"
        aria-hidden="true"
      >
        <div className="py-3 px-2 text-right text-xs text-muted-foreground font-mono leading-[1.6]">
          {Array.from({ length: lineCount }, (_, i) => (
            <div key={i + 1}>{i + 1}</div>
          ))}
        </div>
      </div>

      {/* Code editor */}
      <textarea
        ref={textareaRef}
        value={content}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        readOnly={readOnly}
        spellCheck={false}
        className={cn(
          "w-full h-full pl-14 pr-4 py-3 resize-none",
          "bg-transparent text-foreground font-mono text-sm leading-[1.6]",
          "focus:outline-none focus:ring-0",
          "placeholder:text-muted-foreground"
        )}
        placeholder={readOnly ? "File content will appear here..." : "Start typing..."}
      />
    </div>
  );
}

// Empty state component
export function FilePreviewEmptyState({
  title = "No file selected",
  description = "Select a file from the workspace tree to preview it here.",
  icon,
}: {
  title?: string;
  description?: string;
  icon?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4 p-8">
      {icon || <FileIcon className="size-16 opacity-20" />}
      <div className="text-center">
        <h3 className="font-semibold text-lg mb-1">{title}</h3>
        <p className="text-sm">{description}</p>
      </div>
    </div>
  );
}
