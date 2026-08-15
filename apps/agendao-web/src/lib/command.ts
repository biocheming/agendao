// Slash command contract + helpers for the composer.
// Types mirror the server-side CommandApiSpec served by GET /command
// (crates/agendao-server/src/routes/mod.rs) and the ExecuteCommandRequest /
// response pair of POST /session/{id}/command.

export type CommandArgumentKind =
  | "text"
  | "long_text"
  | "glob_list"
  | "command_line"
  | "integer"
  | "boolean"
  | "enum";

export interface CommandArgumentOptionRecord {
  label: string;
  description?: string;
}

export interface CommandArgumentFieldRecord {
  key: string;
  label: string;
  required: boolean;
  kind: CommandArgumentKind;
  repeatable: boolean;
  options?: CommandArgumentOptionRecord[];
}

export type CommandExecutionMode = "scheduler" | "agent";

export interface CommandInvocationRecord {
  mode: CommandExecutionMode;
  allow_inline_arguments: boolean;
  argument_schema?: CommandArgumentFieldRecord[];
}

export type CommandInteractivePolicy = "none" | "ask_batch_once" | "ask_per_step";

export interface CommandQuestionTemplateRecord {
  id: string;
  header: string;
  field_key: string;
  prompt: string;
  input_kind: CommandArgumentKind;
  options?: CommandArgumentOptionRecord[];
}

export interface CommandInteractiveRecord {
  when_missing_required: CommandInteractivePolicy;
  questions?: CommandQuestionTemplateRecord[];
}

/** CommandSource serializes with external tagging on the server enum. */
export type CommandSourceRecord =
  | "Builtin"
  | { File: string }
  | { Mcp: { server: string; prompt: string } }
  | { Skill: { name: string } };

export interface CommandApiSpec {
  name: string;
  description: string;
  aliases?: string[];
  invocation?: CommandInvocationRecord;
  interactive?: CommandInteractiveRecord;
  source: CommandSourceRecord;
}

export interface ExecuteCommandResponseRecord {
  executed: boolean;
  command: string;
  arguments?: string | null;
  model?: string | null;
  agent?: string | null;
  message_id?: string;
}

/**
 * Returns the in-progress slash query while the composer holds a single slash
 * token ("/" or "/rev"), or null once the token is complete (a whitespace was
 * typed) or the text is not slash-prefixed. Drives popup visibility.
 */
export function slashCommandQueryFromText(text: string): string | null {
  const match = /^\/(\S*)$/.exec(text);
  return match ? match[1] : null;
}

/**
 * Filters commands by name/alias (case-insensitive) and ranks matches:
 * exact name > exact alias > name prefix > alias prefix > name substring >
 * alias substring, ties broken alphabetically. Empty query returns all.
 */
export function filterSlashCommands(
  commands: CommandApiSpec[],
  query: string,
): CommandApiSpec[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [...commands];

  const scored: Array<{ command: CommandApiSpec; score: number }> = [];
  for (const command of commands) {
    const name = command.name.toLowerCase();
    const aliases = (command.aliases ?? []).map((alias) => alias.toLowerCase());
    let score = -1;
    if (name === needle) score = 0;
    else if (aliases.includes(needle)) score = 1;
    else if (name.startsWith(needle)) score = 2;
    else if (aliases.some((alias) => alias.startsWith(needle))) score = 3;
    else if (name.includes(needle)) score = 4;
    else if (aliases.some((alias) => alias.includes(needle))) score = 5;
    if (score >= 0) scored.push({ command, score });
  }

  return scored
    .sort(
      (left, right) =>
        left.score - right.score || left.command.name.localeCompare(right.command.name),
    )
    .map((entry) => entry.command);
}

export interface SlashCommandSubmission {
  /** Canonical command name (alias-resolved) sent as `command`. */
  command: string;
  /** Inline arguments (trimmed); undefined when the submission carries none. */
  args?: string;
  /**
   * Canonical "/name args" text, matching what the server persists as the
   * user message — used for the optimistic transcript entry.
   */
  text: string;
  spec: CommandApiSpec;
}

/**
 * Routes a composer submission: returns a command submission when the text is
 * a slash invocation of a known command (name or alias), otherwise null so
 * the caller falls back to the normal prompt path.
 */
export function parseSlashCommandSubmission(
  text: string,
  commands: CommandApiSpec[],
): SlashCommandSubmission | null {
  const match = /^\/(\S+)(?:\s+(.*))?$/.exec(text.trim());
  if (!match) return null;

  const token = match[1];
  const spec = commands.find(
    (command) => command.name === token || (command.aliases ?? []).includes(token),
  );
  if (!spec) return null;

  const args = match[2]?.trim() || undefined;
  return {
    command: spec.name,
    args,
    text: args ? `/${spec.name} ${args}` : `/${spec.name}`,
    spec,
  };
}
