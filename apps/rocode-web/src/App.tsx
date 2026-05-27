import {
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type FormEvent,
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ComposerSection } from "./components/ComposerSection";
import { ConversationFeedPanel } from "./components/ConversationFeedPanel";
import { DeferredTerminalPanel } from "./components/DeferredTerminalPanel";
import { InteractionOverlays } from "./components/InteractionOverlays";
import { SessionSidebar } from "./components/SessionSidebar";
import { WorkspacePanel, type WorkspacePanelTab } from "./components/WorkspacePanel";
import { loadWebPlugins } from "./web-plugin-loader";
import { api, apiJson, apiUrl } from "./lib/api";
import { cn } from "./lib/utils";
import { useConversationJump } from "./hooks/useConversationJump";
import { useExecutionActivity } from "./hooks/useExecutionActivity";
import { useMultimodalComposer } from "./hooks/useMultimodalComposer";
import { useRuntimeSurface } from "./hooks/useRuntimeSurface";
import { useSchedulerNavigation } from "./hooks/useSchedulerNavigation";
import { useSessionRegistry } from "./hooks/useSessionRegistry";
import { useServerEventStream } from "./hooks/useServerEventStream";
import { useTerminalSessions } from "./hooks/useTerminalSessions";
import { useTranscriptFeedState } from "./hooks/useTranscriptFeedState";
import { useWebBootstrap } from "./hooks/useWebBootstrap";
import { useResizableHeight, useResizableWidth } from "./hooks/useResizableWidth";
import { prepareComposerAttachments } from "./lib/composerAttachments";
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "./lib/contextPressure";
import {
  cacheBustSummaryFromMetadata,
  cacheBustSummaryLabel,
  cacheSemanticsFromTelemetry,
  type CacheEvidenceSummaryRecord,
} from "./lib/cacheDiagnostics";
import {
  contextClosureCoarseDiagnosticLabel,
  contextClosureContractFromTelemetry,
} from "./lib/contextClosureDiagnostics";
import {
  providerDiagnosticFromMetadata,
  providerDiagnosticLabel,
} from "./lib/providerDiagnostics";
import {
  buildWebSessionUrl,
  readWebSessionRoute,
  type WebExternalAdapterProvisioningRoute,
  writeWebSessionRoute,
} from "./lib/webSessionUrl";
import {
  attachmentContainsWorkspacePath,
  attachmentLabel,
  attachmentWorkspacePath,
  appendReferenceToken,
  droppedFiles,
  extractPromptReferences,
  fileUrlFromPath,
  findFirstFile,
  findNodeByPath,
  guessWorkspaceMime,
  parentDirectory,
  removePromptReference,
  resolveWorkspacePath,
  toWorkspaceReferencePath,
} from "./lib/composerContext";
import type {
  AuxiliaryOutputBlock,
  FeedMessage,
  MessageRecord,
  OutputBlock,
  RuntimeSurfaceOutputBlock,
} from "./lib/history";
import {
  applyOutputBlock,
  createOptimisticUserFeedMessage,
  estimateContextTokensFromHistory,
} from "./lib/liveTranscriptState";
import {
  type PermissionInteractionRecord,
  type PromptResponseRecord,
  type QuestionAnswerValue,
  type QuestionInfoResponseRecord,
  type QuestionInteractionRecord,
  questionInteractionFromInfo,
} from "./lib/interaction";
import type {
  ProvisionExternalAdapterSessionRequestRecord,
  ProvisionExternalAdapterSessionResponseRecord,
  PendingCommandInvocationRecord,
  SessionListResponseRecord,
  SessionRecord,
} from "./lib/session";
import {
  type ConfigProvidersResponseRecord,
  type ConnectProtocolOption,
  type KnownProviderEntry,
  type ProviderRecord,
  type ResolveProviderConnectResponseRecord,
  flattenProviderModels,
} from "./lib/provider";
import {
  basenamePath,
  buildSessionTree,
  buildWorkspaceSummaries,
  normalizeSessionRecord,
  normalizeSessionRecords,
} from "./lib/sidebar";
import type { SessionTreeNode, WorkspaceSummary } from "./lib/sidebar";
import {
  type DirectoryCreateResponseRecord,
  type FileContentResponseRecord,
  type FileTreeNodeRecord,
  type PathsResponseRecord,
  type RecentModelRecord,
  type RecentModelsPayloadRecord,
  type UploadFilesResponseRecord,
  type WorkspaceContextRecord,
  workspaceModeFromContext,
  workspaceRootFromContext,
} from "./lib/workspace";
import {
  AlertTriangleIcon,
  FolderTreeIcon,
  GitForkIcon,
  PanelLeftIcon,
  SettingsIcon,
  TerminalSquareIcon,
  XIcon,
} from "lucide-react";
import {
  DEFAULT_WEB_MODE,
  THEMES,
  type ExecutionMode,
  type ThemeId,
} from "./lib/webRuntime";

function readRuntimeBudgetNumber(
  config: Record<string, unknown> | null | undefined,
  snakeKey: string,
  fallback: number,
): number {
  const runtimeBudget = config?.runtimeBudget;
  if (!runtimeBudget || typeof runtimeBudget !== "object" || Array.isArray(runtimeBudget)) {
    return fallback;
  }
  const record = runtimeBudget as Record<string, unknown>;
  const camelKey = snakeKey.replace(/_([a-z])/g, (_, chr: string) => chr.toUpperCase());
  const value = record[snakeKey] ?? record[camelKey];
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : fallback;
}

type PromptPart =
  | {
      type: "text";
      text: string;
    }
  | {
      type: "file";
      url: string;
      filename?: string;
      mime?: string;
    }
  | {
      type: "agent";
      name: string;
    }
  | {
      type: "subtask";
      prompt: string;
      description?: string;
      agent: string;
    };

type PendingCommandInvocation = PendingCommandInvocationRecord;

function runtimeSurfacePreview(block: RuntimeSurfaceOutputBlock): string | null {
  const candidate = [
    block.display?.summary,
    block.summary,
    block.text,
    block.detail,
    block.preview,
    block.body,
  ].find((value) => typeof value === "string" && value.trim().length > 0);
  return typeof candidate === "string" ? candidate.trim() : null;
}

function runtimeSurfaceLabel(block: RuntimeSurfaceOutputBlock): string {
  const candidate = [
    block.title,
    block.event,
    block.display?.header,
    block.kind,
  ].find((value) => typeof value === "string" && value.trim().length > 0);
  return typeof candidate === "string" ? candidate.trim() : block.kind;
}

function runtimeSurfacePhase(block: RuntimeSurfaceOutputBlock): string | null {
  return typeof block.phase === "string" && block.phase.trim() ? block.phase.trim() : null;
}

function runtimeSurfaceDebugDetail(block: OutputBlock): string | undefined {
  if (!("detail" in block)) return undefined;
  return typeof block.detail === "string" ? block.detail : undefined;
}

