use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::custom_fetch::{get_custom_fetch_proxy, CustomFetchRequest};
use crate::message::{Content, Message, Role};
use crate::provider::ProviderError;
use crate::responses_convert::convert_to_openai_responses_input;
use crate::stream::{StreamEvent, StreamResult};
use crate::tools::{prepare_responses_tools, InputTool, ResponsesTool};

use super::helpers::{
    drain_next_sse_frame, extract_sse_data, finish_reason_label, insert_opt_bool,
    insert_opt_string, insert_opt_u64, insert_opt_value, parse_output_items, process_stream_chunk,
    push_include, usage_to_stream_usage, StreamChunkState,
};
use super::types::{
    get_responses_model_config, map_openai_response_finish_reason, FinishReason, LogprobsSetting,
    ResponseMetadata, ResponsesIncludeValue, ResponsesStreamChunk, ResponsesUsage,
};
use super::validation::{
    validate_responses_settings, GenerateOptions, OpenAIResponsesConfig,
    OpenAIResponsesLanguageModel, PreparedArgs, ResponsesGenerateResult,
    ResponsesSettingsValidation, StreamOptions,
};

impl OpenAIResponsesLanguageModel {
    pub fn new(model_id: impl Into<String>, config: OpenAIResponsesConfig) -> Self {
        Self {
            model_id: model_id.into(),
            config,
        }
    }

