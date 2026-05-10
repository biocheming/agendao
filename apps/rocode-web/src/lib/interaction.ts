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
  permission_class?: string;
  permission_class_label?: string;
  scope_key?: string;
  scope_label?: string;
  supported_lifetimes?: string[];
  grant_hint?: string;
  command?: string;
  filepath?: string;
  patterns?: string[];
}

function defaultSupportedLifetimes(permissionClass?: string): string[] {
  switch (permissionClass) {
    case "workspace_write":
    case "external_access":
      return ["once", "turn", "session"];
    case "inspect_read":
    case "dangerous_exec":
      return ["once"];
    default:
      return ["once"];
  }
}

function permissionGrantHint(scope?: string, supportedLifetimes: string[] = []): string | undefined {
  if (supportedLifetimes.length === 0) return undefined;
  const parts = ["once = this request"];
  if (supportedLifetimes.includes("turn")) {
    parts.push(scope ? `turn = current turn for ${scope}` : "turn = current turn");
  }
  if (supportedLifetimes.includes("session")) {
    parts.push(scope ? `session = this session for ${scope}` : "session = this session");
  }
  return parts.join(" | ");
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
  const metadata =
    typeof input?.metadata === "object" && input.metadata
      ? (input.metadata as Record<string, unknown>)
      : null;
  const patterns = Array.isArray(input?.patterns)
    ? input?.patterns.map((value) => String(value ?? "")).filter(Boolean)
    : undefined;
  const permission_class =
    typeof info.permission_class === "string" ? info.permission_class : undefined;
  const supported_lifetimes_source = Array.isArray(info.supported_lifetimes)
    ? info.supported_lifetimes
    : Array.isArray(input?.supported_lifetimes)
      ? input.supported_lifetimes
      : undefined;
  const supported_lifetimes =
    supported_lifetimes_source
      ?.map((value) => String(value ?? ""))
      .filter(Boolean) ?? defaultSupportedLifetimes(permission_class);

  return {
    permission_id: String(event.permissionID ?? ""),
    session_id: sessionId,
    message: typeof info.message === "string" ? info.message : undefined,
    permission: typeof info.tool === "string" ? info.tool : undefined,
    permission_class,
    permission_class_label: permissionClassLabel(permission_class),
    scope_key: typeof info.scope_key === "string" ? info.scope_key : undefined,
    scope_label: typeof info.scope_label === "string" ? info.scope_label : undefined,
    supported_lifetimes,
    grant_hint: permissionGrantHint(
      typeof info.scope_label === "string"
        ? info.scope_label
        : typeof info.scope_key === "string"
          ? info.scope_key
          : undefined,
      supported_lifetimes,
    ),
    command:
      typeof metadata?.command === "string"
        ? metadata.command
        : typeof input?.command === "string"
          ? input.command
          : undefined,
    filepath:
      typeof metadata?.filepath === "string"
        ? metadata.filepath
        : typeof metadata?.path === "string"
          ? metadata.path
          : patterns?.[0],
    patterns,
  };
}

function permissionClassLabel(value?: string): string | undefined {
  switch (value) {
    case "inspect_read":
      return "Inspect read";
    case "workspace_write":
      return "Workspace write";
    case "external_access":
      return "External access";
    case "dangerous_exec":
      return "Dangerous execution";
    default:
      return value ? value.replaceAll("_", " ") : undefined;
  }
}