function RuntimeSurfaceList({
  title,
  blocks,
}: {
  title: string;
  blocks: AuxiliaryOutputBlock[];
}) {
  if (blocks.length === 0) return null;
  const visibleBlocks = blocks.slice(-5).reverse();
  return (
    <div className="grid gap-2.5">
      <div className="flex items-center justify-between gap-2">
        <div className="roc-section-label">{title}</div>
        <span className="roc-badge px-2.5 py-1 text-xs">{blocks.length}</span>
      </div>
      <div className="grid gap-2">
        {visibleBlocks.map((block, index) => {
          const preview = runtimeSurfacePreview(block);
          const phase = runtimeSurfacePhase(block);
          const stableKey = `${block.kind}:${block.id ?? block.live_identity?.part_key ?? index}:${block.ts ?? index}`;
          return (
            <div key={stableKey} className="rounded-2xl border border-border/45 bg-background/66 px-3 py-2.5">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium text-foreground/92">{runtimeSurfaceLabel(block)}</span>
                <span className="roc-badge px-2 py-0.5 text-[10px] uppercase tracking-[0.18em]">
                  {block.kind}
                </span>
                {phase ? (
                  <span className="roc-badge px-2 py-0.5 text-[10px] uppercase tracking-[0.18em]">
                    {phase}
                  </span>
                ) : null}
              </div>
              {preview ? (
                <p className="mt-1.5 line-clamp-3 text-sm leading-6 text-muted-foreground">
                  {preview}
                </p>
              ) : (
                <p className="mt-1.5 text-sm leading-6 text-muted-foreground">No text payload.</p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function resolveActiveModelRef(session: SessionRecord | null, selectedModel: string) {
  const explicit = selectedModel.trim();
  if (explicit) return explicit;
  const hinted = session?.hints?.current_model?.trim();
  if (hinted) return hinted;
  const provider = session?.hints?.model_provider?.trim();
  const model = session?.hints?.model_id?.trim();
  if (provider && model) return `${provider}/${model}`;
  return model || null;
}

const RECENT_MODEL_LIMIT = 5;

function workspaceRecentModelScope(context: WorkspaceContextRecord | null): string | null {
  if (!context) return null;
  return `${context.mode}:${context.identity.workspace_key}`;
}

function splitRecentModelRef(modelRef: string): RecentModelRecord | null {
  const trimmed = modelRef.trim();
  const separator = trimmed.indexOf("/");
  if (separator <= 0 || separator >= trimmed.length - 1) return null;
  const provider = trimmed.slice(0, separator).trim();
  const model = trimmed.slice(separator + 1).trim();
  if (!provider || !model) return null;
  return { provider, model };
}

function pushRecentModel(
  recentModels: RecentModelRecord[],
  modelRef: string,
): RecentModelRecord[] {
  const next = splitRecentModelRef(modelRef);
  if (!next) return recentModels;
  return [
    next,
    ...recentModels.filter(
      (entry) =>
        !(
          entry.provider.toLowerCase() === next.provider.toLowerCase() &&
          entry.model.toLowerCase() === next.model.toLowerCase()
        ),
    ),
  ].slice(0, RECENT_MODEL_LIMIT);
}

const SettingsDrawer = lazy(async () => {
  const module = await import("./components/SettingsDrawer");
  return { default: module.SettingsDrawer };
});

function shellQuoteCommandValue(value: string): string {
  if (!value) return '""';
  if (/^[A-Za-z0-9/_.*:-]+$/.test(value)) return value;
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function splitRepeatableAnswer(answer: string): string[] {
  return answer
    .split(/[\n,\t]/)
    .flatMap((segment) => segment.split(/\s+/))
    .map((value) => value.trim())
    .filter(Boolean);
}

function pendingCommandFromSession(
  session: SessionRecord,
  questionId: string,
): PendingCommandInvocation | null {
  const pending = session.pending_command_invocation ?? session.metadata?.pending_command_invocation;
  if (!pending || typeof pending !== "object") return null;
  const invocation = pending as PendingCommandInvocation;
  if (invocation.questionId && invocation.questionId !== questionId) {
    return null;
  }
  return invocation;
}

function normalizedAnswerValues(
  answer: QuestionAnswerValue | undefined,
  multiple: boolean,
): string[] {
  if (Array.isArray(answer)) {
    return answer.map((value) => value.trim()).filter(Boolean);
  }
  const text = typeof answer === "string" ? answer.trim() : "";
  if (!text) return [];
  if (multiple || /[\n,\t]/.test(text)) {
    return splitRepeatableAnswer(text);
  }
  return [text];
}

function mergePendingCommandArguments(
  pending: PendingCommandInvocation,
  answers: string[][],
): string {
  const parts: string[] = [];
  const raw = pending.rawArguments?.trim() ?? "";
  if (raw) parts.push(raw);
  for (const [index, field] of (pending.missingFields ?? []).entries()) {
    const values = (answers[index] ?? [])
      .flatMap((value) =>
        /[\n,\t]/.test(value) ? splitRepeatableAnswer(value) : [value],
      )
      .map((value) => value.trim())
      .filter(Boolean);
    if (!values.length) continue;
    parts.push(`--${field}`);
    parts.push(...values.map((value) => shellQuoteCommandValue(value)));
  }
  return parts.join(" ").trim();
}

function promptPreviewText(content: string, parts: PromptPart[]): string {
  const trimmed = content.trim();
  if (trimmed) return trimmed;
  const attachmentCount = parts.filter((part) => part.type !== "text").length;
  if (attachmentCount === 0) return "";
  return attachmentCount === 1 ? "[1 attachment]" : `[${attachmentCount} attachments]`;
}

function ingressStabilizationLabel(value: Record<string, unknown> | null | undefined) {
  if (!value) return null;
  const sourceValue = value.source;
  const source =
    typeof sourceValue === "string"
      ? sourceValue
      : sourceValue && typeof sourceValue === "object" && "source" in sourceValue && typeof sourceValue.source === "string"
        ? sourceValue.source
        : "unknown";
  const policy = typeof value.policy === "string" ? value.policy : "metadata_only";
  const batchCount = typeof value.batch_count === "number" ? value.batch_count : 1;
  return batchCount > 1 ? `${source} · ${policy} · batch ${batchCount}` : `${source} · ${policy}`;
}

function modeKey(mode: ExecutionMode): string {
  return `${mode.kind}:${mode.id}`;
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}

function findLastMessage(messages: FeedMessage[], predicate: (message: FeedMessage) => boolean) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (predicate(messages[index])) return messages[index];
  }
  return null;
}

function metadataValue(
  metadata: Record<string, unknown> | null | undefined,
  dottedKey: string,
): unknown {
  if (!metadata) return undefined;
  if (dottedKey in metadata) return metadata[dottedKey];

  const segments = dottedKey.split(".");
  let current: unknown = metadata;
  for (const segment of segments) {
    if (!current || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[segment];
  }
  return current;
}

function previewPathFromMessageMetadata(
  history: MessageRecord[],
  workspaceBasePath: string,
): string | null {
  for (let index = history.length - 1; index >= 0; index -= 1) {
    const message = history[index];
    if (message.role === "user") continue;

    const previewTarget =
      metadataValue(message.metadata, "ui.auto_preview") ??
      metadataValue(message.metadata, "embed.file_path") ??
      metadataValue(message.metadata, "file_path");
    if (typeof previewTarget !== "string" || !previewTarget.trim()) {
      continue;
    }

    return resolveWorkspacePath(workspaceBasePath, previewTarget.trim());
  }
  return null;
}

export default function App() {
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  // P0-2 / P0-3: Transcript authority and ingress contract.
  //
  // Single visible authority:
  //   messages: FeedMessage[] — the canonical conversation feed.
  //
  // Two sanctioned ingress paths (both write to messages):
  //   1. Live flush: applyOutputBlock() via RAF-batched SSE queue
  //   2. History rebuild: mergeHistoryWithLiveBlocks() from server history
  //
  // Input buffers (feed the authority, never read by UI):
  //   pendingOutputBlocksRef — RAF-batched SSE output_block queue
  //   liveBlocksRef           — identity-keyed live cache for dedup
  //
  // Reconciliation input (merged into authority, not independent source):
  //   messageHistory: MessageRecord[] — raw server history
  //   optimisticMessagesRef           — user messages before server ack
  const [selectedMessageIds, setSelectedMessageIds] = useState<Set<string>>(() => new Set());
  const [composer, setComposer] = useState("");
  const [attachments, setAttachments] = useState<PromptPart[]>([]);
  const [providers, setProviders] = useState<ProviderRecord[]>([]);
  const [knownProviders, setKnownProviders] = useState<KnownProviderEntry[]>([]);
  const [connectProtocols, setConnectProtocols] = useState<ConnectProtocolOption[]>([]);
  const [modes, setModes] = useState<ExecutionMode[]>([]);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContextRecord | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedMode, setSelectedMode] = useState(DEFAULT_WEB_MODE);
  const [connectQuery, setConnectQuery] = useState("");
  const [connectProviderId, setConnectProviderId] = useState("");
  const [leftSidebarOpen, setLeftSidebarOpen] = useState(true);
  const [rightSidebarOpen, setRightSidebarOpen] = useState(true);
  const leftResize = useResizableWidth(312, 220, 520, "left");
  const rightResize = useResizableWidth(420, 320, 880, "right");
  const terminalResize = useResizableHeight(320, 180, 640);
  const [connectProtocol, setConnectProtocol] = useState("");
  const [connectApiKey, setConnectApiKey] = useState("");
  const [connectBaseUrl, setConnectBaseUrl] = useState("");
  const [connectResolution, setConnectResolution] =
    useState<ResolveProviderConnectResponseRecord | null>(null);
  const [connectResolveBusy, setConnectResolveBusy] = useState(false);
  const [connectResolveError, setConnectResolveError] = useState<string | null>(null);
  const [connectBusy, setConnectBusy] = useState(false);
  const [theme, setTheme] = useState<ThemeId>("daylight");
  const [showThinking, setShowThinking] = useState(true);
  const [streaming, setStreaming] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [statusLine, setStatusLine] = useState("ready");
  const [latestRuntimeError, setLatestRuntimeError] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [deletingSessions, setDeletingSessions] = useState(false);
  const [question, setQuestion] = useState<QuestionInteractionRecord | null>(null);
  const [permission, setPermission] = useState<PermissionInteractionRecord | null>(null);
  const [questionAnswers, setQuestionAnswers] = useState<Record<number, QuestionAnswerValue>>({});
  const [questionSubmitting, setQuestionSubmitting] = useState(false);
  const [permissionSubmitting, setPermissionSubmitting] = useState(false);
  const [permissionSubmitError, setPermissionSubmitError] = useState<string | null>(null);
  const [permissionSubmitStartedAt, setPermissionSubmitStartedAt] = useState<string | null>(null);
  const [permissionSubmitCompletedAt, setPermissionSubmitCompletedAt] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [composerDragActive, setComposerDragActive] = useState(false);
  const [selectedAttachmentIndex, setSelectedAttachmentIndex] = useState<number | null>(null);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const [fileTree, setFileTree] = useState<FileTreeNodeRecord | null>(null);
  const [serviceRootPath, setServiceRootPath] = useState("");
  const [currentWorkspacePath, setCurrentWorkspacePath] = useState<string | null>(null);
  const [workspaceRootPath, setWorkspaceRootPath] = useState("");
  const [workspaceLoading, setWorkspaceLoading] = useState(false);
  const [selectedWorkspacePath, setSelectedWorkspacePath] = useState<string | null>(null);
  const [selectedWorkspaceType, setSelectedWorkspaceType] = useState<"file" | "directory">(
    "directory",
  );
  const [workspacePanelTab, setWorkspacePanelTab] = useState<WorkspacePanelTab>("files");
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [selectedFileContent, setSelectedFileContent] = useState("");
  const [savedFileContent, setSavedFileContent] = useState("");
  const [fileLoading, setFileLoading] = useState(false);
  const [fileSaving, setFileSaving] = useState(false);
  const [fileDeleting, setFileDeleting] = useState(false);
  const [fileUploading, setFileUploading] = useState(false);
  const [workspaceReloadToken, setWorkspaceReloadToken] = useState(0);
  const [pendingWorkspaceSelection, setPendingWorkspaceSelection] = useState<{
    path: string;
    type: "file" | "directory";
  } | null>(null);
  const feedRef = useRef<HTMLDivElement | null>(null);
  const preferencesReadyRef = useRef(false);
  const routeSyncSourceRef = useRef<"app" | "browser">("app");
  const routeInitializedRef = useRef(false);
  const selectedSessionRef = useRef<string | null>(null);
  const autoPreviewSignatureRef = useRef<string>("");
  const maxPendingOutputBlocks = useMemo(
    () =>
      readRuntimeBudgetNumber(workspaceContext?.config, "web_max_pending_output_blocks", 256),
    [workspaceContext?.config],
  );

  const {
    clearPendingOutputBlockFlush,
    clearTranscriptFeed,
    flushPendingOutputBlocks,
    liveBlocksRef,
    messageHistory,
    messages,
    optimisticMessagesRef,
    pendingOutputBlocksRef,
    queueVisibleLiveSnapshot,
    rebuildFeedFromHistory,
    setMessages,
  } = useTranscriptFeedState({
    maxPendingOutputBlocks,
    selectedSessionRef,
    sessionIds: sessions.map((session) => session.id),
    showThinking,
  });
  const {
    appendRuntimeSurfaceBlock,
    currentRuntimeSurface,
    hasCurrentRuntimeSurface,
    setRuntimeSurfaceBanner,
  } = useRuntimeSurface({
    selectedSessionId,
    sessionIds: sessions.map((session) => session.id),
  });
  // P2-3: viewport budget for rendered messages. When exceeded, only the most
  // recent messages are rendered. Full transcript is preserved in state.
  // Derived from rocode_config::RuntimeBudgetConfig.tui_max_viewport_messages.
  const MAX_RENDERED_MESSAGES = 200;
  const renderedMessages = useMemo(
    () => messages.length > MAX_RENDERED_MESSAGES
      ? messages.slice(messages.length - MAX_RENDERED_MESSAGES)
      : messages,
    [messages, MAX_RENDERED_MESSAGES],
  );
  const connectResolveRequestRef = useRef(0);
  const recentModelScopeRef = useRef<string | null>(null);
  const recentModelAutoSuppressedRef = useRef(false);

  const recentModels = useMemo(
    () => workspaceContext?.recent_models ?? [],
    [workspaceContext?.recent_models],
  );
  const modelOptions = useMemo(() => {
    const options = flattenProviderModels(providers);
    if (recentModels.length === 0) return options;
    const recentKeys = recentModels.map((entry) => `${entry.provider}/${entry.model}`);
    const recentSet = new Set(recentKeys);
    return [
      ...recentKeys
        .map((key) => options.find((option) => option.key === key))
        .filter((option): option is (typeof options)[number] => Boolean(option)),
      ...options.filter((option) => !recentSet.has(option.key)),
    ];
  }, [providers, recentModels]);
  const settingsModeOptions = useMemo(
    () =>
      modes.map((mode) => ({
        key: modeKey(mode),
        label: mode.kind === "agent" ? mode.name : `${mode.kind}:${mode.name}`,
      })),
    [modes],
  );
  const composerReferences = useMemo(() => extractPromptReferences(composer), [composer]);
  const currentSession = useMemo(() => sessions.find((session) => session.id === selectedSessionId) ?? null, [selectedSessionId, sessions]);
  const activeModelRef = useMemo(
    () => resolveActiveModelRef(currentSession, selectedModel),
    [currentSession, selectedModel],
  );
  const activeProviderModel = useMemo(() => {
    if (!activeModelRef) return null;
    const target = activeModelRef.trim();
    for (const provider of providers) {
      for (const model of provider.models ?? []) {
        const fullId = `${provider.id}/${model.id}`;
        if (
          fullId === target ||
          model.id === target ||
          fullId.endsWith(`/${target}`)
        ) {
          return {
            ...model,
            fullId,
            providerId: provider.id,
            providerName: provider.name,
          };
        }
      }
    }
    return null;
  }, [activeModelRef, providers]);
  const persistRecentModel = useCallback(
    async (modelRef: string) => {
      const nextRecentModels = pushRecentModel(recentModels, modelRef);
      if (nextRecentModels === recentModels) return;
      setWorkspaceContext((current) =>
        current ? { ...current, recent_models: nextRecentModels } : current,
      );
      try {
        const response = await apiJson<RecentModelsPayloadRecord>("/workspace/recent-models", {
          method: "PUT",
          body: JSON.stringify({ recent_models: nextRecentModels }),
        });
        setWorkspaceContext((current) =>
          current ? { ...current, recent_models: response.recent_models ?? [] } : current,
        );
      } catch (error) {
        setBanner(`Failed to save recent model: ${formatError(error)}`);
      }
    },
    [recentModels],
  );
  const handleModelChange = useCallback(
    (value: string) => {
      recentModelAutoSuppressedRef.current = value.trim().length === 0;
      setSelectedModel(value);
      if (value.trim()) {
        void persistRecentModel(value);
      }
    },
    [persistRecentModel],
  );
  useEffect(() => {
    const scope = workspaceRecentModelScope(workspaceContext);
    if (!scope) return;
    if (recentModelScopeRef.current !== scope) {
      recentModelScopeRef.current = scope;
      recentModelAutoSuppressedRef.current = false;
    }
    if (selectedModel.trim() || recentModelAutoSuppressedRef.current) return;

    const available = new Set(flattenProviderModels(providers).map((option) => option.key));
    const nextModel = recentModels
      .map((entry) => `${entry.provider}/${entry.model}`)
      .find((modelRef) => available.has(modelRef));
    if (nextModel) {
      setSelectedModel(nextModel);
    }
  }, [providers, recentModels, selectedModel, workspaceContext]);
  const workspaceSummaries = useMemo(
    () => buildWorkspaceSummaries(sessions, serviceRootPath),
    [serviceRootPath, sessions],
  );
  const currentWorkspaceSummary = useMemo(
    () =>
      workspaceSummaries.find((workspace) => workspace.path === currentWorkspacePath) ??
      workspaceSummaries[0] ??
      null,
    [currentWorkspacePath, workspaceSummaries],
  );
  const pluginWorkspacePath =
    currentWorkspaceSummary?.path ||
    currentWorkspacePath ||
    workspaceRootFromContext(workspaceContext) ||
    serviceRootPath ||
    null;
  const resolvedWorkspaceRootPath = workspaceRootFromContext(workspaceContext) || serviceRootPath;
  const resolvedWorkspaceMode = workspaceModeFromContext(workspaceContext);
  const sessionTree = useMemo(
    () => buildSessionTree(sessions, currentWorkspaceSummary?.path ?? null),
    [currentWorkspaceSummary?.path, sessions],
  );
  const selectedAttachment = (selectedAttachmentIndex !== null && attachments[selectedAttachmentIndex]) || attachments[attachments.length - 1] || null;
  const workspaceDirty = Boolean(selectedFilePath) && selectedFileContent !== savedFileContent;
  const workspaceBasePath =
    currentSession?.directory?.trim() ||
    currentWorkspaceSummary?.path ||
    workspaceRootFromContext(workspaceContext) ||
    workspaceRootPath ||
    serviceRootPath ||
    "";
  const workspaceTargetDirectory =
    selectedWorkspaceType === "directory" && selectedWorkspacePath
      ? selectedWorkspacePath
      : selectedFilePath
        ? parentDirectory(selectedFilePath) || workspaceBasePath
        : workspaceBasePath;
  const selectedWorkspaceReference = selectedWorkspacePath ? toWorkspaceReferencePath(selectedWorkspacePath, workspaceBasePath || workspaceRootPath) : null;
  const selectedWorkspaceFilename = selectedWorkspacePath ? selectedWorkspacePath.split("/").filter(Boolean).pop() || selectedWorkspacePath : null;
  const selectedWorkspaceIsRoot = Boolean(selectedWorkspacePath) && selectedWorkspaceType === "directory" && selectedWorkspacePath === (workspaceRootPath || workspaceBasePath);
  const multimodalComposer = useMultimodalComposer({
    apiJson,
    selectedModel,
    attachments,
    scopeKey: `${workspaceContext?.mode ?? "none"}:${workspaceContext?.identity?.workspace_root ?? ""}`,
  });
  const executionActivity = useExecutionActivity({
    selectedSessionId,
    apiJson,
    onError: setBanner,
    onInfo: setBanner,
    statusLine,
    latestRuntimeError,
    awaitingUser: Boolean(question),
    pendingPermission: Boolean(permission),
  });
  const routeHighlightIds = useMemo(() => {
    const route = readWebSessionRoute();
    return route.sessionId === selectedSessionId ? new Set(route.highlightIds) : new Set<string>();
  }, [selectedSessionId, messages.length]);
  const sessionUsage = executionActivity.sessionUsage ?? currentSession?.telemetry?.usage ?? null;
  const composerContextTokens = useMemo(() => {
    const activeEstimate =
      executionActivity.activeStageSummary && isLiveStageStatus(executionActivity.activeStageSummary.status)
        ? executionActivity.activeStageSummary.estimated_context_tokens
        : undefined;
    return currentContextTokensFromSources(sessionUsage?.context_tokens, activeEstimate)
      ?? estimateContextTokensFromHistory(messageHistory);
  }, [executionActivity.activeStageSummary, messageHistory, sessionUsage?.context_tokens]);
  const effectiveRightPanelWidth = useMemo(() => {
    if (workspacePanelTab === "preview") return Math.max(rightResize.width, 640);
    if (workspacePanelTab === "insights") return Math.max(rightResize.width, 460);
    return rightResize.width;
  }, [rightResize.width, workspacePanelTab]);
  const lastAssistantTurnTokens = useMemo(() => {
    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const tokens = message.tokens;
      if (!tokens) continue;
      return {
        input: tokens.input ?? 0,
        output: tokens.output ?? 0,
        cacheRead: tokens.cache_read ?? 0,
        cacheMiss: tokens.cache_miss ?? 0,
        cacheWrite: tokens.cache_write ?? 0,
      };
    }
    return null;
  }, [messageHistory]);
  const latestClosureDiagnostic = useMemo(() => {
    const contextClosure = contextClosureContractFromTelemetry(executionActivity.telemetry);
    if (contextClosure) {
      return contextClosureCoarseDiagnosticLabel(contextClosure);
    }

    const semanticsLabel = cacheSemanticsFromTelemetry(executionActivity.telemetry)?.label;
    if (semanticsLabel) return semanticsLabel;

    const telemetrySummary =
      executionActivity.telemetry?.cache_evidence &&
      typeof executionActivity.telemetry.cache_evidence === "object"
        ? (executionActivity.telemetry.cache_evidence as CacheEvidenceSummaryRecord)
        : null;
    const telemetryLabel = cacheBustSummaryLabel(telemetrySummary);
    if (telemetryLabel) return telemetryLabel;

    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const label = cacheBustSummaryLabel(cacheBustSummaryFromMetadata(message.metadata));
      if (label) return label;
    }
    return null;
  }, [
    executionActivity.telemetry?.cache_evidence,
    executionActivity.telemetry?.context_closure_contract,
    executionActivity.telemetry?.cache_semantics,
    messageHistory,
  ]);
  const latestIngressDiagnostic = useMemo(
    () => ingressStabilizationLabel(executionActivity.telemetry?.ingress_stabilization ?? null),
    [executionActivity.telemetry?.ingress_stabilization],
  );
  const latestProviderDiagnostic = useMemo(() => {
    const telemetrySummary = providerDiagnosticFromMetadata({
      provider_diagnostic: executionActivity.telemetry?.provider_diagnostic_summary ?? null,
    });
    const telemetryLabel = providerDiagnosticLabel(telemetrySummary);
    if (telemetryLabel) return telemetryLabel;

    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const label = providerDiagnosticLabel(providerDiagnosticFromMetadata(message.metadata));
      if (label) return label;
    }
    return null;
  }, [executionActivity.telemetry?.provider_diagnostic_summary, messageHistory]);
  const refreshExecutionActivity = executionActivity.refreshExecutionActivity;
  const applySchedulerStageOutputBlock = executionActivity.applySchedulerStageOutputBlock;
  const applyLiveExecutionOutputBlock = executionActivity.applyLiveExecutionOutputBlock;
  const conversationJump = useConversationJump({
    messages,
    feedRef,
    onMiss: setBanner,
  });
  useEffect(() => {
    const route = readWebSessionRoute();
    const messageId = route.messageId || route.highlightIds[0] || null;
    if (!messageId || route.sessionId !== selectedSessionId) return;
    conversationJump.jumpOrQueueConversationTarget({ messageId, label: messageId });
  }, [conversationJump.jumpOrQueueConversationTarget, messages.length, selectedSessionId]);
  const schedulerNavigation = useSchedulerNavigation({
    sessions,
    selectedSessionId,
    currentSession,
    setSessions,
    setSelectedSessionId,
    apiJson,
    setBanner,
    executionActivity,
    jumpToConversationTarget: conversationJump.jumpOrQueueConversationTarget,
    queueConversationJumpTarget: conversationJump.queueConversationJumpTarget,
  });
  const workspaceLinkLabel = schedulerNavigation.activeStageId ? `stage ${schedulerNavigation.activeStageId}` : schedulerNavigation.currentBreadcrumbProvenance?.toolCallId ? `tool ${schedulerNavigation.currentBreadcrumbProvenance.toolCallId}` : schedulerNavigation.currentBreadcrumbProvenance?.stageId ? `stage ${schedulerNavigation.currentBreadcrumbProvenance.stageId}` : null;
  const workspaceLinkStageId = schedulerNavigation.activeStageId ?? schedulerNavigation.currentBreadcrumbProvenance?.stageId ?? null;
  const terminalSessions = useTerminalSessions({
    api,
    apiJson,
    setBanner,
    enabled: terminalOpen,
    defaultCwd: workspaceBasePath || currentSession?.directory || "",
    sessionId: currentSession?.id ?? selectedSessionId ?? null,
  });

  const loadPendingQuestion = async (requestId: string, sessionId?: string | null) => {
    const questions = await apiJson<QuestionInfoResponseRecord[]>("/question");
    const pending = (questions ?? []).find((candidate) => candidate.id === requestId);
    if (!pending) return;
    const interaction = questionInteractionFromInfo(pending);
    if (sessionId && interaction.session_id && interaction.session_id !== sessionId) {
      return;
    }
    setQuestion(interaction);
    setQuestionAnswers({});
  };

  const sendPromptRequest = async (
    sessionId: string,
    payload: Record<string, unknown>,
  ): Promise<PromptResponseRecord> =>
    apiJson<PromptResponseRecord>(`/session/${sessionId}/prompt`, {
      method: "POST",
      body: JSON.stringify(payload),
    });

  const fetchSessions = useCallback(async (): Promise<SessionRecord[]> => {
    const sessionData = await apiJson<SessionListResponseRecord>("/session?limit=500");
    return normalizeSessionRecords(sessionData?.items ?? []);
  }, [apiJson]);

  const provisionExternalAdapterSession = useCallback(
    async (
      route: WebExternalAdapterProvisioningRoute,
      options: { replace?: boolean } = {},
    ): Promise<string> => {
      const request: ProvisionExternalAdapterSessionRequestRecord = {
        adapter_id: route.adapterId,
        actor_id: route.actorId,
        workspace_id: route.workspaceId,
        route_policy_id: route.routePolicyId,
        scheduler_profile: route.schedulerProfile,
        directory: route.directory,
        project_id: route.projectId,
        title: route.title,
      };
      const provisioned = await apiJson<ProvisionExternalAdapterSessionResponseRecord>(
        "/external-adapter/session/provision",
        {
          method: "POST",
          body: JSON.stringify(request),
        },
      );
      const normalized = normalizeSessionRecord(provisioned.session);
      setSessions((current) =>
        normalizeSessionRecords([normalized, ...current.filter((item) => item.id !== normalized.id)]),
      );
      setCurrentWorkspacePath(
        (current) => normalized.directory?.trim() || request.directory?.trim() || current,
      );
      writeWebSessionRoute(
        {
          sessionId: normalized.id,
          messageId: null,
          highlightIds: [],
          externalProvisioning: null,
        },
        { replace: options.replace ?? true },
      );
      return normalized.id;
    },
    [apiJson],
  );

  const copyMessageLink = async (message: FeedMessage) => {
    if (!selectedSessionId || !message.anchorId) return;
    const relative = buildWebSessionUrl({
      sessionId: selectedSessionId,
      messageId: message.anchorId,
      highlightIds: [],
    });
    const url = new URL(relative, window.location.origin).toString();
    await navigator.clipboard.writeText(url);
    setBanner("Copied message link");
  };

  const toggleMessageSelected = (message: FeedMessage) => {
    if (!message.anchorId) return;
    setSelectedMessageIds((current) => {
      const next = new Set(current);
      if (next.has(message.anchorId!)) next.delete(message.anchorId!);
      else next.add(message.anchorId!);
      return next;
    });
  };

  const copySelectedMessageLink = async () => {
    if (!selectedSessionId || selectedMessageIds.size === 0) return;
    const highlightIds = Array.from(selectedMessageIds);
    const relative = buildWebSessionUrl({
      sessionId: selectedSessionId,
      messageId: highlightIds[0] ?? null,
      highlightIds,
    });
    await navigator.clipboard.writeText(new URL(relative, window.location.origin).toString());
    setBanner(`Copied link for ${highlightIds.length} selected message${highlightIds.length === 1 ? "" : "s"}`);
  };

  const copySelectedMessagesMarkdown = async () => {
    const selected = messages.filter((message) => message.anchorId && selectedMessageIds.has(message.anchorId));
    if (selected.length === 0) return;
    const markdown = selected
      .map((message) => {
        const role = message.role === "user" ? "User" : message.role === "assistant" ? "Assistant" : message.kind;
        const title = message.title?.trim() ? ` - ${message.title.trim()}` : "";
        const text = message.text?.trim() || message.summary?.trim() || "[no text]";
        return `### ${role}${title}\n\n${text}`;
      })
      .join("\n\n---\n\n");
    await navigator.clipboard.writeText(markdown);
    setBanner(`Copied ${selected.length} selected message${selected.length === 1 ? "" : "s"} as Markdown`);
  };

  const { clearPendingSessionRefresh, scheduleSessionRefresh } = useSessionRegistry({
    fetchSessions,
    formatError,
    setBanner,
    setSelectedSessionId,
    setSessions,
  });

  const { reloadCoreSettingsData, reloadProvidersAndModes } = useWebBootstrap({
    apiJson,
    fetchSessions,
    formatError,
    preferencesReadyRef,
    provisionExternalAdapterSession,
    setBanner,
    setConnectProtocol,
    setConnectProtocols,
    setKnownProviders,
    setModes,
    setProviders,
    setSelectedMode,
    setSelectedModel,
    setSelectedSessionId,
    setServiceRootPath,
    setSessions,
    setShowThinking,
    setTheme,
    setWorkspaceContext,
  });

  useEffect(() => {
    if (!selectedWorkspacePath) return;
    const nextIndex = attachments.findIndex((attachment) =>
      attachmentContainsWorkspacePath(attachment, selectedWorkspacePath),
    );
    if (nextIndex >= 0 && nextIndex !== selectedAttachmentIndex) {
      setSelectedAttachmentIndex(nextIndex);
    }
  }, [attachments, selectedAttachmentIndex, selectedWorkspacePath]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  useEffect(() => {
    selectedSessionRef.current = selectedSessionId;
  }, [selectedSessionId]);

  const selectSession = useCallback((sessionId: string | null) => {
    routeSyncSourceRef.current = "app";
    setSelectedSessionId(sessionId);
  }, []);

  useEffect(() => {
    if (!selectedSessionId) return;
    if (routeSyncSourceRef.current === "browser") {
      routeSyncSourceRef.current = "app";
      routeInitializedRef.current = true;
      return;
    }
    const route = readWebSessionRoute();
    if (!routeInitializedRef.current && route.sessionId === selectedSessionId) {
      routeInitializedRef.current = true;
      return;
    }
    if (
      route.sessionId === selectedSessionId &&
      (route.messageId || route.highlightIds.length > 0)
    ) {
      routeInitializedRef.current = true;
      return;
    }
    routeInitializedRef.current = true;
    writeWebSessionRoute({ sessionId: selectedSessionId, messageId: null, highlightIds: [] });
  }, [selectedSessionId]);

  useEffect(() => {
    let active = true;

    const handlePopState = () => {
      const route = readWebSessionRoute();
      if (!route.sessionId && route.externalProvisioning) {
        void (async () => {
          try {
            const sessionId = await provisionExternalAdapterSession(
              route.externalProvisioning!,
              { replace: true },
            );
            if (!active) return;
            routeSyncSourceRef.current = "browser";
            setSelectedSessionId(sessionId);
          } catch (error) {
            if (active) {
              setBanner(`Failed to provision external adapter session: ${formatError(error)}`);
            }
          }
        })();
        return;
      }
      routeSyncSourceRef.current = "browser";
      setSelectedSessionId(route.sessionId);
    };
    window.addEventListener("popstate", handlePopState);
    return () => {
      active = false;
      window.removeEventListener("popstate", handlePopState);
    };
  }, [provisionExternalAdapterSession]);

  useEffect(() => {
    autoPreviewSignatureRef.current = "";
    clearTranscriptFeed();
    setSelectedMessageIds(new Set());
  }, [clearTranscriptFeed, selectedSessionId]);

  useEffect(() => {
    const query = connectQuery.trim();
    if (!query) {
      connectResolveRequestRef.current += 1;
      setConnectResolveBusy(false);
      setConnectResolveError(null);
      setConnectResolution(null);
      return;
    }

    const requestId = connectResolveRequestRef.current + 1;
    connectResolveRequestRef.current = requestId;
    const timer = window.setTimeout(() => {
      setConnectResolveBusy(true);
      setConnectResolveError(null);
      void (async () => {
        try {
          const response = await apiJson<ResolveProviderConnectResponseRecord>(
            "/provider/connect/resolve",
            {
              method: "POST",
              body: JSON.stringify({ query }),
            },
          );
          if (connectResolveRequestRef.current !== requestId) return;
          setConnectResolution(response);
          setConnectProviderId(response.draft.provider_id);
          setConnectBaseUrl(response.draft.base_url ?? "");
          setConnectProtocol(
            response.draft.protocol ?? connectProtocols[0]?.id ?? "openai",
          );
        } catch (error) {
          if (connectResolveRequestRef.current !== requestId) return;
          setConnectResolution(null);
          setConnectResolveError(formatError(error));
        } finally {
          if (connectResolveRequestRef.current === requestId) {
            setConnectResolveBusy(false);
          }
        }
      })();
    }, 120);

    return () => window.clearTimeout(timer);
  }, [apiJson, connectProtocols, connectQuery, knownProviders]);

  useEffect(() => {
    const selectedWorkspace = currentSession?.directory?.trim();
    if (selectedWorkspace) {
      setCurrentWorkspacePath(selectedWorkspace);
      return;
    }
    setCurrentWorkspacePath((current) => {
      if (current && workspaceSummaries.some((workspace) => workspace.path === current)) {
        return current;
      }
      return workspaceSummaries[0]?.path ?? serviceRootPath ?? null;
    });
  }, [currentSession?.directory, serviceRootPath, workspaceSummaries]);

  useEffect(() => {
    if (!feedRef.current) return;
    feedRef.current.scrollTop = feedRef.current.scrollHeight;
  }, [messages]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const selectedLiveBlocks = selectedSessionId ? (liveBlocksRef.current[selectedSessionId] ?? []) : [];
    const pendingVisible = selectedSessionId ? (pendingOutputBlocksRef.current[selectedSessionId] ?? []) : [];
    (
      window as Window & {
        __rocodeWebDebug?: {
          selectedSessionId: string | null;
          showThinking: boolean;
          messages: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string }>;
          liveBlocks: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string; detail?: string; part_key?: string; part_kind?: string }>;
          pendingVisible: Array<{ kind: string; id?: string; tool_call_id?: string; text?: string; detail?: string; part_key?: string; part_kind?: string }>;
        };
      }
    ).__rocodeWebDebug = {
      selectedSessionId,
      showThinking,
      messages: messages.map((message) => ({
        kind: message.kind,
        id: message.id,
        tool_call_id: message.tool_call_id,
        text: message.text?.slice(0, 160),
      })),
      liveBlocks: selectedLiveBlocks.map((block) => ({
        kind: block.kind,
        id: block.id,
        tool_call_id: block.tool_call_id,
        text: block.text?.slice(0, 160),
        detail: runtimeSurfaceDebugDetail(block)?.slice(0, 160),
        part_key: block.live_identity?.part_key,
        part_kind: block.live_identity?.part_kind,
      })),
      pendingVisible: pendingVisible.map((block) => ({
        kind: block.kind,
        id: block.id,
        tool_call_id: block.tool_call_id,
        text: block.text?.slice(0, 160),
        detail: runtimeSurfaceDebugDetail(block)?.slice(0, 160),
        part_key: block.live_identity?.part_key,
        part_kind: block.live_identity?.part_kind,
      })),
    };
  }, [messages, selectedSessionId, showThinking]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        await loadWebPlugins(apiJson, { workspacePath: pluginWorkspacePath });
      } catch (error) {
        if (!cancelled) {
          console.warn("[web-plugin] Reload failed", error);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [pluginWorkspacePath]);

  useEffect(() => {
    if (!preferencesReadyRef.current) return;
    const timer = window.setTimeout(() => {
      void api("/config", {
        method: "PATCH",
        body: JSON.stringify({
          uiPreferences: {
            webTheme: theme,
            webMode: selectedMode || null,
            webModel: selectedModel || null,
            showThinking,
          },
        }),
      }).catch((error) => {
        setBanner(`Failed to persist settings: ${formatError(error)}`);
      });
    }, 150);

    return () => window.clearTimeout(timer);
  }, [theme, selectedMode, selectedModel, showThinking]);

  useEffect(() => {
    if (!selectedSessionId) {
      clearTranscriptFeed();
      setBanner(null);
      autoPreviewSignatureRef.current = "";
      return;
    }

    let cancelled = false;

    const loadHistory = async () => {
      setHistoryLoading(true);
      try {
        const history = await apiJson<MessageRecord[]>(`/session/${selectedSessionId}/message`);
        if (cancelled) return;
        rebuildFeedFromHistory({
          history,
          sessionId: selectedSessionId,
          streaming,
        });
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to load messages: ${formatError(error)}`);
        }
      } finally {
        if (!cancelled) {
          setHistoryLoading(false);
        }
      }
    };

    void loadHistory();
    return () => {
      cancelled = true;
    };
  }, [apiJson, clearTranscriptFeed, rebuildFeedFromHistory, selectedSessionId, streaming]);

  useServerEventStream({
    applyLiveExecutionOutputBlock,
    applySchedulerStageOutputBlock,
    appendRuntimeSurfaceBlock,
    clearPendingOutputBlockFlush,
    clearPendingSessionRefresh,
    flushPendingOutputBlocks,
    onConfigUpdated: reloadProvidersAndModes,
    queueVisibleLiveSnapshot,
    refreshExecutionActivity,
    scheduleSessionRefresh,
    selectedSessionRef,
    setLatestRuntimeError,
    setMessages,
    setPermission,
    setPermissionSubmitCompletedAt,
    setPermissionSubmitError,
    setPermissionSubmitStartedAt,
    setPermissionSubmitting,
    setQuestion,
    setQuestionAnswers,
    setQuestionSubmitting,
    setRuntimeSurfaceBanner,
    setStatusLine,
    setStreaming,
    showThinking,
  });

  useEffect(() => {
    let cancelled = false;

    const loadTree = async () => {
      setWorkspaceLoading(true);
      setFileTree(null);
      setSelectedWorkspacePath(null);
      setSelectedWorkspaceType("directory");
      setSelectedFilePath(null);
      setSelectedFileContent("");
      setSavedFileContent("");

      try {
        const query =
          currentSession?.directory && currentSession.directory.trim()
            ? `?path=${encodeURIComponent(currentSession.directory)}`
            : "";
        const tree = await apiJson<FileTreeNodeRecord>(`/file/tree${query}`);
        if (cancelled) return;
        setFileTree(tree);
        setWorkspaceRootPath(tree.path);
        const preferredNode = pendingWorkspaceSelection
          ? findNodeByPath(tree, pendingWorkspaceSelection.path)
          : null;
        const fallbackFilePath = findFirstFile(tree);
        const fallbackNode = fallbackFilePath ? findNodeByPath(tree, fallbackFilePath) : tree;
        const nextNode = preferredNode ?? fallbackNode;

        setSelectedWorkspacePath(nextNode?.path ?? null);
        setSelectedWorkspaceType(nextNode?.type ?? "directory");
        setSelectedFilePath(nextNode?.type === "file" ? nextNode.path : null);
        setPendingWorkspaceSelection(null);
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to load workspace tree: ${formatError(error)}`);
          setWorkspaceRootPath(currentSession?.directory || "");
        }
      } finally {
        if (!cancelled) {
          setWorkspaceLoading(false);
        }
      }
    };

    void loadTree();
    return () => {
      cancelled = true;
    };
  }, [currentSession?.directory, selectedSessionId, workspaceReloadToken]);

  useEffect(() => {
    if (!selectedFilePath) {
      setSelectedFileContent("");
      setSavedFileContent("");
      return;
    }

    let cancelled = false;

    const loadFile = async () => {
      setFileLoading(true);
      try {
        const response = await apiJson<FileContentResponseRecord>(
          `/file/content?path=${encodeURIComponent(selectedFilePath)}`,
        );
        if (cancelled) return;
        setSelectedFileContent(response.content ?? "");
        setSavedFileContent(response.content ?? "");
      } catch (error) {
        if (!cancelled) {
          setBanner(`Failed to read file: ${formatError(error)}`);
        }
      } finally {
        if (!cancelled) {
          setFileLoading(false);
        }
      }
    };

    void loadFile();
    return () => {
      cancelled = true;
    };
  }, [selectedFilePath]);

  const createSession = async (options?: {
    directory?: string;
    title?: string;
    projectId?: string;
  }) => {
    const requestedDirectory =
      options?.directory?.trim() ||
      currentWorkspaceSummary?.path ||
      currentWorkspacePath ||
      workspaceRootFromContext(workspaceContext) ||
      serviceRootPath ||
      undefined;
    const created = await apiJson<SessionRecord>("/session", {
      method: "POST",
      body: JSON.stringify({
        directory: requestedDirectory,
        title: options?.title,
        project_id: options?.projectId,
      }),
    });
    const normalized = normalizeSessionRecord(created);
    setSessions((current) =>
      normalizeSessionRecords([normalized, ...current.filter((item) => item.id !== normalized.id)]),
    );
    setCurrentWorkspacePath(normalized.directory?.trim() || requestedDirectory || null);
    selectedSessionRef.current = normalized.id;
    setSelectedSessionId(normalized.id);
    return normalized.id;
  };

  const forkSelectedSession = async () => {
    if (!selectedSessionId) return;
    try {
      const forked = normalizeSessionRecord(
        await apiJson<SessionRecord>(`/session/${selectedSessionId}/fork`, {
          method: "POST",
          body: JSON.stringify({ message_id: null }),
        }),
      );
      setSessions((current) =>
        normalizeSessionRecords([forked, ...current.filter((item) => item.id !== forked.id)]),
      );
      setCurrentWorkspacePath(forked.directory?.trim() || currentWorkspacePath || null);
      selectedSessionRef.current = forked.id;
      setSelectedSessionId(forked.id);
      setBanner(`Forked session ${forked.title}`);
    } catch (error) {
      setBanner(`Failed to fork session: ${formatError(error)}`);
    }
  };

  const selectWorkspace = (workspacePath: string) => {
    setCurrentWorkspacePath(workspacePath);
    const workspaceSessions = sessions
      .filter((session) => session.directory?.trim() === workspacePath)
      .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
    const preferred =
      workspaceSessions.find((session) => !session.parent_id) ?? workspaceSessions[0] ?? null;
    if (preferred) {
      setSelectedSessionId(preferred.id);
    }
  };

  const createProject = async (input: { path: string; title?: string }) => {
    const baseRoot = serviceRootPath || workspaceBasePath || workspaceRootPath;
    const targetPath = resolveWorkspacePath(baseRoot, input.path);
    if (!targetPath) {
      setBanner("Project path is required");
      return;
    }

    try {
      const directory = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
        method: "POST",
        body: JSON.stringify({ path: targetPath }),
      });
      const folderName = basenamePath(directory.path);
      await createSession({
        directory: directory.path,
        projectId: folderName,
        title: input.title || `${folderName} workspace`,
      });
      setPendingWorkspaceSelection({ path: directory.path, type: "directory" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created project ${folderName}`);
    } catch (error) {
      setBanner(`Failed to create project: ${formatError(error)}`);
    }
  };

  const deleteSelectedSessions = async (sessionIds: string[]) => {
    const uniqueIds = Array.from(new Set(sessionIds.map((id) => id.trim()).filter(Boolean)));
    if (uniqueIds.length === 0 || deletingSessions) return;

    const sessionById = new Map(sessions.map((session) => [session.id, session]));
    const selectedSet = new Set(uniqueIds);
    const deleteRoots = uniqueIds.filter((sessionId) => {
      let cursor = sessionById.get(sessionId)?.parent_id ?? null;
      while (cursor) {
        if (selectedSet.has(cursor)) return false;
        cursor = sessionById.get(cursor)?.parent_id ?? null;
      }
      return true;
    });

    if (deleteRoots.length === 0) return;

    setDeletingSessions(true);
    setBanner(null);

    try {
      for (const sessionId of deleteRoots) {
        await api(`/session/${sessionId}`, { method: "DELETE" });
      }

      const sessionData = await fetchSessions();
      setSessions(sessionData);

      const currentStillExists =
        selectedSessionId && sessionData.some((session) => session.id === selectedSessionId);
      if (!currentStillExists) {
        const workspacePath = currentWorkspaceSummary?.path ?? currentWorkspacePath;
        const workspaceSessions = sessionData
          .filter((session) => session.directory?.trim() === workspacePath)
          .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
        const fallback =
          workspaceSessions.find((session) => !session.parent_id) ?? workspaceSessions[0] ?? null;
        setSelectedSessionId(fallback?.id ?? null);
      }

      setBanner(`Deleted ${deleteRoots.length} session${deleteRoots.length === 1 ? "" : "s"}.`);
    } catch (error) {
      setBanner(`Failed to delete sessions: ${formatError(error)}`);
    } finally {
      setDeletingSessions(false);
    }
  };

  const submitPrompt = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const content = composer.trim();
    const promptParts = attachments;
    if ((!content && promptParts.length === 0) || streaming) return;

    setBanner(null);

    try {
      const multimodalGate = await multimodalComposer.preflightBeforeSubmit();
      if (multimodalGate.blocked) {
        setBanner(multimodalGate.banner);
        return;
      }
      if (multimodalGate.banner) {
        setBanner(multimodalGate.banner);
      }
    } catch (error) {
      setBanner(`Multimodal preflight unavailable: ${formatError(error)}`);
    }

    let sessionId = selectedSessionRef.current;
    if (!sessionId) {
      try {
        sessionId = await createSession();
      } catch (error) {
        setBanner(`Failed to create session: ${formatError(error)}`);
        return;
      }
    }
    selectedSessionRef.current = sessionId;

    const preview = promptPreviewText(content, promptParts);
    const optimisticMessage = createOptimisticUserFeedMessage(preview);
    const ingressIdempotencyKey =
      optimisticMessage.feedId || `web-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    optimisticMessagesRef.current = {
      ...optimisticMessagesRef.current,
      [sessionId]: [
        ...(optimisticMessagesRef.current[sessionId] ?? []),
        optimisticMessage,
      ],
    };
    setMessages((current) => [...current, optimisticMessage]);
    setComposer("");
    setAttachments([]);
    setStreaming(true);
    setStatusLine("running");
    setLatestRuntimeError(null);

    try {
      const payload: Record<string, unknown> = {
        message: content || undefined,
        idempotency_key: ingressIdempotencyKey,
        ingress_source: "web",
      };
      if (selectedModel) payload.model = selectedModel;
      if (promptParts.length > 0) payload.parts = promptParts;
      if (selectedMode) {
        const [kind, id] = selectedMode.split(":", 2);
        if (kind === "agent") payload.agent = id;
        if (kind === "preset" || kind === "profile") payload.scheduler_profile = id;
      }

      const response = await sendPromptRequest(sessionId, payload);
      if (response.status === "awaiting_user") {
        setStreaming(false);
        setStatusLine("awaiting_user");
        if (response.pending_question_id) {
          await loadPendingQuestion(response.pending_question_id, sessionId);
        }
      }
    } catch (error) {
      setMessages((current) =>
        applyOutputBlock(
          current,
          {
            kind: "status",
            tone: "error",
            text: formatError(error),
          },
          showThinking,
        ),
      );
      setBanner(`Prompt failed: ${formatError(error)}`);
      setStreaming(false);
      setStatusLine("error");
      setLatestRuntimeError(formatError(error));
    }

    try {
      const sessionData = await fetchSessions();
      setSessions(sessionData);
    } catch {
      // best effort
    }
  };

  const attachComposerFiles = async (files: File[], failurePrefix: string) => {
    if (!files.length) return;

    const nextParts = await prepareComposerAttachments(files, {
      workspaceBasePath,
      uploadJson: apiJson,
    }).catch((error) => {
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
      .filter((path): path is string => Boolean(path && path.includes("/.rocode/uploads/")));
    if (uploadedPaths.length && !workspaceDirty) {
      setPendingWorkspaceSelection(
        selectedWorkspacePath
          ? { path: selectedWorkspacePath, type: selectedWorkspaceType }
          : workspaceRootPath
            ? { path: workspaceRootPath, type: "directory" }
            : null,
      );
      setWorkspaceReloadToken((current) => current + 1);
    }
    setBanner(
      nextParts.length === 1
        ? `Attached ${attachmentLabel(nextParts[0])}`
        : `Attached ${nextParts.length} items`,
    );
  };

  const handleFileChange = async (event: ChangeEvent<HTMLInputElement>) => {
    await attachComposerFiles(Array.from(event.target.files ?? []), "Attachment failed");
    event.target.value = "";
  };

  const handleComposerPaste = async (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData.files ?? []).filter((file) =>
      file.type.startsWith("image/"),
    );
    if (!files.length) return;
    event.preventDefault();
    await attachComposerFiles(files, "Image paste failed");
  };

  const handleComposerDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setComposerDragActive(false);
    await attachComposerFiles(droppedFiles(event.dataTransfer), "Drop attach failed");
  };

  const submitQuestion = async () => {
    if (!question) return;
    setQuestionSubmitting(true);
    try {
      const answers = question.questions.map((item, index) =>
        normalizedAnswerValues(questionAnswers[index], Boolean(item.multiple)),
      );
      await api(`/question/${question.request_id}/reply`, {
        method: "POST",
        body: JSON.stringify({ answers }),
      });
      setQuestion(null);
      setQuestionAnswers({});
      const sessionId = question.session_id ?? selectedSessionRef.current;
      if (sessionId) {
        const session = await apiJson<SessionRecord>(`/session/${sessionId}`);
        const pending = pendingCommandFromSession(session, question.request_id);
        if (pending) {
          const argumentsText = mergePendingCommandArguments(pending, answers);
          const response = await sendPromptRequest(sessionId, {
            command: pending.command,
            arguments: argumentsText || undefined,
            model: selectedModel || undefined,
            ingress_source: "web",
            idempotency_key: `web-command-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          });
          if (response.status === "awaiting_user") {
            setStreaming(false);
            setStatusLine("awaiting_user");
            if (response.pending_question_id) {
              await loadPendingQuestion(response.pending_question_id, sessionId);
            }
          } else {
            setStreaming(true);
            setStatusLine("running");
            setLatestRuntimeError(null);
          }
        }
      }
    } catch (error) {
      setBanner(`Question reply failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  };

  const rejectQuestion = async () => {
    if (!question) return;
    setQuestionSubmitting(true);
    try {
      await api(`/question/${question.request_id}/reject`, { method: "POST" });
      setQuestion(null);
      setQuestionAnswers({});
    } catch (error) {
      setBanner(`Question reject failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  };

  const replyPermission = async (reply: "once" | "turn" | "session" | "reject") => {
    const currentPermission = permission;
    if (!currentPermission || permissionSubmitting) return;
    setPermissionSubmitting(true);
    setPermissionSubmitError(null);
    setPermissionSubmitStartedAt(new Date().toISOString());
    try {
      await api(`/permission/${currentPermission.permission_id}/reply`, {
        method: "POST",
        body: JSON.stringify({ reply }),
      });
      setPermissionSubmitCompletedAt(new Date().toISOString());
    } catch (error) {
      const message = formatError(error);
      setBanner(`Permission reply failed: ${message}`);
      setPermissionSubmitError(message);
      setPermissionSubmitting(false);
      setPermissionSubmitCompletedAt(new Date().toISOString());
    }
  };

  const permissionStatusLabel = permissionSubmitError
    ? `Permission reply failed · ${permissionSubmitError}`
    : permissionSubmitting
      ? "Submitting permission reply…"
      : permission
        ? "Pending permission request"
        : permissionSubmitCompletedAt
          ? `Permission reply sent · ${permissionSubmitCompletedAt}`
          : null;
  const permissionStatusTone: "muted" | "warning" | "destructive" = permissionSubmitError
    ? "destructive"
    : permissionSubmitting || permission
      ? "warning"
      : "muted";

  const connectProvider = async () => {
    const providerId = connectProviderId.trim();
    const apiKey = connectApiKey.trim();
    if (!providerId || !apiKey) {
      setBanner("provider_id and api_key are required");
      return;
    }

    const baseUrl = connectBaseUrl.trim();
    const defaultProtocol = connectProtocols[0]?.id || "openai";
    const protocol = connectProtocol.trim() || defaultProtocol;
    const suggestedDraft = connectResolution?.draft ?? null;
    const suggestedBaseUrl = suggestedDraft?.base_url?.trim() ?? "";
    const suggestedProtocol = suggestedDraft?.protocol?.trim() || defaultProtocol;

    setConnectBusy(true);
    try {
      const useKnownQuickConnect =
        suggestedDraft?.mode === "known" &&
        suggestedDraft.provider_id.toLowerCase() === providerId.toLowerCase() &&
        ((baseUrl === suggestedBaseUrl && protocol === suggestedProtocol) || !baseUrl);
      if (!useKnownQuickConnect && !baseUrl) {
        setBanner("Custom or advanced provider connect requires a base URL.");
        return;
      }

      await api("/provider/connect", {
        method: "POST",
        body: JSON.stringify({
          provider_id: providerId,
          api_key: apiKey,
          base_url: useKnownQuickConnect ? undefined : baseUrl,
          protocol: useKnownQuickConnect ? undefined : protocol,
        }),
      });
      setConnectApiKey("");
      setConnectBaseUrl("");
      await reloadCoreSettingsData();
      setBanner(`Connected provider ${providerId}`);
    } catch (error) {
      setBanner(`Provider connect failed: ${formatError(error)}`);
    } finally {
      setConnectBusy(false);
    }
  };

  const lastAssistant = findLastMessage(
    messages,
    (message) => message.kind === "message" && message.role === "assistant",
  );

  const confirmDiscardWorkspaceChanges = (targetLabel: string) => {
    if (!workspaceDirty) {
      return true;
    }

    return window.confirm(
      `Unsaved changes in ${selectedFilePath || "the current file"} will be lost. Continue to ${targetLabel}?`,
    );
  };

  const selectWorkspaceNode = (path: string, typeHint?: "file" | "directory") => {
    const requestedType = typeHint ?? "file";
    if (
      selectedFilePath &&
      workspaceDirty &&
      (path !== selectedWorkspacePath || requestedType !== selectedWorkspaceType) &&
      !confirmDiscardWorkspaceChanges("switch workspace selection")
    ) {
      return false;
    }

    const node = findNodeByPath(fileTree, path);
    if (node) {
      setSelectedWorkspacePath(node.path);
      setSelectedWorkspaceType(node.type);
      setSelectedFilePath(node.type === "file" ? node.path : null);
      setWorkspacePanelTab(node.type === "file" ? "preview" : "files");
      return true;
    }

    setPendingWorkspaceSelection({ path, type: requestedType });
    setWorkspacePanelTab(requestedType === "file" ? "preview" : "files");
    setWorkspaceReloadToken((current) => current + 1);
    return true;
  };

  useEffect(() => {
    if (!selectedSessionId || !workspaceBasePath) return;
    const previewPath = previewPathFromMessageMetadata(messageHistory, workspaceBasePath);
    if (!previewPath) return;

    const signature = `${selectedSessionId}:${previewPath}`;
    if (autoPreviewSignatureRef.current === signature) {
      return;
    }

    if (selectWorkspaceNode(previewPath, "file")) {
      autoPreviewSignatureRef.current = signature;
      setWorkspacePanelTab("preview");
    }
  }, [
    messageHistory,
    selectedSessionId,
    workspaceBasePath,
  ]);

  const locateAttachmentInWorkspace = (attachment: PromptPart) => {
    const path = attachmentWorkspacePath(attachment);
    if (!path) return;
    selectWorkspaceNode(path, attachment.type === "file" && attachment.mime === "application/x-directory" ? "directory" : "file");
    schedulerNavigation.restoreActiveStage();
    setBanner(`Located ${attachmentLabel(attachment)} in workspace`);
  };

  const removeAttachmentAt = (index: number) => {
    setAttachments((current) => current.filter((_, itemIndex) => itemIndex !== index));
    setSelectedAttachmentIndex((current) => {
      if (current === null) return null;
      if (current === index) return null;
      if (current > index) return current - 1;
      return current;
    });
  };

  const saveSelectedFile = async () => {
    if (!selectedFilePath || fileSaving) return;
    setFileSaving(true);
    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: selectedFilePath,
          content: selectedFileContent,
        }),
      });
      setSavedFileContent(selectedFileContent);
      setBanner(`Saved ${selectedFilePath}`);
    } catch (error) {
      setBanner(`Failed to save file: ${formatError(error)}`);
    } finally {
      setFileSaving(false);
    }
  };

  const createWorkspaceDirectory = async () => {
    const requestedPath = window.prompt("New folder path", "notes");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a folder and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("Directory path is required");
      return;
    }

    try {
      const response = await apiJson<DirectoryCreateResponseRecord>("/file/directory", {
        method: "POST",
        body: JSON.stringify({
          path: targetPath,
        }),
      });
      setPendingWorkspaceSelection({ path: response.path, type: "directory" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created directory ${response.path}`);
    } catch (error) {
      setBanner(`Failed to create directory: ${formatError(error)}`);
    }
  };

  const createWorkspaceFile = async () => {
    const requestedPath = window.prompt("New file path", "notes.md");
    if (!requestedPath) return;

    if (!confirmDiscardWorkspaceChanges("create a file and refresh workspace")) {
      return;
    }

    const targetPath = resolveWorkspacePath(workspaceTargetDirectory || workspaceBasePath, requestedPath);
    if (!targetPath) {
      setBanner("File path is required");
      return;
    }

    try {
      await api("/file/content", {
        method: "PUT",
        body: JSON.stringify({
          path: targetPath,
          content: "",
          create_parents: true,
        }),
      });
      setPendingWorkspaceSelection({ path: targetPath, type: "file" });
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Created ${targetPath}`);
    } catch (error) {
      setBanner(`Failed to create file: ${formatError(error)}`);
    }
  };

  const deleteSelectedWorkspaceNode = async () => {
    if (!selectedWorkspacePath || fileDeleting) return;
    if (selectedWorkspaceIsRoot) {
      setBanner("Refusing to delete the workspace root directory");
      return;
    }
    if (!confirmDiscardWorkspaceChanges("delete the selected workspace node")) {
      return;
    }
    if (!window.confirm(`Delete ${selectedWorkspacePath}?`)) return;

    setFileDeleting(true);
    try {
      await api("/file", {
        method: "DELETE",
        body: JSON.stringify({
          path: selectedWorkspacePath,
          recursive: selectedWorkspaceType === "directory",
        }),
      });
      const nextPath =
        selectedWorkspaceType === "file"
          ? parentDirectory(selectedWorkspacePath) || workspaceBasePath
          : parentDirectory(selectedWorkspacePath) || workspaceBasePath;
      setPendingWorkspaceSelection(nextPath ? { path: nextPath, type: "directory" } : null);
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(`Deleted ${selectedWorkspacePath}`);
    } catch (error) {
      setBanner(`Failed to delete selection: ${formatError(error)}`);
    } finally {
      setFileDeleting(false);
    }
  };

  const downloadSelectedFile = () => {
    if (!selectedFilePath) return;
    window.location.assign(apiUrl(`/file/download?path=${encodeURIComponent(selectedFilePath)}`));
  };

  const insertWorkspaceReference = () => {
    if (!selectedWorkspaceReference) return;
    setComposer((current) => appendReferenceToken(current, selectedWorkspaceReference));
    setBanner(`Inserted @${selectedWorkspaceReference}`);
  };

  const attachSelectedWorkspaceNode = () => {
    if (!selectedWorkspacePath) return;

    const nextAttachment: PromptPart = {
      type: "file",
      url: fileUrlFromPath(selectedWorkspacePath),
      filename: selectedWorkspaceReference || selectedWorkspaceFilename || "attachment",
      mime: guessWorkspaceMime(selectedWorkspacePath, selectedWorkspaceType),
    };

    setAttachments((current) => {
      if (current.some((part) => part.type === "file" && part.url === nextAttachment.url)) {
        return current;
      }
      setSelectedAttachmentIndex(current.length);
      return [...current, nextAttachment];
    });
    setBanner(
      selectedWorkspaceType === "directory"
        ? `Attached directory ${selectedWorkspaceReference || selectedWorkspacePath}`
        : `Attached file ${selectedWorkspaceReference || selectedWorkspacePath}`,
    );
  };

  const uploadWorkspaceFiles = async (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    if (!files.length || fileUploading) return;

    if (!confirmDiscardWorkspaceChanges("upload files and refresh workspace")) {
      event.target.value = "";
      return;
    }

    setFileUploading(true);
    try {
      const payloadFiles = await Promise.all(
        files.map(
          (file) =>
            new Promise<{ name: string; content: string; mime?: string }>((resolve, reject) => {
              const reader = new FileReader();
              reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
              reader.onload = () =>
                resolve({
                  name: file.name,
                  content: String(reader.result ?? ""),
                  mime: file.type || undefined,
                });
              reader.readAsDataURL(file);
            }),
        ),
      );

      const response = await apiJson<UploadFilesResponseRecord>("/file/upload", {
        method: "POST",
        body: JSON.stringify({
          path: workspaceTargetDirectory || workspaceBasePath || undefined,
          files: payloadFiles,
        }),
      });

      if (response.files[0]?.path) {
        setPendingWorkspaceSelection({ path: response.files[0].path, type: "file" });
      }
      setWorkspaceReloadToken((current) => current + 1);
      setBanner(
        response.files.length === 1
          ? `Uploaded ${response.files[0]?.name ?? "1 file"}`
          : `Uploaded ${response.files.length} files`,
      );
    } catch (error) {
      setBanner(`Failed to upload files: ${formatError(error)}`);
    } finally {
      event.target.value = "";
      setFileUploading(false);
    }
  };

  return (
    <div className="roc-app-shell flex h-dvh flex-col overflow-hidden bg-background text-foreground font-sans">
      <div className="flex flex-1 overflow-hidden">
        {leftSidebarOpen && (
          <>
            <div className="shrink-0 overflow-hidden border-r border-border/50 bg-sidebar" style={{ width: leftResize.width }}>
              <SessionSidebar
                workspaces={workspaceSummaries}
                currentWorkspacePath={currentWorkspaceSummary?.path ?? null}
                currentWorkspaceLabel={currentWorkspaceSummary?.label ?? null}
                currentWorkspaceRootPath={resolvedWorkspaceRootPath || currentWorkspaceSummary?.path || null}
                currentWorkspaceMode={resolvedWorkspaceMode}
                sessionTree={sessionTree}
                selectedSessionId={selectedSessionId}
                deletingSessions={deletingSessions}
                onCreateProject={(input) => {
                  void createProject(input);
                }}
                onCreateSession={() => {
                  void createSession({
                    directory: (currentWorkspaceSummary?.path ?? serviceRootPath) || undefined,
                  });
                }}
                onDeleteSessions={(sessionIds) => {
                  void deleteSelectedSessions(sessionIds);
                }}
                onSelectWorkspace={selectWorkspace}
                onSelectSession={selectSession}
                onHideSidebar={() => setLeftSidebarOpen(false)}
              />
            </div>
            <div className={leftResize.handleClassName} onMouseDown={leftResize.handleMouseDown} />
          </>
        )}

        <div className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
          {!leftSidebarOpen ? (
            <div className="absolute left-4 top-3 z-20 md:left-5">
              <button
                onClick={() => setLeftSidebarOpen(true)}
                className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
                title="Show sidebar"
              >
                <PanelLeftIcon className="size-4" />
              </button>
            </div>
          ) : null}
          <div className="absolute right-4 top-3 z-20 flex items-center gap-1.5 md:right-5">
            {selectedSessionId ? (
              <button
                onClick={() => {
                  void forkSelectedSession();
                }}
                className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
                title="Fork session"
                aria-label="Fork session"
              >
                <GitForkIcon className="size-4" />
              </button>
            ) : null}
            {!rightSidebarOpen && selectedWorkspaceFilename ? (
              <button
                onClick={() => setRightSidebarOpen(true)}
                className="hidden items-center gap-1.5 rounded-full border border-border/55 bg-background/78 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground md:flex"
                title="Show workspace"
              >
                <span className="truncate max-w-[10rem]">{selectedWorkspaceFilename}</span>
              </button>
            ) : null}
            <button
              onClick={() => setRightSidebarOpen((value) => !value)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={rightSidebarOpen ? "Hide workspace" : "Show workspace"}
            >
              <FolderTreeIcon className={cn("size-4", rightSidebarOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setTerminalOpen((value) => !value)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={terminalOpen ? "Hide terminal" : "Show terminal"}
            >
              <TerminalSquareIcon className={cn("size-4", terminalOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setSettingsOpen(true)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title="Settings"
            >
              <SettingsIcon className="size-4" />
            </button>
          </div>
          {banner ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div className="roc-banner flex items-start gap-3" data-tone="warning">
                <div className="roc-status-orb mt-0.5 shrink-0" data-tone="loading">
                  <AlertTriangleIcon className="size-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="roc-section-label">Attention</div>
                  <p className="mt-1 text-sm leading-6 text-current/92">{banner}</p>
                </div>
                <button
                  type="button"
                  className="roc-banner-dismiss shrink-0"
                  aria-label="Dismiss status message"
                  onClick={() => setBanner(null)}
                >
                  <XIcon className="size-4" />
                </button>
              </div>
            </div>
          ) : null}

          {selectedSessionId && hasCurrentRuntimeSurface ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div className="roc-panel grid gap-4 px-4 py-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <div className="roc-section-label">Runtime Surface</div>
                    <p className="mt-1 text-sm leading-6 text-muted-foreground">
                      Session-scoped runtime events that do not belong in the conversation transcript.
                    </p>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {currentRuntimeSurface.queueItems.length > 0 ? (
                      <span className="roc-badge px-2.5 py-1 text-xs">
                        queue {currentRuntimeSurface.queueItems.length}
                      </span>
                    ) : null}
                    {currentRuntimeSurface.sessionEvents.length > 0 ? (
                      <span className="roc-badge px-2.5 py-1 text-xs">
                        session {currentRuntimeSurface.sessionEvents.length}
                      </span>
                    ) : null}
                    {currentRuntimeSurface.inspectItems.length > 0 ? (
                      <span className="roc-badge px-2.5 py-1 text-xs">
                        inspect {currentRuntimeSurface.inspectItems.length}
                      </span>
                    ) : null}
                  </div>
                </div>
                {currentRuntimeSurface.banner ? (
                  <div className="rounded-2xl border border-amber-500/25 bg-amber-500/8 px-3.5 py-3 text-sm leading-6 text-amber-900 dark:text-amber-100">
                    {currentRuntimeSurface.banner}
                  </div>
                ) : null}
                <div className="grid gap-4 lg:grid-cols-3">
                  <RuntimeSurfaceList title="Queue" blocks={currentRuntimeSurface.queueItems} />
                  <RuntimeSurfaceList title="Session Events" blocks={currentRuntimeSurface.sessionEvents} />
                  <RuntimeSurfaceList title="Inspect" blocks={currentRuntimeSurface.inspectItems} />
                </div>
              </div>
            </div>
          ) : null}

          {selectedMessageIds.size > 0 ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div className="roc-panel flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <span className="text-sm text-muted-foreground">
                  {selectedMessageIds.size} message{selectedMessageIds.size === 1 ? "" : "s"} selected
                </span>
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => void copySelectedMessageLink()}
                  >
                    Copy selected link
                  </button>
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => void copySelectedMessagesMarkdown()}
                  >
                    Copy Markdown
                  </button>
                  <button
                    type="button"
                    className="roc-action roc-action-pill"
                    onClick={() => setSelectedMessageIds(new Set())}
                  >
                    Clear
                  </button>
                </div>
              </div>
            </div>
          ) : null}

          <ConversationFeedPanel
            sessionId={selectedSessionId}
            feedRef={feedRef}
            historyLoading={historyLoading}
            messages={messages}
            highlightedFeedId={conversationJump.highlightedFeedId}
            highlightedMessageIds={routeHighlightIds}
            activeStageId={schedulerNavigation.previewStageId ?? schedulerNavigation.activeStageId}
            activeToolCallId={schedulerNavigation.activeToolCallId}
            selectedMessageIds={selectedMessageIds}
            streaming={streaming}
            onCopyMessageLink={copyMessageLink}
            onToggleMessageSelected={toggleMessageSelected}
            onNavigateStage={schedulerNavigation.navigateToStage}
            onNavigateAttachedSession={schedulerNavigation.navigateToAttachedSession}
          />

          <div className="shrink-0 px-4 pb-5 pt-2 md:px-5">
            <ComposerSection
              composer={composer}
              composerDragActive={composerDragActive}
              streaming={streaming}
              multimodalHints={multimodalComposer.hints}
              allowAudioInput={multimodalComposer.policy?.allow_audio_input ?? true}
              allowImageInput={multimodalComposer.policy?.allow_image_input ?? true}
              allowFileInput={multimodalComposer.policy?.allow_file_input ?? true}
              modeOptions={settingsModeOptions}
              selectedMode={selectedMode}
              onModeChange={setSelectedMode}
              providers={providers}
              recentModels={recentModels}
              selectedModel={selectedModel}
              onModelChange={handleModelChange}
              references={composerReferences}
              attachments={attachments}
              selectedAttachmentIndex={selectedAttachmentIndex}
              selectedAttachment={selectedAttachment}
              selectedWorkspacePath={selectedWorkspacePath}
              workspaceRootPath={workspaceBasePath || workspaceRootPath}
              contextTokensUsed={composerContextTokens}
              contextTokensLimit={activeProviderModel?.context_window ?? null}
              lastTurnInputTokens={lastAssistantTurnTokens?.input ?? null}
              lastTurnOutputTokens={lastAssistantTurnTokens?.output ?? null}
              cacheReadTokens={sessionUsage?.cache_read_tokens ?? lastAssistantTurnTokens?.cacheRead ?? null}
              cacheMissTokens={sessionUsage?.cache_miss_tokens ?? lastAssistantTurnTokens?.cacheMiss ?? null}
              cacheWriteTokens={sessionUsage?.cache_write_tokens ?? lastAssistantTurnTokens?.cacheWrite ?? null}
              closureDiagnosticLabel={latestClosureDiagnostic}
              ingressDiagnosticLabel={latestIngressDiagnostic}
              providerDiagnosticLabel={latestProviderDiagnostic}
              inputPricePerMillion={activeProviderModel?.cost_per_million_input ?? null}
              outputPricePerMillion={activeProviderModel?.cost_per_million_output ?? null}
              activeStageId={schedulerNavigation.activeStageId}
              provenance={schedulerNavigation.currentBreadcrumbProvenance}
              permissionStatusLabel={permissionStatusLabel}
              permissionStatusTone={permissionStatusTone}
              onPreviewStage={schedulerNavigation.previewStage}
              onSubmit={submitPrompt}
              onRemoveReference={(reference) => setComposer((current) => removePromptReference(current, reference))}
              onRemoveAttachment={removeAttachmentAt}
              onSelectAttachment={(index, attachment) => {
                setSelectedAttachmentIndex(index);
                locateAttachmentInWorkspace(attachment as PromptPart);
              }}
              onLocateAttachment={(attachment) => locateAttachmentInWorkspace(attachment as PromptPart)}
              onNavigateStage={schedulerNavigation.navigateToStage}
              onNavigateProvenanceSession={schedulerNavigation.navigateToProvenanceSession}
              onNavigateProvenanceStage={schedulerNavigation.navigateToProvenanceStage}
              onNavigateProvenanceToolCall={schedulerNavigation.navigateToProvenanceToolCall}
              onDragEnter={(event) => {
                if (event.dataTransfer.types.includes("Files")) {
                  setComposerDragActive(true);
                }
              }}
              onDragOver={(event) => {
                if (!event.dataTransfer.types.includes("Files")) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "copy";
                setComposerDragActive(true);
              }}
              onDragLeave={(event) => {
                if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                setComposerDragActive(false);
              }}
              onDrop={(event) => void handleComposerDrop(event)}
              onFileChange={(event) => void handleFileChange(event)}
              onPaste={(event) => void handleComposerPaste(event)}
              onComposerChange={setComposer}
            />
          </div>

          {terminalOpen ? (
            <div className="shrink-0 px-4 pb-5 md:px-5">
              <div className="w-full overflow-hidden rounded-2xl border border-border/35 bg-sidebar shadow-sm">
                <div
                  className={terminalResize.handleClassName}
                  onMouseDown={terminalResize.handleMouseDown}
                  title="Resize terminal"
                />
                <div className="min-h-0 overflow-hidden" style={{ height: terminalResize.height }}>
                  <DeferredTerminalPanel
                    expanded={terminalOpen}
                    onExpand={() => setTerminalOpen(true)}
                    terminal={terminalSessions}
                  />
                </div>
              </div>
            </div>
          ) : null}
        </div>

        {rightSidebarOpen && (
          <>
            <div className={rightResize.handleClassName} onMouseDown={rightResize.handleMouseDown} />
            <div className="shrink-0 overflow-hidden border-l border-border/50 bg-sidebar" style={{ width: effectiveRightPanelWidth }}>
            <WorkspacePanel
              apiJson={apiJson}
              activeTab={workspacePanelTab}
              workspaceLoading={workspaceLoading}
              fileTree={fileTree}
              workspaceRootPath={workspaceRootPath || resolvedWorkspaceRootPath}
              workspaceRootLabel={workspaceRootPath || resolvedWorkspaceRootPath || currentSession?.directory || "project"}
              selectedWorkspacePath={selectedWorkspacePath}
              selectedWorkspaceType={selectedWorkspaceType}
              workspaceLinkLabel={workspaceLinkLabel}
              workspaceLinkStageId={workspaceLinkStageId}
              selectedFilePath={selectedFilePath}
              selectedFileContent={selectedFileContent}
              fileLoading={fileLoading}
              fileSaving={fileSaving}
              fileDeleting={fileDeleting}
              fileUploading={fileUploading}
              workspaceDirty={workspaceDirty}
              selectedWorkspaceIsRoot={selectedWorkspaceIsRoot}
              selectedWorkspaceReference={selectedWorkspaceReference}
              lastAssistant={lastAssistant}
              activeStageId={schedulerNavigation.activeStageId}
              previewStageId={schedulerNavigation.previewStageId}
              executionActivity={executionActivity}
              conversationJump={conversationJump}
              schedulerNavigation={schedulerNavigation}
              onCreateWorkspaceFile={createWorkspaceFile}
              onCreateWorkspaceDirectory={createWorkspaceDirectory}
              onUploadWorkspaceFiles={uploadWorkspaceFiles}
              onSelectWorkspaceNode={selectWorkspaceNode}
              onActiveTabChange={setWorkspacePanelTab}
              onWorkspaceContentChange={setSelectedFileContent}
              onInsertWorkspaceReference={insertWorkspaceReference}
              onAttachSelectedWorkspaceNode={attachSelectedWorkspaceNode}
              onDownloadSelectedFile={downloadSelectedFile}
              onDeleteSelectedWorkspaceNode={deleteSelectedWorkspaceNode}
              onSaveSelectedFile={saveSelectedFile}
            />
          </div>
          </>
        )}
      </div>

      {settingsOpen ? (
        <Suspense
          fallback={
            <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-start justify-end">
              <section className="h-full w-full max-w-md bg-card border-l border-border overflow-y-auto p-6 flex flex-col gap-4">
                <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground py-12">
                  <h3 className="text-sm">Loading settings...</h3>
                  <p className="text-xs">Please wait</p>
                </div>
              </section>
            </div>
          }
        >
          <SettingsDrawer
            onClose={() => setSettingsOpen(false)}
            theme={theme}
            themes={THEMES}
            onThemeChange={(nextTheme) => setTheme(nextTheme as ThemeId)}
            workspaceMode={resolvedWorkspaceMode}
            workspaceRootPath={resolvedWorkspaceRootPath}
            workspaceConfigDir={workspaceContext?.identity?.config_dir ?? null}
            selectedSessionId={selectedSessionId}
            modeOptions={settingsModeOptions}
            selectedMode={selectedMode}
            onModeChange={setSelectedMode}
            modelOptions={modelOptions}
            selectedModel={selectedModel}
            onModelChange={handleModelChange}
            showThinking={showThinking}
            onShowThinkingChange={setShowThinking}
            providers={providers}
            knownProviders={knownProviders}
            connectProtocols={connectProtocols}
            connectQuery={connectQuery}
            onConnectQueryChange={setConnectQuery}
            connectResolution={connectResolution}
            connectResolveBusy={connectResolveBusy}
            connectResolveError={connectResolveError}
            connectProviderId={connectProviderId}
            onConnectProviderIdChange={setConnectProviderId}
            connectProtocol={connectProtocol}
            onConnectProtocolChange={setConnectProtocol}
            connectApiKey={connectApiKey}
            onConnectApiKeyChange={setConnectApiKey}
            connectBaseUrl={connectBaseUrl}
            onConnectBaseUrlChange={setConnectBaseUrl}
            connectBusy={connectBusy}
            onConnectProvider={connectProvider}
            api={api}
            apiJson={apiJson}
            onBanner={setBanner}
            onReloadCoreData={reloadCoreSettingsData}
          />
        </Suspense>
      ) : null}

      <InteractionOverlays
        question={question}
        permission={permission}
        questionAnswers={questionAnswers}
        questionSubmitting={questionSubmitting}
        permissionSubmitting={permissionSubmitting}
        permissionSubmitError={permissionSubmitError}
        permissionSubmitStartedAt={permissionSubmitStartedAt}
        permissionSubmitCompletedAt={permissionSubmitCompletedAt}
        onQuestionAnswerChange={(index, value) =>
          setQuestionAnswers((current) => ({ ...current, [index]: value }))
        }
        onRejectQuestion={rejectQuestion}
        onSubmitQuestion={submitQuestion}
        onReplyPermission={replyPermission}
      />
    </div>
  );
}
