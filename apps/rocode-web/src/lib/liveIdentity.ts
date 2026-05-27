import type { OutputBlock } from "./history";

export const ASSISTANT_TEXT_MAIN_PART_KEY = "text/main";
export const ASSISTANT_REASONING_MAIN_PART_KEY = "reasoning/main";
const TOOL_CALL_PART_KEY_PREFIX = "tool_call/";
const TOOL_RESULT_PART_KEY_PREFIX = "tool_result/";

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

function liveSlotKey(messageId: string, partKey: string): string {
  return `${messageId}:${partKey}`;
}

export function outputBlockLiveSlotKey(block: OutputBlock): string | undefined {
  const messageId = block.live_identity?.message_id?.trim();
  const partKey = block.live_identity?.part_key?.trim();
  if (!messageId || !partKey) return undefined;
  return liveSlotKey(messageId, partKey);
}
