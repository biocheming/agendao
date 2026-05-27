import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import type { MessageRecord, OutputBlock } from "../src/lib/history";
import { primaryDisplayText } from "../src/lib/blockTextPolicy";
import {
  appendLiveBlock,
  applyOutputBlock,
  buildFeedFromHistory,
  createOptimisticUserFeedMessage,
  mergeHistoryWithLiveBlocks,
  pruneLiveBlocksCoveredByHistory,
  resetLiveTranscriptFeedSequence,
  setActiveFeedSequence,
  shouldQueueLiveTranscriptBlock,
} from "../src/lib/liveTranscriptState";
import {
  canonicalLiveExecutionStatus,
  partitionLiveExecutions,
} from "../src/lib/liveExecutionState";
import {
  ASSISTANT_REASONING_MAIN_PART_KEY,
  ASSISTANT_TEXT_MAIN_PART_KEY,
} from "../src/lib/liveIdentity";
import { sanitizeAssistantDisplayText } from "../src/lib/blockPresentation";
import { buildRunTailSummary } from "../src/lib/runTailSummary";
import { stageSummaryText } from "../src/lib/stagePresentation";
import { toolActivityLabel, toolKindLabel } from "../src/lib/toolLabels";
import { registerObservationSink, type ObservationEvent, type ObservationSink } from "../src/lib/observationEvents";

type LiveFixture = {
  shared_turn_cycles: {
    entries: Array<{
      message_id: string;
      message_text: string;
      tool: null | {
        tool_id: string;
        tool_name: string;
        tool_detail: string;
      };
    }>;
    expected: {
      assistant_message_count: number;
      tool_result_count: number;
    };
  };
  tool_progress_exclusion: {
    message: {
      message_id: string;
      text: string;
    };
    tool_running: {
      tool_id: string;
      tool_name: string;
      tool_detail: string;
    };
    tool_result: {
      tool_id: string;
      tool_name: string;
      tool_detail: string;
    };
  };
  scheduler_stage_exclusion: {
    message_id: string;
    stage_id: string;
    stage: string;
    title: string;
    text: string;
    status: string;
  };
  run_tail_contract: {
    completed_status: string;
    completed_usage: {
      input_tokens: number;
      output_tokens: number;
      reasoning_tokens: number;
      total_cost: number;
    };
    error_status: string;
    error_message: string;
    awaiting_user_status: string;
    awaiting_user_detail: string;
  };
};

const fixturePath = path.resolve(
  process.cwd(),
  "../../crates/rocode-command/governance/live_transcript_state_fixture.json",
);
const fixture = JSON.parse(fs.readFileSync(fixturePath, "utf8")) as LiveFixture;

function toolBlock(overrides: Partial<OutputBlock> = {}): OutputBlock {
  return {
    kind: "tool",
    phase: "full",
    role: "assistant",
    live_identity: {
      message_id: "assistant-1",
      part_key: `tool_result/tool-call-1`,
      part_kind: "tool_result",
      phase: "snapshot",
      legacy_block_id: "tool-call-1",
    },
    title: "SkillsList",
    text: '{"category":"literature-research/skills"}',
    ...overrides,
  };
}

function toolBlockWithoutStableToolId(overrides: Partial<OutputBlock> = {}): OutputBlock {
  return {
    kind: "tool",
    phase: "full",
    role: "assistant",
    live_identity: {
      message_id: "assistant-1",
      part_key: "tool_result",
      part_kind: "tool_result",
      phase: "snapshot",
      legacy_block_id: null,
    },
    title: "SkillsList",
    text: '{"category":"literature-research/skills"}',
    ...overrides,
  };
}

function assistantMessageBlock(messageId: string, text: string, overrides: Partial<OutputBlock> = {}): OutputBlock {
  return {
    kind: "message",
    phase: "full",
    role: "assistant",
    id: messageId,
    text,
    live_identity: {
      message_id: messageId,
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "snapshot",
      legacy_block_id: messageId,
    },
    ...overrides,
  };
}

function toolBlockFor(messageId: string, toolId: string, text: string, overrides: Partial<OutputBlock> = {}): OutputBlock {
  return {
    kind: "tool",
    phase: "end",
    role: "assistant",
    id: toolId,
    title: "SkillsList",
    text,
    live_identity: {
      message_id: messageId,
      part_key: `tool_result/${toolId}`,
      part_kind: "tool_result",
      phase: "end",
      legacy_block_id: toolId,
    },
    ...overrides,
  };
}

function runningToolBlockFor(toolId: string, text: string, overrides: Partial<OutputBlock> = {}): OutputBlock {
  return {
    kind: "tool",
    phase: "running",
    role: "assistant",
    id: toolId,
    title: "SkillsList",
    text,
    live_identity: {
      message_id: "assistant-1",
      part_key: `tool_call/${toolId}`,
      part_kind: "tool_call",
      phase: "append",
      legacy_block_id: toolId,
    },
    ...overrides,
  };
}

{
  assert.equal(canonicalLiveExecutionStatus("start"), "running");
  assert.equal(canonicalLiveExecutionStatus("running"), "running");
  assert.equal(canonicalLiveExecutionStatus("full"), "done");
  assert.equal(canonicalLiveExecutionStatus("end"), "done");
  assert.equal(canonicalLiveExecutionStatus("result"), "done");
  assert.equal(canonicalLiveExecutionStatus("error"), "error");

  const partitioned = partitionLiveExecutions([
    {
      id: "tool-a",
      label: "Skill SkillsList",
      status: "running",
      kind: "skill" as const,
      summary: "11 skills · literature-research/skills",
      fields: [{ label: "Scope", value: "literature-research/skills" }],
      preview: { kind: "text", text: "author-network\nsemantic-scholar", truncated: false },
      toolCallId: "tool-a",
      stageId: "stage-1",
      updatedAt: 300,
    },
    {
      id: "tool-b",
      label: "Skill SkillView",
      status: "done",
      kind: "skill" as const,
      summary: "loaded semantic-scholar",
      fields: [],
      preview: { kind: "text", text: "Description: Search Semantic Scholar.", truncated: false },
      toolCallId: "tool-b",
      stageId: "stage-1",
      updatedAt: 200,
    },
    {
      id: "tool-c",
      label: "Tool bash",
      status: "error",
      kind: "tool" as const,
      summary: "command failed",
      fields: [{ label: "Command", value: "bash -lc false" }],
      preview: { kind: "code", text: "bash -lc false", truncated: false },
      toolCallId: "tool-c",
      stageId: "stage-2",
      updatedAt: 100,
    },
  ]);
  assert.deepEqual(
    partitioned.current.map((entry) => entry.id),
    ["tool-a"],
  );
  assert.deepEqual(
    partitioned.recent.map((entry) => entry.id),
    ["tool-b", "tool-c"],
  );
}

{
  assert.equal(toolActivityLabel("skill"), "Skill");
  assert.equal(toolActivityLabel("SkillsList"), "Skill SkillsList");
  assert.equal(toolKindLabel("skill"), "Skill");
  assert.equal(toolKindLabel("tool"), "Tool");
}

{
  const errorTail = buildRunTailSummary({
    statusLine: fixture.run_tail_contract.error_status,
    latestRuntimeError: fixture.run_tail_contract.error_message,
  });
  assert.equal(errorTail.status, fixture.run_tail_contract.error_status);
  assert.equal(errorTail.title, "Run failed");
  assert.equal(errorTail.detail, fixture.run_tail_contract.error_message);

  const permissionTail = buildRunTailSummary({
    statusLine: "running",
    pendingPermission: true,
  });
  assert.equal(permissionTail.status, "awaiting_permission");
  assert.equal(permissionTail.title, "Waiting for permission");

  const awaitingUserTail = buildRunTailSummary({
    statusLine: fixture.run_tail_contract.awaiting_user_status,
    awaitingUser: true,
  });
  assert.equal(awaitingUserTail.status, fixture.run_tail_contract.awaiting_user_status);
  assert.equal(awaitingUserTail.detail, fixture.run_tail_contract.awaiting_user_detail);

  const completeTail = buildRunTailSummary({
    statusLine: fixture.run_tail_contract.completed_status,
    usage: fixture.run_tail_contract.completed_usage,
  });
  assert.equal(completeTail.status, fixture.run_tail_contract.completed_status);
  assert.equal(completeTail.title, "Run complete");
  assert.match(completeTail.detail ?? "", /input 1200/);

  const idleTail = buildRunTailSummary({ statusLine: "idle" });
  assert.equal(idleTail.status, "idle");
  assert.equal(idleTail.title, "Session idle");

  const runtimeStatusTail = buildRunTailSummary({
    statusLine: "ready",
    runtimeStatus: "running",
    activeStageName: "Research",
  });
  assert.equal(runtimeStatusTail.status, "running");
  assert.equal(runtimeStatusTail.title, "Running");
  assert.equal(runtimeStatusTail.detail, "Current stage: Research");

  const readyTail = buildRunTailSummary({ statusLine: "ready" });
  assert.equal(readyTail.status, "ready");
  assert.equal(readyTail.title, "Session ready");

  const reconnectingTail = buildRunTailSummary({ statusLine: "reconnecting" });
  assert.equal(reconnectingTail.status, "reconnecting");
  assert.equal(reconnectingTail.title, "Reconnecting stream");

  const retryingTail = buildRunTailSummary({ statusLine: "retrying" });
  assert.equal(retryingTail.status, "retrying");
  assert.equal(retryingTail.title, "Retrying");
  assert.equal(retryingTail.detail, "Waiting for automatic retry.");

  const compactingTail = buildRunTailSummary({ statusLine: "compacting" });
  assert.equal(compactingTail.status, "compacting");
  assert.equal(compactingTail.title, "Compacting");
  assert.equal(compactingTail.detail, "Preparing a smaller context window.");
}

