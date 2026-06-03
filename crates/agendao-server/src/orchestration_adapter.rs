use agendao_orchestrator::{
    OrchestratorError, PromptExecutionOptions, PromptExecutionResult, SessionDetail, SessionSummary,
};
use std::sync::Arc;

use crate::server::ServerState;

/// 适配器：将 ServerState 包装为 OrchestrationCore 兼容接口
///
/// Phase 3: 验证接口设计
/// Phase 4+: 逐步将逻辑从 ServerState 迁移到真正的 OrchestrationCore
///
/// 注意：Phase 3 只验证接口签名，不实现完整功能
pub struct ServerStateOrchestrationAdapter {
    state: Arc<ServerState>,
}

impl ServerStateOrchestrationAdapter {
    pub fn new(state: Arc<ServerState>) -> Self {
        Self { state }
    }

    /// 执行 prompt（符合 OrchestrationCore::execute_prompt 签名）
    ///
    /// Phase 3: 返回 stub 实现，验证接口签名
    /// Phase 4: 实现真正的执行逻辑
    pub async fn execute_prompt(
        &self,
        _session_id: &str,
        _text: &str,
        _options: PromptExecutionOptions,
    ) -> Result<PromptExecutionResult, OrchestratorError> {
        // Phase 3: 只验证接口签名，不实现完整功能
        // 原因：run_local_scheduler_prompt 在私有模块中，且依赖复杂
        // Phase 4 会提取真正的执行逻辑
        Err(OrchestratorError::Other(
            "ServerStateOrchestrationAdapter::execute_prompt not yet implemented (Phase 4)"
                .to_string(),
        ))
    }

    /// 列出所有会话
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, OrchestratorError> {
        let sessions = self.state.sessions.lock().await;
        let summaries = sessions
            .list()
            .iter()
            .map(|session| {
                let record = session.record();
                SessionSummary {
                    id: session.id.clone(),
                    title: if session.title.is_empty() {
                        None
                    } else {
                        Some(session.title.clone())
                    },
                    created_at: record.created_at.to_rfc3339(),
                    last_message_at: Some(
                        chrono::DateTime::from_timestamp_millis(record.time.updated)
                            .unwrap_or_else(|| chrono::Utc::now())
                            .to_rfc3339(),
                    ),
                }
            })
            .collect();
        Ok(summaries)
    }

    /// 获取会话详情
    pub async fn get_session(&self, session_id: &str) -> Result<SessionDetail, OrchestratorError> {
        let sessions = self.state.sessions.lock().await;
        let session = sessions.get(session_id).ok_or_else(|| {
            OrchestratorError::Other(format!("Session not found: {}", session_id))
        })?;

        let record = session.record();
        Ok(SessionDetail {
            id: session.id.clone(),
            title: if session.title.is_empty() {
                None
            } else {
                Some(session.title.clone())
            },
            messages: record
                .messages
                .iter()
                .map(|msg| {
                    // 提取消息文本内容
                    let content = msg
                        .parts
                        .iter()
                        .filter_map(|part| {
                            if let agendao_types::PartType::Text { text, .. } = &part.part_type {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    agendao_orchestrator::MessageSummary {
                        id: msg.id.clone(),
                        role: format!("{:?}", msg.role), // MessageRole -> String
                        content,
                        created_at: msg.created_at.to_rfc3339(),
                    }
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_adapter_interface_signature() {
        // 验证接口签名正确（不要求实际执行成功）
        let state = Arc::new(ServerState::new());
        let adapter = ServerStateOrchestrationAdapter::new(state);

        let options = PromptExecutionOptions {
            agent_id: None,
            scheduler_profile: Some("default".to_string()),
            model: None,
            variant: None,
            continue_last: false,
            source_origin: None,
            source_surface: None,
            ingress_source: None,
            idempotency_key: None,
        };

        // 验证方法可以调用（类型签名正确）
        let result = adapter.execute_prompt("", "hello", options).await;

        // Phase 3: 预期返回 "not yet implemented" 错误
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet implemented"));
    }

    #[tokio::test]
    async fn test_adapter_list_sessions() {
        let state = Arc::new(ServerState::new());
        let adapter = ServerStateOrchestrationAdapter::new(state);

        let result = adapter.list_sessions().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0); // 新建的 state 没有 session
    }

    #[tokio::test]
    async fn test_adapter_get_session_not_found() {
        let state = Arc::new(ServerState::new());
        let adapter = ServerStateOrchestrationAdapter::new(state);

        let result = adapter.get_session("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
