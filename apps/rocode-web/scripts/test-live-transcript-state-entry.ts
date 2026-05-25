import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import type { MessageRecord, OutputBlock } from "../src/lib/history";
import {
  appendLiveBlock,
  applyOutputBlock,
  mergeHistoryWithLiveBlocks,
  normalizeBlockText,
  pruneLiveBlocksCoveredByHistory,
  resetLiveTranscriptFeedSequence,
  shouldQueueLiveTranscriptBlock,
} from "../src/lib/liveTranscriptState";
import {
  canonicalLiveExecutionStatus,
  partitionLiveExecutions,
} from "../src/lib/liveExecutionState";
import { buildRunTailSummary } from "../src/lib/runTailSummary";
import { toolActivityLabel, toolKindLabel } from "../src/lib/toolLabels";

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
      part_key: "tool_result/tool-call-1",
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
      part_key: "text/main",
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
  let visible = applyOutputBlock([], toolBlock({ id: undefined, text: '{"category":"a"}' }), true);
  visible = applyOutputBlock(visible, toolBlock({ id: undefined, text: '{"category":"b"}' }), true);

  assert.equal(visible.length, 1, "visible feed should upsert same tool snapshot instead of duplicating");
  assert.equal(visible[0]?.text, '{"category":"b"}');
  assert.equal(visible[0]?.id, "tool-call-1");
}

