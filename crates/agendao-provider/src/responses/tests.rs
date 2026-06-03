use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{stream, StreamExt};
use serde_json::json;

use super::helpers::{parse_output_items, process_stream_chunk, StreamChunkState};
use super::*;
use crate::custom_fetch::{
    register_custom_fetch_proxy, unregister_custom_fetch_proxy, CustomFetchProxy,
    CustomFetchRequest, CustomFetchResponse, CustomFetchStreamResponse,
};
use crate::message::{Content, Message};
use crate::provider::ProviderError;
use crate::stream::StreamEvent;

struct FakeCustomFetchProxy;

#[async_trait]
impl CustomFetchProxy for FakeCustomFetchProxy {
    async fn fetch(
        &self,
        _request: CustomFetchRequest,
    ) -> Result<CustomFetchResponse, ProviderError> {
        Ok(CustomFetchResponse {
            status: 200,
            headers: HashMap::new(),
            body: json!({
                "id": "resp_1",
                "model": "gpt-5",
                "output": [
                    {
                        "type": "message",
                        "content": [
                            {"type": "output_text", "text": "hello from proxy"}
                        ]
                    }
                ],
                "usage": {
                    "input_tokens": 1,
                    "output_tokens": 1,
                    "total_tokens": 2
                }
            })
            .to_string(),
        })
    }

    async fn fetch_stream(
        &self,
        _request: CustomFetchRequest,
    ) -> Result<CustomFetchStreamResponse, ProviderError> {
        let frames = vec![
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream_1\"}}\n\n"
                .to_string(),
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream_1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n"
                .to_string(),
            "data: [DONE]\n\n".to_string(),
        ];
        Ok(CustomFetchStreamResponse {
            status: 200,
            headers: HashMap::new(),
            stream: Box::pin(stream::iter(frames.into_iter().map(Ok))),
        })
    }
}

#[test]
fn test_parse_output_items_function_call_and_text() {
    let output = vec![
        json!({
            "type": "message",
            "content": [
                {"type": "output_text", "text": "hello"}
            ]
        }),
        json!({
            "type": "function_call",
            "call_id": "call_1",
            "name": "grep",
            "arguments": "{\"q\":\"hello\"}"
        }),
    ];

    let (parts, has_tool, _logprobs) = parse_output_items(&output);
    assert!(has_tool);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].content_type, "text");
    assert_eq!(parts[1].content_type, "tool_use");
    assert_eq!(
        parts[1].tool_use.as_ref().map(|t| t.name.as_str()),
        Some("grep")
    );
}

#[test]
fn test_stream_state_machine_function_call_lifecycle() {
    let mut stream_state = StreamChunkState::default();

    let added_events = process_stream_chunk(
        ResponsesStreamChunk::OutputItemAdded {
            output_index: 0,
            item: OutputItemAddedItem::FunctionCall {
                id: "item_1".to_string(),
                call_id: "call_1".to_string(),
                name: "grep".to_string(),
                arguments: "{\"q\":\"x\"}".to_string(),
            },
        },
        &mut stream_state,
    );
    assert!(added_events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolInputStart { .. })));

    let done_events = process_stream_chunk(
        ResponsesStreamChunk::OutputItemDone {
            output_index: 0,
            item: OutputItemDoneItem::FunctionCall {
                id: "item_1".to_string(),
                call_id: "call_1".to_string(),
                name: "grep".to_string(),
                arguments: "{\"q\":\"x\"}".to_string(),
                status: Some("completed".to_string()),
            },
        },
        &mut stream_state,
    );
    assert!(done_events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolInputEnd { .. })));
    assert!(done_events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallEnd { .. })));
    assert!(stream_state.has_function_call);
}

#[tokio::test]
async fn test_do_generate_uses_registered_custom_fetch_proxy() {
    register_custom_fetch_proxy("test-provider", Arc::new(FakeCustomFetchProxy));

    let model = OpenAIResponsesLanguageModel::new(
        "gpt-5",
        OpenAIResponsesConfig {
            provider: "test-provider".to_string(),
            ..Default::default()
        },
    );
    let result = model
        .do_generate(GenerateOptions {
            prompt: vec![Message::user("hello".to_string())],
            ..Default::default()
        })
        .await
        .expect("generate via custom fetch should succeed");

    match result.message.content {
        Content::Parts(parts) => {
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].text.as_deref(), Some("hello from proxy"));
        }
        other => panic!("unexpected content: {other:?}"),
    }

    unregister_custom_fetch_proxy("test-provider");
}

#[tokio::test]
async fn test_do_stream_uses_registered_custom_fetch_proxy() {
    register_custom_fetch_proxy("test-provider-stream", Arc::new(FakeCustomFetchProxy));

    let model = OpenAIResponsesLanguageModel::new(
        "gpt-5",
        OpenAIResponsesConfig {
            provider: "test-provider-stream".to_string(),
            ..Default::default()
        },
    );
    let stream = model
        .do_stream(StreamOptions {
            generate: GenerateOptions {
                prompt: vec![Message::user("hello".to_string())],
                ..Default::default()
            },
        })
        .await
        .expect("stream via custom fetch should succeed");

    let events: Vec<_> = stream.collect::<Vec<_>>().await;
    assert!(events
        .iter()
        .any(|event| matches!(event, Ok(StreamEvent::Start))));
    assert!(events
        .iter()
        .any(|event| matches!(event, Ok(StreamEvent::Finish))));
    assert!(events
        .iter()
        .any(|event| matches!(event, Ok(StreamEvent::Done))));

    unregister_custom_fetch_proxy("test-provider-stream");
}

#[tokio::test]
async fn explicit_reasoning_options_enable_responses_reasoning_for_third_party_models() {
    let model = OpenAIResponsesLanguageModel::new(
        "MiniMax-M2.5",
        OpenAIResponsesConfig {
            provider: "test-provider".to_string(),
            ..Default::default()
        },
    );

    let prepared = model
        .get_args(&GenerateOptions {
            prompt: vec![Message::user("hello".to_string())],
            provider_options: Some(ResponsesProviderOptions {
                reasoning_effort: Some("medium".to_string()),
                reasoning_summary: Some("auto".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("prepared args");

    assert_eq!(prepared.body["reasoning"]["effort"], "medium");
    assert_eq!(prepared.body["reasoning"]["summary"], "auto");
}