{
  resetLiveTranscriptFeedSequence();
  const seqA = {
    value: 0,
    nextId() {
      this.value += 1;
      return `A-${this.value}`;
    },
    reset() {
      this.value = 0;
    },
  };
  const seqB = {
    value: 0,
    nextId() {
      this.value += 1;
      return `B-${this.value}`;
    },
    reset() {
      this.value = 0;
    },
  };

  const releaseA = setActiveFeedSequence(seqA);
  assert.equal(
    createOptimisticUserFeedMessage("owned by A").feedId,
    "A-1",
    "first owner must control feed ids",
  );

  const releaseB = setActiveFeedSequence(seqB);
  assert.equal(
    createOptimisticUserFeedMessage("owned by B").feedId,
    "B-1",
    "later owner must override active feed sequence",
  );

  releaseA();
  assert.equal(
    createOptimisticUserFeedMessage("B still owns after A cleanup").feedId,
    "B-2",
    "older owner cleanup must not steal feed-sequence ownership from newer owner",
  );

  releaseB();
  assert.equal(
    createOptimisticUserFeedMessage("default restored").feedId,
    "feed-1",
    "releasing last owner must restore default feed sequence",
  );
}

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock([], toolBlock({ id: undefined, text: '{"category":"a"}' }), true);
  visible = applyOutputBlock(visible, toolBlock({ id: undefined, text: '{"category":"b"}' }), true);

  assert.equal(visible.length, 1, "visible feed should upsert same tool snapshot instead of duplicating");
  assert.equal(visible[0]?.text, '{"category":"b"}');
  assert.equal(visible[0]?.id, "tool-call-1");
}

{
  // P2-3: Changed from part_kind "tool_call" to "tool_result" because P1-5
  // isTranscriptBearingIdentity excludes tool_call from the visible feed.
  // tool_result IS transcript-bearing, so the same-ID-overwrite test remains valid.
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock(
    [],
    {
      kind: "tool",
      phase: "end",
      role: "assistant",
      detail: '{"command":"echo a"}',
      live_identity: {
        message_id: "assistant-1",
        part_key: `tool_result/tool-call-0`,
        part_kind: "tool_result",
        phase: "end",
        legacy_block_id: "tool-call-0",
      },
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "tool",
      phase: "end",
      role: "assistant",
      detail: '{"command":"echo b"}',
      live_identity: {
        message_id: "assistant-2",
        part_key: `tool_result/tool-call-0`,
        part_kind: "tool_result",
        phase: "end",
        legacy_block_id: "tool-call-0",
      },
    },
    true,
  );

  assert.equal(visible.length, 2, "same raw tool_call_id in different messages must not overwrite");
  assert.deepEqual(
    visible.map((message) => `${message.id}:${message.text}`),
    [
      'tool-call-0:{"command":"echo a"}',
      'tool-call-0:{"command":"echo b"}',
    ],
  );
}

{
  const contractText = primaryDisplayText(
    toolBlock({
      id: undefined,
      text: '{"category":"raw-json"}',
      detail: "compatibility detail",
      preview: "compatibility preview",
      display: {
        summary: "11 skills in literature-research/skills",
        fields: [
          { label: "Scope", value: "literature-research/skills" },
          { label: "Count", value: "11" },
        ],
        preview: {
          kind: "text",
          text: "author-network\nsemantic-scholar",
          truncated: false,
        },
      },
    }),
  );

  assert.equal(
    contractText,
    "11 skills in literature-research/skills",
    "tool transcript text should prefer shared display summary over raw compatibility payloads",
  );
}

{
  const contractFieldText = primaryDisplayText({
    kind: "tool",
    phase: "full",
    role: "assistant",
    display: {
      fields: [
        { label: "Query", value: "Xu Ximing" },
        { label: "Hits", value: "24" },
      ],
    },
    text: '{"raw":"payload"}',
  });

  assert.equal(
    contractFieldText,
    "Query: Xu Ximing\nHits: 24",
    "tool transcript text should fall back to display fields before raw tool bodies",
  );
}

{
  const compatibilityText = primaryDisplayText({
    kind: "tool",
    phase: "full",
    role: "assistant",
    detail: "compatibility detail fallback",
  });

  assert.equal(
    compatibilityText,
    "compatibility detail fallback",
    "compatibility tool detail should remain available when no display contract exists",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    toolBlockWithoutStableToolId({ id: undefined, text: '{"category":"no-stable-id"}' }),
    true,
  );

  assert.equal(
    visible.length,
    0,
    "identity-bearing tool block without stable tool id must stay out of visible transcript feed",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const weirdMessagePhase: OutputBlock = {
    kind: "message",
    phase: "weird_phase",
    role: "assistant",
    text: "should not insert",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "snapshot",
      legacy_block_id: "assistant-1",
    },
  };

  const visible = applyOutputBlock([], weirdMessagePhase, true);
  assert.equal(
    visible.length,
    0,
    "identity-bearing message with unknown phase must not fall back to presentation insert",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    {
      kind: "status",
      tone: "error",
      text: "transport down",
    },
    true,
  );
  assert.equal(
    visible.length,
    0,
    "status blocks belong to run-tail/banner surfaces and must not enter authoritative transcript feed",
  );
}

{
  const retained = appendLiveBlock(
    [],
    {
      kind: "queue_item",
      text: "queued prompt",
      phase: "full",
    },
  );
  assert.equal(
    retained.length,
    0,
    "queue items are auxiliary execution state and must not enter retained live transcript cache",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const compatibilityWeirdMessagePhase: OutputBlock = {
    kind: "message",
    phase: "weird_phase",
    role: "assistant",
    text: "should not insert",
  };

  const visible = applyOutputBlock([], compatibilityWeirdMessagePhase, true);
  assert.equal(
    visible.length,
    0,
    "compatibility message with unknown phase must not fall back to presentation insert",
  );
}

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock(
    [],
    {
      kind: "message",
      phase: "delta",
      role: "assistant",
      id: "compat-msg-1",
      text: "这",
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "message",
      phase: "delta",
      role: "assistant",
      id: "compat-msg-1",
      text: "是一段",
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "message",
      phase: "delta",
      role: "assistant",
      id: "compat-msg-1",
      text: "完整回复",
    },
    true,
  );

  assert.equal(visible.length, 1, "compatibility message deltas must accumulate into one visible block");
  assert.equal(visible[0]?.id, "compat-msg-1");
  assert.equal(visible[0]?.text, "这是一段完整回复");
}

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock(
    [],
    {
      kind: "reasoning",
      phase: "delta",
      role: "assistant",
      id: "compat-think-1",
      text: "先",
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "reasoning",
      phase: "delta",
      role: "assistant",
      id: "compat-think-1",
      text: "检索",
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "reasoning",
      phase: "delta",
      role: "assistant",
      id: "compat-think-1",
      text: "再归纳",
    },
    true,
  );

  assert.equal(visible.length, 1, "compatibility reasoning deltas must accumulate into one visible block");
  assert.equal(visible[0]?.kind, "reasoning");
  assert.equal(visible[0]?.id, "compat-think-1");
  assert.equal(visible[0]?.text, "先检索再归纳");
}

{
  resetLiveTranscriptFeedSequence();
  const unknownBlock: OutputBlock = {
    kind: "mystery_block",
    phase: "full",
    text: "should not insert",
  };

  const visible = applyOutputBlock([], unknownBlock, true);
  assert.equal(
    visible.length,
    0,
    "unknown block kinds must not enter authoritative transcript feed through generic compatibility presentation fallback",
  );
}

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock(
    [],
    {
      kind: "tool",
      phase: "running",
      role: "assistant",
      id: "compat-tool-a",
      title: "Bash",
      detail: '{"command":"echo a"}',
    },
    true,
  );
  visible = applyOutputBlock(
    visible,
    {
      kind: "tool",
      phase: "done",
      role: "assistant",
      id: "compat-tool-a",
      title: "Bash",
      detail: "result a",
    },
    true,
  );

  assert.equal(visible.length, 2, "compatibility tool running/result must be separate visible blocks");
  assert.deepEqual(
    visible.map((message) => `${message.phase}:${message.id}:${message.text}`),
    [
      'running:compat-tool-a:{"command":"echo a"}',
      "done:compat-tool-a:result a",
    ],
  );
}

{
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(
    liveBlocks,
    {
      kind: "tool",
      phase: "running",
      role: "assistant",
      id: "compat-tool-a",
      title: "Bash",
      detail: '{"command":"echo a"}',
    },
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    {
      kind: "tool",
      phase: "running",
      role: "assistant",
      id: "compat-tool-b",
      title: "Bash",
      detail: '{"command":"echo b"}',
    },
  );

  assert.equal(liveBlocks.length, 2, "different compatibility tool ids must retain distinct live cache slots");
  assert.deepEqual(
    liveBlocks.map((block) => `${block.id}:${primaryDisplayText(block)}`),
    [
      'compat-tool-a:{"command":"echo a"}',
      'compat-tool-b:{"command":"echo b"}',
    ],
  );
}

// Phase W1: session_event must not enter the conversation feed.
{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    {
      kind: "session_event",
      id: "evt-1",
      event: "subtask",
      title: "Subtask · inspect scheduler",
      status: "pending",
      body: "delegated",
    },
    true,
  );
  assert.equal(visible.length, 0, "session_event must not enter conversation feed");
}

// Phase W1: inspect must not enter the conversation feed.
{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    {
      kind: "inspect",
      id: "inspect-1",
      summary: "2 stage events",
      body: "stage-1\nstage-2",
    },
    true,
  );
  assert.equal(visible.length, 0, "inspect must not enter conversation feed");
}