{
  const contractText = normalizeBlockText(
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
  const contractFieldText = normalizeBlockText({
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
  const compatibilityText = normalizeBlockText({
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
      part_key: "text/main",
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
  const visible = applyOutputBlock(
    [],
    {
      kind: "session_event",
      event: "subtask",
      title: "Subtask · inspect scheduler",
      status: "pending",
      body: "delegated",
    },
    true,
  );
  assert.equal(visible.length, 1);
  assert.equal(visible[0]?.kind, "session_event");
}

{
  resetLiveTranscriptFeedSequence();
  const visible = applyOutputBlock(
    [],
    {
      kind: "inspect",
      summary: "2 stage events",
      body: "stage-1\nstage-2",
    },
    true,
  );
  assert.equal(visible.length, 1);
  assert.equal(visible[0]?.kind, "inspect");
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
      part_key: "text/main",
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
      part_key: "text/main",
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
      part_key: "reasoning/main",
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
      part_key: "reasoning/main",
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

{
  resetLiveTranscriptFeedSequence();
  let visible = applyOutputBlock([], toolBlock({ id: undefined, phase: "start", text: "" }), true);
  visible = applyOutputBlock(
    visible,
    toolBlock({
      id: undefined,
      phase: "running",
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
      text: fixture.tool_progress_exclusion.tool_running.tool_detail,
    }),
    true,
  );
  visible = applyOutputBlock(
    visible,
    toolBlock({
      id: undefined,
      phase: "running",
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
      text: fixture.tool_progress_exclusion.tool_running.tool_detail,
    }),
    true,
  );

  assert.equal(
    visible.length,
    0,
    "tool running detail is progress-state and must not enter authoritative visible transcript feed",
  );
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
  const emptyAssistantStart: OutputBlock = {
    kind: "message",
    phase: "start",
    role: "assistant",
    text: "",
    live_identity: {
      message_id: "assistant-1",
      part_key: "text/main",
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
      part_key: "text/main",
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

{
  let liveBlocks: OutputBlock[] = [];
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlock({
      id: undefined,
      phase: "running",
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
      text: fixture.tool_progress_exclusion.tool_running.tool_detail,
    }),
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlock({
      id: undefined,
      phase: "running",
      title: fixture.tool_progress_exclusion.tool_running.tool_name,
      text: fixture.tool_progress_exclusion.tool_running.tool_detail,
    }),
  );

  assert.equal(
    liveBlocks.length,
    0,
    "tool running detail is progress-state and must not enter retained live transcript cache",
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
    toolBlock({ id: undefined, phase: "running", text: '{"category":"literature-research/skills"}' }),
  );
  liveBlocks = appendLiveBlock(
    liveBlocks,
    toolBlock({ id: undefined, phase: "running", text: '{"category":"scientific-skills"}' }),
  );

  const rebuilt = mergeHistoryWithLiveBlocks(history, liveBlocks, true);
  const toolMessages = rebuilt.filter((message) => message.kind === "tool");

  assert.equal(
    toolMessages.length,
    0,
    "rebuilt feed must exclude running tool detail because retained live cache no longer treats it as transcript authority",
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
      parts: [{ id: "text-part-1", type: "text", text: "final answer" }],
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
        part_key: "text/main",
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
      parts: [{ id: "reasoning-part-1", type: "reasoning", text: "main reasoning" }],
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
        part_key: "reasoning/main",
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
        part_key: "reasoning/branch-a",
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
    "history without output_block.live_identity must only prune the canonical reasoning/main slot",
  );
  assert.equal(
    pruned[0]?.live_identity?.part_key,
    "reasoning/branch-a",
    "history without output_block.live_identity must not over-prune non-main reasoning branches",
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
        part_key: "reasoning/main",
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
    reasoningWithPartKey("msg-1", "reasoning/main", "main thinking", "full"),
  );
  assert.equal(afterMain.length, 1, "reasoning/main must insert live block");

  const afterBranch = appendLiveBlock(
    afterMain,
    reasoningWithPartKey("msg-1", "reasoning/branch-a", "branch analysis", "full"),
  );
  assert.equal(
    afterBranch.length,
    2,
    "reasoning/branch-a must not collide with reasoning/main in live cache",
  );
  assert.equal(afterBranch[0]?.text, "main thinking");
  assert.equal(afterBranch[1]?.text, "branch analysis");

  // Updating reasoning/main must not affect reasoning/branch-a.
  const afterMainUpdate = appendLiveBlock(
    afterBranch,
    reasoningWithPartKey("msg-1", "reasoning/main", "main thinking revised", "full"),
  );
  assert.equal(
    afterMainUpdate.length,
    2,
    "updating reasoning/main must not delete reasoning/branch-a",
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
    reasoningWithPartKey("msg-1", "reasoning/main", "main thinking", "full"),
    true,
  );
  assert.equal(messages.length, 1, "first reasoning part must insert");

  messages = applyOutputBlock(
    messages,
    reasoningWithPartKey("msg-1", "reasoning/branch-a", "branch analysis", "full"),
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
    reasoningWithPartKey("msg-1", "reasoning/main", "main thinking", "full"),
    reasoningWithPartKey("msg-1", "reasoning/branch-a", "branch analysis", "full"),
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

// T3-adapted: persisted history parts without output_block.live_identity
// still cover the canonical main slot for assistant text/reasoning.
// This is a deliberate backward-compatibility fallback, not a generic
// "incomplete history never prunes" rule.
{
  const liveBlocks: OutputBlock[] = [
    assistantMessageBlock("msg-1", "complete live text", { phase: "full" }),
  ];

  // Persisted history has a text part but no output_block/live_identity.
  // Fallback slot inference must still cover text/main.
  const persistedTextHistory: MessageRecord[] = [{
    id: "msg-1",
    role: "assistant",
    parts: [{ id: "p1", type: "text", text: "partial" }],
  }];

  const pruned = pruneLiveBlocksCoveredByHistory(persistedTextHistory, liveBlocks);
  // The history part has no output_block, so slotKey inference uses
  // "msg-1:text/main". The live block has live_identity with part_key
  // "text/main". slotKey matches, so the canonical text slot is absorbed.
  assert.equal(pruned.length, 0, "persisted text part must cover canonical text/main live block");

  // Persisted reasoning-only history must not prune the text/main slot.
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
    "persisted reasoning-only history must not prune canonical text/main live block",
  );
  assert.equal(prunedReasoningOnly[0]?.text, "complete live text");
}
