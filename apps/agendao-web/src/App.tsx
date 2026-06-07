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
import { ComposerSection } from "./components/composer/ComposerSection";
import { ConversationFeedPanel } from "./components/chat/ConversationFeedPanel";
import { SessionHeader } from "./components/session/SessionHeader";
import { DeferredTerminalPanel } from "./components/terminal/DeferredTerminalPanel";
import { InteractionOverlays } from "./components/overlays/InteractionOverlays";
import { loadWebPlugins } from "./web-plugin-loader";
import { api, apiJson } from "./lib/api";
import { cn } from "./lib/utils";
import { useAgendaoStore } from "./store";
import { useI18n } from "./i18n/I18nProvider";
import {
  formatError,
  modeKey,
  mergePendingCommandArguments,
  pendingCommandFromSession,
  promptPreviewText,
  pushRecentModel,
  readRuntimeBudgetNumber,
  resolveActiveModelRef,
  runtimeSurfaceDebugDetail,
  runtimeSurfaceLabel,
  runtimeSurfacePhase,
  runtimeSurfacePreview,
  normalizedAnswerValues,
  workspaceRecentModelScope,
  type PromptPart,
} from "./lib/display";
import { useExecutionActivity } from "./hooks/useExecutionActivity";
import { useMultimodalComposer } from "./hooks/useMultimodalComposer";
import { useRuntimeSurface } from "./hooks/useRuntimeSurface";
import { useSchedulerNavigation } from "./hooks/useSchedulerNavigation";
import { useSessionCoordinator } from "./hooks/useSessionCoordinator";
import { useTerminalSessions } from "./hooks/useTerminalSessions";
import { useTranscriptCoordinator } from "./hooks/useTranscriptCoordinator";
import { useWebBootstrap } from "./hooks/useWebBootstrap";
import { useWorkspaceCoordinator } from "./hooks/useWorkspaceCoordinator";
import { useResizableHeight, useResizableWidth } from "./hooks/useResizableWidth";
import { useProviderConnectForm } from "./hooks/useProviderConnectForm";
import { useDiagnosticsFromTelemetry } from "./hooks/useDiagnosticsFromTelemetry";
import { useProjectCreation } from "./hooks/useProjectCreation";
import { prepareComposerAttachments } from "./lib/composerAttachments";
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "./lib/contextPressure";
import {
  attachmentContainsWorkspacePath,
  attachmentLabel,
  attachmentWorkspacePath,
  droppedFiles,
} from "./lib/composerContext";
import type { RuntimeSurfaceOutputBlock } from "./lib/history";
import {
  applyOutputBlock,
  createOptimisticUserFeedMessage,
  estimateContextTokensFromHistory,
} from "./lib/liveTranscriptState";
import {
  type PromptResponseRecord,
} from "./lib/interaction";
import type { SessionRecord } from "./lib/session";
import {
  flattenProviderModels,
} from "./lib/provider";
import {
  buildSessionTree,
  buildWorkspaceSummaries,
} from "./lib/sidebar";
import {
  type RecentModelsPayloadRecord,
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
  THEMES,
  type ThemeId,
} from "./lib/webRuntime";

const SettingsDrawer = lazy(async () => {
  const module = await import("./components/settings/SettingsDrawer");
  return { default: module.SettingsDrawer };
});

const SessionSidebar = lazy(async () => {
  const module = await import("./components/session/SessionSidebar");
  return { default: module.SessionSidebar };
});

const WorkspacePanel = lazy(async () => {
  const module = await import("./components/workspace/WorkspacePanel");
  return { default: module.WorkspacePanel };
});

const THEME_FAVICON_SRC: Record<ThemeId, string> = {
  daylight: `${import.meta.env.BASE_URL}brand/agendao-icon-mark-daylight.svg`,
  sunset: `${import.meta.env.BASE_URL}brand/agendao-icon-mark-sunset.svg`,
  cobalt: `${import.meta.env.BASE_URL}brand/agendao-icon-mark-cobalt.svg`,
};

type RuntimeSurfaceTab = "queue" | "session" | "inspect";