// Phase W1: status must not enter the conversation feed.
{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    { kind: "status", tone: "error", text: "something failed" },
    true,
  );
  assert.equal(visible.length, 0, "status must not enter conversation feed");
}

// Phase W1: session_event / inspect must not even enter transcript queue.
{
  assert.equal(
    shouldQueueLiveTranscriptBlock({
      kind: "session_event",
      id: "evt-2",
      event: "subtask",
      title: "Queued event",
    }),
    false,
    "session_event must be rejected at transcript queue ingress",
  );
  assert.equal(
    shouldQueueLiveTranscriptBlock({
      kind: "inspect",
      id: "inspect-2",
      summary: "debug payload",
    }),
    false,
    "inspect must be rejected at transcript queue ingress",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const emptyStart: OutputBlock = {
    kind: "message",
    phase: "start",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "start",
      legacy_block_id: "assistant-1",
    },
  };
  const emptyFull: OutputBlock = {
    kind: "message",
    phase: "full",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "snapshot",
      legacy_block_id: "assistant-1",
    },
  };

  let visible = applyOutputBlock([], emptyStart, true);
  visible = applyOutputBlock(visible, emptyFull, true);
  assert.equal(
    visible.length,
    0,
    "empty assistant boundaries must not materialize blank visible feed entries",
  );
}

{
  assert.equal(
    fixture.run_tail_contract.completed_status,
    "complete",
    "shared fixture should declare complete status for run-tail contract",
  );
  assert.equal(
    fixture.run_tail_contract.error_status,
    "error",
    "shared fixture should declare error status for run-tail contract",
  );
  assert.equal(
    fixture.run_tail_contract.awaiting_user_status,
    "awaiting_user",
    "shared fixture should declare awaiting_user status for run-tail contract",
  );
  assert.ok(
    fixture.run_tail_contract.completed_usage.input_tokens > 0,
    "shared fixture should carry non-zero completion usage",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const emptyReasoningStart: OutputBlock = {
    kind: "reasoning",
    phase: "start",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
      part_kind: "assistant_reasoning",
      phase: "start",
      legacy_block_id: "assistant-1",
    },
  };
  const emptyReasoningFull: OutputBlock = {
    kind: "reasoning",
    phase: "full",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
      part_kind: "assistant_reasoning",
      phase: "snapshot",
      legacy_block_id: "assistant-1",
    },
  };

  let visible = applyOutputBlock([], emptyReasoningStart, true);
  visible = applyOutputBlock(visible, emptyReasoningFull, true);
  assert.equal(
    visible.length,
    0,
    "empty reasoning boundaries must not materialize blank visible feed entries",
  );
}

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock([], assistantMessageBlock("assistant-1", "现在我已掌握"), true);
  visible = applyOutputBlock(
    visible,
    assistantMessageBlock("assistant-1", "现在我已掌握充分信息，以下是完整调研报告。"),
    true,
  );

  assert.equal(visible.length, 1, "non-prefix full snapshots should still keep one assistant feed entry");
  assert.equal(
    visible[0]?.text,
    "现在我已掌握充分信息，以下是完整调研报告。",
    "later full snapshot must replace earlier partial assistant content",
  );
}

// P1-5: tool_call blocks are excluded from the transcript feed.
// The dedup algorithm is tested using tool_result identity blocks,
// which are transcript-bearing and follow the same upsert path.
{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock([], toolBlock({ id: undefined, phase: "start", text: "" }), true);
  visible = applyOutputBlock(
    visible,
    toolBlockFor("assistant-1", "tool-call-1", fixture.tool_progress_exclusion.tool_running.tool_detail, {
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
    }),
    true,
  );
  visible = applyOutputBlock(
    visible,
    toolBlockFor("assistant-1", "tool-call-1", fixture.tool_progress_exclusion.tool_running.tool_detail, {
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
    }),
    true,
  );

  // The preceding start block (phase="start", empty text) does not
  // produce a visible entry — boundary signals are not content.
  assert.equal(
    visible.length,
    1,
    "tool result detail must dedup into single live TOOL block per upsertFeedMessage",
  );
  assert.equal(visible[0]?.kind, "tool");
  assert.equal(visible[0]?.tool_call_id, "tool-call-1");
  assert.equal(visible[0]?.text, fixture.tool_progress_exclusion.tool_running.tool_detail);
}

{
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(liveBlocks, toolBlock({ id: undefined, phase: "full", text: '{"category":"a"}' }));
  liveBlocks = appendLiveBlock(liveBlocks, toolBlock({ id: undefined, phase: "end", text: '{"category":"done"}' }));

  assert.equal(liveBlocks.length, 1, "live cache should retain final non-text snapshot on end");
  assert.equal(liveBlocks[0]?.text, '{"category":"done"}');
  assert.equal(liveBlocks[0]?.phase, "end");
  assert.equal(liveBlocks[0]?.id, "tool-call-1");
}

{
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(
    liveBlocks,
    {
      kind: "tool",
      phase: "running",
      role: "assistant",
      detail: "{\"command\":\"curl\"}",
      live_identity: {
        message_id: "assistant-1",
        part_key: `tool_result/tool-call-9`,
        part_kind: "tool_result",
        phase: "snapshot",
        legacy_block_id: "tool-call-9",
      },
    },
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    {
      kind: "tool",
      phase: "running",
      role: "assistant",
      detail: "{\"command\":\"curl -s https://api.semanticscholar.org\"}",
      live_identity: {
        message_id: "assistant-1",
        part_key: `tool_result/tool-call-9`,
        part_kind: "tool_result",
        phase: "snapshot",
        legacy_block_id: "tool-call-9",
      },
    },
  );

  assert.equal(liveBlocks.length, 1, "running tool snapshots should retain one live slot");
  assert.equal(
    primaryDisplayText(liveBlocks[0] as OutputBlock),
    "{\"command\":\"curl -s https://api.semanticscholar.org\"}",
    "later richer tool snapshot must replace the earlier shorter one",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const history = buildFeedFromHistory([
    {
      id: "tool-message-1",
      role: "tool",
      parts: [
        {
          id: "part-1",
          type: "tool_result",
          output_block: {
            kind: "tool",
            phase: "done",
            id: "tool-call-0",
            tool_call_id: "tool-call-0",
            detail: "result a",
          },
        },
      ],
    },
    {
      id: "tool-message-2",
      role: "tool",
      parts: [
        {
          id: "part-2",
          type: "tool_result",
          output_block: {
            kind: "tool",
            phase: "done",
            id: "tool-call-0",
            tool_call_id: "tool-call-0",
            detail: "result b",
          },
        },
      ],
    },
  ] as MessageRecord[], true);

  assert.equal(history.length, 2, "history tool results with reused raw call_id must stay distinct");
  assert.deepEqual(
    history.map((message) => `${message.id}:${message.tool_call_id}:${message.text}`),
    [
      "tool-message-1:part-1:tool:tool-call-0:result a",
      "tool-message-2:part-2:tool:tool-call-0:result b",
    ],
  );
}

{
  const emptyAssistantStart: OutputBlock = {
    kind: "message",
    phase: "start",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "start",
      legacy_block_id: "assistant-1",
    },
  };
  const emptyAssistantDelta: OutputBlock = {
    kind: "message",
    phase: "delta",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "append",
      legacy_block_id: "assistant-1",
    },
  };

  let liveBlocks = appendLiveBlock([], emptyAssistantStart);
  liveBlocks = appendLiveBlock(liveBlocks, emptyAssistantDelta);
  assert.equal(
    liveBlocks.length,
    0,
    "empty assistant start/delta must not enter retained live cache",
  );
}

{
  const liveBlocks = appendLiveBlock(
    [],
    toolBlockWithoutStableToolId({ id: undefined, text: '{"category":"no-stable-id"}' }),
  );

  assert.equal(
    liveBlocks.length,
    0,
    "identity-bearing tool block without stable tool id must not enter retained live transcript cache",
  );
}

// P1-5 adapted: tool_call is excluded from live cache. Test dedup
// algorithm with tool_result identity (transcript-bearing).
{
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlockFor("assistant-1", "tool-call-1", fixture.tool_progress_exclusion.tool_running.tool_detail, {
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
    }),
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlockFor("assistant-1", "tool-call-1", fixture.tool_progress_exclusion.tool_running.tool_detail, {
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
    }),
  );

  assert.equal(
    liveBlocks.length,
    1,
    "tool result detail should retain one live transcript cache slot per tool call",
  );
  assert.equal(liveBlocks[0]?.tool_call_id, "tool-call-1");
  assert.equal(liveBlocks[0]?.text, fixture.tool_progress_exclusion.tool_running.tool_detail);
}

{
  resetLiveTranscriptFeedSequence();
  const history: MessageRecord[] = [
    {
      id: "user-1",
      role: "user",
      parts: [{ id: "part-1", type: "text", text: "search skills" }],
    },
  ];
  const liveBlocks = [
    toolBlock({ id: undefined, phase: "end", text: '{"category":"scientific-skills"}' }),
  ];

  const rebuilt = mergeHistoryWithLiveBlocks(history, liveBlocks, true);
  const toolMessages = rebuilt.filter((message) => message.kind === "tool");

  assert.equal(toolMessages.length, 1, "rebuild feed should preserve final retained tool snapshot");
  assert.equal(toolMessages[0]?.text, '{"category":"scientific-skills"}');
}

{
  resetLiveTranscriptFeedSequence();
  const history: MessageRecord[] = [
    {
      id: "user-1",
      role: "user",
      parts: [{ id: "part-1", type: "text", text: "search skills" }],
    },
  ];
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlockFor("assistant-1", "tool-call-1", '{"category":"literature-research/skills"}'),
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlockFor("assistant-1", "tool-call-2", '{"category":"scientific-skills"}'),
  );

  const rebuilt = mergeHistoryWithLiveBlocks(history, liveBlocks, true);
  const toolMessages = rebuilt.filter((message) => message.kind === "tool");

  assert.equal(
    toolMessages.length,
    2,
    "P1-5: rebuilt feed should preserve distinct tool_result blocks without collapsing",
  );
  assert.deepEqual(
    toolMessages.map((message) => message.tool_call_id),
    ["tool-call-1", "tool-call-2"],
  );
}

