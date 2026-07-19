import { useCallback } from "react";
import {
  formatError,
  mergePendingCommandArguments,
  normalizedAnswerValues,
  pendingCommandFromSession,
} from "../lib/display";
import type { PromptResponseRecord } from "../lib/interaction";
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
  const setQuestion = useAgendaoStore((s) => s.setQuestion);
  const setQuestionAnswers = useAgendaoStore((s) => s.setQuestionAnswers);
  const setQuestionSubmitting = useAgendaoStore((s) => s.setQuestionSubmitting);
  const setPermission = useAgendaoStore((s) => s.setPermission);
  const setPermissionSubmitting = useAgendaoStore((s) => s.setPermissionSubmitting);
  const setPermissionSubmitError = useAgendaoStore((s) => s.setPermissionSubmitError);
  const setPermissionSubmitStartedAt = useAgendaoStore((s) => s.setPermissionSubmitStartedAt);
  const setPermissionSubmitCompletedAt = useAgendaoStore((s) => s.setPermissionSubmitCompletedAt);
  const setStreaming = useAgendaoStore((s) => s.setStreaming);
  const setStatusLine = useAgendaoStore((s) => s.setStatusLine);
  const setLatestRuntimeError = useAgendaoStore((s) => s.setLatestRuntimeError);
  const setBanner = useAgendaoStore((s) => s.setBanner);

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
  }, [
    api,
    apiJson,
    loadPendingQuestion,
    question,
    questionAnswers,
    selectedModel,
    selectedSessionId,
    sendPromptRequest,
    setBanner,
    setLatestRuntimeError,
    setQuestion,
    setQuestionAnswers,
    setQuestionSubmitting,
    setStatusLine,
    setStreaming,
  ]);

  const rejectQuestion = useCallback(async () => {
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
  }, [api, question, setBanner, setQuestion, setQuestionAnswers, setQuestionSubmitting]);

  const replyPermission = useCallback(
    async (reply: "once" | "turn" | "session" | "reject") => {
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
    },
    [
      api,
      permission,
      permissionSubmitting,
      setBanner,
      setPermission,
      setPermissionSubmitCompletedAt,
      setPermissionSubmitError,
      setPermissionSubmitStartedAt,
      setPermissionSubmitting,
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
