import {
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
import { BannerNotice } from "./components/chat/BannerNotice";
import {
  RuntimeSurfaceSection,
  type RuntimeSurfaceTab,
} from "./components/execution/RuntimeSurfaceSection";
import { loadWebPlugins } from "./web-plugin-loader";
import { api, apiJson } from "./lib/api";
import { cn } from "./lib/utils";
import { useAgendaoStore } from "./store";
import { useI18n } from "./i18n/I18nProvider";
import {
  formatError,
  pushRecentModel,
  readRuntimeBudgetNumber,
  resolveActiveModelRef,
  runtimeSurfaceLabel,
  workspaceRecentModelScope,
  type PromptPart,
} from "./lib/display";
import { useComposerAttachments } from "./hooks/useComposerAttachments";
import { useExecutionActivity } from "./hooks/useExecutionActivity";
import { useInteractionReplies } from "./hooks/useInteractionReplies";
import { useMultimodalComposer } from "./hooks/useMultimodalComposer";
import { usePromptSubmission } from "./hooks/usePromptSubmission";
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
import {
  currentContextTokensFromSources,
  isLiveStageStatus,
} from "./lib/contextPressure";
import {
  attachmentContainsWorkspacePath,
} from "./lib/composerContext";
import {
  estimateContextTokensFromHistory,
} from "./lib/liveTranscriptState";
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
  FolderTreeIcon,
  GitForkIcon,
  PanelLeftIcon,
  SettingsIcon,
  TerminalSquareIcon,
} from "lucide-react";
import {
  type ThemeId,
} from "./lib/webRuntime";

const SettingsPage = lazy(async () => {
  const module = await import("./components/settings/SettingsPage");
  return { default: module.SettingsPage };
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

export default function App() {
  const { t } = useI18n();
  // ============================================================
  // Store-backed state (replaces 24 individual useState calls)
  // ============================================================
  const sessions = useAgendaoStore((s) => s.sessions);
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const setSelectedMessageIds = useAgendaoStore((s) => s.setSelectedMessageIds);
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
  const route = useAgendaoStore((s) => s.route);
  const setRoute = useAgendaoStore((s) => s.setRoute);
  const statusLine = useAgendaoStore((s) => s.statusLine);
  const latestRuntimeError = useAgendaoStore((s) => s.latestRuntimeError);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const deletingSessions = useAgendaoStore((s) => s.deletingSessions);
  const question = useAgendaoStore((s) => s.question);
  const permission = useAgendaoStore((s) => s.permission);
  const questionAnswers = useAgendaoStore((s) => s.questionAnswers);
  const setQuestionAnswers = useAgendaoStore((s) => s.setQuestionAnswers);
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
  const leftResize = useResizableWidth(312, 220, 520, "left");
  const rightResize = useResizableWidth(420, 320, 880, "right");
  const terminalResize = useResizableHeight(320, 180, 640);
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

  const {
    clearPendingSessionRefresh,
    createSession,
    deleteSelectedSessions,
    exportSession,
    forkSelectedSession,
    forkSessionFromMessage,
    provisionExternalAdapterSession,
    refreshSessions,
    renameSession,
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
    editAndResendMessage,
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
    forkSessionFromMessage,
    formatError,
    maxPendingOutputBlocks,
    onConfigUpdated: reloadProvidersAndModes,
    onPrimeComposerFromPrompt: (text) => {
      setComposer(text);
      setAttachments([]);
      setSelectedAttachmentIndex(null);
    },
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
  const {
    attachComposerFiles,
    clearComposerNotice,
    composerNotice,
    handleComposerDrop,
    handleComposerPaste,
    handleFileChange,
    removeAttachmentAt,
  } = useComposerAttachments({
    apiJson,
    reloadWorkspacePreservingSelection,
    workspaceBasePath,
    workspaceDirty,
  });
  const { sendPromptRequest, stopActivePrompt, submitPrompt } = usePromptSubmission({
    apiJson,
    clearComposerNotice,
    createSession,
    loadPendingQuestion,
    optimisticMessagesRef,
    preflightBeforeSubmit: multimodalComposer.preflightBeforeSubmit,
    refreshSessions,
    setMessages,
  });
  const {
    permissionStatusLabel,
    permissionStatusTone,
    rejectQuestion,
    replyPermission,
    submitQuestion,
  } = useInteractionReplies({
    api,
    apiJson,
    loadPendingQuestion,
    sendPromptRequest,
  });

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
      <SettingsPage
        api={api}
        apiJson={apiJson}
        connectForm={connectForm}
        connectFormActions={connectFormActions}
        connectProtocols={connectProtocols}
        knownProviders={knownProviders}
        modes={modes}
        onBanner={setBanner}
        onClose={() => setRoute("workbench")}
        onModeChange={setSelectedMode}
        onModelChange={handleModelChange}
        onReloadCoreData={reloadCoreSettingsData}
        onShowThinkingChange={setShowThinking}
        onThemeChange={setTheme}
        providers={providers}
        recentModels={recentModels}
        selectedMode={selectedMode}
        selectedModel={selectedModel}
        selectedSessionId={selectedSessionId}
        showThinking={showThinking}
        theme={theme}
        workspaceConfigDir={workspaceContext?.identity?.config_dir ?? null}
        workspaceMode={resolvedWorkspaceMode}
        workspaceRootPath={resolvedWorkspaceRootPath}
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
                  onExportSession={(sessionId) => {
                    void exportSession(sessionId);
                  }}
                  onRenameSession={(sessionId, title) => {
                    void renameSession(sessionId, title);
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
          <BannerNotice />

          <RuntimeSurfaceSection
            activeRuntimeSurfaceTab={activeRuntimeSurfaceTab}
            currentRuntimeSurface={currentRuntimeSurface}
            hasCurrentRuntimeSurface={hasCurrentRuntimeSurface}
            hasRuntimeSurfaceContent={hasRuntimeSurfaceContent}
            runtimeSurfaceExpanded={runtimeSurfaceExpanded}
            runtimeSurfaceSummary={runtimeSurfaceSummary}
            runtimeSurfaceTabs={runtimeSurfaceTabs}
            selectedSessionId={selectedSessionId}
            setRuntimeSurfaceExpanded={setRuntimeSurfaceExpanded}
            setRuntimeSurfaceTab={setRuntimeSurfaceTab}
          />

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
            telemetry={executionActivity.telemetry}
            onCopyMessageLink={copyMessageLink}
            onCopySelectedMessageLink={copySelectedMessageLink}
            onCopySelectedMessagesMarkdown={copySelectedMessagesMarkdown}
            onEditAndResendMessage={editAndResendMessage}
            onClearSelectedMessages={() => setSelectedMessageIds(new Set())}
            onToggleMessageSelected={toggleMessageSelected}
            onNavigateStage={schedulerNavigation.navigateToStage}
            onNavigateAttachedSession={schedulerNavigation.navigateToAttachedSession}
            onAbortStage={(stageId) => void executionActivity.abortSchedulerStage(stageId)}
            stageAbortingId={executionActivity.stageAbortingId}
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
              onStopStreaming={stopActivePrompt}
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
              onAttachFiles={(files, failurePrefix) => void attachComposerFiles(files, failurePrefix)}
              onFileChange={(event) => void handleFileChange(event)}
              onPaste={(event) => void handleComposerPaste(event)}
              composerNotice={composerNotice}
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
