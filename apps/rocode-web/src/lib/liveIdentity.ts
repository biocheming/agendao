import type { OutputBlock } from "./history";

export const ASSISTANT_TEXT_MAIN_PART_KEY = "text/main";
export const ASSISTANT_REASONING_MAIN_PART_KEY = "reasoning/main";
export const ASSISTANT_TEXT_PART_KEY_PREFIX = "text/";
export const ASSISTANT_REASONING_PART_KEY_PREFIX = "reasoning/";
export const TOOL_CALL_PART_KEY_PREFIX = "tool_call/";
export const TOOL_RESULT_PART_KEY_PREFIX = "tool_result/";
export const SCHEDULER_STAGE_PART_KEY_PREFIX = "scheduler/";

export function assistantTextPartKey(segment: string): string {
  return `${ASSISTANT_TEXT_PART_KEY_PREFIX}${segment}`;
}

export function assistantReasoningPartKey(segment: string): string {
  return `${ASSISTANT_REASONING_PART_KEY_PREFIX}${segment}`;
}

export function toolCallPartKey(toolCallId: string): string {
  return `${TOOL_CALL_PART_KEY_PREFIX}${toolCallId}`;
}

export function toolResultPartKey(toolCallId: string): string {
  return `${TOOL_RESULT_PART_KEY_PREFIX}${toolCallId}`;
}

export function schedulerStagePartKey(stageId: string): string {
  return `${SCHEDULER_STAGE_PART_KEY_PREFIX}${stageId}`;
}

export function toolIdFromPartKey(partKey: string | null | undefined): string | null {
  const trimmed = partKey?.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith(TOOL_CALL_PART_KEY_PREFIX)) {
    return trimmed.slice(TOOL_CALL_PART_KEY_PREFIX.length).trim() || null;
  }
  if (trimmed.startsWith(TOOL_RESULT_PART_KEY_PREFIX)) {
    return trimmed.slice(TOOL_RESULT_PART_KEY_PREFIX.length).trim() || null;
  }
  return null;
}

export function liveSlotKey(messageId: string, partKey: string): string {
  return `${messageId}:${partKey}`;
}

export function outputBlockLiveSlotKey(block: OutputBlock): string | undefined {
  const messageId = block.live_identity?.message_id?.trim();
  const partKey = block.live_identity?.part_key?.trim();
  if (!messageId || !partKey) return undefined;
  return liveSlotKey(messageId, partKey);
}