{
  resetLiveTranscriptFeedSequence();
  const history: MessageRecord[] = [
    {
      id: "user-1",
      role: "user",
      parts: [{ id: "part-1", type: "text", text: "search skills" }],
    },
  ];
  const liveBlocks = [
    toolBlockWithoutStableToolId({ id: undefined, text: '{"category":"no-stable-id"}' }),
  ];

  const rebuilt = mergeHistoryWithLiveBlocks(history, liveBlocks, true);
  const toolMessages = rebuilt.filter((message) => message.kind === "tool");

  assert.equal(
    toolMessages.length,
    0,
    "rebuild feed must not materialize tool blocks that lack a stable transcript tool id",
  );
}

{
  resetLiveTranscriptFeedSequence();
  let visible = [];
  for (const entry of fixture.shared_turn_cycles.entries) {
    visible = applyOutputBlock(
      visible,
      assistantMessageBlock(entry.message_id, entry.message_text),
      true,
    );
    if (entry.tool) {
      visible = applyOutputBlock(
        visible,
        toolBlockFor(entry.message_id, entry.tool.tool_id, entry.tool.tool_detail, {
          title: entry.tool.tool_name,
        }),
        true,
      );
    }
  }

  const assistantMessages = visible.filter((message) => message.kind === "message");
  const toolMessages = visible.filter((message) => message.kind === "tool");

  assert.equal(
    assistantMessages.length,
    fixture.shared_turn_cycles.expected.assistant_message_count,
    "shared sample should preserve five assistant message boundaries",
  );
  assert.equal(
    toolMessages.length,
    fixture.shared_turn_cycles.expected.tool_result_count,
    "shared sample should preserve four tool cycles without duplication",
  );
  assert.equal(
    new Set(assistantMessages.map((message) => message.id)).size,
    fixture.shared_turn_cycles.expected.assistant_message_count,
  );
  assert.equal(
    new Set(toolMessages.map((message) => message.id)).size,
    fixture.shared_turn_cycles.expected.tool_result_count,
  );
}

{
  const history: MessageRecord[] = [
    {
      id: "assistant-1",
      role: "assistant",
      parts: [
        {
          id: "tool-part-1",
          type: "tool_result",
          output_block: toolBlock({
            id: undefined,
            phase: "end",
            text: '{"category":"scientific-skills"}',
          }),
        },
      ],
    },
  ];

  const pruned = pruneLiveBlocksCoveredByHistory(history, [
    toolBlock({ id: undefined, phase: "end", text: '{"category":"scientific-skills"}' }),
  ]);

  assert.equal(
    pruned.length,
    0,
    "authoritative history must absorb same-slot final tool snapshots from live cache",
  );
}

{
  const history: MessageRecord[] = [
    {
      id: "assistant-1",
      role: "assistant",
      parts: [{
        id: "text-part-1",
        type: "text",
        text: "final answer",
        output_block: assistantMessageBlock("assistant-1", "final answer", { phase: "full" }),
      }],
    },
  ];

  const pruned = pruneLiveBlocksCoveredByHistory(history, [
    {
      kind: "message",
      phase: "full",
      role: "assistant",
      text: "final answer",
      live_identity: {
        message_id: "assistant-1",
        part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
        part_kind: "assistant_text",
        phase: "snapshot",
        legacy_block_id: "assistant-1",
      },
      id: "assistant-1",
    },
  ]);

  assert.equal(
    pruned.length,
    0,
    "authoritative history must absorb assistant text snapshots from live cache after reconcile",
  );
}

{
  const history: MessageRecord[] = [
    {
      id: "assistant-1",
      role: "assistant",
      parts: [{
        id: "reasoning-part-1",
        type: "reasoning",
        text: "main reasoning",
        output_block: {
          kind: "reasoning",
          phase: "full",
          role: "assistant",
          text: "main reasoning",
          live_identity: {
            message_id: "assistant-1",
            part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
            part_kind: "assistant_reasoning",
            phase: "snapshot",
            legacy_block_id: "assistant-1",
          },
          id: "assistant-1",
        },
      }],
    },
  ];

  const pruned = pruneLiveBlocksCoveredByHistory(history, [
    {
      kind: "reasoning",
      phase: "full",
      role: "assistant",
      text: "main reasoning",
      live_identity: {
        message_id: "assistant-1",
        part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
        part_kind: "assistant_reasoning",
        phase: "snapshot",
        legacy_block_id: "assistant-1",
      },
      id: "assistant-1",
    },
    {
      kind: "reasoning",
      phase: "full",
      role: "assistant",
      text: "branch reasoning",
      live_identity: {
        message_id: "assistant-1",
        part_key: `reasoning/branch-a`,
        part_kind: "assistant_reasoning",
        phase: "snapshot",
        legacy_block_id: "assistant-1",
      },
      id: "assistant-1",
    },
  ]);

  assert.equal(
    pruned.length,
    1,
    "history output_block.live_identity must only prune the owned reasoning slot",
  );
  assert.equal(
    pruned[0]?.live_identity?.part_key,
    `reasoning/branch-a`,
    "history output_block.live_identity must not over-prune non-owned reasoning branches",
  );
}

{
  const schedulerProgress: OutputBlock = {
    kind: "scheduler_stage",
    role: "assistant",
    phase: "full",
    id: fixture.scheduler_stage_exclusion.stage_id,
    stage_id: fixture.scheduler_stage_exclusion.stage_id,
    stage: fixture.scheduler_stage_exclusion.stage,
    status: fixture.scheduler_stage_exclusion.status,
    text: fixture.scheduler_stage_exclusion.text,
  };

  assert.equal(
    shouldQueueLiveTranscriptBlock(schedulerProgress),
    false,
    "scheduler progress without transcript identity should stay out of visible transcript feed",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const schedulerProgressWithIdentity: OutputBlock = {
    kind: "scheduler_stage",
    role: "assistant",
    phase: "full",
    stage_id: fixture.scheduler_stage_exclusion.stage_id,
    stage: fixture.scheduler_stage_exclusion.stage,
    status: fixture.scheduler_stage_exclusion.status,
    text: fixture.scheduler_stage_exclusion.text,
    live_identity: {
      message_id: fixture.scheduler_stage_exclusion.message_id,
      part_key: `scheduler/${fixture.scheduler_stage_exclusion.stage_id}`,
      part_kind: "scheduler_stage",
      phase: "snapshot",
      legacy_block_id: null,
    },
  };

  const visible = applyOutputBlock([], schedulerProgressWithIdentity, true);
  assert.equal(
    visible.length,
    0,
    "scheduler stage with live identity must still stay out of transcript feed",
  );
}

{
  const schedulerProgressWithIdentity: OutputBlock = {
    kind: "scheduler_stage",
    role: "assistant",
    phase: "full",
    stage_id: fixture.scheduler_stage_exclusion.stage_id,
    stage: fixture.scheduler_stage_exclusion.stage,
    status: fixture.scheduler_stage_exclusion.status,
    text: fixture.scheduler_stage_exclusion.text,
    live_identity: {
      message_id: fixture.scheduler_stage_exclusion.message_id,
      part_key: `scheduler/${fixture.scheduler_stage_exclusion.stage_id}`,
      part_kind: "scheduler_stage",
      phase: "snapshot",
      legacy_block_id: null,
    },
  };

  const retained = appendLiveBlock([], schedulerProgressWithIdentity);
  assert.equal(
    retained.length,
    0,
    "scheduler stage with live identity must not enter retained live transcript cache",
  );
}

{
  resetLiveTranscriptFeedSequence();
  const schedulerProgressWithIdentity: OutputBlock = {
    kind: "scheduler_stage",
    role: "assistant",
    phase: "full",
    stage_id: fixture.scheduler_stage_exclusion.stage_id,
    stage: fixture.scheduler_stage_exclusion.stage,
    status: fixture.scheduler_stage_exclusion.status,
    text: fixture.scheduler_stage_exclusion.text,
    live_identity: {
      message_id: fixture.scheduler_stage_exclusion.message_id,
      part_key: `scheduler/${fixture.scheduler_stage_exclusion.stage_id}`,
      part_kind: "scheduler_stage",
      phase: "snapshot",
      legacy_block_id: null,
    },
  };

  let visible = applyOutputBlock([], schedulerProgressWithIdentity, true);
  visible = applyOutputBlock(visible, toolBlock({ id: undefined, text: '{"category":"mixed"}' }), true);

  assert.equal(
    visible.length,
    1,
    "non-transcript scheduler progress must not occupy transcript feed slots in mixed turns",
  );
  assert.equal(visible[0]?.kind, "tool");
  assert.equal(visible[0]?.text, '{"category":"mixed"}');
}

{
  resetLiveTranscriptFeedSequence();
  const history: MessageRecord[] = [
    {
      id: fixture.scheduler_stage_exclusion.message_id,
      role: "assistant",
      metadata: {
        scheduler_stage: fixture.scheduler_stage_exclusion.stage,
        scheduler_stage_id: fixture.scheduler_stage_exclusion.stage_id,
        scheduler_stage_status: fixture.scheduler_stage_exclusion.status,
        scheduler_stage_index: 1,
        scheduler_stage_total: 3,
      },
      parts: [{ id: "part-1", type: "text", text: fixture.scheduler_stage_exclusion.text }],
    },
  ];

  const rebuilt = mergeHistoryWithLiveBlocks(history, [], true);
  assert.equal(
    rebuilt.filter((message) => message.kind === "scheduler_stage").length,
    0,
    "history rebuild must not materialize scheduler stage into authoritative transcript feed",
  );
}

// ── Web Phase 1 regression: End finalize + streaming text contracts ─────

// T1: message start -> delta* -> full -> end.
// Phase 2: deltas silently accumulate in live cache; only full/end upsert
// into visible feed. The full block carries complete coalesced text.
{
  let messages: ReturnType<typeof applyOutputBlock> = [];
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "", { phase: "start" }),
    true,
  );
  // Deltas are silent in visible feed (Phase 2).
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "fragment", { phase: "delta" }),
    true,
  );
  assert.equal(messages.length, 0, "delta must not create visible feed entry");
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "another fragment", { phase: "delta" }),
    true,
  );
  assert.equal(messages.length, 0, "repeated deltas must not touch visible feed");
  // Full snapshot carries the authoritative text and upserts.
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "hello world", { phase: "full" }),
    true,
  );
  assert.equal(messages.length, 1, "full must upsert into visible feed");
  assert.equal(messages[0]?.text, "hello world");
  // End finalizes without duplicating.
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "", { phase: "end" }),
    true,
  );
  assert.equal(messages.length, 1, "end must not duplicate visible block");
  assert.equal(messages[0]?.text, "hello world", "end must retain full-snapshot text");
}

