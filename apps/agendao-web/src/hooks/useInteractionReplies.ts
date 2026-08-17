import { useCallback } from "react";
import {
  formatError,
  mergePendingCommandArguments,
  normalizedAnswerValues,
  pendingCommandFromSession,
} from "../lib/display";
import type { PermissionReplyChoice, PromptResponseRecord } from "../lib/interaction";
import type { SessionRecord } from "../lib/session";
import { useAgendaoStore } from "../store";

interface UseInteractionRepliesOptions {
  api: (path: string, options?: RequestInit) => Promise<Response>;
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  loadPendingQuestion: (requestId: string, sessionId?: string | null) => Promise<void>;
  sendPromptRequest: (
    sessionId: string,
    payload: Record<string, unknown>,
  ) => Promise<PromptResponseRecord>;
}

export function useInteractionReplies({
  api,
  apiJson,
  loadPendingQuestion,
  sendPromptRequest,
}: UseInteractionRepliesOptions) {
  const question = useAgendaoStore((s) => s.question);
  const questionAnswers = useAgendaoStore((s) => s.questionAnswers);
  const permission = useAgendaoStore((s) => s.permission);
  const permissionSubmitting = useAgendaoStore((s) => s.permissionSubmitting);
  const permissionSubmitError = useAgendaoStore((s) => s.permissionSubmitError);
  const permissionSubmitCompletedAt = useAgendaoStore((s) => s.permissionSubmitCompletedAt);
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const selectedModel = useAgendaoStore((s) => s.selectedModel);
  const applySessionRunStatus = useAgendaoStore((s) => s.applySessionRunStatus);
  const setSessionQuestion = useAgendaoStore((s) => s.setSessionQuestion);
  const setQuestionAnswers = useAgendaoStore((s) => s.setQuestionAnswers);
  const setQuestionSubmitting = useAgendaoStore((s) => s.setQuestionSubmitting);
  const setSessionPermission = useAgendaoStore((s) => s.setSessionPermission);
  const setPermissionSubmitting = useAgendaoStore((s) => s.setPermissionSubmitting);
  const setPermissionSubmitError = useAgendaoStore((s) => s.setPermissionSubmitError);
  const setPermissionSubmitStartedAt = useAgendaoStore((s) => s.setPermissionSubmitStartedAt);
  const setPermissionSubmitCompletedAt = useAgendaoStore((s) => s.setPermissionSubmitCompletedAt);
  const setSessionLatestRuntimeError = useAgendaoStore((s) => s.setSessionLatestRuntimeError);
  const setBanner = useAgendaoStore((s) => s.setBanner);

  // The overlay can show an interaction owned by a session other than the
  // selected one; clearing the owner's view must also close the visible
  // overlay when it is the one being displayed.
  const clearDisplayedQuestionIf = useCallback((requestId: string) => {
    const displayed = useAgendaoStore.getState().question;
    if (displayed?.request_id === requestId) {
      useAgendaoStore.setState({ question: null });
    }
  }, []);

  const clearDisplayedPermissionIf = useCallback((permissionId: string) => {
    const displayed = useAgendaoStore.getState().permission;
    if (displayed?.permission_id === permissionId) {
      useAgendaoStore.setState({ permission: null });
    }
  }, []);

  const submitQuestion = useCallback(async () => {
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
      const questionSessionId = question.session_id ?? selectedSessionId;
      if (questionSessionId) {
        setSessionQuestion(questionSessionId, null);
      }
      clearDisplayedQuestionIf(question.request_id);
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
            applySessionRunStatus(sessionId, "waiting_on_user");
            if (response.pending_question_id) {
              await loadPendingQuestion(response.pending_question_id, sessionId);
            }
          } else {
            applySessionRunStatus(sessionId, "running");
            setSessionLatestRuntimeError(sessionId, null);
          }
        }
      }
    } catch (error) {
      setBanner(`Question reply failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  }, [
    api,
    apiJson,
    applySessionRunStatus,
    loadPendingQuestion,
    question,
    questionAnswers,
    selectedModel,
    selectedSessionId,
    sendPromptRequest,
    setBanner,
    setQuestionAnswers,
    setQuestionSubmitting,
    setSessionLatestRuntimeError,
    setSessionQuestion,

    clearDisplayedQuestionIf,
  ]);

  const rejectQuestion = useCallback(async () => {
    if (!question) return;
    setQuestionSubmitting(true);
    try {
      await api(`/question/${question.request_id}/reject`, { method: "POST" });
      const questionSessionId = question.session_id ?? selectedSessionId;
      if (questionSessionId) {
        setSessionQuestion(questionSessionId, null);
      }
      clearDisplayedQuestionIf(question.request_id);
      setQuestionAnswers({});
    } catch (error) {
      setBanner(`Question reject failed: ${formatError(error)}`);
    } finally {
      setQuestionSubmitting(false);
    }
  }, [
    api,
    question,
    selectedSessionId,
    setBanner,
    setQuestionAnswers,
    setQuestionSubmitting,
    setSessionQuestion,

    clearDisplayedQuestionIf,
  ]);

  const replyPermission = useCallback(
    async (reply: PermissionReplyChoice) => {
      const currentPermission = permission;
      if (!currentPermission || permissionSubmitting) return;
      const permissionSessionId = currentPermission.session_id ?? selectedSessionId;
      setPermissionSubmitting(true);
      setPermissionSubmitError(null);
      setPermissionSubmitStartedAt(new Date().toISOString());
      try {
        const sessionMode =
          reply === "trust_workspace"
            ? "trusted_workspace"
            : reply === "full_access"
              ? "unsandboxed_yolo"
              : null;
        if (sessionMode) {
          if (!permissionSessionId) {
            throw new Error("Cannot change permission mode without an active session");
          }
          await api(`/session/${encodeURIComponent(permissionSessionId)}/permission`, {
            method: "PATCH",
            body: JSON.stringify({ mode: sessionMode }),
          });
        }
        await api(`/permission/${currentPermission.permission_id}/reply`, {
          method: "POST",
          body: JSON.stringify({ reply: sessionMode ? "once" : reply }),
        });
        if (permissionSessionId) {
          setSessionPermission(permissionSessionId, null);
        }
        clearDisplayedPermissionIf(currentPermission.permission_id);
        setPermissionSubmitting(false);
        setPermissionSubmitCompletedAt(new Date().toISOString());
      } catch (error) {
        const message = formatError(error);
        setBanner(`Permission reply failed: ${message}`);
        setPermissionSubmitError(message);
        setPermissionSubmitting(false);
        setPermissionSubmitCompletedAt(new Date().toISOString());
      }
    },
    [
      api,
      permission,
      permissionSubmitting,
      selectedSessionId,
      setBanner,
      setPermissionSubmitCompletedAt,
      setPermissionSubmitError,
      setPermissionSubmitStartedAt,
      setPermissionSubmitting,
      setSessionPermission,

      clearDisplayedPermissionIf,
    ],
  );

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

  return {
    permissionStatusLabel,
    permissionStatusTone,
    rejectQuestion,
    replyPermission,
    submitQuestion,
  };
}
