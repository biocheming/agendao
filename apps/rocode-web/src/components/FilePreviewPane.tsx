"use client";

import { useState, useEffect, useRef } from "react";
import { FileIcon } from "lucide-react";
import { apiUrl } from "@/lib/api";
import { webPluginRegistry } from "@/web-plugin-registry";

type RendererKind = "iframe" | "markdown" | "code" | "image" | "pdf" | "text";

function extensionCandidates(filePath: string): string[] {
  const fileName = filePath.split("/").pop() ?? filePath;
  const segments = fileName
    .toLowerCase()
    .split(".")
    .map((segment) => segment.trim())
    .filter(Boolean);

  if (segments.length <= 1) return [];

  const suffixes: string[] = [];
  for (let start = 1; start < segments.length; start += 1) {
    suffixes.push(segments.slice(start).join("."));
  }
  return suffixes;
}

function resolvePluginExtension(filePath: string): string | null {
  for (const candidate of extensionCandidates(filePath)) {
    if (webPluginRegistry.hasRenderer(candidate)) {
      return candidate;
    }
  }
  return null;
}

function resolveBuiltinRenderer(ext: string): RendererKind {
  const map: Record<string, RendererKind> = {
    md: "markdown",
    mdx: "markdown",
    py: "code",
    rs: "code",
    ts: "code",
    tsx: "code",
    js: "code",
    jsx: "code",
    go: "code",
    java: "code",
    c: "code",
    cpp: "code",
    h: "code",
    json: "code",
    yaml: "code",
    yml: "code",
    toml: "code",
    css: "code",
    sh: "code",
    sql: "code",
    xml: "code",
    png: "image",
    jpg: "image",
    jpeg: "image",
    gif: "image",
    svg: "image",
    webp: "image",
    pdf: "pdf",
    html: "iframe",
    htm: "iframe",
  };
  return map[ext] ?? "text";
}

// --- Sub-renderers ---

function IframePreview({ filePath }: { filePath: string }) {
  return (
    <iframe
      src={apiUrl(`/file/download?path=${encodeURIComponent(filePath)}`)}
      title={filePath}
      className="w-full h-full border-0"
      sandbox=""
    />
  );
}

function ImagePreview({ filePath }: { filePath: string }) {
  return (
    <div className="flex items-center justify-center h-full p-4">
      <img
        src={apiUrl(`/file/download?path=${encodeURIComponent(filePath)}`)}
        alt={filePath.split("/").pop() ?? filePath}
        className="max-w-full max-h-full object-contain"
      />
    </div>
  );
}

function PdfPreview({ filePath }: { filePath: string }) {
  return (
    <iframe
      src={apiUrl(`/file/download?path=${encodeURIComponent(filePath)}`)}
      title={filePath}
      className="w-full h-full border-0"
    />
  );
}

function TextPreview({ filePath }: { filePath: string }) {
  const [content, setContent] = useState<string>("");
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    fetch(apiUrl(`/file/content?path=${encodeURIComponent(filePath)}`))
      .then((r) => r.json())
      .then((data) => setContent(data.content ?? ""))
      .catch(() => setContent("Failed to load file"))
      .finally(() => setLoading(false));
  }, [filePath]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground/60">
        <span className="text-[10px]">Loading...</span>
      </div>
    );
  }

  return (
    <pre className="h-full overflow-auto p-4 font-mono text-[12.5px] leading-5 text-foreground whitespace-pre">
      {content}
    </pre>
  );
}

function PluginRendererWrapper({ filePath, ext }: { filePath: string; ext: string }) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const renderer = webPluginRegistry.getRenderer(ext);
    if (renderer && containerRef.current) {
      while (containerRef.current.firstChild) {
        containerRef.current.removeChild(containerRef.current.firstChild);
      }
      const el = renderer({ filePath });
      if (el) {
        containerRef.current.appendChild(el);
      }
    }
  }, [filePath, ext]);

  return <div ref={containerRef} className="h-full" />;
}

// --- Main component ---

interface FilePreviewPaneProps {
  filePath: string | null;
}

export function FilePreviewPane({ filePath }: FilePreviewPaneProps) {
  if (!filePath) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-3 p-8">
        <FileIcon className="size-10 opacity-20" />
        <p className="text-[10.5px]">Select a file to preview</p>
      </div>
    );
  }

  const pluginExt = resolvePluginExtension(filePath);
  const ext = extensionCandidates(filePath).at(-1) ?? "";

  if (pluginExt) {
    return <PluginRendererWrapper filePath={filePath} ext={pluginExt} />;
  }

  switch (resolveBuiltinRenderer(ext)) {
    case "iframe":
      return <IframePreview filePath={filePath} />;
    case "image":
      return <ImagePreview filePath={filePath} />;
    case "pdf":
      return <PdfPreview filePath={filePath} />;
    case "markdown":
    case "code":
    case "text":
    default:
      return <TextPreview filePath={filePath} />;
  }
}
