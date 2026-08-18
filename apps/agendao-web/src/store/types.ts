import type {
  ConnectProtocolOption,
  KnownProviderEntry,
  ProviderRecord,
} from "../lib/provider";
import type { CommandApiSpec } from "../lib/command";
import type { ExecutionMode, ThemeId } from "../lib/webRuntime";
import type { WorkspaceContextRecord, FileTreeNodeRecord } from "../lib/workspace";
import type { SessionRecord } from "../lib/session";
import type {
  FeedMessage,
  MessageRecord,
} from "../lib/history";
import type {
  PermissionInteractionRecord,
  QuestionAnswerValue,
  QuestionInteractionRecord,
} from "../lib/interaction";
import type { WorkspacePanelTab } from "../components/workspace/WorkspacePanel";
import type { SessionTaskLedger } from "../lib/taskLedger";
import type { AuxiliaryOutputBlock } from "../lib/history";
import type {
  BreadcrumbProvenance,
  SessionBreadcrumb,
} from "../hooks/useSchedulerNavigation";

// ============================================================
// Shared types
// ============================================================

export type PromptPart =
  | { type: "text"; text: string }
  | { type: "file"; url: string; filename?: string; mime?: string };

/** SetStateFn<T> accepts either a direct value or a functional updater, matching React's Dispatch<SetStateAction<T>>. */
export type SetStateFn<T> = T | ((prev: T) => T);

export type StoreSet = (
  partial:
    | Partial<AgendaoState>
    | ((state: AgendaoState) => Partial<AgendaoState>),
) => void;

export type StoreGet = () => AgendaoState;

export function resolveSetState<T>(value: SetStateFn<T>, current: T): T {
  return typeof value === "function" ? (value as (prev: T) => T)(current) : value;
}

export type RuntimeSurfaceKey = "sessionEvents" | "inspectItems" | "queueItems";

export type NoticeTone = "info" | "success" | "warning" | "error";

export interface AppNotice {
  id: number;
  tone: NoticeTone;
  message: string;
  at: number;
}

/** Per-session live runtime state mirrored from the event stream. */
export interface SessionRuntimeView {
  streaming: boolean;
  statusLine: string;
  latestRuntimeError: string | null;
  question: QuestionInteractionRecord | null;
  permission: PermissionInteractionRecord | null;
}

export const DEFAULT_SESSION_RUNTIME_VIEW: SessionRuntimeView = {
  streaming: false,
  statusLine: "ready",
  latestRuntimeError: null,
  question: null,
  permission: null,
};

export interface SessionRuntimeSurface {
  banner: string | null;
  sessionEvents: AuxiliaryOutputBlock[];
  inspectItems: AuxiliaryOutputBlock[];
  queueItems: AuxiliaryOutputBlock[];
}

// ============================================================
// State interface — each slice returns Pick<AgendaoState, ...>
// ============================================================

export interface AgendaoState {
  // === Config Surface ===
  providers: ProviderRecord[];
  knownProviders: KnownProviderEntry[];
  connectProtocols: ConnectProtocolOption[];
  modes: ExecutionMode[];
  slashCommands: CommandApiSpec[];
  workspaceContext: WorkspaceContextRecord | null;
  selectedModel: string;
  selectedMode: string;
  showThinking: boolean;
  serviceRootPath: string;
  theme: ThemeId;

  setProviders: (providers: SetStateFn<ProviderRecord[]>) => void;
  setKnownProviders: (knownProviders: SetStateFn<KnownProviderEntry[]>) => void;
  setConnectProtocols: (protocols: SetStateFn<ConnectProtocolOption[]>) => void;
  setModes: (modes: SetStateFn<ExecutionMode[]>) => void;
  setSlashCommands: (commands: SetStateFn<CommandApiSpec[]>) => void;
  setWorkspaceContext: (context: SetStateFn<WorkspaceContextRecord | null>) => void;
  setSelectedModel: (model: SetStateFn<string>) => void;
  setSelectedMode: (mode: SetStateFn<string>) => void;
  setShowThinking: (show: SetStateFn<boolean>) => void;
  setServiceRootPath: (path: SetStateFn<string>) => void;
  setTheme: (theme: SetStateFn<ThemeId>) => void;

  // === UI Layout ===
  route: "workbench" | "settings";
  leftSidebarOpen: boolean;
  rightSidebarOpen: boolean;
  terminalOpen: boolean;
  banner: string | null;

