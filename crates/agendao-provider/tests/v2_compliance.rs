use agendao_provider::{
    assemble_tool_calls, parse_anthropic_value, parse_openai_value, ProviderError, StreamEvent,
    StreamResult,
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

async fn openai_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let raw_events: Vec<StreamEvent> = frames
        .iter()
        .flat_map(|frame| parse_openai_value(frame.clone()))
        .collect();
    let stream = futures::stream::iter(raw_events.into_iter().map(Ok::<_, ProviderError>));
    collect_stream(assemble_tool_calls(Box::pin(stream))).await
}

async fn anthropic_events(frames: &[serde_json::Value]) -> Vec<StreamEvent> {
    let raw_events: Vec<StreamEvent> = frames
        .iter()
        .filter_map(|frame| parse_anthropic_value(frame.clone()))
        .collect();
    let stream = futures::stream::iter(raw_events.into_iter().map(Ok::<_, ProviderError>));
    collect_stream(assemble_tool_calls(Box::pin(stream))).await
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

    let events = openai_events(&frames).await;
    let signature = json_events_signature(events);
    assert!(signature.iter().any(|event| event.contains("Hello")));
    assert!(signature
        .iter()
        .any(|event| event.contains("prompt_tokens")));
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

    let events = openai_events(&frames).await;
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallEnd { id, input, .. }
            if id == "tool-call-0" && input["path"] == "/tmp/file"
    )));
}

#[tokio::test]
async fn anthropic_mixed_stream_compliance() {
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

    let events = anthropic_events(&frames).await;
    assert!(events
        .iter()
        .any(|event| matches!(event, StreamEvent::TextDelta(text) if text == "done")));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallEnd { input, .. } if input["path"] == "/tmp/file"
    )));
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

/// Regression: SSE decoder must handle multi-line frames.
#[tokio::test]
async fn dashscope_multiline_sse_frame() {
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
