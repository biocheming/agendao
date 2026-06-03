const WAITING_LABELS: Record<string, string> = {
  agent: "agent task",
  dispatch: "tool dispatch",
  model: "model response",
  retry_backoff: "retry delay",
  tool: "tool result",
  user: "user input",
  user_approval: "user approval",
};

const EVENT_LABELS: Record<string, string> = {
  "prompt.scheduler.stage.abort.finalized": "Stage abort finalized",
  "prompt.scheduler.stage.abort.requested": "Stage abort requested",
  "prompt.scheduler.stage.step": "Stage step updated",
  "prompt.scheduler.stage.tool.end": "Tool call finished",
  "prompt.scheduler.stage.tool.start": "Tool call started",
  "scheduler.stage.started": "Stage started",
  "scheduler.stage.waiting": "Stage waiting",
  tool_call: "Tool call updated",
};

function humanizeTokenSequence(value: string) {
  return value
    .split(/[\s._-]+/)
    .filter(Boolean)
    .join(" ");
}

function sentenceCase(value: string) {
  if (!value) return value;
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
}

// P2-3 RETAINED HEURISTIC: Display-only label beautification.
// Maps machine-readable wait targets to human-readable labels.
// Only affects UI display text, never transcript routing or content semantics.
export function humanizeStageWaitTarget(value?: string | null) {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  const key = trimmed.toLowerCase();
  if (WAITING_LABELS[key]) return WAITING_LABELS[key];
  if (/^[a-z0-9._-]+$/.test(trimmed)) {
    return humanizeTokenSequence(trimmed);
  }
  return trimmed;
}

// P2-3 RETAINED HEURISTIC: Display-only label beautification.
// Maps machine-readable stage event names to human-readable labels.
// Only affects UI display text, never transcript routing or content semantics.
export function humanizeStageEvent(value?: string | null) {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  const key = trimmed.toLowerCase();
  if (EVENT_LABELS[key]) return EVENT_LABELS[key];
  if (/^[a-z0-9._-]+$/.test(trimmed)) {
    return sentenceCase(humanizeTokenSequence(trimmed));
  }
  return trimmed;
}
