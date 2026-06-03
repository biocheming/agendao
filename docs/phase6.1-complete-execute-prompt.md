# Phase 6.1: 完整的 execute_prompt - 多轮对话支持

## 目标

实现完整的 `execute_prompt_with_session`，支持：
1. **多轮对话**：从 SessionManager 加载历史消息
2. **会话持久化**：保存用户消息和助手响应到 session
3. **Usage 统计**：累积 token 使用量
4. **会话创建**：自动创建新会话（如果不存在）

**不包括**（留给后续阶段）：
- ⚠️ 工具调用循环（Phase 6.2）
- ⚠️ 流式输出（Phase 6.3）
- ⚠️ 上下文压缩（Phase 6.4）

## 实现细节

### 1. 核心流程

```rust
pub async fn execute_prompt_with_session(
    sessions: &Arc<Mutex<SessionManager>>,
    providers: &Arc<RwLock<ProviderRegistry>>,
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<PromptExecutionResult, OrchestratorError> {
    // 1. 加载或创建会话
    let mut sessions_guard = sessions.lock().await;
    let session = sessions_guard.get_or_create(session_id);
    
    // 2. 添加用户消息到会话
    let user_msg_id = session.add_user_message(text);
    
    // 3. 构造完整的 prompt（包括历史消息）
    let messages = build_messages_from_session(session);
    
    // 4. 解析 provider 和 model
    let (provider_id, model_id) = parse_model_spec(options.model.as_deref());
    
    // 5. 获取 provider
    drop(sessions_guard); // 释放锁，避免死锁
    let providers_guard = providers.read().await;
    let provider = providers_guard.get(&provider_id)?;
    
    // 6. 构造请求
    let request = ChatRequest {
        model: model_id.clone(),
        messages,
        max_tokens: Some(4096),
        temperature: None,
        top_p: None,
        system: None,
        tools: None, // Phase 6.2 will add tools
        stream: Some(false),
        provider_options: None,
        variant: options.variant.clone(),
    };
    
    // 7. 调用 provider
    let response = provider.chat(request).await?;
    
    // 8. 提取响应文本
    let text = extract_response_text(&response);
    
    // 9. 保存助手响应到会话
    let mut sessions_guard = sessions.lock().await;
    let session = sessions_guard.get_mut(session_id)?;
    let assistant_msg_id = session.add_assistant_message(&text);
    
    // 10. 更新 usage 统计
    if let Some(usage) = &response.usage {
        session.add_usage(usage.prompt_tokens, usage.completion_tokens);
    }
    
    // 11. 返回结果
    Ok(PromptExecutionResult {
        session_id: session_id.to_string(),
        message_id: assistant_msg_id,
        text,
        usage: response.usage.map(|u| UsageInfo {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
    })
}
```

### 2. SessionManager 扩展

**文件**: `crates/agendao-session-core/src/lib.rs`

需要添加的方法：

```rust
impl SessionManager {
    /// Get or create a session
    pub fn get_or_create(&mut self, session_id: &str) -> &mut Session {
        if !self.sessions.contains_key(session_id) {
            let session = Session::new(session_id.to_string());
            self.sessions.insert(session_id.to_string(), session);
        }
        self.sessions.get_mut(session_id).unwrap()
    }
    
    /// Get mutable session
    pub fn get_mut(&mut self, session_id: &str) -> Option<&mut Session> {
        self.sessions.get_mut(session_id)
    }
}

impl Session {
    /// Add user message to session
    pub fn add_user_message(&mut self, text: &str) -> String {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let message = Message {
            id: msg_id.clone(),
            role: MessageRole::User,
            parts: vec![Part {
                part_type: PartType::Text {
                    text: text.to_string(),
                    cache_control: None,
                },
            }],
            created_at: chrono::Utc::now(),
        };
        self.record.messages.push(message);
        self.record.time.updated = chrono::Utc::now().timestamp_millis();
        msg_id
    }
    
    /// Add assistant message to session
    pub fn add_assistant_message(&mut self, text: &str) -> String {
        let msg_id = uuid::Uuid::new_v4().to_string();
        let message = Message {
            id: msg_id.clone(),
            role: MessageRole::Assistant,
            parts: vec![Part {
                part_type: PartType::Text {
                    text: text.to_string(),
                    cache_control: None,
                },
            }],
            created_at: chrono::Utc::now(),
        };
        self.record.messages.push(message);
        self.record.time.updated = chrono::Utc::now().timestamp_millis();
        msg_id
    }
    
    /// Add usage statistics
    pub fn add_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        self.record.usage.input_tokens += input_tokens;
        self.record.usage.output_tokens += output_tokens;
        self.record.usage.total_tokens += input_tokens + output_tokens;
    }
}
```

### 3. 消息转换

**文件**: `crates/agendao-orchestrator/src/prompt_execution.rs`

