use agendao_provider::{
    assemble_tool_calls, parse_ethnopic_value, parse_openai_value, pipeline_to_stream_result,
    runtime::pipeline::Pipeline, ProviderError, StreamEvent, StreamResult,
};
use bytes::Bytes;
use futures::StreamExt;

fn json_events_signature(events: Vec<StreamEvent>) -> Vec<String> {
    let mut non_usage = Vec::new();
    let mut usage = Vec::new();
    for event in events {
        match event {
            StreamEvent::Usage { .. } => usage.push(event),
            _ => non_usage.push(event),
        }
    }
    non_usage.extend(usage);
    non_usage
        .into_iter()
        .map(|event| serde_json::to_string(&event).expect("event should serialize"))
        .collect()
}

async fn collect_stream(stream: StreamResult) -> Vec<StreamEvent> {
    stream
        .map(|item| item.expect("stream item should be ok"))
        .collect::<Vec<_>>()
        .await
}

fn build_sse_payload(frames: &[serde_json::Value]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(&frame.to_string());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

async fn legacy_openai_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let raw_events: Vec<StreamEvent> = frames
        .iter()
        .flat_map(|frame| parse_openai_value(frame.clone()))
        .collect();
    let stream = futures::stream::iter(raw_events.into_iter().map(Ok::<_, ProviderError>));
    collect_stream(assemble_tool_calls(Box::pin(stream))).await
}

async fn pipeline_openai_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let payload = build_sse_payload(frames);
    let bytes_stream =
        futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(payload))]);
    let pipeline = Pipeline::openai_default();
    let driver_stream = pipeline.process_stream(Box::pin(bytes_stream));
    collect_stream(pipeline_to_stream_result(driver_stream)).await
}

async fn legacy_ethnopic_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let raw_events: Vec<StreamEvent> = frames
        .iter()
        .filter_map(|frame| parse_ethnopic_value(frame.clone()))
        .collect();
    let stream = futures::stream::iter(raw_events.into_iter().map(Ok::<_, ProviderError>));
    collect_stream(assemble_tool_calls(Box::pin(stream))).await
}

async fn pipeline_ethnopic_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let payload = build_sse_payload(frames);
    let bytes_stream =
        futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(payload))]);
    let pipeline = Pipeline::ethnopic_default();
    let driver_stream = pipeline.process_stream(Box::pin(bytes_stream));
    collect_stream(pipeline_to_stream_result(driver_stream)).await
}

#[tokio::test]
async fn openai_text_stream_compliance() {
    let frames = vec![
        serde_json::json!({
            "choices": [{
                "delta": { "content": "Hello" },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 2
            }
        }),
    ];

    let legacy = legacy_openai_events(&frames).await;
    let pipeline = pipeline_openai_events(&frames).await;

    assert_eq!(
        json_events_signature(pipeline),
        json_events_signature(legacy)
    );
}

#[tokio::test]
async fn openai_tool_call_stream_compliance() {
    let frames = vec![
        serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_0",
                        "function": {
                            "name": "read",
                            "arguments": "{\"path\":\"/tmp/file\"}"
                        }
                    }]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 1
            }
        }),
    ];

    let legacy = legacy_openai_events(&frames).await;
    let pipeline = pipeline_openai_events(&frames).await;

    assert_eq!(
        json_events_signature(pipeline),
        json_events_signature(legacy)
    );
}

#[tokio::test]
async fn ethnopic_mixed_stream_compliance() {
    let frames = vec![
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "call_0",
                "name": "read"
            }
        }),
        serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"path\":\"/tmp/file\"}"
            }
        }),
        serde_json::json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {
                "type": "text_delta",
                "text": "done"
            }
        }),
        serde_json::json!({
            "type": "message_stop"
        }),
    ];

    let legacy = legacy_ethnopic_events(&frames).await;
    let pipeline = pipeline_ethnopic_events(&frames).await;

    assert_eq!(
        json_events_signature(pipeline),
        json_events_signature(legacy)
    );
}

#[tokio::test]
async fn malformed_json_flush_recovery() {
    let stream = futures::stream::iter(vec![
        Ok::<_, ProviderError>(StreamEvent::ToolCallStart {
            id: "tool-call-0".to_string(),
            name: "read".to_string(),
        }),
        Ok::<_, ProviderError>(StreamEvent::ToolCallDelta {
            id: "tool-call-0".to_string(),
            input: "{\"path\":\"/tmp/file\"".to_string(),
        }),
        Ok::<_, ProviderError>(StreamEvent::Done),
    ]);

    let output = collect_stream(assemble_tool_calls(Box::pin(stream))).await;
    let tool_end = output.into_iter().find_map(|event| match event {
        StreamEvent::ToolCallEnd { input, .. } => Some(input),
        _ => None,
    });

    assert_eq!(
        tool_end,
        Some(serde_json::Value::String(
            "{\"path\":\"/tmp/file\"".to_string()
        ))
    );
}

/// Regression: DashScope-style SSE with `event:xxx\ndata:{...}` multi-line frames
/// and `data:` without trailing space.
#[tokio::test]
async fn dashscope_multiline_sse_frame_pipeline() {
    // Simulate DashScope raw SSE: "event:content_block_delta\ndata:{...}\n\n"
    let payload = concat!(
        "event:message_start\n",
        "data:{\"message\":{\"model\":\"qwen3.5-plus\",\"id\":\"msg_1\",\"role\":\"assistant\",\"type\":\"message\",\"content\":[],\"usage\":{\"input_tokens\":5,\"output_tokens\":0}},\"type\":\"message_start\"}\n",
        "\n",
        "event:content_block_start\n",
        "data:{\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"},\"index\":0}\n",
        "\n",
        "event:content_block_delta\n",
        "data:{\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"},\"type\":\"content_block_delta\",\"index\":0}\n",
        "\n",
        "event:message_stop\n",
        "data:{\"type\":\"message_stop\"}\n",
        "\n",
    );

    let bytes_stream =
        futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(payload))]);
    let pipeline = Pipeline::ethnopic_default();
    let driver_stream = pipeline.process_stream(Box::pin(bytes_stream));
    let events = collect_stream(pipeline_to_stream_result(driver_stream)).await;

    // Must contain at least one TextDelta with "Hello"
    let has_text = events
        .iter()
        .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Hello"));
    assert!(
        has_text,
        "pipeline should parse multi-line SSE frames from DashScope; events: {:?}",
        events
    );
}

/// Regression: legacy SSE decoder must also handle multi-line frames.
#[tokio::test]
async fn dashscope_multiline_sse_frame_legacy() {
    use agendao_provider::decode_sse_stream;

    let payload = concat!(
        "event:content_block_delta\n",
        "data:{\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"},\"type\":\"content_block_delta\",\"index\":0}\n",
        "\n",
        "event:message_stop\n",
        "data:{\"type\":\"message_stop\"}\n",
        "\n",
    );

    let bytes_stream =
        futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(Bytes::from(payload))]);
    let json_stream = decode_sse_stream(bytes_stream)
        .await
        .expect("decode should succeed");

    let values: Vec<serde_json::Value> = json_stream
        .map(|item| item.expect("stream item"))
        .collect::<Vec<_>>()
        .await;

    assert_eq!(
        values.len(),
        2,
        "should parse 2 JSON values from multi-line SSE frames"
    );
    assert_eq!(
        values[0]["delta"]["text"].as_str(),
        Some("Hi"),
        "first frame should contain text delta"
    );
    assert_eq!(
        values[1]["type"].as_str(),
        Some("message_stop"),
        "second frame should be message_stop"
    );
}
