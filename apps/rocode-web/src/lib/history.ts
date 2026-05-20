import type { PersistedMultimodalExplain } from "./multimodal";

// P3-E: Shared live identity contract — matches rocode_types::LiveMessagePartIdentity wire format.
export interface LiveMessagePartIdentity {
  message_id: string;
  part_key: string;
  part_kind: "assistant_text" | "assistant_reasoning" | "tool_call" | "tool_result" | "scheduler_stage";
  phase: "start" | "delta" | "snapshot" | "end";
  legacy_block_id?: string | null;
}

export interface OutputField {
  label?: string;
  value?: string;
  tone?: string;
}

export interface OutputPreview {
  kind?: string;
  text?: string;
  truncated?: boolean;
}

export interface OutputDisplay {
  header?: string;
  summary?: string;
  fields?: OutputField[];
  preview?: OutputPreview | null;
}

export interface OutputPresentation {
  group?: string;
  slot?: string;
  rank?: number;
  sequence?: number | null;
}

export interface OutputBlock {
  kind: string;
  phase?: string;
  role?: string;
  metadata?: Record<string, unknown> | null;
  title?: string;
  event?: string;
  text?: string;
  tone?: string;
  silent?: boolean;
  id?: string;
  /** P3-E: Stable identity from server for routing without heuristics. */
  live_identity?: LiveMessagePartIdentity | null;
  name?: string;
  stage_id?: string;
  tool_call_id?: string;
  status?: string;
  summary?: string;
  fields?: OutputField[];
  preview?: string;
  body?: string;
  ts?: number;
  profile?: string;
  stage?: string;
  stage_index?: number;
  stage_total?: number;
  step?: number;
  focus?: string;
  last_event?: string;
  waiting_on?: string;
  activity?: string;
  attached_session_id?: string;
  active_skills?: string[];
  active_agents?: string[];
  active_categories?: string[];
  prompt_tokens?: number;
  completion_tokens?: number;
  reasoning_tokens?: number;
  cache_read_tokens?: number;
  cache_miss_tokens?: number;
  cache_write_tokens?: number;
  decision?: {
    title?: string;
    fields?: Array<{ label?: string; value?: string; tone?: string }>;
    sections?: Array<{ title?: string; body?: string }>;
  } | null;
  detail?: string;
  display?: OutputDisplay | null;
  structured?: unknown;
  presentation?: OutputPresentation;
}

export interface MessagePartRecord {
  id: string;
  type: string;
  text?: string;
  ignored?: boolean;
  synthetic?: boolean;
  file?: {
    url: string;
    filename: string;
    mime: string;
  };
  output_block?: OutputBlock;
}

export interface MessageRecord {
  id: string;
  role: string;
  mode?: string | null;
  tokens?: {
    input: number;
    context?: number;
    output: number;
    reasoning: number;
    cache_read: number;
    cache_miss: number;
    cache_write: number;
  };
  parts?: MessagePartRecord[];
  metadata?: Record<string, unknown> | null;
  multimodal?: PersistedMultimodalExplain | null;
}

export interface FeedMessage extends OutputBlock {
  feedId: string;
  anchorId?: string;
  text: string;
}
