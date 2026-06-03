import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SessionInsightsPanel } from "../src/components/SessionInsightsPanel";

const activity = {
  telemetry: {
    runtime: {
      session_id: "sess-runtime",
      run_status: "running",
    },
    stages: [],
    topology: {
      active_count: 0,
      running_count: 0,
      waiting_count: 0,
      done_count: 0,
      roots: [],
    },
    usage: {
      input_tokens: 0,
      output_tokens: 0,
      reasoning_tokens: 0,
      cache_write_tokens: 0,
      cache_read_tokens: 0,
      cache_miss_tokens: 0,
      total_cost: 0,
    },
    tool_result_governance: {
      single_result_governed_count: 3,
      batch_governed_count: 2,
      transcript_fallback_count: 1,
      artifact_fallback_count: 2,
      total_original_chars: 150_000,
      total_displayed_chars: 24_000,
    },
  },
  sessionInsights: {
    id: "sess-insights",
    title: "Governance Regression",
    directory: "/tmp/project",
    updated: 1_715_000_000_000,
    telemetry: {
      version: "v5",
      usage: {
        input_tokens: 10,
        output_tokens: 20,
        reasoning_tokens: 5,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        cache_miss_tokens: 0,
        total_cost: 0,
      },
      stage_summaries: [],
      tool_result_governance: {
        single_result_governed_count: 0,
        batch_governed_count: 0,
        transcript_fallback_count: 0,
        artifact_fallback_count: 0,
        total_original_chars: 0,
        total_displayed_chars: 0,
      },
      last_run_status: "completed",
      updated_at: 1_715_000_000_000,
    },
  },
  sessionUsage: null,
  activeStageSummary: null,
  activityFilters: {
    stageId: "",
    executionId: "",
    eventType: "",
  },
  activityPage: 1,
  activityLoading: false,
  refreshExecutionActivity: async () => {},
};

const html = renderToStaticMarkup(
  React.createElement(SessionInsightsPanel, {
    activity,
    apiJson: async () => {
      throw new Error("apiJson should not be called in governance render test");
    },
  }),
);

assert.match(html, /Tool Result Governance/);
assert.match(html, /single 3/);
assert.match(html, /batch 2/);
assert.match(html, /transcript fallback 1/);
assert.match(html, /artifact 2/);
assert.match(html, /150,000/);
assert.match(html, /24,000/);
assert.doesNotMatch(html, /single 0/);
assert.doesNotMatch(html, /batch 0/);
assert.doesNotMatch(html, /transcript fallback 0/);
