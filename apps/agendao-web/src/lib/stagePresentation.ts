// P2-3: Single-point scheduler stage display contract.
//
// SchedulerStageCard and ExecutionActivityPanel previously each had
// independent chains for extracting stage summaries, titles, and labels.
// This file centralizes the extraction logic so components only render.
//
// BOUNDARY: humanizeStageEvent / humanizeStageWaitTarget (in stageSignals.ts)
// are display-only label beautifiers. They must never make semantic decisions
// about transcript routing, text priority, or structured content stripping.

import type { FeedBlock, SchedulerStageOutputBlock } from "./history";
import { humanizeStageEvent, humanizeStageWaitTarget } from "./stageSignals";

export function compactText(value: unknown): string {
  return String(value ?? "").replace(/\s+/g, " ").trim();
}

export function normalizeValue(value: unknown): { structured: boolean; text: string } {
  const text = String(value ?? "").trim();
  if (!text) return { structured: false, text: "" };

  const candidate = text.startsWith("{") || text.startsWith("[");
  if (candidate) {
    try {
      return { structured: true, text: JSON.stringify(JSON.parse(text), null, 2) };
    } catch {
      // Not valid JSON — treat as plain text.
    }
  }

  return {
    structured:
      text.includes("\n") || text.length > 140 || text.includes("{") || text.includes("["),
    text,
  };
}

export function excerptText(value: unknown, maxLength = 120): string | null {
  const text = compactText(value);
  if (!text) return null;
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1)}…`;
}

// P2-3: Stage summary is resolved by a single explicit chain:
// focus (explicit server field) → humanized last_event → raw text.
// Components must not build their own chain.
export function stageSummaryText(block: SchedulerStageOutputBlock | FeedBlock<"scheduler_stage">): string | null {
  const focus = compactText(block.focus);
  if (focus) return focus;

  const lastEventLabel = humanizeStageEvent(block.last_event);
  const lastEvent = lastEventLabel ? compactText(lastEventLabel) : null;
  if (lastEvent) return lastEvent;

  return compactText(block.text) || null;
}

// P2-3: Stage title resolved by explicit priority: title → stage name → default.
// Components must not invent their own title chain.
export function stageDisplayTitle(block: SchedulerStageOutputBlock | FeedBlock<"scheduler_stage">): string {
  return block.title || block.stage || "Scheduler Stage";
}

// P2-3: Human-readable labels for wait target / last event.
// These delegate to stageSignals.ts for pure label beautification.
export { humanizeStageEvent, humanizeStageWaitTarget };

// P2-3: Token summary chips. Centralized here so both SchedulerStageCard
// and any future stage display component produce identical token labels.
export function stageTokenChips(block: SchedulerStageOutputBlock | FeedBlock<"scheduler_stage">): string[] {
  return [
    block.prompt_tokens ? `input ${formatCompactTokenCount(block.prompt_tokens)}` : null,
    block.completion_tokens ? `output ${formatCompactTokenCount(block.completion_tokens)}` : null,
    block.reasoning_tokens ? `reasoning ${formatCompactTokenCount(block.reasoning_tokens)}` : null,
    block.cache_read_tokens ? `cache read ${formatCompactTokenCount(block.cache_read_tokens)}` : null,
    block.cache_miss_tokens ? `cache miss ${formatCompactTokenCount(block.cache_miss_tokens)}` : null,
    block.cache_write_tokens ? `cache write ${formatCompactTokenCount(block.cache_write_tokens)}` : null,
  ].filter((v): v is string => v !== null);
}

function formatCompactTokenCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  return String(Math.round(value));
}