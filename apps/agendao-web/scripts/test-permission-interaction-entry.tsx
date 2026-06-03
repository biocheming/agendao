import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { InteractionOverlays } from "../src/components/InteractionOverlays";

const html = renderToStaticMarkup(
  React.createElement(InteractionOverlays, {
    question: null,
    permission: {
      permission_id: "perm-1",
      session_id: "sess-1",
      message: "Allow cargo test?",
      permission: "bash",
      supported_lifetimes: ["once", "turn"],
      permission_class: "dangerous_exec",
      scope_label: "Shell commands: cargo",
      matcher_label: "Command family: cargo *",
      grant_target_summary: "Shell commands: cargo",
      risk_tags: ["dangerous_exec"],
      command: "cargo test",
    },
    questionAnswers: {},
    questionSubmitting: false,
    permissionSubmitting: false,
    permissionSubmitError: "network down",
    permissionSubmitStartedAt: "2026-05-17T10:00:00Z",
    permissionSubmitCompletedAt: "2026-05-17T10:00:02Z",
    onQuestionAnswerChange: () => {},
    onRejectQuestion: () => {},
    onSubmitQuestion: () => {},
    onReplyPermission: () => {},
  }),
);

assert.match(html, /permission-submit-error/);
assert.match(html, /network down/);
assert.match(html, /permission-submit-started/);
assert.match(html, /2026-05-17T10:00:00Z/);
assert.match(html, /permission-submit-completed/);
assert.match(html, /2026-05-17T10:00:02Z/);
assert.match(html, /Shell commands: cargo/);
assert.match(html, /Command family: cargo \*/);
assert.match(html, /dangerous_exec/);
