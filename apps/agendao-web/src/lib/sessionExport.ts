import type { MessageRecord } from "./history";

// Client-side session export (the server has no export endpoint). The web UI
// fetches `GET /session/{id}/message` (MessageInfo[]) and serializes it here —
// mirrors the TUI transcript_to_text contract (user/assistant text only, tool
// details skipped), rendered as Markdown.
export interface SessionExportInput {
  title?: string | null;
  sessionId: string;
  exportedAt: Date;
  messages: MessageRecord[];
}

const TEXT_PART_TYPE = "text";
const REASONING_PART_TYPE = "reasoning";
const REASONING_MARKER = "> *(reasoning)*";

function visibleTexts(message: MessageRecord): { texts: string[]; hasReasoning: boolean } {
  const texts: string[] = [];
  let hasReasoning = false;
  for (const part of message.parts ?? []) {
    if (part.ignored) continue;
    if (part.type === TEXT_PART_TYPE) {
      const text = part.text ?? "";
      if (text.trim()) texts.push(text.trim());
    } else if (part.type === REASONING_PART_TYPE) {
      hasReasoning = true;
    }
    // Everything else (tool_call / tool_result / file / output_block ...) is
    // skipped on purpose: export keeps the conversation narrative only.
  }
  return { texts, hasReasoning };
}

export function buildSessionExportMarkdown({
  title,
  sessionId,
  exportedAt,
  messages,
}: SessionExportInput): string {
  const heading = title?.trim() || sessionId;
  const lines: string[] = [
    `# ${heading}`,
    "",
    `- Session: ${sessionId}`,
    `- Exported at: ${exportedAt.toISOString()}`,
    "",
  ];

  for (const message of messages) {
    const role = message.role?.trim() || "assistant";
    const { texts, hasReasoning } = visibleTexts(message);
    if (texts.length === 0 && !hasReasoning) continue;
    lines.push(`## ${role}`, "");
    if (hasReasoning) {
      lines.push(REASONING_MARKER, "");
    }
    for (const text of texts) {
      lines.push(text, "");
    }
  }

  return `${lines.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n`;
}

export function sessionExportFileName(
  title: string | null | undefined,
  sessionId: string,
): string {
  const base = (title?.trim() || sessionId)
    .replace(/[\\/:*?"<>|]+/g, "-")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^-+|-+$/g, "");
  return `${base || sessionId}.md`;
}
