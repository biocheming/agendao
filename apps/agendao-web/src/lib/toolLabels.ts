const RAW_TOOL_LABELS: Record<string, string> = {
  read: "Read",
  write: "Write",
  edit: "Edit",
  multiedit: "MultiEdit",
  bash: "Bash",
  glob: "Glob",
  grep: "Grep",
  ls: "Ls",
  websearch: "WebSearch",
  webfetch: "WebFetch",
  task: "Task",
  task_flow: "TaskFlow",
  question: "Question",
  todo_read: "TodoRead",
  todo_write: "TodoWrite",
  apply_patch: "ApplyPatch",
  skill: "Skill",
  lsp: "LSP",
  batch: "Batch",
  codesearch: "CodeSearch",
  context_docs: "ContextDocs",
  github_research: "GitHubResearch",
  repo_history: "RepoHistory",
  media_inspect: "MediaInspect",
  browser_session: "BrowserSession",
  shell_session: "ShellSession",
  ast_grep_search: "AstGrepSearch",
  ast_grep_replace: "AstGrepReplace",
  plan_enter: "PlanEnter",
  plan_exit: "PlanExit",
};

function looksLikeToolIdentifier(value: string) {
  return /^[A-Za-z0-9._:-]+$/.test(value);
}

function humanizeToolIdentifier(value: string) {
  const direct = RAW_TOOL_LABELS[value.toLowerCase()];
  if (direct) return direct;

  let result = "";
  for (let index = 0; index < value.length; index += 1) {
    const ch = value[index]!;
    if (ch === "_" || ch === "-") continue;
    const prev = index > 0 ? value[index - 1] : "";
    if (index === 0 || prev === "_" || prev === "-") {
      result += ch.toUpperCase();
    } else {
      result += ch;
    }
  }
  return result || value;
}

// P2-3 RETAINED HEURISTIC: Display-only label beautification.
// isSkillToolName guesses whether a tool name refers to a skill operation
// based on hardcoded name patterns. It only affects UI labels, never
// transcript routing, text priority, or structured content decisions.
export function isSkillToolName(value?: string | null) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return false;
  return (
    normalized === "skill" ||
    normalized === "skillslist" ||
    normalized === "skillview" ||
    normalized === "skillscategories" ||
    normalized.startsWith("skill")
  );
}

// P2-3 RETAINED HEURISTIC: Display-only label beautification.
// humanizes machine tool identifiers into human-readable labels.
// Only affects UI display text, never transcript routing or content semantics.
export function toolActivityLabel(value?: string | null) {
  const trimmed = value?.trim();
  if (!trimmed) return "Tool";
  if (!looksLikeToolIdentifier(trimmed)) return trimmed;

  const display = humanizeToolIdentifier(trimmed);
  if (isSkillToolName(trimmed)) {
    return display === "Skill" ? "Skill" : `Skill ${display}`;
  }
  return display;
}

export function toolKindLabel(kind: "tool" | "skill") {
  return kind === "skill" ? "Skill" : "Tool";
}
