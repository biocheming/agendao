import {
  DEFAULT_SESSION_RUNTIME_VIEW,
  resolveSetState,
  type AgendaoState,
  type SessionRuntimeView,
  type StoreGet,
  type StoreSet,
} from "./types";

export function createStreamingSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "streaming"
  | "statusLine"
  | "latestRuntimeError"
  | "question"
  | "permission"
  | "runtimeViews"
  | "questionAnswers"
  | "questionSubmitting"
  | "permissionSubmitting"
  | "permissionSubmitError"
  | "permissionSubmitStartedAt"
  | "permissionSubmitCompletedAt"
  | "setSessionStreaming"
  | "setSessionStatusLine"
  | "setSessionLatestRuntimeError"
  | "setSessionQuestion"
  | "setSessionPermission"
  | "applySessionRunStatus"
  | "syncRuntimeViewForSelection"
  | "clearSessionRuntimeState"
  | "setQuestionAnswers"
  | "setQuestionSubmitting"
  | "setPermissionSubmitting"
  | "setPermissionSubmitError"
  | "setPermissionSubmitStartedAt"
  | "setPermissionSubmitCompletedAt"
> {
  // Writes the session's view and, when that session is selected, mirrors
  // the same patch into the top-level fields the UI reads.
  const updateSessionView = (
    sessionId: string,
    patch: (view: SessionRuntimeView) => Partial<SessionRuntimeView>,
  ) => {
    const current = get().runtimeViews[sessionId] ?? DEFAULT_SESSION_RUNTIME_VIEW;
    const patchValue = patch(current);
    set({
      runtimeViews: {
        ...get().runtimeViews,
        [sessionId]: { ...current, ...patchValue },
      },
    });
    if (get().selectedSessionId === sessionId) {
      set(patchValue as Partial<AgendaoState>);
    }
  };

  return {
    streaming: false,
    statusLine: "ready",
    latestRuntimeError: null,
    question: null,
    permission: null,
    runtimeViews: {},
    questionAnswers: {},
    questionSubmitting: false,
    permissionSubmitting: false,
    permissionSubmitError: null,
    permissionSubmitStartedAt: null,
    permissionSubmitCompletedAt: null,

    setSessionStreaming: (sessionId, streaming) =>
      updateSessionView(sessionId, (view) => ({
        streaming: resolveSetState(streaming, view.streaming),
      })),
    setSessionStatusLine: (sessionId, statusLine) =>
      updateSessionView(sessionId, (view) => ({
        statusLine: resolveSetState(statusLine, view.statusLine),
      })),
    setSessionLatestRuntimeError: (sessionId, latestRuntimeError) =>
      updateSessionView(sessionId, (view) => ({
        latestRuntimeError: resolveSetState(latestRuntimeError, view.latestRuntimeError),
      })),
    setSessionQuestion: (sessionId, question) =>
      updateSessionView(sessionId, (view) => ({
        question: resolveSetState(question, view.question),
      })),
    setSessionPermission: (sessionId, permission) =>
      updateSessionView(sessionId, (view) => ({
        permission: resolveSetState(permission, view.permission),
      })),

    applySessionRunStatus: (sessionId, rawStatus) => {
      if (rawStatus === "idle") {
        updateSessionView(sessionId, () => ({
          streaming: false,
          statusLine: "idle",
          latestRuntimeError: null,
        }));
      } else if (rawStatus === "waiting_on_user") {
        updateSessionView(sessionId, () => ({
          streaming: false,
          statusLine: "awaiting_user",
          latestRuntimeError: null,
        }));
      } else if (rawStatus === "running" || rawStatus === "waiting_on_tool") {
        updateSessionView(sessionId, () => ({
          streaming: true,
          statusLine: "running",
          latestRuntimeError: null,
        }));
      } else if (rawStatus === "compacting") {
        updateSessionView(sessionId, () => ({
          streaming: true,
          statusLine: "compacting",
          latestRuntimeError: null,
        }));
      } else if (rawStatus === "cancelling") {
        updateSessionView(sessionId, () => ({
          streaming: true,
          statusLine: "cancelling",
          latestRuntimeError: null,
        }));
      } else if (rawStatus === "blocked" || rawStatus === "sleeping") {
        updateSessionView(sessionId, () => ({
          streaming: false,
          statusLine: rawStatus,
          latestRuntimeError: null,
        }));
      }
    },

    syncRuntimeViewForSelection: (sessionId) => {
      if (!sessionId) {
        set({
          streaming: DEFAULT_SESSION_RUNTIME_VIEW.streaming,
          statusLine: DEFAULT_SESSION_RUNTIME_VIEW.statusLine,
          latestRuntimeError: null,
          question: null,
          permission: null,
          questionAnswers: {},
        });
        return;
      }
      const view = get().runtimeViews[sessionId] ?? DEFAULT_SESSION_RUNTIME_VIEW;
      set({
        streaming: view.streaming,
        statusLine: view.statusLine,
        latestRuntimeError: view.latestRuntimeError,
        question: view.question,
        permission: view.permission,
        // A question restored from another session must not keep stale
        // answers typed for the previously shown one.
        ...(view.question ? { questionAnswers: {} } : {}),
      });
    },

    clearSessionRuntimeState: (sessionId) => {
      const { [sessionId]: _removed, ...remaining } = get().runtimeViews;
      set({ runtimeViews: remaining });
      if (get().selectedSessionId === sessionId) {
        set({
          streaming: false,
          statusLine: "ready",
          latestRuntimeError: null,
          question: null,
          permission: null,
          questionAnswers: {},
        });
      }
    },

    setQuestionAnswers: (questionAnswers) =>
      set({
        questionAnswers: resolveSetState(questionAnswers, get().questionAnswers),
      }),
    setQuestionSubmitting: (questionSubmitting) =>
      set({
        questionSubmitting: resolveSetState(questionSubmitting, get().questionSubmitting),
      }),
    setPermissionSubmitting: (permissionSubmitting) =>
      set({
        permissionSubmitting: resolveSetState(
          permissionSubmitting,
          get().permissionSubmitting,
        ),
      }),
    setPermissionSubmitError: (permissionSubmitError) =>
      set({
        permissionSubmitError: resolveSetState(
          permissionSubmitError,
          get().permissionSubmitError,
        ),
      }),
    setPermissionSubmitStartedAt: (permissionSubmitStartedAt) =>
      set({
        permissionSubmitStartedAt: resolveSetState(
          permissionSubmitStartedAt,
          get().permissionSubmitStartedAt,
        ),
      }),
    setPermissionSubmitCompletedAt: (permissionSubmitCompletedAt) =>
      set({
        permissionSubmitCompletedAt: resolveSetState(
          permissionSubmitCompletedAt,
          get().permissionSubmitCompletedAt,
        ),
      }),
  };
}
