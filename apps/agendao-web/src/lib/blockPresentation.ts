// P2-3: Compatibility presentation helpers.
//
// These functions handle the case where the model/provider leaks structured
// JSON envelopes (choices, candidates, usage blocks) into visible message
// text.  They are compatibility fallbacks — the server-side
// strip_trailing_provider_response_json already performs the same cleanup,
// but Web must handle older payloads that pre-date server-side stripping.
//
// BOUNDARY: Only assistant messages may trigger tail-envelope stripping.
// Reasoning, tool, status, scheduler_stage, and all other block kinds
// MUST NOT pass through this cleaning chain — their text is structured
// output that should never be silently truncated.

import { emitObservationEvent } from "./observationEvents";

const ENVELOPE_GUESS_KEYS = [
  "kind",
  "phase",
  "role",
  "text",
  "parts",
  "metadata",
  "output_block",
  "display",
  "structured",
  "summary",
  "fields",
  "tool_call_id",
  "stage_id",
  "choices",
  "usage",
  "object",
  "created",
  "model",
  "candidates",
  "output",
];

function isCompatibilityStructuredEnvelope(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value as Record<string, unknown>);
  if (keys.length === 0) return false;
  return keys.some((key) => ENVELOPE_GUESS_KEYS.includes(key));
}

function stripTrailingCompatibilityEnvelope(text: string): string {
  const trimmed = text.trimEnd();
  const candidateStarts = [
    trimmed.lastIndexOf("\n\n{"),
    trimmed.lastIndexOf("\n{"),
    trimmed.lastIndexOf("\n\n["),
  ];

  for (const startIndex of candidateStarts) {
    if (startIndex < 0) continue;
    const candidate = trimmed.slice(startIndex).trimStart();
    if (!(candidate.startsWith("{") || candidate.startsWith("["))) continue;
    try {
      const parsed = JSON.parse(candidate);
      if (!isCompatibilityStructuredEnvelope(parsed)) continue;
      const prefix = trimmed.slice(0, startIndex).trimEnd();
      if (!prefix) continue;
      emitObservationEvent(() => ({ ts: Date.now(), kind: "legacy_fallback_used", blockKind: "message", phase: undefined, blockId: undefined, route: undefined, legacyPath: "envelope", historyMessageCount: undefined }));
      return prefix;
    } catch {
      continue;
    }
  }

  return trimmed;
}

// P2-3: Single entry point for assistant display text sanitization.
// Only assistant-role MESSAGE blocks are eligible for tail-envelope stripping.
// The kind parameter is the primary boundary guard — reasoning, tool, status,
// scheduler_stage, and all other block kinds MUST pass through unchanged.
// The role check is a secondary guard for defense-in-depth.
export function sanitizeAssistantDisplayText(text: string, kind: string, role?: string | null): string {
  const raw = text.trimEnd();
  if (!raw) return raw;
  if (kind !== "message") return raw;
  if ((role ?? "assistant") !== "assistant") return raw;
  return stripTrailingCompatibilityEnvelope(raw);
}