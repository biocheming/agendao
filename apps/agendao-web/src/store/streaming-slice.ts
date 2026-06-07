import { resolveSetState, type AgendaoState, type StoreGet, type StoreSet } from "./types";

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
  | "questionAnswers"
  | "questionSubmitting"
  | "permissionSubmitting"
  | "permissionSubmitError"
  | "permissionSubmitStartedAt"
  | "permissionSubmitCompletedAt"
  | "setStreaming"
  | "setStatusLine"
  | "setLatestRuntimeError"
  | "setQuestion"
  | "setPermission"
  | "setQuestionAnswers"
  | "setQuestionSubmitting"
  | "setPermissionSubmitting"
  | "setPermissionSubmitError"
  | "setPermissionSubmitStartedAt"
  | "setPermissionSubmitCompletedAt"
> {
  return {
    streaming: false,
    statusLine: "ready",
    latestRuntimeError: null,
    question: null,
    permission: null,
    questionAnswers: {},
    questionSubmitting: false,
    permissionSubmitting: false,
    permissionSubmitError: null,
    permissionSubmitStartedAt: null,
    permissionSubmitCompletedAt: null,

    setStreaming: (streaming) => set({ streaming: resolveSetState(streaming, get().streaming) }),
    setStatusLine: (statusLine) => set({ statusLine: resolveSetState(statusLine, get().statusLine) }),
    setLatestRuntimeError: (latestRuntimeError) =>
      set({ latestRuntimeError: resolveSetState(latestRuntimeError, get().latestRuntimeError) }),
    setQuestion: (question) => set({ question: resolveSetState(question, get().question) }),
    setPermission: (permission) =>
      set({ permission: resolveSetState(permission, get().permission) }),
    setQuestionAnswers: (questionAnswers) =>
      set({ questionAnswers: resolveSetState(questionAnswers, get().questionAnswers) }),
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