// T1-reasoning: reasoning delta silent, full upserts, end finalizes.
{
  function reasoningBlock(messageId: string, text: string, overrides: Partial<OutputBlock> = {}): OutputBlock {
    return {
      kind: "reasoning",
      phase: "delta",
      role: "assistant",
      id: messageId,
      text,
      live_identity: {
        message_id: messageId,
        part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
        part_kind: "assistant_reasoning",
        phase: "snapshot",
        legacy_block_id: messageId,
      },
      ...overrides,
    };
  }

  let messages: ReturnType<typeof applyOutputBlock> = [];
  messages = applyOutputBlock(messages, reasoningBlock("msg-1", "", { phase: "start" }), true);
  // Phase 2: deltas are silent.
  messages = applyOutputBlock(messages, reasoningBlock("msg-1", "fragment", { phase: "delta" }), true);
  assert.equal(messages.length, 0, "reasoning delta must not touch visible feed");
  // Full upserts.
  messages = applyOutputBlock(
    messages,
    reasoningBlock("msg-1", "thinking more", { phase: "full" }),
    true,
  );
  assert.equal(messages.length, 1, "reasoning full must upsert into visible feed");
  assert.equal(messages[0]?.text, "thinking more");
  // Empty end is no-op.
  messages = applyOutputBlock(messages, reasoningBlock("msg-1", "", { phase: "end" }), true);
  assert.equal(messages.length, 1, "reasoning end must not duplicate");
  assert.equal(messages[0]?.text, "thinking more");
}

// T4: appendLiveBlock end marks streaming text phase="end" and preserves text.
{
  const live: OutputBlock[] = [];
  const afterDelta = appendLiveBlock(
    live,
    assistantMessageBlock("msg-1", "partial text", { phase: "delta" }),
  );
  assert.equal(afterDelta.length, 1, "delta must insert live block");
  assert.equal(afterDelta[0]?.text, "partial text");

  const afterEnd = appendLiveBlock(
    afterDelta,
    assistantMessageBlock("msg-1", "", { phase: "end" }),
  );
  assert.equal(afterEnd.length, 1, "end must not prune streaming text block");
  assert.equal(
    afterEnd[0]?.phase,
    "end",
    "end must set retained block phase to end for downstream settle detection",
  );
  assert.equal(
    afterEnd[0]?.text,
    "partial text",
    "end must preserve accumulated text from prior deltas when end payload is empty",
  );
}

// T4-end-with-text: when end carries accumulated text, use it.
{
  const live: OutputBlock[] = [];
  const afterEnd = appendLiveBlock(
    live,
    assistantMessageBlock("msg-1", "final consolidated text", { phase: "end" }),
  );
  assert.equal(afterEnd.length, 1, "end with text must retain the block");
  assert.equal(afterEnd[0]?.phase, "end");
  assert.equal(afterEnd[0]?.text, "final consolidated text");
}

// T4-snapshot-accumulate: repeated full snapshots for the same live slot may
// arrive as token-sized increments on Web SSE and must accumulate instead of
// collapsing to the last token.
{
  const reasoningBlock = (text: string, phase: OutputBlock["phase"]): OutputBlock => ({
    kind: "reasoning",
    phase,
    role: "assistant",
    text,
    live_identity: {
      message_id: "msg-snapshot-1",
      part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
      part_kind: "assistant_reasoning",
      phase: phase === "start" ? "start" : phase === "end" ? "end" : "snapshot",
      legacy_block_id: "msg-snapshot-1",
    },
  });

  let live: OutputBlock[] = [];
  live = appendLiveBlock(live, reasoningBlock("", "start"));
  live = appendLiveBlock(live, reasoningBlock("for", "full"));
  live = appendLiveBlock(live, reasoningBlock("", "end"));
  live = appendLiveBlock(live, reasoningBlock("", "start"));
  live = appendLiveBlock(live, reasoningBlock(" papers", "full"));
  live = appendLiveBlock(live, reasoningBlock("", "end"));
  live = appendLiveBlock(live, reasoningBlock("", "start"));
  live = appendLiveBlock(live, reasoningBlock(".", "full"));
  live = appendLiveBlock(live, reasoningBlock("", "end"));

  assert.equal(live.length, 1, "repeated full snapshots for one reasoning slot must retain one live block");
  assert.equal(live[0]?.text, "for papers.", "token-sized full snapshots must accumulate in arrival order");
  assert.equal(live[0]?.phase, "end", "final end must settle the accumulated reasoning block");
}

// T4-punctuation-suppression: punctuation-only live snapshots without prior
// accumulated text must not materialize standalone cards.
{
  let live: OutputBlock[] = [];
  live = appendLiveBlock(
    live,
    assistantMessageBlock("msg-punct-1", "。", { phase: "full" }),
  );
  assert.equal(
    live.length,
    0,
    "punctuation-only full snapshot without prior text must stay suppressed until meaningful text arrives",
  );

  live = appendLiveBlock(
    live,
    assistantMessageBlock("msg-punct-1", "。检索开始", { phase: "full" }),
  );
  assert.equal(live.length, 1, "meaningful follow-up snapshot must materialize the live text block");
  assert.equal(live[0]?.text, "。检索开始");
}

// T5: multi-part reasoning — distinct part_keys must not collide in live cache.
{
  function reasoningWithPartKey(
    messageId: string,
    partKey: string,
    text: string,
    phase: string,
  ): OutputBlock {
    return {
      kind: "reasoning",
      phase,
      role: "assistant",
      text,
      live_identity: {
        message_id: messageId,
        part_key: partKey,
        part_kind: "assistant_reasoning" as const,
        phase: "snapshot" as const,
        legacy_block_id: messageId,
      },
    };
  }

  const live: OutputBlock[] = [];
  const afterMain = appendLiveBlock(
    live,
    reasoningWithPartKey("msg-1", ASSISTANT_REASONING_MAIN_PART_KEY, "main thinking", "full"),
  );
  assert.equal(afterMain.length, 1, "main reasoning slot must insert live block");

  const afterBranch = appendLiveBlock(
    afterMain,
    reasoningWithPartKey("msg-1", `reasoning/branch-a`, "branch analysis", "full"),
  );
  assert.equal(
    afterBranch.length,
    2,
    "branch reasoning slot must not collide with main reasoning in live cache",
  );
  assert.equal(afterBranch[0]?.text, "main thinking");
  assert.equal(afterBranch[1]?.text, "branch analysis");

  // Updating reasoning/main must not affect reasoning/branch-a.
  const afterMainUpdate = appendLiveBlock(
    afterBranch,
    reasoningWithPartKey("msg-1", ASSISTANT_REASONING_MAIN_PART_KEY, "main thinking revised", "full"),
  );
  assert.equal(
    afterMainUpdate.length,
    2,
    "updating main reasoning must not delete branch reasoning",
  );
  assert.equal(afterMainUpdate[0]?.text, "main thinking revised");
  assert.equal(afterMainUpdate[1]?.text, "branch analysis");
}

// T5-visible: multi-part reasoning in visible feed must not collide.
{
  function reasoningWithPartKey(
    messageId: string,
    partKey: string,
    text: string,
    phase: string,
  ): OutputBlock {
    return {
      kind: "reasoning",
      phase,
      role: "assistant",
      text,
      live_identity: {
        message_id: messageId,
        part_key: partKey,
        part_kind: "assistant_reasoning" as const,
        phase: "snapshot" as const,
        legacy_block_id: messageId,
      },
    };
  }

  let messages: ReturnType<typeof applyOutputBlock> = [];
  messages = applyOutputBlock(
    messages,
    reasoningWithPartKey("msg-1", ASSISTANT_REASONING_MAIN_PART_KEY, "main thinking", "full"),
    true,
  );
  assert.equal(messages.length, 1, "first reasoning part must insert");

  messages = applyOutputBlock(
    messages,
    reasoningWithPartKey("msg-1", `reasoning/branch-a`, "branch analysis", "full"),
    true,
  );
  assert.equal(
    messages.length,
    2,
    "second reasoning part with different part_key must not overwrite first in visible feed",
  );
  assert.equal(messages[0]?.text, "main thinking");
  assert.equal(messages[1]?.text, "branch analysis");
}