  setRoute: (route: SetStateFn<"workbench" | "settings">) => void;
  setLeftSidebarOpen: (open: SetStateFn<boolean>) => void;
  setRightSidebarOpen: (open: SetStateFn<boolean>) => void;
  setTerminalOpen: (open: SetStateFn<boolean>) => void;
  notices: AppNotice[];
  /** Push (or clear with null) a notice; tone inferred from the message. */
  setBanner: (message: SetStateFn<string | null>, tone?: NoticeTone) => void;
  pushNotice: (message: string, tone?: NoticeTone) => void;
  dismissNotice: (id: number) => void;
  clearBanner: () => void;

  // === Session List ===
  sessions: SessionRecord[];
  selectedSessionId: string | null;
  deletingSessions: boolean;

  setSessions: (sessions: SetStateFn<SessionRecord[]>) => void;
  setSelectedSessionId: (id: SetStateFn<string | null>) => void;
  selectSession: (id: string | null) => void;
  setDeletingSessions: (deleting: SetStateFn<boolean>) => void;

  // === Composer ===
  composer: string;
  attachments: PromptPart[];
  selectedAttachmentIndex: number | null;
  composerDragActive: boolean;

  setComposer: (text: SetStateFn<string>) => void;
  setAttachments: (parts: SetStateFn<PromptPart[]>) => void;
  removeAttachmentAt: (index: number) => void;
  selectAttachment: (index: number | null) => void;
  setComposerDragActive: (active: SetStateFn<boolean>) => void;
  clearComposer: () => void;

  // === Streaming / Runtime ===
  // `streaming`/`statusLine`/`question`/`permission`/`latestRuntimeError`
  // are the view of the *selected* session; `runtimeViews` keeps the live
  // state of every session so switching (or an SSE reconnect) can restore
  // the correct view instead of leaking one session's runtime into another.
  streaming: boolean;
  statusLine: string;
  latestRuntimeError: string | null;
  question: QuestionInteractionRecord | null;
  permission: PermissionInteractionRecord | null;
  runtimeViews: Record<string, SessionRuntimeView>;
  questionAnswers: Record<number, QuestionAnswerValue>;
  questionSubmitting: boolean;
  permissionSubmitting: boolean;
  permissionSubmitError: string | null;
  permissionSubmitStartedAt: string | null;
  permissionSubmitCompletedAt: string | null;

  setSessionStreaming: (sessionId: string, value: SetStateFn<boolean>) => void;
  setSessionStatusLine: (sessionId: string, line: SetStateFn<string>) => void;
  setSessionLatestRuntimeError: (
    sessionId: string,
    error: SetStateFn<string | null>,
  ) => void;
  setSessionQuestion: (
    sessionId: string,
    q: SetStateFn<QuestionInteractionRecord | null>,
  ) => void;
  setSessionPermission: (
    sessionId: string,
    p: SetStateFn<PermissionInteractionRecord | null>,
  ) => void;
  /** Map a server `run_status` onto the session's streaming/status view. */
  applySessionRunStatus: (sessionId: string, rawStatus: string) => void;
  /** Re-derive the top-level runtime view after the selection changed. */
  syncRuntimeViewForSelection: (sessionId: string | null) => void;
  /** Drop every runtime entry for a session (used when deleting it). */
  clearSessionRuntimeState: (sessionId: string) => void;
  setQuestionAnswers: (answers: SetStateFn<Record<number, QuestionAnswerValue>>) => void;
  setQuestionSubmitting: (value: SetStateFn<boolean>) => void;
  setPermissionSubmitting: (value: SetStateFn<boolean>) => void;
  setPermissionSubmitError: (error: SetStateFn<string | null>) => void;
  setPermissionSubmitStartedAt: (ts: SetStateFn<string | null>) => void;
  setPermissionSubmitCompletedAt: (ts: SetStateFn<string | null>) => void;

  // === Transcript Feed ===
  messages: FeedMessage[];
  messageHistory: MessageRecord[];
  selectedMessageIds: Set<string>;
  historyLoading: boolean;