function RuntimeSurfaceList({
  title,
  blocks,
  emptyLabel,
}: {
  title: string;
  blocks: RuntimeSurfaceOutputBlock[];
  emptyLabel: string;
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-border/40 bg-background/78">
      <div className="flex items-center justify-between border-b border-border/35 px-3 py-2.5">
        <h3 className="text-sm font-medium text-foreground">{title}</h3>
        <span className="text-xs text-muted-foreground">{blocks.length}</span>
      </div>
      {blocks.length === 0 ? (
        <div className="px-3 py-5 text-sm text-muted-foreground">{emptyLabel}</div>
      ) : (
        <div className="max-h-[124px] space-y-2 overflow-auto px-3 py-2.5">
          {blocks.map((block) => {
            const preview = runtimeSurfacePreview(block);
            const phase = runtimeSurfacePhase(block);
            return (
              <article
                key={block.id ?? `${block.kind}:${block.event ?? block.title ?? preview ?? Math.random()}`}
                className="rounded-lg border border-border/30 bg-card/70 px-2.5 py-2"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">
                    {runtimeSurfaceLabel(block)}
                  </span>
                  {phase ? (
                    <span className="roc-badge px-2 py-0.5 text-[11px]">{phase}</span>
                  ) : null}
                </div>
                {preview ? (
                  <p className="mt-2 whitespace-pre-wrap text-sm leading-6 text-muted-foreground">
                    {preview}
                  </p>
                ) : null}
                {runtimeSurfaceDebugDetail(block) ? (
                  <p className="mt-2 whitespace-pre-wrap text-xs leading-5 text-muted-foreground/80">
                    {runtimeSurfaceDebugDetail(block)}
                  </p>
                ) : null}
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}

export default function App() {
  const { t } = useI18n();
  // ============================================================
  // Store-backed state (replaces 24 individual useState calls)
  // ============================================================
  const sessions = useAgendaoStore((s) => s.sessions);
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const setSelectedMessageIds = useAgendaoStore((s) => s.setSelectedMessageIds);
  const composer = useAgendaoStore((s) => s.composer);
  const setComposer = useAgendaoStore((s) => s.setComposer);
  const attachments = useAgendaoStore((s) => s.attachments);
  const setAttachments = useAgendaoStore((s) => s.setAttachments);
  const providers = useAgendaoStore((s) => s.providers);
  const knownProviders = useAgendaoStore((s) => s.knownProviders);
  const connectProtocols = useAgendaoStore((s) => s.connectProtocols);
  const modes = useAgendaoStore((s) => s.modes);
  const workspaceContext = useAgendaoStore((s) => s.workspaceContext);
  const selectedModel = useAgendaoStore((s) => s.selectedModel);
  const setSelectedModel = useAgendaoStore((s) => s.setSelectedModel);
  const selectedMode = useAgendaoStore((s) => s.selectedMode);
  const setSelectedMode = useAgendaoStore((s) => s.setSelectedMode);
  const theme = useAgendaoStore((s) => s.theme);
  const setTheme = useAgendaoStore((s) => s.setTheme);
  const showThinking = useAgendaoStore((s) => s.showThinking);
  const setShowThinking = useAgendaoStore((s) => s.setShowThinking);
  const streaming = useAgendaoStore((s) => s.streaming);
  const route = useAgendaoStore((s) => s.route);
  const setRoute = useAgendaoStore((s) => s.setRoute);
  const statusLine = useAgendaoStore((s) => s.statusLine);
  const latestRuntimeError = useAgendaoStore((s) => s.latestRuntimeError);
  const banner = useAgendaoStore((s) => s.banner);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const deletingSessions = useAgendaoStore((s) => s.deletingSessions);
  const question = useAgendaoStore((s) => s.question);
  const permission = useAgendaoStore((s) => s.permission);
  const questionAnswers = useAgendaoStore((s) => s.questionAnswers);
  const setQuestion = useAgendaoStore((s) => s.setQuestion);
  const setQuestionAnswers = useAgendaoStore((s) => s.setQuestionAnswers);
  const setQuestionSubmitting = useAgendaoStore((s) => s.setQuestionSubmitting);
  const setPermission = useAgendaoStore((s) => s.setPermission);
  const setPermissionSubmitCompletedAt = useAgendaoStore((s) => s.setPermissionSubmitCompletedAt);
  const setPermissionSubmitError = useAgendaoStore((s) => s.setPermissionSubmitError);
  const setPermissionSubmitStartedAt = useAgendaoStore((s) => s.setPermissionSubmitStartedAt);
  const setPermissionSubmitting = useAgendaoStore((s) => s.setPermissionSubmitting);
  const setStreaming = useAgendaoStore((s) => s.setStreaming);
  const setStatusLine = useAgendaoStore((s) => s.setStatusLine);
  const setLatestRuntimeError = useAgendaoStore((s) => s.setLatestRuntimeError);
  const setCurrentWorkspacePath = useAgendaoStore((s) => s.setCurrentWorkspacePath);
  const setSelectedAttachmentIndex = useAgendaoStore((s) => s.selectAttachment);
  const setWorkspaceContext = useAgendaoStore((s) => s.setWorkspaceContext);
  const questionSubmitting = useAgendaoStore((s) => s.questionSubmitting);
  const permissionSubmitting = useAgendaoStore((s) => s.permissionSubmitting);
  const permissionSubmitError = useAgendaoStore((s) => s.permissionSubmitError);
  const permissionSubmitStartedAt = useAgendaoStore((s) => s.permissionSubmitStartedAt);
  const permissionSubmitCompletedAt = useAgendaoStore((s) => s.permissionSubmitCompletedAt);
  const setComposerDragActive = useAgendaoStore((s) => s.setComposerDragActive);
  const selectedAttachmentIndex = useAgendaoStore((s) => s.selectedAttachmentIndex);
  const terminalOpen = useAgendaoStore((s) => s.terminalOpen);
  const setTerminalOpen = useAgendaoStore((s) => s.setTerminalOpen);
  const serviceRootPath = useAgendaoStore((s) => s.serviceRootPath);
  const currentWorkspacePath = useAgendaoStore((s) => s.currentWorkspacePath);
  const workspacePanelTab = useAgendaoStore((s) => s.workspacePanelTab);
  const selectedWorkspacePath = useAgendaoStore((s) => s.selectedWorkspacePath);
  const leftSidebarOpen = useAgendaoStore((s) => s.leftSidebarOpen);
  const setLeftSidebarOpen = useAgendaoStore((s) => s.setLeftSidebarOpen);
  const rightSidebarOpen = useAgendaoStore((s) => s.rightSidebarOpen);
  const setRightSidebarOpen = useAgendaoStore((s) => s.setRightSidebarOpen);
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
  const [connectForm, connectFormActions] = useProviderConnectForm(
    connectProtocols, apiJson as <T>(url: string, init?: RequestInit) => Promise<T>, formatError,
  );
  const connectQuery = connectForm.query;
  const setConnectQuery = connectFormActions.setQuery;
  const connectProviderId = connectForm.providerId;
  const setConnectProviderId = connectFormActions.setProviderId;
  const leftResize = useResizableWidth(312, 220, 520, "left");
  const rightResize = useResizableWidth(420, 320, 880, "right");
  const terminalResize = useResizableHeight(320, 180, 640);
  const connectProtocol = connectForm.protocol;
  const setConnectProtocol = connectFormActions.setProtocol;
  const connectApiKey = connectForm.apiKey;
  const setConnectApiKey = connectFormActions.setApiKey;
  const connectBaseUrl = connectForm.baseUrl;
  const setConnectBaseUrl = connectFormActions.setBaseUrl;
  const connectResolution = connectForm.resolution;
  const connectResolveBusy = connectForm.resolveBusy;
  const connectResolveError = connectForm.resolveError;
  const connectBusy = connectForm.busy;
  const setConnectBusy = connectFormActions.setBusy;
  const feedRef = useRef<HTMLDivElement | null>(null);
  const preferencesReadyRef = useRef(false);
  const maxPendingOutputBlocks = useMemo(
    () =>
      readRuntimeBudgetNumber(workspaceContext?.config, "web_max_pending_output_blocks", 256),
    [workspaceContext?.config],
  );

  const {
    currentRuntimeSurface,
    hasCurrentRuntimeSurface,
  } = useRuntimeSurface();
  const [runtimeSurfaceExpanded, setRuntimeSurfaceExpanded] = useState(false);
  const [runtimeSurfaceTab, setRuntimeSurfaceTab] = useState<RuntimeSurfaceTab>("queue");
  // P2-3: viewport budget for rendered messages. When exceeded, only the most
  // recent messages are rendered. Full transcript is preserved in state.
  // Derived from agendao_config::RuntimeBudgetConfig.tui_max_viewport_messages.
  // connectResolveRequestRef moved to useProviderConnectForm
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
    [recentModels, setBanner, setWorkspaceContext],
  );
  const handleModelChange = useCallback(
    (value: string) => {
      recentModelAutoSuppressedRef.current = value.trim().length === 0;
      setSelectedModel(value);
      if (value.trim()) {
        void persistRecentModel(value);
      }
    },
    [persistRecentModel, setSelectedModel],
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
  }, [providers, recentModels, selectedModel, setSelectedModel, workspaceContext]);
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
  const sessionUsage = executionActivity.sessionUsage ?? currentSession?.telemetry?.usage ?? null;
  const runtimeSurfaceTabs = useMemo(
    () => [
      {
        key: "queue" as const,
        label: t("app.runtimeSurfaceQueue"),
        count: currentRuntimeSurface.queueItems.length,
        blocks: currentRuntimeSurface.queueItems,
      },
      {
        key: "session" as const,
        label: t("app.runtimeSurfaceSessionEvents"),
        count: currentRuntimeSurface.sessionEvents.length,
        blocks: currentRuntimeSurface.sessionEvents,
      },
      {
        key: "inspect" as const,
        label: t("app.runtimeSurfaceInspect"),
        count: currentRuntimeSurface.inspectItems.length,
        blocks: currentRuntimeSurface.inspectItems,
      },
    ],
    [
      currentRuntimeSurface.inspectItems,
      currentRuntimeSurface.queueItems,
      currentRuntimeSurface.sessionEvents,
      t,
    ],
  );
  const hasRuntimeSurfaceContent = Boolean(currentRuntimeSurface.banner)
    || runtimeSurfaceTabs.some((tab) => tab.count > 0);
  const activeRuntimeSurfaceTab = useMemo(
    () =>
      runtimeSurfaceTabs.find((tab) => tab.key === runtimeSurfaceTab)
      ?? runtimeSurfaceTabs.find((tab) => tab.count > 0)
      ?? runtimeSurfaceTabs[0],
    [runtimeSurfaceTab, runtimeSurfaceTabs],
  );
  const runtimeSurfaceSummary = useMemo(() => {
    if (currentRuntimeSurface.banner?.trim()) {
      return currentRuntimeSurface.banner.trim().split("\n")[0] ?? currentRuntimeSurface.banner.trim();
    }
    const firstQueue = currentRuntimeSurface.queueItems[0];
    if (firstQueue) {
      return t("app.runtimeSurfaceQueueSummary", {
        count: currentRuntimeSurface.queueItems.length,
        label: runtimeSurfaceLabel(firstQueue),
      });
    }
    const firstSessionEvent = currentRuntimeSurface.sessionEvents[0];
    if (firstSessionEvent) {
      return t("app.runtimeSurfaceSessionSummary", {
        count: currentRuntimeSurface.sessionEvents.length,
        label: runtimeSurfaceLabel(firstSessionEvent),
      });
    }
    const firstInspect = currentRuntimeSurface.inspectItems[0];
    if (firstInspect) {
      return t("app.runtimeSurfaceInspectSummary", {
        count: currentRuntimeSurface.inspectItems.length,
        label: runtimeSurfaceLabel(firstInspect),
      });
    }
    return t("app.runtimeSurfaceIdle");
  }, [currentRuntimeSurface.banner, currentRuntimeSurface.inspectItems, currentRuntimeSurface.queueItems, currentRuntimeSurface.sessionEvents, t]);

  useEffect(() => {
    if (!hasRuntimeSurfaceContent) {
      setRuntimeSurfaceExpanded(false);
      return;
    }
    const preferredTab = runtimeSurfaceTabs.find((tab) => tab.count > 0)?.key ?? "queue";
    setRuntimeSurfaceTab((current) => {
      if (runtimeSurfaceTabs.some((tab) => tab.key === current && tab.count > 0)) {
        return current;
      }
      return preferredTab;
    });
  }, [hasRuntimeSurfaceContent, runtimeSurfaceTabs]);
  const effectiveRightPanelWidth = useMemo(() => {
    if (workspacePanelTab === "preview") return Math.max(rightResize.width, 640);
    if (workspacePanelTab === "insights") return Math.max(rightResize.width, 460);
    return rightResize.width;
  }, [rightResize.width, workspacePanelTab]);
  const refreshExecutionActivity = executionActivity.refreshExecutionActivity;
  const applySchedulerStageOutputBlock = executionActivity.applySchedulerStageOutputBlock;
  const applyLiveExecutionOutputBlock = executionActivity.applyLiveExecutionOutputBlock;
  const terminalSessions = useTerminalSessions({
    api,
    apiJson,
    setBanner,
    enabled: terminalOpen,
    defaultCwd: currentSession?.directory?.trim() || currentWorkspaceSummary?.path || serviceRootPath || "",
    sessionId: currentSession?.id ?? selectedSessionId ?? null,
  });

  const sendPromptRequest = async (
    sessionId: string,
    payload: Record<string, unknown>,
  ): Promise<PromptResponseRecord> =>
    apiJson<PromptResponseRecord>(`/session/${sessionId}/prompt`, {
      method: "POST",
      body: JSON.stringify(payload),
    });

  const {
    clearPendingSessionRefresh,
    createSession,
    deleteSelectedSessions,
    forkSelectedSession,
    provisionExternalAdapterSession,
    refreshSessions,
    scheduleSessionRefresh,
    selectSession,
    selectWorkspace,
  } = useSessionCoordinator({
    api,
    apiJson,
    currentWorkspacePath,
    currentWorkspaceSummaryPath: currentWorkspaceSummary?.path ?? null,
    formatError,
    selectedSessionId,
    serviceRootPath,
    workspaceContextRootPath: workspaceRootFromContext(workspaceContext),
  });

  const { reloadCoreSettingsData, reloadProvidersAndModes } = useWebBootstrap({
    apiJson,
    formatError,
    preferencesReadyRef,
    provisionExternalAdapterSession,
  });
  const {
    conversationJump,
    copyMessageLink,
    copySelectedMessageLink,
    copySelectedMessagesMarkdown,
    loadPendingQuestion,
    messageHistory,
    optimisticMessagesRef,
    routeHighlightIds,
    setMessages,
    toggleMessageSelected,
  } = useTranscriptCoordinator({
    apiJson,
    applyLiveExecutionOutputBlock,
    applySchedulerStageOutputBlock,
    clearPendingSessionRefresh,
    feedRef,
    formatError,
    maxPendingOutputBlocks,
    onConfigUpdated: reloadProvidersAndModes,
    refreshExecutionActivity,
    scheduleSessionRefresh,
  });
  const composerContextTokens = useMemo(() => {
    const activeEstimate =
      executionActivity.activeStageSummary && isLiveStageStatus(executionActivity.activeStageSummary.status)
        ? executionActivity.activeStageSummary.estimated_context_tokens
        : undefined;
    return currentContextTokensFromSources(sessionUsage?.context_tokens, activeEstimate)
      ?? estimateContextTokensFromHistory(messageHistory);
  }, [executionActivity.activeStageSummary, messageHistory, sessionUsage?.context_tokens]);
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
  const { latestClosureDiagnostic, latestIngressDiagnostic, latestProviderDiagnostic } =
    useDiagnosticsFromTelemetry(executionActivity.telemetry, messageHistory);
  const {
    attachSelectedWorkspaceNode,
    createWorkspaceDirectory,
    createWorkspaceFile,
    insertWorkspaceReference,
    locateAttachmentInWorkspace,
    reloadWorkspacePreservingSelection,
    reloadWorkspaceWithSelection,
    selectWorkspaceNode,
    ensureWorkspaceNodeLoaded,
    selectedWorkspaceFilename,
    uploadWorkspaceFiles,
    workspaceBasePath,
    workspaceDirty,
  } = useWorkspaceCoordinator({
    api,
    apiJson,
    currentSessionDirectory: currentSession?.directory,
    currentWorkspaceSummaryPath: currentWorkspaceSummary?.path ?? null,
    formatError,
    messageHistory,
    selectedSessionId,
    serviceRootPath,
    workspaceContext,
  });
  const createProject = useProjectCreation({
    apiJson,
    serviceRootPath,
    workspaceBasePath,
    createSession,
    reloadWorkspaceWithSelection,
  });
  const schedulerNavigation = useSchedulerNavigation({
    apiJson,
    executionActivity,
    jumpToConversationTarget: conversationJump.jumpOrQueueConversationTarget,
    queueConversationJumpTarget: conversationJump.queueConversationJumpTarget,
  });
  const workspaceLinkLabel = schedulerNavigation.activeStageId ? `stage ${schedulerNavigation.activeStageId}` : schedulerNavigation.currentBreadcrumbProvenance?.toolCallId ? `tool ${schedulerNavigation.currentBreadcrumbProvenance.toolCallId}` : schedulerNavigation.currentBreadcrumbProvenance?.stageId ? `stage ${schedulerNavigation.currentBreadcrumbProvenance.stageId}` : null;
  const workspaceLinkStageId = schedulerNavigation.activeStageId ?? schedulerNavigation.currentBreadcrumbProvenance?.stageId ?? null;

  useEffect(() => {
    if (!selectedWorkspacePath) return;
    const nextIndex = attachments.findIndex((attachment) =>
      attachmentContainsWorkspacePath(attachment, selectedWorkspacePath),
    );
    if (nextIndex >= 0 && nextIndex !== selectedAttachmentIndex) {
      setSelectedAttachmentIndex(nextIndex);
    }
  }, [attachments, selectedAttachmentIndex, selectedWorkspacePath, setSelectedAttachmentIndex]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    const favicon = document.getElementById("theme-favicon");
    if (favicon instanceof HTMLLinkElement) {
      favicon.href = THEME_FAVICON_SRC[theme];
    }
  }, [theme]);

  // Provider connect resolution moved to useProviderConnectForm

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
  }, [currentSession?.directory, serviceRootPath, setCurrentWorkspacePath, workspaceSummaries]);

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
  }, [selectedMode, selectedModel, setBanner, showThinking, theme]);

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

    let sessionId = selectedSessionId;
    if (!sessionId) {
      try {
        sessionId = await createSession();
      } catch (error) {
        setBanner(`Failed to create session: ${formatError(error)}`);
        return;
      }
    }

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
      await refreshSessions();
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
      .filter((path): path is string => Boolean(path && path.includes("/.agendao/uploads/")));
    if (uploadedPaths.length && !workspaceDirty) {
      reloadWorkspacePreservingSelection();
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
      const sessionId = question.session_id ?? selectedSessionId;
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
      setPermission(null);
      setPermissionSubmitting(false);
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

  const removeAttachmentAt = (index: number) => {
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
  };

  const settingsPage = (
    <Suspense
      fallback={
        <div className="roc-app-shell flex h-dvh flex-col overflow-hidden bg-background text-foreground font-sans">
          <div className="mx-auto flex h-full w-full max-w-[110rem] flex-1 items-start justify-center px-4 py-6 md:px-6">
            <section className="flex h-full w-full flex-col rounded-[28px] border border-border/60 bg-card px-6 py-8 shadow-sm">
              <div className="flex flex-col items-center justify-center gap-2 text-muted-foreground py-12">
                <h3 className="text-sm">{t("app.loadingSettings")}</h3>
                <p className="text-xs">{t("app.pleaseWait")}</p>
              </div>
            </section>
          </div>
        </div>
      }
    >
      <SettingsDrawer
        onClose={() => setRoute("workbench")}
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
  );

  const workbenchPage = (
    <div className="roc-app-shell flex h-dvh flex-col overflow-hidden bg-background text-foreground font-sans">
      <div className="flex flex-1 overflow-hidden">
        {leftSidebarOpen && (
          <>
            <div className="shrink-0 overflow-hidden border-r border-border/50 bg-sidebar" style={{ width: leftResize.width }}>
              <Suspense
                fallback={
                  <div className="flex h-full items-center justify-center px-4 text-sm text-muted-foreground">
                    {t("app.loadingSessions")}
                  </div>
                }
              >
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
              </Suspense>
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
                title={t("app.showSidebar")}
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
                title={t("app.forkSession")}
                aria-label={t("app.forkSession")}
              >
                <GitForkIcon className="size-4" />
              </button>
            ) : null}
            {!rightSidebarOpen && selectedWorkspaceFilename ? (
              <button
                onClick={() => setRightSidebarOpen(true)}
                className="hidden items-center gap-1.5 rounded-full border border-border/55 bg-background/78 px-3 py-1.5 text-xs text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground md:flex"
                title={t("app.showWorkspace")}
              >
                <span className="truncate max-w-[10rem]">{selectedWorkspaceFilename}</span>
              </button>
            ) : null}
            <button
              onClick={() => setRightSidebarOpen((value) => !value)}
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={rightSidebarOpen ? t("app.hideWorkspace") : t("app.showWorkspace")}
            >
              <FolderTreeIcon className={cn("size-4", rightSidebarOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setTerminalOpen((value) => !value)}
              data-testid="terminal-toggle"
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={terminalOpen ? t("app.hideTerminal") : t("app.showTerminal")}
            >
              <TerminalSquareIcon className={cn("size-4", terminalOpen && "text-foreground")} />
            </button>
            <button
              onClick={() => setRoute("settings")}
              data-testid="settings-open"
              className="rounded-lg border border-border/50 bg-background/78 p-1.5 text-muted-foreground shadow-sm backdrop-blur transition-colors hover:bg-muted hover:text-foreground"
              title={t("app.settings")}
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
                  <div className="roc-section-label">{t("app.attention")}</div>
                  <p className="mt-1 text-sm leading-6 text-current/92">{banner}</p>
                </div>
                <button
                  type="button"
                  className="roc-banner-dismiss shrink-0"
                  aria-label={t("app.dismissStatusMessage")}
                  onClick={() => setBanner(null)}
                >
                  <XIcon className="size-4" />
                </button>
              </div>
            </div>
          ) : null}

          {selectedSessionId && hasCurrentRuntimeSurface && hasRuntimeSurfaceContent ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <div
                className="roc-panel max-h-[240px] overflow-hidden px-0 py-0"
                data-testid="runtime-surface"
                data-expanded={runtimeSurfaceExpanded ? "true" : "false"}
              >
                <button
                  type="button"
                  data-testid="runtime-surface-toggle"
                  className="flex h-10 w-full items-center justify-between gap-3 px-4 text-left"
                  aria-expanded={runtimeSurfaceExpanded}
                  title={runtimeSurfaceExpanded ? t("app.runtimeSurfaceHideDetails") : t("app.runtimeSurfaceDetails")}
                  onClick={() => setRuntimeSurfaceExpanded((value) => !value)}
                >
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium text-foreground">{runtimeSurfaceSummary}</p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {runtimeSurfaceTabs.map((tab) =>
                      tab.count > 0 ? (
                        <span key={tab.key} className="roc-badge px-2 py-0.5 text-[11px]">
                          {tab.label} {tab.count}
                        </span>
                      ) : null,
                    )}
                  </div>
                </button>
                {runtimeSurfaceExpanded ? (
                  <div
                    className="max-h-[196px] overflow-hidden border-t border-border/40 px-3 pb-3 pt-2.5"
                    data-testid="runtime-surface-expanded"
                  >
                    <div className="mb-2 flex flex-wrap items-center gap-1.5" data-testid="runtime-surface-tabs">
                      {runtimeSurfaceTabs.map((tab) => (
                        <button
                          key={tab.key}
                          type="button"
                          data-testid={`runtime-surface-tab-${tab.key}`}
                          className={cn(
                            "inline-flex h-7 items-center rounded-full px-2.5 text-[11px] font-medium transition-colors",
                            activeRuntimeSurfaceTab.key === tab.key
                              ? "bg-foreground/8 text-foreground"
                              : "text-muted-foreground hover:bg-accent/45 hover:text-foreground",
                          )}
                          onClick={() => setRuntimeSurfaceTab(tab.key)}
                        >
                          {tab.label}
                        </button>
                      ))}
                    </div>
                    {currentRuntimeSurface.banner ? (
                      <div
                        className="mb-2 rounded-lg border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-sm leading-5 text-amber-900 dark:text-amber-100"
                        data-testid="runtime-surface-banner"
                      >
                        {currentRuntimeSurface.banner}
                      </div>
                    ) : null}
                    <RuntimeSurfaceList
                      title={activeRuntimeSurfaceTab.label}
                      blocks={activeRuntimeSurfaceTab.blocks}
                      emptyLabel={t("app.noEventsYet")}
                    />
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}

          {selectedSessionId ? (
            <div className="mx-auto w-full max-w-[88rem] px-4 pt-3 md:px-5">
              <SessionHeader
                title={currentSession?.title || "(untitled)"}
                subtitle={currentSession?.directory || null}
                usageSummary={executionActivity.runTailSummary.title}
                usageTitle={executionActivity.runTailSummary.detail}
                modeLabel={selectedMode || null}
                modelLabel={selectedModel || null}
                activeStageId={schedulerNavigation.activeStageId}
                currentWorkspaceReference={workspaceBasePath || resolvedWorkspaceRootPath || null}
                breadcrumbs={schedulerNavigation.sessionBreadcrumbs}
                provenance={schedulerNavigation.currentBreadcrumbProvenance}
                onNavigateStage={schedulerNavigation.navigateToStage}
                onNavigateBreadcrumb={schedulerNavigation.navigateToBreadcrumb}
                onNavigateProvenanceSession={schedulerNavigation.navigateToProvenanceSession}
                onNavigateProvenanceStage={schedulerNavigation.navigateToProvenanceStage}
                onNavigateProvenanceToolCall={schedulerNavigation.navigateToProvenanceToolCall}
              />
            </div>
          ) : null}

          <ConversationFeedPanel
            sessionId={selectedSessionId}
            feedRef={feedRef}
            highlightedFeedId={conversationJump.highlightedFeedId}
            highlightedMessageIds={routeHighlightIds}
            activeStageId={schedulerNavigation.previewStageId ?? schedulerNavigation.activeStageId}
            activeToolCallId={schedulerNavigation.activeToolCallId}
            onCopyMessageLink={copyMessageLink}
            onCopySelectedMessageLink={copySelectedMessageLink}
            onCopySelectedMessagesMarkdown={copySelectedMessagesMarkdown}
            onClearSelectedMessages={() => setSelectedMessageIds(new Set())}
            onToggleMessageSelected={toggleMessageSelected}
            onNavigateStage={schedulerNavigation.navigateToStage}
            onNavigateAttachedSession={schedulerNavigation.navigateToAttachedSession}
          />

          <div className="shrink-0 px-4 pb-5 pt-2 md:px-5">
            <ComposerSection
              multimodalHints={multimodalComposer.hints}
              allowAudioInput={multimodalComposer.policy?.allow_audio_input ?? true}
              allowImageInput={multimodalComposer.policy?.allow_image_input ?? true}
              allowFileInput={multimodalComposer.policy?.allow_file_input ?? true}
              onModelChange={handleModelChange}
              workspaceRootPath={workspaceBasePath || resolvedWorkspaceRootPath || ""}
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
            />
          </div>

          {terminalOpen ? (
            <div className="shrink-0 px-4 pb-5 md:px-5">
              <div className="w-full overflow-hidden rounded-2xl border border-border/35 bg-sidebar shadow-sm">
                <div
                  className={terminalResize.handleClassName}
                  onMouseDown={terminalResize.handleMouseDown}
                  title={t("app.resizeTerminal")}
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
              <Suspense
                fallback={
                  <div className="flex h-full items-center justify-center px-4 text-sm text-muted-foreground">
                    {t("app.loadingWorkspace")}
                  </div>
                }
              >
                <WorkspacePanel
                  apiJson={apiJson}
                  workspaceRootLabel={workspaceBasePath || resolvedWorkspaceRootPath || currentSession?.directory || "project"}
                  workspaceLinkLabel={workspaceLinkLabel}
                  workspaceLinkStageId={workspaceLinkStageId}
                  executionActivity={executionActivity}
                  schedulerNavigation={schedulerNavigation}
                  onCreateWorkspaceFile={createWorkspaceFile}
                  onCreateWorkspaceDirectory={createWorkspaceDirectory}
                  onUploadWorkspaceFiles={uploadWorkspaceFiles}
                  onSelectWorkspaceNode={selectWorkspaceNode}
                  onExpandWorkspaceNode={ensureWorkspaceNodeLoaded}
                  onInsertWorkspaceReference={insertWorkspaceReference}
                  onAttachSelectedWorkspaceNode={attachSelectedWorkspaceNode}
                />
              </Suspense>
            </div>
          </>
        )}
      </div>

    </div>
  );

  return (
    <>
      {route === "settings" ? settingsPage : workbenchPage}
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
    </>
  );
}