// T5-history-merge: multi-part reasoning via history + live merge must
// not collide. mergeLiveTextBlock uses slotKey() for streaming text
// matching during mergeHistoryWithLiveBlocks.
{
  function reasoningWithPartKey(
    messageId: string,
    partKey: string,
    text: string,
    phase: string,
  ): OutputBlock {
    return {
      kind: "reasoning",
      phase,
      role: "assistant",
      text,
      live_identity: {
        message_id: messageId,
        part_key: partKey,
        part_kind: "assistant_reasoning" as const,
        phase: "snapshot" as const,
        legacy_block_id: messageId,
      },
    };
  }

  const liveBlocks: OutputBlock[] = [
    reasoningWithPartKey("msg-1", ASSISTANT_REASONING_MAIN_PART_KEY, "main thinking", "full"),
    reasoningWithPartKey("msg-1", `reasoning/branch-a`, "branch analysis", "full"),
  ];

  // Full history covers both reasoning parts.
  const fullHistory: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [],
  }];

  const merged = mergeHistoryWithLiveBlocks(fullHistory, liveBlocks, true);
  const reasoningBlocks = merged.filter((m) => m.kind === "reasoning");
  assert.equal(
    reasoningBlocks.length,
    2,
    "history+live merge must preserve distinct part_keys as separate reasoning blocks",
  );

  // Prune at slotKey granularity: history with no output_blocks
  // should not prune any streaming text live blocks (slotKey requires
  // output_block.live_identity to populate coveredIds).
  const pruned = pruneLiveBlocksCoveredByHistory(fullHistory, liveBlocks);
  assert.equal(
    pruned.length,
    2,
    "history without output_block.live_identity must not prune slot-keyed live blocks",
  );
}

// Phase 2 regression: buildFeedFromHistory must render persisted
// text and reasoning parts via synthetic "full" blocks (not "delta",
// which is a silent no-op in the visible feed).
{
  const { buildFeedFromHistory } = await import("../src/lib/liveTranscriptState");

  const history: MessageRecord[] = [{
    id: "assistant-1",
    role: "assistant",
    parts: [
      { id: "p1", type: "reasoning", text: "thinking aloud" },
      { id: "p2", type: "text", text: "hello world" },
    ],
  }];

  const feed = buildFeedFromHistory(history, true);
  const reasoning = feed.filter((m) => m.kind === "reasoning");
  const text = feed.filter((m) => m.kind === "message");

  assert.equal(reasoning.length, 1, "persisted reasoning part must render one visible block");
  assert.equal(reasoning[0]?.text, "thinking aloud", "reasoning text must be preserved");
  assert.equal(text.length, 1, "persisted text part must render one visible block");
  assert.equal(text[0]?.text, "hello world", "assistant text must be preserved");
}

// T3-history-main-text-bridge: live text/main updates must merge into the
// existing persisted assistant text card instead of inserting a duplicate
// card just because history uses msg_id:message while live uses msg_id+part_key.
{
  const history: MessageRecord[] = [{
    id: "msg-bridge-1",
    role: "assistant",
    parts: [{ id: "p1", type: "text", text: "好的，我来使用 Semantic Scholar API" }],
  }];

  let messages = buildFeedFromHistory(history, true);
  assert.equal(messages.length, 1, "history must build one assistant card");

  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-bridge-1", "好的，我来使用 Semantic Scholar API 来检索论文。"),
    true,
  );

  assert.equal(
    messages.length,
    1,
    "live text/main must update the persisted main assistant card instead of inserting a second card",
  );
  assert.equal(messages[0]?.text, "好的，我来使用 Semantic Scholar API 来检索论文。");
}

// T3-history-main-text-no-shrink: once a persisted/larger assistant main card
// exists, a shorter live full snapshot for the same main slot must not shrink
// it to punctuation or another truncated fragment.
{
  const history: MessageRecord[] = [{
    id: "msg-bridge-2",
    role: "assistant",
    parts: [{ id: "p1", type: "text", text: "好的，我来使用 Semantic Scholar API 来检索论文" }],
  }];

  let messages = buildFeedFromHistory(history, true);
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-bridge-2", "。"),
    true,
  );

  assert.equal(messages.length, 1, "short live snapshot must not create a duplicate card");
  assert.equal(
    messages[0]?.text,
    "好的，我来使用 Semantic Scholar API 来检索论文",
    "shorter live snapshot must not overwrite the richer persisted assistant text",
  );
}

// T4: full/end mixed finalize — delta → full → delta → end converges to
// one block with correct accumulated text. This covers the coalescer
// interleaving full snapshots and trailing deltas before End.
{
  let messages: ReturnType<typeof applyOutputBlock> = [];
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "", { phase: "start" }),
    true,
  );
  // Delta: silent (Phase 2).
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "fragment-a", { phase: "delta" }),
    true,
  );
  assert.equal(messages.length, 0);
  // Full snapshot carries coalesced text.
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "fragment-a more-text", { phase: "full" }),
    true,
  );
  assert.equal(messages.length, 1);
  assert.equal(messages[0]?.text, "fragment-a more-text");
  // Trailing delta: silent.
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", " trailing", { phase: "delta" }),
    true,
  );
  assert.equal(messages.length, 1);
  // End with accumulated trailing text.
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-1", "fragment-a more-text trailing", { phase: "end" }),
    true,
  );
  assert.equal(messages.length, 1, "full+end mix must converge to one block");
  assert.equal(messages[0]?.text, "fragment-a more-text trailing");
}

// T4-order: live transcript must preserve arrival order across reasoning,
// tool, and assistant blocks. Web must not regroup by block kind.
{
  let messages: ReturnType<typeof applyOutputBlock> = [];
  messages = applyOutputBlock(
    messages,
    {
      kind: "reasoning",
      phase: "full",
      role: "assistant",
      text: "先检索作者身份",
      live_identity: {
        message_id: "msg-order-1",
        part_key: ASSISTANT_REASONING_MAIN_PART_KEY,
        part_kind: "assistant_reasoning",
        phase: "snapshot",
        legacy_block_id: "msg-order-1",
      },
    },
    true,
  );
  messages = applyOutputBlock(
    messages,
    runningToolBlockFor("tool-order-1", '{"url":"https://api.semanticscholar.org/..."}', {
      title: "WebFetch",
      live_identity: {
        message_id: "msg-order-1",
        part_key: `tool_call/tool-order-1`,
        part_kind: "tool_call",
        phase: "append",
        legacy_block_id: "tool-order-1",
      },
    }),
    true,
  );
  messages = applyOutputBlock(
    messages,
    toolBlockFor("msg-order-1", "tool-order-1", "api.semanticscholar.org · application/json", {
      title: "WebFetch",
    }),
    true,
  );
  messages = applyOutputBlock(
    messages,
    assistantMessageBlock("msg-order-2", "检索到 49 篇论文。"),
    true,
  );

  assert.deepEqual(
    messages.map((message) => `${message.kind}:${message.phase}:${message.text}`),
    [
      "reasoning:full:先检索作者身份",
      "tool:end:api.semanticscholar.org · application/json",
      "message:full:检索到 49 篇论文。",
    ],
    "P1-5: tool_call blocks excluded from transcript feed; live transcript "
    + "must preserve cross-kind arrival order of remaining blocks",
  );
}

// T3-adapted: without server-issued output_block.live_identity, persisted
// history must not claim transcript slot ownership. Web no longer invents
// canonical part_key names for history prune.
{
  const liveBlocks: OutputBlock[] = [
    assistantMessageBlock("msg-1", "complete live text", { phase: "full" }),
  ];

  // Persisted history text without output_block/live_identity must not
  // prune the live transcript slot.
  const persistedTextHistory: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [{ id: "p1", type: "text", text: "partial" }],
  }];

  const pruned = pruneLiveBlocksCoveredByHistory(persistedTextHistory, liveBlocks);
  assert.equal(
    pruned.length,
    1,
    "persisted text without output_block.live_identity must not prune text/main live block",
  );
  assert.equal(pruned[0]?.text, "complete live text");

  // Persisted reasoning-only history also must not prune the text/main slot.
  const reasoningOnlyHistory: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [{ id: "p1", type: "reasoning", text: "thinking" }],
  }];

  const textLiveBlock: OutputBlock[] = [
    assistantMessageBlock("msg-1", "complete live text", { phase: "full" }),
  ];
  const prunedReasoningOnly = pruneLiveBlocksCoveredByHistory(reasoningOnlyHistory, textLiveBlock);
  assert.equal(
    prunedReasoningOnly.length,
    1,
    "persisted reasoning-only history must not prune text/main live block",
  );
  assert.equal(prunedReasoningOnly[0]?.text, "complete live text");
}

// ── Phase W4: live/history/reconcile contract tests ───────────────────

