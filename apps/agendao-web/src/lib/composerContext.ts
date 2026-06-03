import { apiUrl } from "@/lib/api";

const FILE_REFERENCE_REGEX = /(?:^|([^\w`]))@(\.?[^\s`,.]*(?:\.[^\s`,.]+)*)/g;
export const INLINE_IMAGE_MAX_BYTES = 512 * 1024;
export const INLINE_ATTACHMENT_MAX_BYTES = 512 * 1024;
export const TEXT_PREVIEW_MAX_LINES = 24;
export const TEXT_PREVIEW_MAX_CHARS = 1800;

const CODE_TOKEN_REGEX =
  /(\/\/.*$|#.*$|--.*$|\/\*[\s\S]*?\*\/|"(?:\\.|[^"])*"|'(?:\\.|[^'])*'|`(?:\\.|[^`])*`|\b(?:fn|let|const|function|return|if|else|for|while|match|struct|enum|impl|class|import|export|from|pub|async|await|try|catch|throw|use|mod|interface|type|true|false|null|None|Some|where|self|SELECT|FROM|WHERE|INSERT|UPDATE|DELETE|CREATE|ALTER|DROP)\b|\b\d+(?:\.\d+)?\b)/gm;

const ATTACHMENT_LANGUAGE_SPECS: Array<{
  label: string;
  exts: string[];
  mimeFragments: string[];
  codeLike: boolean;
}> = [
  { label: "Rust", exts: [".rs"], mimeFragments: ["rust"], codeLike: true },
  {
    label: "TypeScript",
    exts: [".ts", ".tsx"],
    mimeFragments: ["typescript"],
    codeLike: true,
  },
  {
    label: "JavaScript",
    exts: [".js", ".jsx", ".mjs", ".cjs"],
    mimeFragments: ["javascript", "ecmascript"],
    codeLike: true,
  },
  { label: "Python", exts: [".py"], mimeFragments: ["python"], codeLike: true },
  {
    label: "Shell",
    exts: [".sh", ".bash", ".zsh"],
    mimeFragments: ["shellscript", "x-sh"],
    codeLike: true,
  },
  { label: "Go", exts: [".go"], mimeFragments: ["x-go"], codeLike: true },
  { label: "Java", exts: [".java"], mimeFragments: ["java"], codeLike: true },
  {
    label: "C/C++",
    exts: [".c", ".cc", ".cpp", ".h", ".hpp"],
    mimeFragments: ["x-c", "x-c++"],
    codeLike: true,
  },
  { label: "SQL", exts: [".sql"], mimeFragments: ["sql"], codeLike: true },
  {
    label: "JSON",
    exts: [".json", ".jsonc"],
    mimeFragments: ["json"],
    codeLike: true,
  },
  { label: "YAML", exts: [".yaml", ".yml"], mimeFragments: ["yaml"], codeLike: true },
  { label: "TOML", exts: [".toml"], mimeFragments: ["toml"], codeLike: true },
  { label: "HTML", exts: [".html"], mimeFragments: ["html"], codeLike: true },
  { label: "CSS", exts: [".css"], mimeFragments: ["css"], codeLike: true },
  { label: "Markdown", exts: [".md"], mimeFragments: ["markdown"], codeLike: false },
  { label: "Text", exts: [".txt", ".log"], mimeFragments: ["text/plain"], codeLike: false },
];

export interface FilePromptPart {
  type: "file";
  url: string;
  filename?: string;
  mime?: string;
}

export interface ComposerAttachmentRecord {
  type: string;
  url?: string;
  filename?: string;
  mime?: string;
}

export interface WorkspaceTreeNodeRecord {
  path: string;
  type: "file" | "directory";
  children?: WorkspaceTreeNodeRecord[];
}

function attachmentLanguageSpec(part: ComposerAttachmentRecord) {
  const filename = (part.filename || "").toLowerCase();
  const mime = (part.mime || "").toLowerCase();

  return ATTACHMENT_LANGUAGE_SPECS.find(
    (spec) =>
      spec.exts.some((ext) => filename.endsWith(ext)) ||
      spec.mimeFragments.some((fragment) => mime.includes(fragment)),
  );
}

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function attachmentLabel(part: ComposerAttachmentRecord): string {
  return part.type === "file" ? part.filename || "attachment" : part.type;
}

export function attachmentTone(part: ComposerAttachmentRecord): string {
  if (part.mime === "application/x-directory") {
    return "directory";
  }
  if (part.mime?.startsWith("image/") || part.url?.startsWith("data:image/")) {
    return "image";
  }
  if (part.url?.startsWith("file://")) {
    return "workspace";
  }
  return "file";
}

export function attachmentSource(part: ComposerAttachmentRecord): string {
  if (part.url?.startsWith("data:image/")) {
    return "inline image";
  }
  if (part.url?.startsWith("data:")) {
    return "inline file";
  }
  if (part.url?.startsWith("file://") && part.url.includes("/.agendao/uploads/")) {
    return "uploaded";
  }
  if (part.url?.startsWith("file://")) {
    return "workspace";
  }
  return "remote";
}

export function attachmentKind(part: ComposerAttachmentRecord): string {
  if (part.mime === "application/x-directory") {
    return "directory";
  }
  if (part.mime?.startsWith("image/")) {
    return "image";
  }
  if (part.mime?.startsWith("text/")) {
    return "text";
  }
  return "file";
}

export function filePathFromUrl(url?: string): string | null {
  if (!url?.startsWith("file://")) {
    return null;
  }

  const path = url.slice("file://".length);
  try {
    return decodeURI(path);
  } catch {
    return path;
  }
}

export function attachmentWorkspacePath(part: ComposerAttachmentRecord): string | null {
  return filePathFromUrl(part.url);
}

export function attachmentContainsWorkspacePath(
  part: ComposerAttachmentRecord,
  selectedPath: string | null,
): boolean {
  const attachmentPath = attachmentWorkspacePath(part);
  if (!attachmentPath || !selectedPath) {
    return false;
  }

  if (part.mime === "application/x-directory") {
    return selectedPath === attachmentPath || selectedPath.startsWith(`${attachmentPath}/`);
  }

  return attachmentPath === selectedPath;
}

export function attachmentPreviewUrl(part: ComposerAttachmentRecord): string | null {
  if (part.url?.startsWith("data:image/")) {
    return part.url;
  }

  const workspacePath = attachmentWorkspacePath(part);
  if (workspacePath && part.mime?.startsWith("image/")) {
    return apiUrl(`/file/download?path=${encodeURIComponent(workspacePath)}`);
  }

  return null;
}

export function attachmentDownloadUrl(part: ComposerAttachmentRecord): string | null {
  const workspacePath = attachmentWorkspacePath(part);
  return workspacePath ? apiUrl(`/file/download?path=${encodeURIComponent(workspacePath)}`) : null;
}

export function attachmentTextPreview(part: ComposerAttachmentRecord): string | null {
  if (!part.mime?.startsWith("text/") || !part.url?.startsWith("data:")) {
    return null;
  }

  const commaIndex = part.url.indexOf(",");
  if (commaIndex < 0) {
    return null;
  }

  const metadata = part.url.slice(5, commaIndex);
  const payload = part.url.slice(commaIndex + 1);

  try {
    if (metadata.includes(";base64")) {
      return atob(payload);
    }
    return decodeURIComponent(payload);
  } catch {
    return null;
  }
}

export function attachmentLooksLikeCode(part: ComposerAttachmentRecord): boolean {
  return attachmentLanguageSpec(part)?.codeLike ?? false;
}

export function attachmentLanguageLabel(part: ComposerAttachmentRecord): string | null {
  return attachmentLanguageSpec(part)?.label ?? null;
}

export function attachmentTextPreviewState(part: ComposerAttachmentRecord): {
  preview: string | null;
  truncated: boolean;
} {
  const text = attachmentTextPreview(part);
  if (!text) {
    return { preview: null, truncated: false };
  }

  const lines = text.split("\n");
  const exceedsLines = lines.length > TEXT_PREVIEW_MAX_LINES;
  const exceedsChars = text.length > TEXT_PREVIEW_MAX_CHARS;
  if (!exceedsLines && !exceedsChars) {
    return { preview: text, truncated: false };
  }

  let preview = text.slice(0, TEXT_PREVIEW_MAX_CHARS);
  if (exceedsLines) {
    preview = lines.slice(0, TEXT_PREVIEW_MAX_LINES).join("\n");
  }
  preview = preview.trimEnd();
  if (!preview.endsWith("...")) {
    preview = `${preview}\n...`;
  }

  return { preview, truncated: true };
}

export function attachmentHighlightedHtml(text: string): string {
  let result = "";
  let lastIndex = 0;

  for (const match of text.matchAll(CODE_TOKEN_REGEX)) {
    const token = match[0];
    const index = match.index ?? 0;
    result += escapeHtml(text.slice(lastIndex, index));

    const className =
      token.startsWith("//") ||
      token.startsWith("#") ||
      token.startsWith("--") ||
      token.startsWith("/*")
        ? "comment"
        : token.startsWith('"') || token.startsWith("'") || token.startsWith("`")
          ? "string"
          : /^\d/.test(token)
            ? "number"
            : "keyword";

    result += `<span class="code-token ${className}">${escapeHtml(token)}</span>`;
    lastIndex = index + token.length;
  }

  result += escapeHtml(text.slice(lastIndex));
  return result;
}

export function extractPromptReferences(template: string): string[] {
  const result: string[] = [];
  const seen = new Set<string>();

  for (const match of template.matchAll(FILE_REFERENCE_REGEX)) {
    const value = match[2];
    if (!value || seen.has(value)) continue;
    seen.add(value);
    result.push(value);
  }

  return result;
}

export function removePromptReference(template: string, reference: string): string {
  const escaped = reference.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return template
    .replace(new RegExp(`(^|\\s)@${escaped}(?=\\s|$)`, "g"), "$1")
    .replace(/\s{2,}/g, " ")
    .trim();
}

export function isInlinePreviewImage(
  file: Pick<File, "type" | "size">,
  maxBytes = INLINE_IMAGE_MAX_BYTES,
): boolean {
  return file.type.startsWith("image/") && file.size <= maxBytes;
}

export function shouldInlineAttachment(
  file: Pick<File, "type" | "size">,
  maxBytes = INLINE_ATTACHMENT_MAX_BYTES,
): boolean {
  return file.size <= maxBytes;
}

async function readFileAsPromptPart(file: File): Promise<FilePromptPart> {
  return new Promise<FilePromptPart>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
    reader.onload = () =>
      resolve({
        type: "file",
        url: String(reader.result ?? ""),
        filename: file.name,
        mime: file.type || undefined,
      });
    reader.readAsDataURL(file);
  });
}

export async function readFilesAsPromptParts(files: File[]): Promise<FilePromptPart[]> {
  return Promise.all(files.map((file) => readFileAsPromptPart(file)));
}

export async function preparePromptPartsFromFiles(
  files: File[],
  options: {
    uploadLargeFile?: (file: File) => Promise<FilePromptPart>;
    inlineImageMaxBytes?: number;
    inlineAttachmentMaxBytes?: number;
  } = {},
): Promise<FilePromptPart[]> {
  const result: FilePromptPart[] = [];

  for (const file of files) {
    if (
      options.uploadLargeFile &&
      (!shouldInlineAttachment(file, options.inlineAttachmentMaxBytes) ||
        (file.type.startsWith("image/") && !isInlinePreviewImage(file, options.inlineImageMaxBytes)))
    ) {
      result.push(await options.uploadLargeFile(file));
      continue;
    }

    result.push(await readFileAsPromptPart(file));
  }

  return result;
}

export function clipboardImageFiles(items: Iterable<DataTransferItem>): File[] {
  return Array.from(items)
    .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
}

export async function readClipboardImagePromptParts(
  items: Iterable<DataTransferItem>,
): Promise<FilePromptPart[]> {
  return readFilesAsPromptParts(clipboardImageFiles(items));
}

export function droppedFiles(dataTransfer: DataTransfer): File[] {
  return Array.from(dataTransfer.files ?? []);
}

export function defaultAttachmentUploadPath(workspaceBasePath: string): string {
  const trimmed = workspaceBasePath.trim().replace(/\/+$/, "");
  return trimmed ? `${trimmed}/.agendao/uploads` : ".agendao/uploads";
}

export function fileUrlFromPath(path: string): string {
  return `file://${encodeURI(path)}`;
}

export function findFirstFile<T extends WorkspaceTreeNodeRecord>(node: T | null): string | null {
  if (!node) return null;
  if (node.type === "file") return node.path;
  for (const child of node.children ?? []) {
    const match = findFirstFile(child as T);
    if (match) return match;
  }
  return null;
}

export function findNodeByPath<T extends WorkspaceTreeNodeRecord>(
  node: T | null,
  path: string | null,
): T | null {
  if (!node || !path) return null;
  if (node.path === path) return node;
  for (const child of node.children ?? []) {
    const match = findNodeByPath(child as T, path);
    if (match) return match;
  }
  return null;
}

export function parentDirectory(path: string): string {
  const trimmed = path.trim().replace(/\/+$/, "");
  const index = trimmed.lastIndexOf("/");
  if (index <= 0) return "";
  return trimmed.slice(0, index);
}

export function resolveWorkspacePath(basePath: string, inputPath: string): string {
  const trimmed = inputPath.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("/")) return trimmed;
  const base = basePath.trim().replace(/\/+$/, "");
  const relative = trimmed.replace(/^\/+/, "");
  return base ? `${base}/${relative}` : relative;
}

export function toWorkspaceReferencePath(path: string, workspaceRoot: string): string {
  const normalizedPath = path.trim().replace(/\/+$/, "");
  const normalizedRoot = workspaceRoot.trim().replace(/\/+$/, "");

  if (!normalizedRoot) {
    return normalizedPath.split("/").filter(Boolean).pop() ?? normalizedPath;
  }

  if (normalizedPath === normalizedRoot) {
    return ".";
  }

  const prefix = `${normalizedRoot}/`;
  if (normalizedPath.startsWith(prefix)) {
    return normalizedPath.slice(prefix.length);
  }

  return normalizedPath;
}

export function appendReferenceToken(currentText: string, reference: string): string {
  const token = `@${reference}`;
  if (!currentText.trim()) return token;
  if (currentText.includes(token)) return currentText;
  return /\s$/.test(currentText) ? `${currentText}${token}` : `${currentText} ${token}`;
}

export function guessWorkspaceMime(path: string, fileType: "file" | "directory"): string {
  if (fileType === "directory") {
    return "application/x-directory";
  }

  const lower = path.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".gif")) return "image/gif";
  if (lower.endsWith(".webp")) return "image/webp";
  if (lower.endsWith(".svg")) return "image/svg+xml";
  if (
    [
      ".md",
      ".txt",
      ".rs",
      ".toml",
      ".json",
      ".jsonc",
      ".yaml",
      ".yml",
      ".ts",
      ".tsx",
      ".js",
      ".jsx",
      ".css",
      ".html",
      ".sh",
      ".py",
      ".go",
      ".java",
      ".c",
      ".cc",
      ".cpp",
      ".h",
      ".hpp",
      ".sql",
    ].some((ext) => lower.endsWith(ext))
  ) {
    return "text/plain";
  }

  return "application/octet-stream";
}
