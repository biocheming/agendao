import type { PersistedMultimodalExplain } from "./multimodal";

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

export interface OutputBlock {
  kind: string;
  phase?: string;
  role?: string;
  title?: string;
  event?: string;
  text?: string;
  tone?: string;
  silent?: boolean;
  id?: string;
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
  child_session_id?: string;
  active_skills?: string[];
  active_agents?: string[];
  active_categories?: string[];
  prompt_tokens?: number;
  completion_tokens?: number;
  reasoning_tokens?: number;
  cache_read_tokens?: number;
  cache_write_tokens?: number;
  decision?: {
    title?: string;
    fields?: Array<{ label?: string; value?: string; tone?: string }>;
    sections?: Array<{ title?: string; body?: string }>;
  } | null;
  detail?: string;
  display?: OutputDisplay | null;
  structured?: unknown;
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
  parts?: MessagePartRecord[];
  metadata?: Record<string, unknown> | null;
  multimodal?: PersistedMultimodalExplain | null;
}

export interface FeedMessage extends OutputBlock {
  feedId: string;
  text: string;
}