// P2-3 adapted: W4-T1 uses tool_result (transcript-bearing) identity so
// the live cache correctly retains distinct tool blocks.
// W4-T1: two different tool_result_id blocks in live cache must
// not overwrite each other.
{
  function runningToolBlock(toolId: string, text?: string): OutputBlock {
    return {
      kind: "tool",
      phase: "full",
      role: "assistant",
      text: text ?? `detail-${toolId}`,
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/${toolId}`,
        part_kind: "tool_result" as const,
        phase: "snapshot" as const,
        legacy_block_id: toolId,
      },
    };
  }

  let live: OutputBlock[] = [];
  live = appendLiveBlock(live, runningToolBlock("call-a", "detail-a"));
  live = appendLiveBlock(live, runningToolBlock("call-b", "detail-b"));
  assert.equal(live.length, 2, "two running tools must retain distinct live cache slots");
  assert.equal(live[0]?.text, "detail-a");
  assert.equal(live[1]?.text, "detail-b");

  // Updating call-a must not affect call-b.
  const afterUpdate = appendLiveBlock(live, runningToolBlock("call-a", "detail-a-revised"));
  assert.equal(afterUpdate.length, 2, "updating one tool must not delete the other");
  assert.equal(afterUpdate[0]?.text, "detail-a-revised");
  assert.equal(afterUpdate[1]?.text, "detail-b");
}

// P1-5 adapted W4-T2: tool_call blocks are excluded from the transcript
// feed. Only tool_result blocks (resultToolBlock) appear. Running tool
// state belongs to the execution panel, not the transcript.
{
  function runningToolBlockW4(toolId: string, text?: string): OutputBlock {
    return {
      kind: "tool",
      phase: "full",
      role: "assistant",
      text: text ?? `detail-${toolId}`,
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_call/${toolId}`,
        part_kind: "tool_call" as const,
        phase: "snapshot" as const,
        legacy_block_id: toolId,
      },
    };
  }

  function resultToolBlock(toolId: string, text?: string): OutputBlock {
    return {
      kind: "tool",
      phase: "done",
      role: "assistant",
      text: text ?? `result-${toolId}`,
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/${toolId}`,
        part_kind: "tool_result" as const,
        phase: "end" as const,
        legacy_block_id: toolId,
      },
    };
  }

  let messages: ReturnType<typeof applyOutputBlock> = [];
  // P1-5: tool_call is excluded from the transcript feed.
  messages = applyOutputBlock(messages, runningToolBlockW4("call-1", "running-detail"), true);
  assert.equal(messages.length, 0, "P1-5: tool_call must NOT create visible transcript entry");

  // Result block creates the first visible entry.
  messages = applyOutputBlock(messages, resultToolBlock("call-1", "result-text"), true);
  assert.equal(
    messages.length,
    1,
    "P1-5: tool_result must create visible entry; tool_call is excluded",
  );
  assert.equal(messages[0]?.text, "result-text");
}

// W4-T3: history rebuild for tool blocks must preserve distinct
// running tool entries without collapsing them.
{
  const history: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [
      {
        id: "tp-1",
        type: "tool_call",
        output_block: {
          kind: "tool",
          phase: "done",
          role: "assistant",
          text: "history-result-a",
          live_identity: {
            message_id: "msg-1",
            part_key: "tool_result/call-a",
            part_kind: "tool_result" as const,
            phase: "end" as const,
            legacy_block_id: "call-a",
          },
        },
      },
    ],
  }];

  const liveBlocks: OutputBlock[] = [
    {
      kind: "tool",
      phase: "full",
      role: "assistant",
      text: "live-running-b",
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/call-b`,
        part_kind: "tool_result" as const,
        phase: "snapshot" as const,
        legacy_block_id: "call-b",
      },
    },
  ];

  const merged = mergeHistoryWithLiveBlocks(history, liveBlocks, true);
  const toolMsgs = merged.filter((m) => m.kind === "tool");
  assert.equal(
    toolMsgs.length,
    2,
    "history result and live running must both appear in rebuilt feed",
  );
}

// W4-T5: history rebuild must preserve tool_call/tool_result semantics so the
// card layer can keep TOOL RUNNING / TOOL RESULT labels after reload.
{
  const history: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [
      {
        id: "tp-1",
        type: "tool_call",
        output_block: {
          kind: "tool",
          phase: "running",
          text: "history-running",
          tool_call_id: "call-a",
        },
      },
      {
        id: "tp-2",
        type: "tool_result",
        output_block: {
          kind: "tool",
          phase: "done",
          text: "history-result",
          tool_call_id: "call-a",
        },
      },
    ],
  }];

  const rebuilt = buildFeedFromHistory(history, true);
  const toolMsgs = rebuilt.filter((message) => message.kind === "tool");
  assert.equal(toolMsgs.length, 2, "history rebuild should keep both tool entries");
  assert.equal(toolMsgs[0]?.metadata?.rocode_web_history_part_kind, "tool_call");
  assert.equal(toolMsgs[1]?.metadata?.rocode_web_history_part_kind, "tool_result");
}

// W4-T4: prune at tool level — history covering one tool call must
// not prune a different tool's live block.
{
  const history: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [{
      id: "tp-1",
      type: "tool_result",
      output_block: {
        kind: "tool",
        phase: "done",
        text: "result-a",
        live_identity: {
          message_id: "msg-1",
          part_key: "tool_result/call-a",
          part_kind: "tool_result" as const,
          phase: "end" as const,
          legacy_block_id: "call-a",
        },
      },
    }],
  }];

  const liveBlocks: OutputBlock[] = [
    {
      kind: "tool",
      phase: "full",
      text: "running-b",
      live_identity: {
        message_id: "msg-1",
        part_key: "tool_call/call-b",
        part_kind: "tool_call" as const,
        phase: "snapshot" as const,
        legacy_block_id: "call-b",
      },
    },
  ];

  const pruned = pruneLiveBlocksCoveredByHistory(history, liveBlocks);
  assert.equal(
    pruned.length,
    1,
    "history covering call-a must not prune call-b's live block",
  );
  assert.equal(pruned[0]?.text, "running-b");
}

// ── P2-3: Fixture tests for centralized block display contracts ──────

// P2-3-Fixture-1: assistant text with trailing JSON that is NOT an envelope
// (e.g. in markdown code block). sanitizeAssistantDisplayText must preserve it.
{
  const text = "这是回复文本\n```json\n{\"key\":\"value\"}\n```";
  const cleaned = sanitizeAssistantDisplayText(text, "message", "assistant");
  assert.equal(
    cleaned,
    text,
    "assistant text with markdown-fenced JSON must not be stripped (not an envelope)",
  );
}

// P2-3-Fixture-2: assistant text ending with a real compatibility structured
// envelope. sanitizeAssistantDisplayText must strip the trailing JSON.
{
  const prefix = "这是回复文本";
  // keys chosen from ENVELOPE_GUESS_KEYS to trigger isCompatibilityStructuredEnvelope
  const envelope = JSON.stringify({
    kind: "choices",
    choices: [{ text: "candidate" }],
    usage: { input: 100, output: 50 },
  }, null, 2);
  const text = `${prefix}\n\n${envelope}`;
  const cleaned = sanitizeAssistantDisplayText(text, "message", "assistant");
  assert.equal(
    cleaned,
    prefix,
    "assistant text with trailing structured envelope must be stripped of trailing JSON",
  );
  assert.ok(
    !cleaned.includes("choices"),
    "cleaned text must not contain envelope keys",
  );
}

// P2-3-Fixture-3: tool block with new display contract (display.summary present).
// primaryDisplayText must prefer display.summary over old legacy fields.
{
  const block: OutputBlock = {
    kind: "tool",
    phase: "end",
    role: "assistant",
    display: {
      summary: "Fetched 3 papers from Semantic Scholar",
      header: "WebFetch",
      fields: [{ label: "URL", value: "https://api.semanticscholar.org" }],
    },
    detail: '{"command":"curl -s https://api.semanticscholar.org"}',
    text: '{"command":"curl"}',
  };
  const text = primaryDisplayText(block);
  assert.equal(
    text,
    "Fetched 3 papers from Semantic Scholar",
    "tool with display.summary must prefer it over legacy detail/text",
  );
}

// P2-3-Fixture-4: tool block with old detail-only payload (no display contract).
// primaryDisplayText must fall back to detail.
{
  const block: OutputBlock = {
    kind: "tool",
    phase: "end",
    role: "assistant",
    detail: '{"command":"curl -s https://api.semanticscholar.org"}',
  };
  const text = primaryDisplayText(block);
  assert.equal(
    text,
    '{"command":"curl -s https://api.semanticscholar.org"}',
    "tool without display contract must fall back to detail field",
  );
}

// P2-3-Fixture-5: scheduler_stage with focus, last_event, and text all present.
// stageSummaryText must prefer focus over lastEventLabel over raw text.
{
  const block = {
    kind: "scheduler_stage" as const,
    focus: "Researching protein structures",
    last_event: "scheduler.stage.waiting",
    text: "Stage is waiting for tool result",
    status: "running",
  };
  const summary = stageSummaryText(block as SchedulerStageOutputBlock);
  assert.equal(
    summary,
    "Researching protein structures",
    "stageSummaryText must prefer focus field over last_event and raw text",
  );
}

// P2-3-Fixture-6: status block with title, text, and summary all present.
// primaryDisplayText must use STATUS_CHAIN priority.
{
  const block: OutputBlock = {
    kind: "status",
    phase: "full",
    title: "Status Title",
    text: "Status text content",
    summary: "Status summary",
    tone: "info",
  };
  const text = primaryDisplayText(block);
  assert.equal(
    text,
    "Status text content",
    "status block must prefer text over title/summary per STATUS_CHAIN",
  );
}

// P2-3-Fixture-7: reasoning text must NOT go through assistant cleaning,
// even when role="assistant". The kind parameter is the primary boundary
// guard — only kind="message" triggers envelope stripping.
{
  const prefix = "推理过程分析";
  const envelope = JSON.stringify({
    choices: [{ kind: "candidate", text: "output" }],
    usage: { input: 50, output: 20 },
  });
  const text = `${prefix}\n\n${envelope}`;

  // reasoning kind with role="assistant" — the real scenario.
  // kind guard must prevent stripping.
  const cleanedReasoning = sanitizeAssistantDisplayText(text, "reasoning", "assistant");
  assert.equal(
    cleanedReasoning,
    text,
    "reasoning text with trailing JSON must NOT be stripped even when role=assistant — kind guard must block",
  );

  // message kind with role="assistant" — envelope stripping IS expected.
  const cleanedMessage = sanitizeAssistantDisplayText(text, "message", "assistant");
  assert.equal(
    cleanedMessage,
    prefix,
    "message kind with role=assistant must still strip trailing envelope (regression check)",
  );

  // non-assistant role as additional guard.
  const cleanedSystem = sanitizeAssistantDisplayText(text, "message", "system");
  assert.equal(
    cleanedSystem,
    text,
    "message kind with non-assistant role must NOT strip (role guard defense-in-depth)",
  );
}

