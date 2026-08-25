import { useCallback, type FormEvent } from "react";
import {
  parseSlashCommandSubmission,
  type ExecuteCommandResponseRecord,
  type SlashCommandSubmission,
} from "../lib/command";
import { formatError, promptPreviewText } from "../lib/display";
import type { FeedMessage } from "../lib/history";
import type { PromptResponseRecord } from "../lib/interaction";
import { schedulerChoiceFromId } from "../lib/webRuntime";
import {
  applyOutputBlock,
  createOptimisticUserFeedMessage,
} from "../lib/liveTranscriptState";
import { useAgendaoStore } from "../store";
import type { SetStateFn } from "../store/types";

interface UsePromptSubmissionOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  clearComposerNotice: () => void;
  createSession: () => Promise<string>;
  loadPendingQuestion: (requestId: string, sessionId?: string | null) => Promise<void>;
  optimisticMessagesRef: { current: Record<string, FeedMessage[]> };
  preflightBeforeSubmit: () => Promise<{ blocked: boolean; banner: string | null }>;
  refreshSessions: () => Promise<unknown>;
  setMessages: (messages: SetStateFn<FeedMessage[]>) => void;
}

export function usePromptSubmission({
  apiJson,
  clearComposerNotice,
  createSession,
  loadPendingQuestion,
  optimisticMessagesRef,
  preflightBeforeSubmit,
  refreshSessions,
  setMessages,
}: UsePromptSubmissionOptions) {
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const composer = useAgendaoStore((s) => s.composer);
  const attachments = useAgendaoStore((s) => s.attachments);
  const slashCommands = useAgendaoStore((s) => s.slashCommands);
  const selectedModel = useAgendaoStore((s) => s.selectedModel);
  const selectedReasoningEffort = useAgendaoStore((s) => s.selectedReasoningEffort);
  const selectedMode = useAgendaoStore((s) => s.selectedMode);
  const streaming = useAgendaoStore((s) => s.streaming);
  const showThinking = useAgendaoStore((s) => s.showThinking);
  const setComposer = useAgendaoStore((s) => s.setComposer);
  const setAttachments = useAgendaoStore((s) => s.setAttachments);
  const applySessionRunStatus = useAgendaoStore((s) => s.applySessionRunStatus);
  const setSessionStreaming = useAgendaoStore((s) => s.setSessionStreaming);
  const setSessionStatusLine = useAgendaoStore((s) => s.setSessionStatusLine);
  const setSessionLatestRuntimeError = useAgendaoStore((s) => s.setSessionLatestRuntimeError);
  const setBanner = useAgendaoStore((s) => s.setBanner);

  const sendPromptRequest = useCallback(
    async (
      sessionId: string,
      payload: Record<string, unknown>,
    ): Promise<PromptResponseRecord> =>
      apiJson<PromptResponseRecord>(`/session/${sessionId}/prompt`, {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    [apiJson],
  );

  const sendCommandRequest = useCallback(
    async (
      sessionId: string,
      payload: Record<string, unknown>,
    ): Promise<ExecuteCommandResponseRecord> =>
      apiJson<ExecuteCommandResponseRecord>(`/session/${sessionId}/command`, {
        method: "POST",
        body: JSON.stringify(payload),
      }),
    [apiJson],
  );

  // Slash command path: POST /session/{id}/command. The server persists the
  // "/name args" user message itself; the streaming reset (true→false edge)
  // re-triggers a history reconcile so the persisted messages (and the
  // "Command queued" ack) show up.
  const submitSlashCommand = useCallback(
    async (submission: SlashCommandSubmission) => {
      let sessionId = selectedSessionId;
      if (!sessionId) {
        try {
          sessionId = await createSession();
        } catch (error) {
          setBanner(`Failed to create session: ${formatError(error)}`);
          return;
        }
      }

      const optimisticMessage = createOptimisticUserFeedMessage(submission.text);
      optimisticMessagesRef.current = {
        ...optimisticMessagesRef.current,
        [sessionId]: [
          ...(optimisticMessagesRef.current[sessionId] ?? []),
          optimisticMessage,
        ],
      };
      setMessages((current) => [...current, optimisticMessage]);
      setComposer("");
      clearComposerNotice();
      setSessionStreaming(sessionId, true);
      setSessionStatusLine(sessionId, "running");
      setSessionLatestRuntimeError(sessionId, null);

      try {
        const payload: Record<string, unknown> = { command: submission.command };
        if (submission.args) payload.arguments = submission.args;
        if (selectedModel) payload.model = selectedModel;
        payload.reasoning_effort = selectedReasoningEffort;
        if (selectedMode) {
          const [kind, id] = selectedMode.split(":", 2);
          if (kind === "agent") payload.agent = id;
          if (kind === "scheduler") payload.scheduler = schedulerChoiceFromId(id);
        }
        const response = await sendCommandRequest(sessionId, payload);
        if (response.status === "awaiting_user") {
          applySessionRunStatus(sessionId, "waiting_on_user");
          if (response.pending_question_id) {
            await loadPendingQuestion(response.pending_question_id, sessionId);
          }
        } else if (response.status === "queued") {
          applySessionRunStatus(sessionId, "running");
        } else {
          setSessionStreaming(sessionId, false);
          setSessionStatusLine(sessionId, "ready");
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
        setBanner(`Command failed: ${formatError(error)}`);
        setSessionStreaming(sessionId, false);
        setSessionStatusLine(sessionId, "error");
        setSessionLatestRuntimeError(sessionId, formatError(error));
      }

      try {
        await refreshSessions();
      } catch {
        // best effort
      }
    },
    [
      applySessionRunStatus,
      clearComposerNotice,
      createSession,
      loadPendingQuestion,
      optimisticMessagesRef,
      refreshSessions,
      selectedMode,
      selectedModel,
      selectedReasoningEffort,
      selectedSessionId,
      sendCommandRequest,
      setBanner,
      setComposer,
      setSessionLatestRuntimeError,
      setSessionStatusLine,
      setSessionStreaming,
      setMessages,
      showThinking,
    ],
  );

  const submitPrompt = useCallback(
    async (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const content = composer.trim();
      const promptParts = attachments;
      if ((!content && promptParts.length === 0) || streaming) return;

      setBanner(null);

      // Slash commands only route when the composer carries no attachments —
      // the command endpoint cannot carry parts, so attachment submissions keep
      // the normal prompt path untouched.
      const slashSubmission =
        promptParts.length === 0 ? parseSlashCommandSubmission(content, slashCommands) : null;
      if (slashSubmission) {
        await submitSlashCommand(slashSubmission);
        return;
      }

      try {
        const multimodalGate = await preflightBeforeSubmit();
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
      clearComposerNotice();
      setSessionStreaming(sessionId, true);
      setSessionStatusLine(sessionId, "running");
      setSessionLatestRuntimeError(sessionId, null);

      try {
        const payload: Record<string, unknown> = {
          message: content || undefined,
          idempotency_key: ingressIdempotencyKey,
          ingress_source: "web",
        };
        if (selectedModel) payload.model = selectedModel;
        payload.reasoning_effort = selectedReasoningEffort;
        if (promptParts.length > 0) payload.parts = promptParts;
        if (selectedMode) {
          const [kind, id] = selectedMode.split(":", 2);
          if (kind === "agent") payload.agent = id;
          if (kind === "scheduler") payload.scheduler = schedulerChoiceFromId(id);
        }

        const response = await sendPromptRequest(sessionId, payload);
        if (response.status === "awaiting_user") {
          applySessionRunStatus(sessionId, "waiting_on_user");
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
        setSessionStreaming(sessionId, false);
        setSessionStatusLine(sessionId, "error");
        setSessionLatestRuntimeError(sessionId, formatError(error));
      }

      try {
        await refreshSessions();
      } catch {
        // best effort
      }
    },
    [
      applySessionRunStatus,
      attachments,
      clearComposerNotice,
      composer,
      createSession,
      loadPendingQuestion,
      optimisticMessagesRef,
      preflightBeforeSubmit,
      refreshSessions,
      selectedMode,
      selectedModel,
      selectedReasoningEffort,
      selectedSessionId,
      sendPromptRequest,
      setAttachments,
      setBanner,
      setComposer,
      setSessionLatestRuntimeError,
      setSessionStatusLine,
      setSessionStreaming,
      setMessages,
      showThinking,
      slashCommands,
      streaming,
      submitSlashCommand,
    ],
  );

  const stopActivePrompt = useCallback(async () => {
    if (!selectedSessionId || !streaming) return;

    setBanner(null);
    setSessionStatusLine(selectedSessionId, "cancelling");

    try {
      const response = await apiJson<{ dropped_queued_prompts?: number } | null>(
        `/session/${selectedSessionId}/abort`,
        { method: "POST" },
      );
      const dropped = Number(response?.dropped_queued_prompts ?? 0);
      if (dropped > 0) {
        setBanner(
          `Stopped run — dropped ${dropped} queued prompt${dropped === 1 ? "" : "s"}`,
        );
      }
    } catch (error) {
      setBanner(`Failed to stop session: ${formatError(error)}`);
      setSessionStatusLine(selectedSessionId, "running");
    }
  }, [
    apiJson,
    selectedSessionId,
    setBanner,
    setSessionStatusLine,
    streaming,
  ]);

  return {
    sendPromptRequest,
    stopActivePrompt,
    submitPrompt,
  };
}
