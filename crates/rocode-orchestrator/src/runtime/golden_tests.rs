#[cfg(test)]
mod tests {
    use crate::runtime::events::*;
    use crate::runtime::policy::*;
    use crate::runtime::run_loop;
    use crate::runtime::traits::*;
    use async_trait::async_trait;
    use futures::{stream, StreamExt};
    use rocode_provider::{StreamEvent, StreamResult, ToolDefinition};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    // =====================================================================
    // Fake implementations – scriptable, deterministic, reproducible.
    // =====================================================================

    /// FakeModelCaller returns pre-configured streams in sequence.
    struct FakeModelCaller {
        streams: Mutex<Vec<Vec<StreamEvent>>>,
    }

    impl FakeModelCaller {
        fn new(streams: Vec<Vec<StreamEvent>>) -> Self {
            // Reverse so we can pop from the end.
            let mut s = streams;
            s.reverse();
            Self {
                streams: Mutex::new(s),
            }
        }
    }

    #[async_trait]
    impl ModelCaller for FakeModelCaller {
        async fn call_stream(&self, _req: LoopRequest) -> Result<StreamResult, LoopError> {
            let events = self
                .streams
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| LoopError::Other("no more fake streams".into()))?;
            Ok(Box::pin(stream::iter(
                events
                    .into_iter()
                    .map(Ok::<_, rocode_provider::ProviderError>),
            )))
        }
    }

    /// FakeToolDispatcher returns pre-configured results per tool name.
    struct FakeToolDispatcher {
        definitions: Vec<ToolDefinition>,
        /// (tool_name) -> (output, is_error)
        results: std::collections::HashMap<String, (String, bool)>,
        /// Records all executed calls in order.
        executed: Mutex<Vec<(String, String, serde_json::Value)>>,
    }

    impl FakeToolDispatcher {
        fn new() -> Self {
            Self {
                definitions: vec![
                    ToolDefinition {
                        name: "read".into(),
                        description: Some("read a file".into()),
                        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
                    },
                    ToolDefinition {
                        name: "write".into(),
                        description: Some("write a file".into()),
                        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}}),
                    },
                ],
                results: std::collections::HashMap::new(),
                executed: Mutex::new(Vec::new()),
            }
        }

        fn with_result(mut self, tool: &str, output: &str, is_error: bool) -> Self {
            self.results.insert(tool.into(), (output.into(), is_error));
            self
        }

        fn executed_calls(&self) -> Vec<(String, String, serde_json::Value)> {
            self.executed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ToolDispatcher for FakeToolDispatcher {
        async fn execute(&self, call: &ToolCallReady) -> ToolResult {
            self.executed.lock().unwrap().push((
                call.id.clone(),
                call.name.clone(),
                call.arguments.clone(),
            ));

            let (output, is_error) = self
                .results
                .get(&call.name)
                .cloned()
                .unwrap_or_else(|| (format!("tool:{}:ok", call.name), false));

            ToolResult {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                output,
                is_error,
                title: None,
                metadata: None,
            }
        }

        async fn list_definitions(&self) -> Vec<ToolDefinition> {
            self.definitions.clone()
        }
    }

    /// RecordingSink captures all events for golden comparison.
    #[derive(Default)]
    struct RecordingSink {
        events: Vec<LoopEvent>,
        tool_results: Vec<(String, String, String, bool)>, // (call_id, name, output, is_error)
        step_boundaries: Vec<StepBoundary>,
    }

    #[async_trait]
    impl LoopSink for RecordingSink {
        async fn on_event(&mut self, ev: &LoopEvent) -> Result<(), LoopError> {
            self.events.push(ev.clone());
            Ok(())
        }

        async fn on_tool_result(
            &mut self,
            call: &ToolCallReady,
            result: &ToolResult,
        ) -> Result<(), LoopError> {
            self.tool_results.push((
                call.id.clone(),
                call.name.clone(),
                result.output.clone(),
                result.is_error,
            ));
            Ok(())
        }

        async fn on_step_boundary(&mut self, ctx: &StepBoundary) -> Result<(), LoopError> {
            self.step_boundaries.push(ctx.clone());
            Ok(())
        }
    }

    fn default_policy() -> LoopPolicy {
        LoopPolicy {
            max_steps: Some(10),
            tool_dedup: ToolDedupScope::Global,
            on_tool_error: ToolErrorStrategy::ReportAndContinue,
        }
    }

    fn user_msg(text: &str) -> rocode_provider::Message {
        rocode_provider::Message::user(text.to_string())
    }

    // =====================================================================
    // Golden tests
    // =====================================================================

    /// Fixture 1: Pure text response – no tool calls.
    /// Expected: 1 step, EndTurn, content = "hello world".
    #[tokio::test]
    async fn golden_pure_text_response() {
        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::TextDelta("hello ".into()),
            StreamEvent::TextDelta("world".into()),
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("hi")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.content, "hello world");
        assert_eq!(outcome.total_steps, 1);
        assert_eq!(outcome.total_tool_calls, 0);
        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);

        // Sink received text chunks.
        let text_chunks: Vec<_> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                LoopEvent::TextChunk(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_chunks, vec!["hello ", "world"]);

        // No tool results.
        assert!(sink.tool_results.is_empty());

        // Step boundaries: Start(1) + End(1).
        assert_eq!(sink.step_boundaries.len(), 2);
        assert!(matches!(
            &sink.step_boundaries[0],
            StepBoundary::Start { step: 1 }
        ));
        assert!(matches!(
            &sink.step_boundaries[1],
            StepBoundary::End {
                step: 1,
                finish_reason: FinishReason::EndTurn,
                tool_calls_count: 0,
                ..
            }
        ));
    }

    /// Fixture 2: Single tool call → model finishes.
    /// Expected: 2 steps, 1 tool call, EndTurn.
    #[tokio::test]
    async fn golden_single_tool_call() {
        let model = FakeModelCaller::new(vec![
            // Step 1: model calls read tool.
            vec![
                StreamEvent::TextDelta("Let me read that.".into()),
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/tmp/a.txt"}),
                },
                StreamEvent::Done,
            ],
            // Step 2: model responds with final text.
            vec![
                StreamEvent::TextDelta("File contains: hello".into()),
                StreamEvent::Done,
            ],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "hello", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("read /tmp/a.txt")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.content, "File contains: hello");
        assert_eq!(outcome.total_steps, 2);
        assert_eq!(outcome.total_tool_calls, 1);
        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);

        // Tool was executed exactly once.
        let calls = tools.executed_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tc-1");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2, json!({"path": "/tmp/a.txt"}));

        // Sink received tool result.
        assert_eq!(sink.tool_results.len(), 1);
        assert_eq!(sink.tool_results[0].0, "tc-1");
        assert_eq!(sink.tool_results[0].2, "hello");
        assert!(!sink.tool_results[0].3); // not error
    }

    /// Fixture 3: Multi-step tool loop (read → write → done).
    /// Expected: 3 steps, 2 tool calls.
    #[tokio::test]
    async fn golden_multi_step_tool_loop() {
        let model = FakeModelCaller::new(vec![
            // Step 1: read
            vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/tmp/in.txt"}),
                },
                StreamEvent::Done,
            ],
            // Step 2: write
            vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-2".into(),
                    name: "write".into(),
                    input: json!({"path": "/tmp/out.txt", "content": "transformed"}),
                },
                StreamEvent::Done,
            ],
            // Step 3: done
            vec![StreamEvent::TextDelta("Done!".into()), StreamEvent::Done],
        ]);
        let tools = FakeToolDispatcher::new()
            .with_result("read", "raw data", false)
            .with_result("write", "ok", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("transform file")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.content, "Done!");
        assert_eq!(outcome.total_steps, 3);
        assert_eq!(outcome.total_tool_calls, 2);
        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);

        // Both tools executed in order.
        let calls = tools.executed_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[1].1, "write");

        // Step boundaries: 3 starts + 3 ends = 6 total.
        assert_eq!(sink.step_boundaries.len(), 6);
    }

    /// Fixture 4: Empty tool name is filtered.
    /// Expected: model's empty-name tool call is ignored, loop still finishes.
    #[tokio::test]
    async fn golden_empty_tool_name_filtered() {
        let model = FakeModelCaller::new(vec![
            // Step 1: model emits a tool call with empty name + valid tool call.
            vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-bad".into(),
                    name: "  ".into(),
                    input: json!({}),
                },
                StreamEvent::ToolCallEnd {
                    id: "tc-good".into(),
                    name: "read".into(),
                    input: json!({"path": "/tmp/x"}),
                },
                StreamEvent::Done,
            ],
            // Step 2: done
            vec![StreamEvent::TextDelta("ok".into()), StreamEvent::Done],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "data", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        // Only the valid tool call was executed.
        let calls = tools.executed_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tc-good");
        assert_eq!(outcome.total_tool_calls, 1);
    }

    /// Fixture 5: Max steps exceeded.
    /// Expected: FinishReason::MaxSteps after policy.max_steps.
    #[tokio::test]
    async fn golden_max_steps_exceeded() {
        // Model always returns a tool call, never finishes.
        let mut streams = Vec::new();
        for i in 0..5 {
            streams.push(vec![
                StreamEvent::ToolCallEnd {
                    id: format!("tc-{}", i),
                    name: "read".into(),
                    input: json!({"path": "/tmp/loop"}),
                },
                StreamEvent::Done,
            ]);
        }
        let model = FakeModelCaller::new(streams);
        let tools = FakeToolDispatcher::new().with_result("read", "data", false);
        let mut sink = RecordingSink::default();

        let policy = LoopPolicy {
            max_steps: Some(3),
            ..default_policy()
        };

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            &NeverCancel,
            vec![user_msg("loop forever")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.finish_reason, FinishReason::MaxSteps);
        assert_eq!(outcome.total_steps, 3);
        assert_eq!(outcome.total_tool_calls, 3);
    }

    /// Fixture 6: Error event from model stream.
    /// Expected: error is reported to sink but loop continues to next step
    /// if the stream ends naturally.
    #[tokio::test]
    async fn golden_error_event() {
        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::TextDelta("partial ".into()),
            StreamEvent::Error("model overloaded".into()),
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        // Model returned no tool calls, so loop ends.
        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);
        assert_eq!(outcome.content, "partial ");

        // Error event was passed to sink.
        let errors: Vec<_> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                LoopEvent::Error(msg) => Some(msg.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(errors, vec!["model overloaded"]);

        // Step end reports had_error = true.
        let end = &sink.step_boundaries[1];
        assert!(matches!(
            end,
            StepBoundary::End {
                had_error: true,
                ..
            }
        ));
    }

    /// Fixture 6b: Provider stream error (Err from stream) aborts loop.
    /// Expected: LoopError::ModelError returned, StepBoundary::End emitted.
    #[tokio::test]
    async fn golden_provider_stream_error_aborts() {
        use futures::stream;

        // Custom model that produces a stream with an Err item.
        struct ErrorStreamModel;

        #[async_trait]
        impl ModelCaller for ErrorStreamModel {
            async fn call_stream(&self, _req: LoopRequest) -> Result<StreamResult, LoopError> {
                let items: Vec<Result<StreamEvent, rocode_provider::ProviderError>> = vec![
                    Ok(StreamEvent::TextDelta("partial ".into())),
                    Err(rocode_provider::ProviderError::NetworkError(
                        "connection reset".into(),
                    )),
                ];
                Ok(Box::pin(stream::iter(items)))
            }
        }

        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let result = run_loop(
            &ErrorStreamModel,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await;

        // Loop should abort with ModelError, not return Ok(EndTurn).
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), LoopError::ModelError(msg) if msg.contains("connection reset"))
        );

        // Error event was dispatched to sink before abort.
        let errors: Vec<_> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                LoopEvent::Error(msg) => Some(msg.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("connection reset"));

        // StepBoundary::End was emitted with Error finish_reason.
        let end_boundaries: Vec<_> = sink
            .step_boundaries
            .iter()
            .filter(|b| matches!(b, StepBoundary::End { .. }))
            .collect();
        assert_eq!(end_boundaries.len(), 1);
        assert!(matches!(
            end_boundaries[0],
            StepBoundary::End {
                had_error: true,
                ..
            }
        ));
    }

    /// Fixture 7: Reasoning events mixed with text.
    /// Expected: reasoning chunks appear in sink events alongside text chunks.
    #[tokio::test]
    async fn golden_reasoning_events() {
        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::ReasoningStart { id: "r-1".into() },
            StreamEvent::ReasoningDelta {
                id: "r-1".into(),
                text: "thinking about this...".into(),
            },
            StreamEvent::ReasoningEnd { id: "r-1".into() },
            StreamEvent::TextDelta("The answer is 42".into()),
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("what is the answer?")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.content, "The answer is 42");

        // Reasoning chunk was captured.
        let reasoning: Vec<_> = sink
            .events
            .iter()
            .filter_map(|e| match e {
                LoopEvent::ReasoningChunk { id, text } => Some((id.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(reasoning, vec![("r-1", "thinking about this...")]);
    }

    #[tokio::test]
    async fn golden_reasoning_is_replayed_into_followup_model_request() {
        struct CapturingModelCaller {
            streams: Mutex<Vec<Vec<StreamEvent>>>,
            requests: Mutex<Vec<LoopRequest>>,
        }

        impl CapturingModelCaller {
            fn new(streams: Vec<Vec<StreamEvent>>) -> Self {
                let mut reversed = streams;
                reversed.reverse();
                Self {
                    streams: Mutex::new(reversed),
                    requests: Mutex::new(Vec::new()),
                }
            }

            fn requests(&self) -> Vec<LoopRequest> {
                self.requests.lock().unwrap().clone()
            }
        }

        #[async_trait]
        impl ModelCaller for CapturingModelCaller {
            async fn call_stream(&self, req: LoopRequest) -> Result<StreamResult, LoopError> {
                self.requests.lock().unwrap().push(req);
                let events = self
                    .streams
                    .lock()
                    .unwrap()
                    .pop()
                    .ok_or_else(|| LoopError::Other("no more fake streams".into()))?;
                Ok(Box::pin(stream::iter(
                    events
                        .into_iter()
                        .map(Ok::<_, rocode_provider::ProviderError>),
                )))
            }
        }

        let model = CapturingModelCaller::new(vec![
            vec![
                StreamEvent::ReasoningDelta {
                    id: "r-1".into(),
                    text: "need tool output first".into(),
                },
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/tmp/x"}),
                },
                StreamEvent::Done,
            ],
            vec![StreamEvent::TextDelta("done".into()), StreamEvent::Done],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "file data", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("inspect /tmp/x")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);
        assert_eq!(outcome.total_steps, 2);

        let requests = model.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].messages.len(), 3);

        match &requests[1].messages[1].content {
            rocode_provider::Content::Parts(parts) => {
                assert_eq!(parts[0].content_type, "reasoning");
                assert_eq!(parts[0].text.as_deref(), Some("need tool output first"));
                assert_eq!(parts[1].content_type, "tool_use");
            }
            other => panic!("expected assistant parts in second request, got {other:?}"),
        }
    }

    /// Fixture 8: Cancellation at checkpoint 1 (before model call).
    #[tokio::test]
    async fn golden_cancel_before_model_call() {
        struct ImmediateCancel;
        impl CancelToken for ImmediateCancel {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::TextDelta("should not see this".into()),
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &ImmediateCancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.finish_reason, FinishReason::Cancelled);
        // No events should have been sent to sink (cancelled before model call).
        assert!(sink.events.is_empty());
    }

    /// Fixture 9: Cancellation at checkpoint 3 (before tool dispatch).
    #[tokio::test]
    async fn golden_cancel_before_tool_dispatch() {
        // Cancel after stream is consumed but before tool execution.
        struct CancelAfterStream {
            stream_consumed: Arc<Mutex<bool>>,
        }
        impl CancelToken for CancelAfterStream {
            fn is_cancelled(&self) -> bool {
                // Return false during stream consumption, true before tool dispatch.
                *self.stream_consumed.lock().unwrap()
            }
        }

        // We need a custom model that sets the flag after stream ends.
        struct CancellingModel {
            inner: FakeModelCaller,
            flag: Arc<Mutex<bool>>,
        }

        #[async_trait]
        impl ModelCaller for CancellingModel {
            async fn call_stream(&self, req: LoopRequest) -> Result<StreamResult, LoopError> {
                let stream = self.inner.call_stream(req).await?;
                let flag = self.flag.clone();
                // Wrap stream to set flag after Done.
                Ok(Box::pin(stream.inspect(move |event| {
                    if let Ok(StreamEvent::Done) = event {
                        *flag.lock().unwrap() = true;
                    }
                })))
            }
        }

        let flag = Arc::new(Mutex::new(false));
        let model = CancellingModel {
            inner: FakeModelCaller::new(vec![vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/tmp/x"}),
                },
                StreamEvent::Done,
            ]]),
            flag: flag.clone(),
        };

        let cancel = CancelAfterStream {
            stream_consumed: flag,
        };

        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &cancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.finish_reason, FinishReason::Cancelled);
        // Tool was NOT executed.
        assert!(tools.executed_calls().is_empty());
        // StepBoundary::End was emitted even on cancel (P2-1 fix).
        let end_boundaries: Vec<_> = sink
            .step_boundaries
            .iter()
            .filter(|b| matches!(b, StepBoundary::End { .. }))
            .collect();
        assert_eq!(end_boundaries.len(), 1);
        assert!(matches!(
            end_boundaries[0],
            StepBoundary::End {
                finish_reason: FinishReason::Cancelled,
                ..
            }
        ));
    }

    /// Fixture 10: tool_call_id dedup (Global scope).
    #[tokio::test]
    async fn golden_tool_call_id_dedup_global() {
        let model = FakeModelCaller::new(vec![
            // Step 1: two tool calls, one with duplicate id.
            vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/a"}),
                },
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(), // duplicate!
                    name: "read".into(),
                    input: json!({"path": "/b"}),
                },
                StreamEvent::Done,
            ],
            // Step 2: done
            vec![StreamEvent::TextDelta("done".into()), StreamEvent::Done],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "data", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        // total_tool_calls counts all model-requested calls (2), not dispatches (1).
        assert_eq!(outcome.total_tool_calls, 2);
        let calls = tools.executed_calls();
        assert_eq!(calls.len(), 1); // only 1 actual dispatch (second was deduped)
        assert_eq!(calls[0].2, json!({"path": "/a"})); // first one wins
                                                       // Sink was notified for both: dispatched + deduped (P2-2 fix).
        assert_eq!(sink.tool_results.len(), 2);
        assert!(!sink.tool_results[0].3); // dispatched: not error
        assert!(!sink.tool_results[1].3); // deduped: not error
        assert!(sink.tool_results[1].2.contains("skipped"));
    }

    /// Fixture 11: Tool error with ReportAndContinue policy.
    #[tokio::test]
    async fn golden_tool_error_report_and_continue() {
        let model = FakeModelCaller::new(vec![
            vec![
                StreamEvent::ToolCallEnd {
                    id: "tc-1".into(),
                    name: "read".into(),
                    input: json!({"path": "/nonexistent"}),
                },
                StreamEvent::Done,
            ],
            vec![
                StreamEvent::TextDelta("I see the error, let me try something else.".into()),
                StreamEvent::Done,
            ],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "file not found", true);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("read /nonexistent")],
        )
        .await
        .unwrap();

        // Loop continued past the error.
        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);
        assert_eq!(outcome.total_steps, 2);

        // Error result was recorded.
        assert_eq!(sink.tool_results.len(), 1);
        assert!(sink.tool_results[0].3); // is_error
    }

    /// Fixture 12: Tool error with Fail policy.
    #[tokio::test]
    async fn golden_tool_error_fail_policy() {
        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::ToolCallEnd {
                id: "tc-1".into(),
                name: "read".into(),
                input: json!({"path": "/bad"}),
            },
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new().with_result("read", "permission denied", true);
        let mut sink = RecordingSink::default();

        let policy = LoopPolicy {
            on_tool_error: ToolErrorStrategy::Fail,
            ..default_policy()
        };

        let result = run_loop(
            &model,
            &tools,
            &mut sink,
            &policy,
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LoopError::ToolDispatchError { .. }
        ));
        // StepBoundary::End was emitted before error return (P2-1 fix).
        let end_boundaries: Vec<_> = sink
            .step_boundaries
            .iter()
            .filter(|b| matches!(b, StepBoundary::End { .. }))
            .collect();
        assert_eq!(end_boundaries.len(), 1);
        assert!(matches!(
            end_boundaries[0],
            StepBoundary::End {
                had_error: true,
                ..
            }
        ));
    }

    /// Fixture 13: ToolCallStart/Delta streaming → assembled into ToolCallReady.
    /// Tests that assemble_tool_calls integration works within run_loop.
    #[tokio::test]
    async fn golden_tool_call_assembly() {
        let model = FakeModelCaller::new(vec![
            vec![
                StreamEvent::ToolCallStart {
                    id: "tc-1".into(),
                    name: "read".into(),
                },
                StreamEvent::ToolCallDelta {
                    id: "tc-1".into(),
                    input: r#"{"path":""#.into(),
                },
                StreamEvent::ToolCallDelta {
                    id: "tc-1".into(),
                    input: r#"/tmp/x"}"#.into(),
                },
                StreamEvent::Done,
            ],
            vec![StreamEvent::TextDelta("content".into()), StreamEvent::Done],
        ]);
        let tools = FakeToolDispatcher::new().with_result("read", "file data", false);
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("read it")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.total_tool_calls, 1);
        let calls = tools.executed_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, "read");
        // Verify the assembled arguments are correct.
        assert_eq!(calls[0].2, json!({"path": "/tmp/x"}));

        // Sink received ToolCallProgress events during streaming.
        let progress_count = sink
            .events
            .iter()
            .filter(|e| matches!(e, LoopEvent::ToolCallProgress { .. }))
            .count();
        assert!(progress_count >= 1, "should have progress events");
    }

    /// Fixture 14: FinishStep with usage information.
    #[tokio::test]
    async fn golden_usage_tracking() {
        let model = FakeModelCaller::new(vec![vec![
            StreamEvent::TextDelta("hello".into()),
            StreamEvent::FinishStep {
                finish_reason: Some("stop".into()),
                usage: rocode_provider::StreamUsage {
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    context_tokens: 100,
                    reasoning_tokens: 20,
                    cache_read_tokens: 10,
                    cache_miss_tokens: 5,
                    cache_write_tokens: 5,
                },
                provider_metadata: None,
            },
            StreamEvent::Done,
        ]]);
        let tools = FakeToolDispatcher::new();
        let mut sink = RecordingSink::default();

        let outcome = run_loop(
            &model,
            &tools,
            &mut sink,
            &default_policy(),
            &NeverCancel,
            vec![user_msg("test")],
        )
        .await
        .unwrap();

        assert_eq!(outcome.finish_reason, FinishReason::EndTurn);

        // Step end should carry usage.
        let end = &sink.step_boundaries[1];
        if let StepBoundary::End { usage, .. } = end {
            let u = usage.as_ref().expect("should have usage");
            assert_eq!(u.prompt_tokens, 100);
            assert_eq!(u.completion_tokens, 50);
            assert_eq!(u.reasoning_tokens, 20);
        } else {
            panic!("expected StepBoundary::End");
        }
    }
}