```rust
/// Build messages from session history
fn build_messages_from_session(session: &Session) -> Vec<agendao_provider::Message> {
    session
        .record()
        .messages
        .iter()
        .map(|msg| {
            let content = msg
                .parts
                .iter()
                .filter_map(|part| {
                    if let PartType::Text { text, .. } = &part.part_type {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            
            match msg.role {
                MessageRole::User => agendao_provider::Message::user(&content),
                MessageRole::Assistant => agendao_provider::Message::assistant(&content),
                _ => agendao_provider::Message::user(&content), // Fallback
            }
        })
        .collect()
}
```

## 架构影响

### 符合 AgenDao 宪法

- **第一条（唯一执行内核）**：所有 LLM 循环由 `OrchestrationCore` 驱动
- **第五条（唯一状态所有权）**：`SessionManager` 是会话状态的唯一所有者
- **第九条（副作用路径唯一）**：会话修改通过 `SessionManager` API，不直接操作内部字段

### 数据流

```
User Input
    ↓
OrchestrationCore.execute_prompt()
    ↓
SessionManager.get_or_create(session_id)
    ↓
Session.add_user_message(text)
    ↓
build_messages_from_session(session)
    ↓
Provider.chat(request)
    ↓
Session.add_assistant_message(response)
    ↓
Session.add_usage(tokens)
    ↓
Return PromptExecutionResult
```

## 测试策略

### 单元测试

```rust
#[tokio::test]
async fn test_execute_prompt_with_session() {
    let sessions = Arc::new(Mutex::new(SessionManager::new()));
    let providers = Arc::new(RwLock::new(ProviderRegistry::new()));
    
    // Register mock provider
    // ...
    
    // First turn
    let result1 = execute_prompt_with_session(
        &sessions,
        &providers,
        "test-session",
        "Hello",
        &PromptExecutionOptions::default(),
    ).await.unwrap();
    
    assert_eq!(result1.session_id, "test-session");
    
    // Second turn - should include history
    let result2 = execute_prompt_with_session(
        &sessions,
        &providers,
        "test-session",
        "How are you?",
        &PromptExecutionOptions::default(),
    ).await.unwrap();
    
    // Verify session has 4 messages (2 user + 2 assistant)
    let sessions_guard = sessions.lock().await;
    let session = sessions_guard.get("test-session").unwrap();
    assert_eq!(session.record().messages.len(), 4);
}
```

### 集成测试

1. **多轮对话测试**
   - 创建会话
   - 发送多个消息
   - 验证历史消息正确传递给 provider

2. **Usage 统计测试**
   - 发送多个消息
   - 验证 token 使用量累积

3. **会话持久化测试**
   - 创建会话并发送消息
   - 重新加载会话
   - 验证消息历史完整

## 已知限制

1. **无工具调用**：当前实现不支持工具调用循环
   - 如果 provider 返回 tool_calls，会被忽略
   - Phase 6.2 将添加工具调用支持

2. **无流式输出**：只支持完整响应
   - `stream: Some(false)` 硬编码
   - Phase 6.3 将添加流式输出支持

3. **无上下文压缩**：历史消息无限增长
   - 可能超过 provider 的 context window
   - Phase 6.4 将添加上下文压缩

4. **无并发控制**：同一会话的并发请求可能冲突
   - SessionManager 使用 Mutex，但没有请求级别的锁
   - 未来可能需要添加请求队列

## 性能考虑

### 锁策略

```rust
// ❌ 错误：持有锁期间调用 provider（可能阻塞很久）
let sessions_guard = sessions.lock().await;
let session = sessions_guard.get(session_id)?;
let messages = build_messages_from_session(session);
let response = provider.chat(request).await?; // 持有锁！
drop(sessions_guard);

// ✅ 正确：先读取数据，释放锁，再调用 provider
let messages = {
    let sessions_guard = sessions.lock().await;
    let session = sessions_guard.get(session_id)?;
    build_messages_from_session(session)
}; // 锁自动释放
let response = provider.chat(request).await?;
```

### 内存使用

- 每个会话的消息历史存储在内存中
- 长会话可能消耗大量内存
- 未来可能需要：
  - 消息分页加载
  - 旧消息持久化到磁盘
  - 上下文窗口滑动

## 验收标准

- [ ] `execute_prompt_with_session` 函数实现
- [ ] `SessionManager::get_or_create` 方法实现
- [ ] `SessionManager::get_mut` 方法实现
- [ ] `Session::add_user_message` 方法实现
- [ ] `Session::add_assistant_message` 方法实现
- [ ] `Session::add_usage` 方法实现
- [ ] `build_messages_from_session` 函数实现
- [ ] `OrchestrationCore::execute_prompt` 调用新实现
- [ ] 单元测试通过
- [ ] 整个 workspace 编译通过
- [ ] 多轮对话测试通过
- [ ] Usage 统计测试通过

## 下一步（Phase 6.2）

实现工具调用支持：
1. 检测 provider 响应中的 tool_calls
2. 执行工具调用（通过 ToolRegistry）
3. 将工具结果添加到消息历史
4. 继续 LLM 循环直到没有 tool_calls
5. 保存完整的对话历史（包括工具调用）
