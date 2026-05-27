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

export interface SchedulerStageDecisionSection {
  title?: string;
  body?: string;
}

export interface SchedulerStageDecisionField {
  label?: string;
  value?: string;
  tone?: string;
}

export interface SchedulerStageDecision {
  title?: string;
  fields?: SchedulerStageDecisionField[];
  sections?: SchedulerStageDecisionSection[];
}

export type OutputBlockKind =
  | "inspect"
  | "message"
  | "multimodal_info"
  | "queue_item"
  | "reasoning"
  | "scheduler_stage"
  | "session_event"
  | "status"
  | "tool";

interface OutputBlockBase {
  kind: OutputBlockKind;
  phase?: string;
  role?: string;
  metadata?: Record<string, unknown> | null;
  presentation?: OutputPresentation;
  ts?: number;
  id?: string;
  title?: string;
  text?: string;
  summary?: string;
  fields?: OutputField[];
  /** P3-E: Stable identity from server for routing without heuristics. */
  live_identity?: LiveMessagePartIdentity | null;
}

interface CompatibilityDisplayMixin {
  preview?: string;
  body?: string;
  detail?: string;
  display?: OutputDisplay | null;
  structured?: unknown;
}

export interface MessageOutputBlock extends OutputBlockBase {
  kind: "message";
}

export interface ReasoningOutputBlock extends OutputBlockBase {
  kind: "reasoning";
}

export interface ToolOutputBlock extends OutputBlockBase, CompatibilityDisplayMixin {
  kind: "tool";
  name?: string;
  tool_call_id?: string;
  stage_id?: string;
}

export interface SchedulerStageOutputBlock extends OutputBlockBase {
  kind: "scheduler_stage";
  stage_id?: string;
  profile?: string;
  status?: string;
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
  decision?: SchedulerStageDecision | null;
}

export interface StatusOutputBlock extends OutputBlockBase {
  kind: "status";
  tone?: string;
  silent?: boolean;
}

export interface RuntimeSurfaceOutputBlock extends OutputBlockBase, CompatibilityDisplayMixin {
  event?: string;
}

export interface SessionEventOutputBlock extends RuntimeSurfaceOutputBlock {
  kind: "session_event";
}

export interface QueueItemOutputBlock extends RuntimeSurfaceOutputBlock {
  kind: "queue_item";
}

export interface InspectOutputBlock extends RuntimeSurfaceOutputBlock {
  kind: "inspect";
}

export interface MultimodalInfoOutputBlock extends OutputBlockBase {
  kind: "multimodal_info";
}

export type OutputBlock =
  | InspectOutputBlock
  | MessageOutputBlock
  | MultimodalInfoOutputBlock
  | QueueItemOutputBlock
  | ReasoningOutputBlock
  | SchedulerStageOutputBlock
  | SessionEventOutputBlock
  | StatusOutputBlock
  | ToolOutputBlock;

export type OutputBlockOfKind<K extends OutputBlockKind> = Extract<OutputBlock, { kind: K }>;
export type AuxiliaryOutputBlock = SessionEventOutputBlock | QueueItemOutputBlock | InspectOutputBlock;
export type DisplayContractOutputBlock = ToolOutputBlock | AuxiliaryOutputBlock;

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

export interface FeedMessageMeta {
  feedId: string;
  anchorId?: string;
  text: string;
}

export type FeedBlock<K extends OutputBlockKind> = OutputBlockOfKind<K> & FeedMessageMeta;
export type FeedMessage = { [K in OutputBlockKind]: FeedBlock<K> }[OutputBlockKind];

export function isMessageOutputBlock(block: OutputBlock): block is MessageOutputBlock {
  return block.kind === "message";
}

export function isReasoningOutputBlock(block: OutputBlock): block is ReasoningOutputBlock {
  return block.kind === "reasoning";
}

export function isToolOutputBlock(block: OutputBlock): block is ToolOutputBlock {
  return block.kind === "tool";
}

export function isSchedulerStageOutputBlock(block: OutputBlock): block is SchedulerStageOutputBlock {
  return block.kind === "scheduler_stage";
}

export function isStatusOutputBlock(block: OutputBlock): block is StatusOutputBlock {
  return block.kind === "status";
}

export function isMultimodalInfoOutputBlock(block: OutputBlock): block is MultimodalInfoOutputBlock {
  return block.kind === "multimodal_info";
}

export function feedStageId(message: FeedMessage): string | undefined {
  return isToolOutputBlock(message) || isSchedulerStageOutputBlock(message)
    ? message.stage_id
    : undefined;
}

export function feedToolCallId(message: FeedMessage): string | undefined {
  return isToolOutputBlock(message) ? message.tool_call_id : undefined;
}

export function feedAttachedSessionId(message: FeedMessage): string | undefined {
  return isSchedulerStageOutputBlock(message) ? message.attached_session_id : undefined;
}

export function isStreamingTextOutputBlock(
  block: OutputBlock,
): block is MessageOutputBlock | ReasoningOutputBlock {
  return block.kind === "message" || block.kind === "reasoning";
}

// P2-3: Single-point display contract detection. A block has a display contract
// when the server populated at least one display.* sub-field. This is used by
// text policy and tool presentation to decide whether to prefer display fields
// or fall back to legacy compatibility fields (detail, raw text, preview).
export function hasDisplayContract(block: DisplayContractOutputBlock): boolean {
  return Boolean(
    block.display?.summary?.trim()
    || (block.display?.fields?.length && block.display.fields.length > 0)
    || block.display?.preview?.text?.trim(),
  );
}
