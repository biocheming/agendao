use crate::cache::ProviderProfileFingerprint;
use crate::protocol::{ProviderAdapter, ProviderConfig};
use crate::provider::{ModelInfo as RuntimeModelInfo, Provider, ProviderError};
use crate::runtime::ProviderRuntime;
use crate::{ChatRequest, ChatResponse, ProviderApiShape, StreamResult};
use async_trait::async_trait;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;

/// Runtime provider instance combining provider adapter + config + models.
pub struct ProviderInstance {
    id: String,
    name: String,
    config: ProviderConfig,
    adapter: Arc<dyn ProviderAdapter>,
    client: Client,
    models: HashMap<String, RuntimeModelInfo>,
    runtime: Option<ProviderRuntime>,
    provider_profile_fingerprint: Option<ProviderProfileFingerprint>,
    api_shape: Option<ProviderApiShape>,
}

/// 流式 stall 看门狗兜底值（秒）：per-model `stream_stall_timeout_secs` 未配置时启用。
/// 取 120s 是宽松兜底——reasoning 模型思考间隙可能较长，但"连接挂死"
/// （TCP 活着但永无数据）与"模型慢"在该尺度下无法区分，宁可选宁长勿短；
/// 需要更短的用户可在 Model Settings 里按模型下调。
const DEFAULT_STREAM_STALL_TIMEOUT_SECS: u64 = 120;

impl ProviderInstance {
    pub fn new(
        id: String,
        name: String,
        config: ProviderConfig,
        adapter: Arc<dyn ProviderAdapter>,
        models: HashMap<String, RuntimeModelInfo>,
    ) -> Self {
        Self {
            id,
            name,
            config,
            adapter,
            // 只设 connect timeout:防止端点不可达时连接永不释放;
            // 不设总超时,避免误杀长时间流式输出(reasoning 模型尤甚)。
            client: Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            models,
            runtime: None,
            provider_profile_fingerprint: None,
            api_shape: None,
        }
    }

    pub fn with_provider_profile_fingerprint(
        mut self,
        fingerprint: ProviderProfileFingerprint,
    ) -> Self {
        self.provider_profile_fingerprint = Some(fingerprint);
        self
    }

    pub fn with_api_shape(mut self, api_shape: ProviderApiShape) -> Self {
        self.api_shape = Some(api_shape);
        self
    }

    pub fn with_runtime(mut self, runtime: ProviderRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn get_model(&self, id: &str) -> Option<&RuntimeModelInfo> {
        self.models.get(id)
    }

    pub fn models(&self) -> Vec<RuntimeModelInfo> {
        self.models.values().cloned().collect()
    }
}

#[async_trait]
impl Provider for ProviderInstance {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn provider_profile_fingerprint(&self) -> Option<ProviderProfileFingerprint> {
        self.provider_profile_fingerprint.clone()
    }

    fn api_shape(&self) -> Option<ProviderApiShape> {
        self.api_shape
    }

    fn models(&self) -> Vec<RuntimeModelInfo> {
        self.models.values().cloned().collect()
    }