    fn build_headers(&self, accept: &str) -> HashMap<String, String> {
        let mut headers = HashMap::from([
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), accept.to_string()),
        ]);
        headers.extend((self.config.headers)());
        headers
    }

    pub async fn get_args(&self, options: &GenerateOptions) -> Result<PreparedArgs, ProviderError> {
        let model_config = get_responses_model_config(&self.model_id);
        let provider_options = options.provider_options.clone().unwrap_or_default();
        let supports_reasoning = model_config.is_reasoning_model
            || provider_options.reasoning_effort.is_some()
            || provider_options.reasoning_summary.is_some();
        let mut warnings = validate_responses_settings(ResponsesSettingsValidation {
            model_config: &model_config,
            options: &provider_options,
            top_k: options.top_k,
            seed: options.seed,
            presence_penalty: options.presence_penalty,
            frequency_penalty: options.frequency_penalty,
            stop_sequences: options.stop_sequences.as_deref(),
            temperature: options.temperature,
            top_p: options.top_p,
        });

        let strict_json_schema = provider_options.strict_json_schema.unwrap_or(false);
        let prepared_tools = prepare_responses_tools(
            options.tools.as_deref(),
            options.tool_choice.as_ref(),
            strict_json_schema,
        );

        let has_local_shell_tool = prepared_tools
            .tools
            .as_ref()
            .map(|tools| {
                tools
                    .iter()
                    .any(|tool| matches!(tool, ResponsesTool::LocalShell {}))
            })
            .unwrap_or(false);

        let store = provider_options.store.unwrap_or(true);
        let (input, convert_warnings) = convert_to_openai_responses_input(
            &options.prompt,
            model_config.system_message_mode,
            self.config.file_id_prefixes.as_deref(),
            store,
            has_local_shell_tool,
        )
        .await;

        warnings.extend(convert_warnings);
        warnings.extend(prepared_tools.tool_warnings);

        let mut include = provider_options.include.clone().unwrap_or_default();
        if provider_options
            .logprobs
            .as_ref()
            .and_then(LogprobsSetting::top_logprobs)
            .is_some()
        {
            push_include(
                &mut include,
                ResponsesIncludeValue::MessageOutputTextLogprobs,
            );
        }

        if let Some(tools) = &prepared_tools.tools {
            let has_web_search = tools.iter().any(|tool| {
                matches!(
                    tool,
                    ResponsesTool::WebSearch { .. } | ResponsesTool::WebSearchPreview { .. }
                )
            });
            if has_web_search {
                push_include(
                    &mut include,
                    ResponsesIncludeValue::WebSearchCallActionSources,
                );
            }
            let has_code_interpreter = tools
                .iter()
                .any(|tool| matches!(tool, ResponsesTool::CodeInterpreter { .. }));
            if has_code_interpreter {
                push_include(
                    &mut include,
                    ResponsesIncludeValue::CodeInterpreterCallOutputs,
                );
            }
        }

        let mut body = json!({
            "model": self.model_id,
            "input": input,
        });
        let obj = body.as_object_mut().ok_or_else(|| {
            ProviderError::InvalidRequest("failed to build responses request body".to_string())
        })?;

        if let Some(tools) = prepared_tools.tools {
            obj.insert(
                "tools".to_string(),
                serde_json::to_value(tools).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize tools: {}", e))
                })?,
            );
        }
        if let Some(tool_choice) = prepared_tools.tool_choice {
            obj.insert(
                "tool_choice".to_string(),
                serde_json::to_value(tool_choice).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize tool choice: {}", e))
                })?,
            );
        }
        if let Some(max_output_tokens) = options.max_output_tokens {
            obj.insert(
                "max_output_tokens".to_string(),
                Value::Number(max_output_tokens.into()),
            );
        }
        if !supports_reasoning {
            if let Some(temperature) = options.temperature {
                obj.insert("temperature".to_string(), json!(temperature));
            }
            if let Some(top_p) = options.top_p {
                obj.insert("top_p".to_string(), json!(top_p));
            }
        }
        if model_config.required_auto_truncation {
            obj.insert("truncation".to_string(), Value::String("auto".to_string()));
        }

        if !include.is_empty() {
            obj.insert(
                "include".to_string(),
                serde_json::to_value(include).map_err(|e| {
                    ProviderError::InvalidRequest(format!("failed to serialize include: {}", e))
                })?,
            );
        }

        let mut text_obj = serde_json::Map::new();
        if let Some(top_n) = provider_options
            .logprobs
            .as_ref()
            .and_then(LogprobsSetting::top_logprobs)
        {
            text_obj.insert("logprobs".to_string(), Value::Bool(true));
            text_obj.insert("top_logprobs".to_string(), json!(top_n));
        }
        if let Some(verbosity) = provider_options.text_verbosity.clone() {
            text_obj.insert(
                "verbosity".to_string(),
                serde_json::to_value(verbosity).map_err(|e| {
                    ProviderError::InvalidRequest(format!(
                        "failed to serialize text verbosity: {}",
                        e
                    ))
                })?,
            );
        }
        if let Some(format) = &options.response_format {
            text_obj.insert("format".to_string(), format.clone());
        }
        if !text_obj.is_empty() {
            obj.insert("text".to_string(), Value::Object(text_obj));
        }

        if supports_reasoning {
            let mut reasoning = serde_json::Map::new();
            if let Some(effort) = provider_options.reasoning_effort.clone() {
                reasoning.insert("effort".to_string(), Value::String(effort));
            }
            if let Some(summary) = provider_options.reasoning_summary.clone() {
                reasoning.insert("summary".to_string(), Value::String(summary));
            }
            if !reasoning.is_empty() {
                obj.insert("reasoning".to_string(), Value::Object(reasoning));
            }
        }

        insert_opt_string(obj, "instructions", provider_options.instructions);
        insert_opt_u64(
            obj,
            "max_tool_calls",
            provider_options.max_tool_calls.map(u64::from),
        );
        insert_opt_value(obj, "metadata", provider_options.metadata);
        insert_opt_bool(
            obj,
            "parallel_tool_calls",
            provider_options.parallel_tool_calls,
        );
        insert_opt_string(
            obj,
            "previous_response_id",
            provider_options.previous_response_id,
        );
        insert_opt_string(obj, "prompt_cache_key", provider_options.prompt_cache_key);
        insert_opt_string(obj, "safety_identifier", provider_options.safety_identifier);
        if let Some(service_tier) = provider_options.service_tier {
            obj.insert(
                "service_tier".to_string(),
                serde_json::to_value(service_tier).map_err(|e| {
                    ProviderError::InvalidRequest(format!(
                        "failed to serialize service tier: {}",
                        e
                    ))
                })?,
            );
        }
        insert_opt_bool(obj, "store", provider_options.store);
        insert_opt_string(obj, "user", provider_options.user);

        let web_search_tool_name = options.tools.as_deref().and_then(|tools| {
            tools.iter().find_map(|tool| match tool {
                InputTool::ProviderDefined { id, .. }
                    if id == "openai.web_search" || id == "openai.web_search_preview" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
        });

        Ok(PreparedArgs {
            web_search_tool_name,
            body,
            warnings,
        })
    }

    pub async fn do_generate(
        &self,
        options: GenerateOptions,
    ) -> Result<ResponsesGenerateResult, ProviderError> {
        let prepared = self.get_args(&options).await?;
        let url = (self.config.url)("/responses", &self.model_id);
        let headers = self.build_headers("application/json");
        let request_body = serde_json::to_string(&prepared.body)
            .map_err(|e| ProviderError::InvalidRequest(format!("failed to encode body: {}", e)))?;

        let (status_code, raw) = if let Some(proxy) = get_custom_fetch_proxy(&self.config.provider)
        {
            let response = proxy
                .fetch(CustomFetchRequest {
                    url,
                    method: "POST".to_string(),
                    headers: headers.clone(),
                    body: Some(request_body),
                })
                .await?;
            (response.status, response.body)
        } else {
            let client = self.config.client.clone().unwrap_or_default();
            let mut request = client.post(url);
            for (k, v) in &headers {
                request = request.header(k, v);
            }
            let response = request
                .body(request_body)
                .send()
                .await
                .map_err(|e| ProviderError::NetworkError(e.to_string()))?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|e| {
                let mut msg = e.to_string();
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    msg.push_str(": ");
                    msg.push_str(&cause.to_string());
                    source = cause.source();
                }
                ProviderError::ApiError(msg)
            })?;
            (status, body)
        };
        if status_code >= 400 {
            return Err(ProviderError::ApiErrorWithStatus {
                message: raw,
                status_code,
            });
        }

        let body: Value = serde_json::from_str(&raw)
            .map_err(|e| ProviderError::ApiError(format!("invalid responses payload: {}", e)))?;

        let output = body
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (parts, has_function_call, logprobs) = parse_output_items(&output);

        let usage = body
            .get("usage")
            .cloned()
            .and_then(|v| serde_json::from_value::<ResponsesUsage>(v).ok())
            .unwrap_or_default();

        let incomplete_reason = body
            .get("incomplete_details")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("reason"))
            .and_then(Value::as_str);
        let finish_reason = map_openai_response_finish_reason(incomplete_reason, has_function_call);

        let metadata = ResponseMetadata {
            response_id: body
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            model_id: body
                .get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            timestamp: body.get("created_at").and_then(Value::as_u64),
            service_tier: body
                .get("service_tier")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            logprobs: if logprobs.is_empty() {
                None
            } else {
                Some(logprobs)
            },
        };

        let message = Message {
            role: Role::Assistant,
            content: if parts.is_empty() {
                Content::Text(String::new())
            } else {
                Content::Parts(parts)
            },
            cache_control: None,
            provider_options: None,
        };

        Ok(ResponsesGenerateResult {
            message,
            finish_reason,
            usage,
            metadata,
            warnings: prepared.warnings,
        })
    }

    pub async fn do_stream(&self, options: StreamOptions) -> Result<StreamResult, ProviderError> {
        let mut prepared = self.get_args(&options.generate).await?;
        let body_obj = prepared.body.as_object_mut().ok_or_else(|| {
            ProviderError::InvalidRequest("invalid responses request".to_string())
        })?;
        body_obj.insert("stream".to_string(), Value::Bool(true));

        let url = (self.config.url)("/responses", &self.model_id);
        let headers = self.build_headers("text/event-stream");
        let request_body = serde_json::to_string(&prepared.body)
            .map_err(|e| ProviderError::InvalidRequest(format!("failed to encode body: {}", e)))?;
        let text_stream: Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>> =
            if let Some(proxy) = get_custom_fetch_proxy(&self.config.provider) {
                let response = proxy
                    .fetch_stream(CustomFetchRequest {
                        url,
                        method: "POST".to_string(),
                        headers: headers.clone(),
                        body: Some(request_body),
                    })
                    .await?;
                if response.status >= 400 {
                    return Err(ProviderError::ApiErrorWithStatus {
                        message: format!(
                            "custom fetch stream request failed with status {}",
                            response.status
                        ),
                        status_code: response.status,
                    });
                }
                response.stream
            } else {
                let client = self.config.client.clone().unwrap_or_default();
                let mut request = client.post(url);
                for (k, v) in &headers {
                    request = request.header(k, v);
                }
                let response = request
                    .body(request_body)
                    .send()
                    .await
                    .map_err(|e| ProviderError::NetworkError(e.to_string()))?;
                let status = response.status();
                if !status.is_success() {
                    let body = response
                        .text()
                        .await
                        .unwrap_or_else(|error| format!("<body read failed: {}>", error));
                    return Err(ProviderError::ApiErrorWithStatus {
                        message: body,
                        status_code: status.as_u16(),
                    });
                }
                Box::pin(
                    response
                        .bytes_stream()
                        .map(|chunk_result| match chunk_result {
                            Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).to_string()),
                            Err(err) => Err(ProviderError::StreamError(err.to_string())),
                        }),
                )
            };
        let metadata_extractor = self
            .config
            .metadata_extractor
            .as_ref()
            .map(|extractor| extractor.create_stream_extractor());

        let (tx, rx) = mpsc::channel::<Result<StreamEvent, ProviderError>>(256);
        tokio::spawn(async move {
            if let Err(error) = tx.send(Ok(StreamEvent::Start)).await {
                tracing::debug!(
                    error = %error,
                    "Failed to send start event for responses stream"
                );
                return;
            }
            if let Err(error) = tx.send(Ok(StreamEvent::StartStep)).await {
                tracing::debug!(
                    error = %error,
                    "Failed to send start-step event for responses stream"
                );
                return;
            }

            let tx = tx;
            let mut text_stream = text_stream;
            let mut stream_metadata_extractor = metadata_extractor;

            let mut buffer = String::new();
            let mut stream_state = StreamChunkState::default();

            while let Some(chunk_result) = text_stream.next().await {
                let chunk = match chunk_result {
                    Ok(text) => text,
                    Err(err) => {
                        if let Err(send_error) = tx.send(Err(err)).await {
                            tracing::debug!(
                                error = %send_error,
                                "Failed to forward responses stream error to consumer"
                            );
                        }
                        return;
                    }
                };
                buffer.push_str(&chunk);

                while let Some(frame) = drain_next_sse_frame(&mut buffer) {
                    let Some(data) = extract_sse_data(&frame) else {
                        continue;
                    };
                    if data == "[DONE]" {
                        break;
                    }

                    let parsed_value: Value = match serde_json::from_str(&data) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if let Some(extractor) = stream_metadata_extractor.as_mut() {
                        extractor.process_chunk(&parsed_value);
                    }

                    let parsed_chunk: ResponsesStreamChunk = serde_json::from_value(parsed_value)
                        .unwrap_or(ResponsesStreamChunk::Unknown);

                    for event in process_stream_chunk(parsed_chunk, &mut stream_state) {
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                }
            }

            if stream_state.text_open && tx.send(Ok(StreamEvent::TextEnd)).await.is_err() {
                return;
            }
            for ongoing in stream_state
                .ongoing_tool_calls
                .drain()
                .map(|(_, ongoing)| ongoing)
            {
                if tx
                    .send(Ok(StreamEvent::ToolInputEnd {
                        id: ongoing.tool_call_id,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            for reasoning in stream_state
                .active_reasoning
                .drain()
                .map(|(_, reasoning)| reasoning)
            {
                if tx
                    .send(Ok(StreamEvent::ReasoningEnd {
                        id: reasoning.canonical_id,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            let mut provider_metadata = json!({
                "response_id": stream_state.response_id,
                "service_tier": stream_state.service_tier,
            });
            if !stream_state.logprobs.is_empty() {
                provider_metadata["logprobs"] =
                    serde_json::to_value(&stream_state.logprobs).unwrap_or(Value::Null);
            }
            if let Some(extractor) = stream_metadata_extractor.as_ref() {
                if let Some(extra) = extractor.build_metadata() {
                    provider_metadata["metadata"] =
                        serde_json::to_value(extra).unwrap_or(Value::Null);
                }
            }

            let resolved_reason = if stream_state.finish_reason == FinishReason::Unknown {
                map_openai_response_finish_reason(None, stream_state.has_function_call)
            } else {
                stream_state.finish_reason
            };

            if tx
                .send(Ok(StreamEvent::FinishStep {
                    finish_reason: Some(finish_reason_label(resolved_reason).to_string()),
                    usage: usage_to_stream_usage(&stream_state.usage),
                    provider_metadata: Some(provider_metadata),
                }))
                .await
                .is_err()
            {
                return;
            }
            if tx.send(Ok(StreamEvent::Finish)).await.is_err() {
                return;
            }
            if let Err(error) = tx.send(Ok(StreamEvent::Done)).await {
                tracing::debug!(
                    error = %error,
                    "Failed to send done event for responses stream"
                );
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