  setMessages: (msgs: SetStateFn<FeedMessage[]>) => void;
  setMessageHistory: (history: SetStateFn<MessageRecord[]>) => void;
  setSelectedMessageIds: (ids: SetStateFn<Set<string>>) => void;
  clearTranscriptFeed: () => void;
  toggleMessageSelected: (message: FeedMessage) => void;
  clearSelectedMessages: () => void;
  setHistoryLoading: (loading: SetStateFn<boolean>) => void;

  // === Runtime Navigation ===
  runtimeSurfaceBySession: Record<string, SessionRuntimeSurface>;
  activeStageContext: {
    stageId?: string | null;
    executionId?: string | null;
    toolCallId?: string | null;
    label?: string | null;
    sessionId?: string | null;
  } | null;
  previewStageId: string | null;
  sessionBreadcrumbs: SessionBreadcrumb[];

  // === Task Ledger ===
  taskLedgers: Record<string, SessionTaskLedger>;
  setTaskLedger: (
    sessionId: string,
    ledger: SessionTaskLedger | null | ((prev: SessionTaskLedger | null) => SessionTaskLedger | null),
  ) => void;
  clearTaskLedger: (sessionId: string) => void;

  setRuntimeSurfaceBySession: (value: SetStateFn<Record<string, SessionRuntimeSurface>>) => void;
  setActiveStageContext: (value: SetStateFn<AgendaoState["activeStageContext"]>) => void;
  setPreviewStageId: (value: SetStateFn<string | null>) => void;
  setSessionBreadcrumbs: (value: SetStateFn<SessionBreadcrumb[]>) => void;
  appendRuntimeSurfaceBlock: (
    sessionId: string,
    key: RuntimeSurfaceKey,
    block: AuxiliaryOutputBlock,
    limit: number,
  ) => void;
  setRuntimeSurfaceBanner: (sessionId: string, banner: string | null) => void;
  clearRuntimeSurfaceForMissingSessions: (sessionIds: string[]) => void;
  currentRuntimeSurfaceFor: (sessionId: string | null) => SessionRuntimeSurface;
  hasRuntimeSurfaceFor: (sessionId: string | null) => boolean;
  currentBreadcrumbProvenanceFor: (selectedSessionId: string | null) => BreadcrumbProvenance | null;

  // === Workspace ===
  fileTree: FileTreeNodeRecord | null;
  workspaceRootPath: string;
  workspaceLoading: boolean;
  workspaceNodeLoading: Record<string, boolean>;
  selectedFilePath: string | null;
  selectedFileContent: string;
  savedFileContent: string;
  fileLoading: boolean;
  fileSaving: boolean;
  fileDeleting: boolean;
  fileUploading: boolean;
  currentWorkspacePath: string | null;
  workspacePanelTab: WorkspacePanelTab;
  selectedWorkspacePath: string | null;
  selectedWorkspaceType: "file" | "directory";
  pendingWorkspaceSelection: { path: string; type: "file" | "directory" } | null;
  workspaceReloadToken: number;
  workspaceDirty: boolean;

  setFileTree: (tree: SetStateFn<FileTreeNodeRecord | null>) => void;
  setWorkspaceRootPath: (path: SetStateFn<string>) => void;
  setWorkspaceLoading: (loading: SetStateFn<boolean>) => void;
  setWorkspaceNodeLoading: (path: string, loading: boolean) => void;
  setSelectedFilePath: (path: SetStateFn<string | null>) => void;
  setSelectedFileContent: (content: SetStateFn<string>) => void;
  setSavedFileContent: (content: SetStateFn<string>) => void;
  setFileLoading: (loading: SetStateFn<boolean>) => void;
  setFileSaving: (saving: SetStateFn<boolean>) => void;
  setFileDeleting: (deleting: SetStateFn<boolean>) => void;
  setFileUploading: (uploading: SetStateFn<boolean>) => void;
  setCurrentWorkspacePath: (path: SetStateFn<string | null>) => void;
  setWorkspacePanelTab: (tab: SetStateFn<WorkspacePanelTab>) => void;
  setSelectedWorkspacePath: (path: SetStateFn<string | null>) => void;
  setSelectedWorkspaceType: (type: SetStateFn<"file" | "directory">) => void;
  setPendingWorkspaceSelection: (selection: SetStateFn<{ path: string; type: "file" | "directory" } | null>) => void;
  triggerWorkspaceReload: () => void;
  setWorkspaceDirty: (dirty: SetStateFn<boolean>) => void;
}
