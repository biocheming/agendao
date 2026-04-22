export interface QuestionOption {
  label: string;
  description?: string;
}

export interface QuestionItem {
  question: string;
  header?: string;
  multiple?: boolean;
  options?: QuestionOption[];
}

export interface QuestionInteractionRecord {
  request_id: string;
  session_id?: string;
  questions: QuestionItem[];
}

export interface PermissionInteractionRecord {
  permission_id: string;
  session_id?: string;
  message?: string;
  permission?: string;
  command?: string;
  filepath?: string;
  patterns?: string[];
}

export interface PromptResponseRecord {
  status: string;
  ok?: boolean;
  session_id?: string;
  pending_question_id?: string;
  command?: string;
  missing_fields?: string[];
}

export interface QuestionInfoResponseRecord {
  id: string;
  session_id?: string;
  sessionId?: string;
  questions?: string[];
  options?: string[][];
  items?: Array<{
    question: string;
    header?: string;
    multiple?: boolean;
    options?: Array<{
      label: string;
      description?: string;
    }>;
  }>;
}

export type QuestionAnswerValue = string | string[];

export function normalizeQuestionItems(input: unknown): QuestionItem[] {
  if (!Array.isArray(input)) return [];
  return input
    .map((candidate) => {
      const item = (candidate ?? {}) as Record<string, unknown>;
      const options = Array.isArray(item.options)
        ? item.options
            .map((option) => {
              if (typeof option === "string") {
                return { label: option };
              }
              if (!option || typeof option !== "object") return null;
              const record = option as Record<string, unknown>;
              const label =
                typeof record.label === "string"
                  ? record.label
                  : typeof record.value === "string"
                    ? record.value
                    : "";
              if (!label) return null;
              return {
                label,
                description:
                  typeof record.description === "string" ? record.description : undefined,
              };
            })
            .filter((option): option is QuestionOption => Boolean(option))
        : undefined;
      const question = typeof item.question === "string" ? item.question : "";
      if (!question) return null;
      const out: QuestionItem = { question };
      if (typeof item.header === "string") out.header = item.header;
      if (item.multiple) out.multiple = true;
      if (options) out.options = options;
      return out;
    })
    .filter((item): item is QuestionItem => item !== null);
}

function questionSessionId(
  info: Pick<QuestionInfoResponseRecord, "session_id" | "sessionId">,
): string | undefined {
  return typeof info.session_id === "string"
    ? info.session_id
    : typeof info.sessionId === "string"
      ? info.sessionId
      : undefined;
}

export function questionInteractionFromInfo(
  info: QuestionInfoResponseRecord,
): QuestionInteractionRecord {
  const items = normalizeQuestionItems(info.items);
  if (items.length > 0) {
    return {
      request_id: info.id,
      session_id: questionSessionId(info),
      questions: items,
    };
  }
  const questions = Array.isArray(info.questions) ? info.questions : [];
  const options = Array.isArray(info.options) ? info.options : [];
  return {
    request_id: info.id,
    session_id: questionSessionId(info),
    questions: questions.map((question, index) => ({
      question,
      multiple: false,
      options: Array.isArray(options[index])
        ? options[index]
            .map((label) => (typeof label === "string" && label ? { label } : null))
            .filter((option): option is QuestionOption => Boolean(option))
        : undefined,
    })),
  };
}

export function questionInteractionFromEvent(
  event: Record<string, unknown>,
  sessionId?: string,
): QuestionInteractionRecord {
  return questionInteractionFromInfo({
    id: String(event.requestID ?? ""),
    session_id: sessionId,
    items:
      Array.isArray(event.questions) &&
      event.questions.some((candidate) => candidate && typeof candidate === "object")
        ? (event.questions as QuestionInfoResponseRecord["items"])
        : [],
    questions:
      Array.isArray(event.questions) &&
      event.questions.every((candidate) => typeof candidate === "string")
        ? (event.questions as string[])
        : undefined,
  });
}

export function permissionInteractionFromEvent(
  event: Record<string, unknown>,
  sessionId?: string,
): PermissionInteractionRecord {
  const info = (event.info ?? {}) as Record<string, unknown>;
  const input =
    typeof info.input === "object" && info.input ? (info.input as Record<string, unknown>) : null;
  const patterns = Array.isArray(input?.patterns)
    ? input?.patterns.map((value) => String(value ?? "")).filter(Boolean)
    : undefined;

  return {
    permission_id: String(event.permissionID ?? ""),
    session_id: sessionId,
    message: typeof info.message === "string" ? info.message : undefined,
    permission: typeof info.tool === "string" ? info.tool : undefined,
    command: typeof input?.command === "string" ? input.command : undefined,
    filepath: patterns?.[0],
    patterns,
  };
}