// P2-3-Fixture-8: multimodal_info block must NOT go through tool/status display strategies.
// primaryDisplayText must fall through to the default DISPLAY_FIRST_CHAIN.
// Verify that the text is NOT empty (proves it hits the default chain),
// and that DISPLAY_FIRST_CHAIN order is respected (summary before body).
{
  const block: OutputBlock = {
    kind: "multimodal_info",
    phase: "full",
    text: "Image analysis: detected protein structure diagram",
    summary: "Multimodal summary override",
  };
  const text = primaryDisplayText(block);
  // DISPLAY_FIRST_CHAIN: displaySummary → blockSummary → displayFields → ...
  // Since block.summary exists, it wins over rawText.
  assert.equal(
    text,
    "Multimodal summary override",
    "multimodal_info must use default DISPLAY_FIRST_CHAIN (summary before text)",
  );

  const blockNoSummary: OutputBlock = {
    kind: "multimodal_info",
    phase: "full",
    text: "Image analysis: detected protein structure diagram",
  };
  const textNoSummary = primaryDisplayText(blockNoSummary);
  assert.equal(
    textNoSummary,
    "Image analysis: detected protein structure diagram",
    "multimodal_info without display/summary must fall through to rawText via DISPLAY_FIRST_CHAIN",
  );
}

// ── P2-3: Identity contract tests — tool_call vs tool_result routing ──

// P2-3-Contract-1: REAL tool_call blocks must be excluded from the transcript
// feed per P1-5 isTranscriptBearingIdentity contract.
{
  resetLiveTranscriptFeedSequence();
  const messages = applyOutputBlock(
    [],
    runningToolBlockFor("call-1", "running-detail"),
    true,
  );
  assert.equal(
    messages.length,
    0,
    "P1-5: tool_call identity block must NOT enter visible transcript feed",
  );
}

// P2-3-Contract-2: REAL tool_call blocks must be excluded from the live
// cache — appendLiveBlock calls shouldQueueLiveTranscriptBlock which
// filters non_transcript_live routes.
{
  let live: OutputBlock[] = [];
  live = appendLiveBlock(
    live,
    runningToolBlockFor("call-1", "running-detail"),
  );
  assert.equal(
    live.length,
    0,
    "P1-5: tool_call must NOT enter live transcript cache",
  );
}

// P2-3-Contract-3: REAL tool_result blocks must enter the transcript feed.
{
  resetLiveTranscriptFeedSequence();
  const messages = applyOutputBlock(
    [],
    {
      kind: "tool",
      phase: "end",
      role: "assistant",
      text: "result-text",
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/call-1`,
        part_kind: "tool_result",
        phase: "end",
        legacy_block_id: "call-1",
      },
    },
    true,
  );
  assert.equal(
    messages.length,
    1,
    "P1-5: tool_result identity block must enter visible transcript feed",
  );
  assert.equal(messages[0]?.text, "result-text");
}

// P2-3-Contract-4: REAL tool_result blocks must enter the live cache.
{
  let live: OutputBlock[] = [];
  live = appendLiveBlock(
    live,
    {
      kind: "tool",
      phase: "full",
      role: "assistant",
      text: "result-text",
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/call-1`,
        part_kind: "tool_result",
        phase: "snapshot",
        legacy_block_id: "call-1",
      },
    },
  );
  assert.equal(
    live.length,
    1,
    "P1-5: tool_result identity block must enter live transcript cache",
  );
  assert.equal(live[0]?.text, "result-text");
}

// P2-3-Contract-5: tool_call and tool_result with the SAME call_id must
// occupy different slots. tool_call goes to execution panel (non-transcript),
// tool_result goes to transcript. In pruneLiveBlocksCoveredByHistory,
// non-transcript blocks are ALWAYS kept (route !== "transcript" → keep).
{
  const liveBlocks: OutputBlock[] = [
    {
      kind: "tool",
      phase: "full",
      text: "running-b",
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_call/call-b`,
        part_kind: "tool_call" as const,
        phase: "snapshot" as const,
        legacy_block_id: "call-b",
      },
    },
    {
      kind: "tool",
      phase: "done",
      text: "result-b",
      live_identity: {
        message_id: "msg-1",
        part_key: `tool_result/call-b`,
        part_kind: "tool_result" as const,
        phase: "end" as const,
        legacy_block_id: "call-b",
      },
    },
  ];

  const history: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [{
      id: "tp-1",
      type: "tool_result",
      output_block: {
        kind: "tool",
        phase: "done",
        text: "result-b",
        live_identity: {
          message_id: "msg-1",
          part_key: `tool_result/call-b`,
          part_kind: "tool_result" as const,
          phase: "end" as const,
          legacy_block_id: "call-b",
        },
      },
    }],
  }];

  // pruneLiveBlocksCoveredByHistory keeps non-transcript blocks (tool_call)
  // but removes transcript blocks covered by history (tool_result).
  const pruned = pruneLiveBlocksCoveredByHistory(history, liveBlocks);
  assert.equal(
    pruned.length,
    1,
    "P1-5: tool_call in live cache must survive prune even when tool_result for same call_id is covered by history",
  );
  assert.equal(pruned[0]?.text, "running-b");
  assert.equal(pruned[0]?.live_identity?.part_kind, "tool_call");
}

// ── P2-4: Unified observation event trace tests ────────────────────────

function observationCollector(): { events: ObservationEvent[]; sink: ObservationSink } {
  const events: ObservationEvent[] = [];
  return { events, sink: (event) => events.push(event) };
}

// P2-4-T1: Message full block traverses all 5 pipeline stages in order.
{
  const collector = observationCollector();
  registerObservationSink(collector.sink);

  const block: OutputBlock = {
    kind: "message",
    phase: "full",
    role: "assistant",
    text: "Hello world",
    live_identity: {
      message_id: "msg-1",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "snapshot",
    },
  };

  applyOutputBlock([], block, true);
  registerObservationSink(null);

  const kinds = collector.events.map((e) => e.kind);
  assert.ok(
    kinds.includes("block_received"),
    "P2-4-T1: must emit block_received",
  );
  assert.ok(
    kinds.includes("block_normalized"),
    "P2-4-T1: must emit block_normalized",
  );
  assert.ok(
    kinds.includes("block_committed"),
    "P2-4-T1: must emit block_committed for message full block",
  );
  // Verify ordering: received before normalized before committed
  const recvIdx = kinds.indexOf("block_received");
  const normIdx = kinds.indexOf("block_normalized");
  const commitIdx = kinds.indexOf("block_committed");
  assert.ok(
    recvIdx < normIdx && normIdx < commitIdx,
    "P2-4-T1: event order must be received → normalized → committed",
  );
}

// P2-4-T2: Compatibility route triggers legacy_fallback_used with legacyPath="route".
{
  const collector = observationCollector();
  registerObservationSink(collector.sink);

  const compatibilityBlock: OutputBlock = {
    kind: "message",
    phase: "full",
    role: "assistant",
    text: "No identity",
    // No live_identity — routes to compatibility
  };

  applyOutputBlock([], compatibilityBlock, true);
  registerObservationSink(null);

  const routeFallback = collector.events.find(
    (e) => e.kind === "legacy_fallback_used" && e.legacyPath === "route",
  );
  assert.ok(
    routeFallback,
    "P2-4-T2: compatibility-routed block must emit legacy_fallback_used with legacyPath=route",
  );
}

// P2-4-T3: history_rebuilt emits with correct historyMessageCount.
{
  const collector = observationCollector();
  registerObservationSink(collector.sink);

  const history: MessageRecord[] = [
    { id: "hist-1", role: "assistant", parts: [] },
    { id: "hist-2", role: "user", parts: [] },
  ];

  buildFeedFromHistory(history, true);
  registerObservationSink(null);

  const rebuilt = collector.events.find((e) => e.kind === "history_rebuilt");
  assert.ok(rebuilt, "P2-4-T3: buildFeedFromHistory must emit history_rebuilt");
  assert.equal(
    rebuilt?.historyMessageCount,
    2,
    "P2-4-T3: historyMessageCount must match history.length",
  );
}

// P2-4-T4: Compatibility delta triggers both route and streaming_delta legacy events.
{
  const collector = observationCollector();
  registerObservationSink(collector.sink);

  const compatibilityDelta: OutputBlock = {
    kind: "message",
    phase: "delta",
    role: "assistant",
    text: "streaming",
    id: "old-block-1",
    // No live_identity
  };

  applyOutputBlock([], compatibilityDelta, true);
  registerObservationSink(null);

  const fallbackEvents = collector.events.filter(
    (e) => e.kind === "legacy_fallback_used",
  );
  const hasRoute = fallbackEvents.some((e) => e.legacyPath === "route");
  const hasStreamingDelta = fallbackEvents.some(
    (e) => e.legacyPath === "streaming_delta",
  );
  assert.ok(
    hasRoute,
    "P2-4-T4: compatibility delta must emit legacy fallback for route",
  );
  assert.ok(
    hasStreamingDelta,
    "P2-4-T4: compatibility delta must emit legacy fallback for streaming_delta",
  );
}

// P2-4-T5: No sink registered — pipeline functions correctly without observation.
{
  const block: OutputBlock = {
    kind: "message",
    phase: "full",
    role: "assistant",
    text: "Silent",
    live_identity: {
      message_id: "msg-s",
      part_key: ASSISTANT_TEXT_MAIN_PART_KEY,
      part_kind: "assistant_text",
      phase: "snapshot",
    },
  };

  const messages = applyOutputBlock([], block, true);
  assert.ok(
    messages.length > 0,
    "P2-4-T5: pipeline must produce correct output when no observation sink is registered",
  );
}