    fn get_model(&self, id: &str) -> Option<&RuntimeModelInfo> {
        self.models.get(id)
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let _permit = if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    preflight.check().await?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Non-streaming requests get an optional per-request total timeout.
        // Streaming requests deliberately never get one (a long stream would be
        // killed mid-flight); use stream_stall_timeout_secs there instead.
        let timeout_secs = request.timeout_secs;
        let pending = self.adapter.chat(&self.client, &self.config, request);
        let result = match timeout_secs {
            Some(secs) => {
                match tokio::time::timeout(std::time::Duration::from_secs(secs), pending).await {
                    Ok(result) => result,
                    Err(_elapsed) => Err(ProviderError::Timeout),
                }
            }
            None => pending.await,
        };

        if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    match &result {
                        Ok(_) => preflight.on_success(),
                        Err(_) => preflight.on_failure(),
                    }
                }
            }
        }

        result
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<StreamResult, ProviderError> {
        let _permit = if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    preflight.check().await?
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // 流式 stall 看门狗默认开启：per-model 配置可覆盖，未配置时用宽松
        // 兜底值。此前默认 None = 连接挂起（连上但无数据）会永远等待——
        // run 永不结束，TUI 侧 20fps 重绘空转烧 CPU（实测空闲 ~18%）。
        let stall_timeout_secs = request
            .stream_stall_timeout_secs
            .or(Some(DEFAULT_STREAM_STALL_TIMEOUT_SECS));
        let result = self
            .adapter
            .chat_stream(&self.client, &self.config, request)
            .await
            .map(|stream| match stall_timeout_secs {
                Some(secs) => {
                    crate::stream::with_stall_watchdog(stream, std::time::Duration::from_secs(secs))
                }
                None => stream,
            });

        if let Some(runtime) = &self.runtime {
            if runtime.is_preflight_enabled() {
                if let Some(preflight) = &runtime.preflight {
                    match &result {
                        Ok(_) => preflight.on_success(),
                        Err(_) => preflight.on_failure(),
                    }
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Choice, Message, StreamEvent};
    use futures::StreamExt;

    /// Adapter whose non-streaming chat hangs forever, to exercise the
    /// per-request timeout wiring in `ProviderInstance::chat`.
    struct HangingAdapter;

    #[async_trait]
    impl ProviderAdapter for HangingAdapter {
        async fn chat(
            &self,
            _client: &Client,
            _config: &ProviderConfig,
            _request: ChatRequest,
        ) -> Result<ChatResponse, ProviderError> {
            futures::future::pending::<()>().await;
            unreachable!("pending future never completes")
        }

        async fn chat_stream(
            &self,
            _client: &Client,
            _config: &ProviderConfig,
            _request: ChatRequest,
        ) -> Result<StreamResult, ProviderError> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    /// Adapter whose stream yields one event, then goes idle forever.
    struct IdleStreamAdapter;

    #[async_trait]
    impl ProviderAdapter for IdleStreamAdapter {
        async fn chat(
            &self,
            _client: &Client,
            _config: &ProviderConfig,
            request: ChatRequest,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                id: "stub".to_string(),
                model: request.model,
                choices: vec![Choice {
                    index: 0,
                    message: Message::assistant("ok"),
                    finish_reason: Some("stop".to_string()),
                }],
                usage: None,
            })
        }

        async fn chat_stream(
            &self,
            _client: &Client,
            _config: &ProviderConfig,
            _request: ChatRequest,
        ) -> Result<StreamResult, ProviderError> {
            let stream = futures::stream::once(async { Ok(StreamEvent::Start) })
                .chain(futures::stream::pending());
            Ok(Box::pin(stream))
        }
    }

    fn instance_with(adapter: Arc<dyn ProviderAdapter>) -> ProviderInstance {
        ProviderInstance::new(
            "stub".to_string(),
            "stub".to_string(),
            ProviderConfig::new("stub", "http://localhost", "key"),
            adapter,
            HashMap::new(),
        )
    }

    #[tokio::test]
    async fn chat_without_timeout_secs_keeps_existing_behavior() {
        let instance = instance_with(Arc::new(IdleStreamAdapter));
        let response = instance
            .chat(ChatRequest::new("m", vec![Message::user("hi")]))
            .await
            .expect("chat should succeed without a timeout configured");
        assert_eq!(response.model, "m");
    }

    #[tokio::test]
    async fn chat_timeout_secs_aborts_hanging_request() {
        let instance = instance_with(Arc::new(HangingAdapter));
        let mut request = ChatRequest::new("m", vec![Message::user("hi")]);
        request.timeout_secs = Some(1);

        let started = std::time::Instant::now();
        let result = instance.chat(request).await;
        assert!(
            matches!(result, Err(ProviderError::Timeout)),
            "hanging request must surface as ProviderError::Timeout"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "timeout should abort promptly"
        );
    }

    #[tokio::test]
    async fn chat_stream_stall_watchdog_terminates_idle_stream() {
        let instance = instance_with(Arc::new(IdleStreamAdapter));
        let mut request = ChatRequest::new("m", vec![Message::user("hi")]);
        // Watchdog granularity is seconds; use 1s and an adapter that idles
        // forever after the first event.
        request.stream_stall_timeout_secs = Some(1);

        let mut stream = instance
            .chat_stream(request)
            .await
            .expect("stream should be created");
        assert!(matches!(stream.next().await, Some(Ok(StreamEvent::Start))));
        match stream.next().await {
            Some(Err(ProviderError::StreamError(message))) => {
                assert!(message.contains("stall timeout"));
            }
            other => panic!("expected stall StreamError, got: {other:?}"),
        }
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn chat_stream_without_stall_config_passes_events_through() {
        let instance = instance_with(Arc::new(IdleStreamAdapter));
        let request = ChatRequest::new("m", vec![Message::user("hi")]);

        let mut stream = instance
            .chat_stream(request)
            .await
            .expect("stream should be created");
        assert!(matches!(stream.next().await, Some(Ok(StreamEvent::Start))));
        // No watchdog configured: the stream simply pends; drop it here.
    }
}
