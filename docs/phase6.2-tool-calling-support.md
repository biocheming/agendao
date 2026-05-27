# Phase 6.2: 工具调用支持

## 目标

在 `execute_prompt_with_session` 中添加工具调用循环支持：
1. **检测 tool_calls**：识别 provider 响应中的工具调用请求
2. **执行工具**：通过 ToolRegistry 执行工具
3. **保存工具调用和结果**：添加到会话历史
4. **继续循环**：将工具结果发送回 LLM，直到没有新的 tool_calls
5. **完整历史**：保存包括工具调用在内的完整对话

**不包括**（留给后续阶段）：
- ⚠️ 流式输出（Phase 6.3）
- ⚠️ 上下文压缩（Phase 6.4）
- ⚠️ 并发工具执行（未来优化）

## 实现细节

### 1. 核心流程

```rust
pub async fn execute_prompt_with_tools(
    sessions: &Arc<Mutex<SessionManager>>,
    providers: &Arc<RwLock<ProviderRegistry>>,
    tools: &Arc<RwLock<ToolRegistry>>,  // 新增
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<PromptExecutionResult, OrchestratorError> {
    // 1. 添加用户消息
    // 2. 构造 messages（包括历史）
    // 3. 获取可用工具列表
    // 4. 进入 LLM 循环
    loop {
        // 4.1 调用 provider（带工具定义）
        let response = provider.chat(request).await?;
        
        // 4.2 保存助手响应
        let assistant_msg_id = save_assistant_message(&response);
        
        // 4.3 检查是否有 tool_calls
        if let Some(tool_calls) = extract_tool_calls(&response) {
            // 4.4 执行每个工具调用
            for tool_call in tool_calls {
                let result = execute_tool(tools, &tool_call).await?;
                
                // 4.5 保存工具结果到会话
                save_tool_result(session, &tool_call, &result);
                
                // 4.6 添加到 messages（用于下一轮）
                messages.push(tool_result_message);
            }
            
            // 4.7 继续循环（发送工具结果给 LLM）
            continue;
        } else {
            // 4.8 没有 tool_calls，结束循环
            break;
        }
    }
    
    // 5. 返回最终结果
}
```

### 2. 工具调用检测

```rust
/// Extract tool calls from provider response
fn extract_tool_calls(response: &ChatResponse) -> Option<Vec<ToolCall>> {
    response.choices.first().and_then(|choice| {
        match &choice.message.content {
            Content::Parts(parts) => {
                let tool_calls: Vec<_> = parts
                    .iter()
                    .filter_map(|part| {
                        if let ContentPart::ToolUse { id, name, input } = part {
                            Some(ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                
                if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                }
            }
            _ => None,
        }
    })
}
```

### 3. 工具执行

```rust
/// Execute a tool call
async fn execute_tool(
    tools: &Arc<RwLock<ToolRegistry>>,
    tool_call: &ToolCall,
) -> Result<ToolResult, OrchestratorError> {
    let tools_guard = tools.read().await;
    
    let tool = tools_guard
        .get(&tool_call.name)
        .ok_or_else(|| OrchestratorError::Other(format!("Tool not found: {}", tool_call.name)))?;
    
    // Execute tool
    let result = tool.execute(tool_call.input.clone()).await?;
    
    Ok(ToolResult {
        tool_call_id: tool_call.id.clone(),
        content: result.output,
        is_error: result.error.is_some(),
    })
}
```

### 4. 保存工具调用和结果

```rust
impl Session {
    /// Add tool call to session (Phase 6.2)
    pub fn add_tool_call(&mut self, tool_call: &ToolCall) -> String {
        let part_id = format!("prt_{}", Uuid::new_v4());
        let now = Utc::now();
        
        // Find the last assistant message and add tool call part
        if let Some(last_msg) = self.record.messages.last_mut() {
            if last_msg.role == MessageRole::Assistant {
                last_msg.parts.push(MessagePart {
                    id: part_id.clone(),
                    part_type: PartType::ToolCall {
                        id: tool_call.id.clone(),
                        name: tool_call.name.clone(),
                        input: tool_call.input.clone(),
                        status: ToolCallStatus::Pending,
                        raw: None,
                        state: None,
                    },
                    created_at: now,
                    message_id: Some(last_msg.id.clone()),
                });
            }
        }
        
        part_id
    }
    
    /// Add tool result to session (Phase 6.2)
    pub fn add_tool_result(&mut self, tool_result: &ToolResult) -> String {
        let msg_id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now();
        
        let message = SessionMessage {
            id: msg_id.clone(),
            session_id: self.id.clone(),
            role: MessageRole::Tool,
            parts: vec![MessagePart {
                id: format!("prt_{}", Uuid::new_v4()),
                part_type: PartType::ToolResult {
                    tool_call_id: tool_result.tool_call_id.clone(),
                    content: tool_result.content.clone(),
                    is_error: tool_result.is_error,
                    title: None,
                    metadata: None,
                    attachments: None,
                },
                created_at: now,
                message_id: Some(msg_id.clone()),
            }],
            created_at: now,
            metadata: HashMap::new(),
            usage: None,
            finish: None,
        };
        
        self.record.messages.push(message);
        self.record.time.updated = now.timestamp_millis();
        self.updated_at = now;
        
        msg_id
    }
}
```

### 5. 消息转换（包括工具）

```rust
/// Build messages from session history (including tool calls and results)
fn build_messages_with_tools(session: &Session) -> Vec<Message> {
    session
        .record()
        .messages
        .iter()
        .map(|msg| {
            let mut content_parts = Vec::new();
            
            for part in &msg.parts {
                match &part.part_type {
                    PartType::Text { text, .. } => {
                        content_parts.push(ContentPart::Text {
                            text: text.clone(),
                        });
                    }
                    PartType::ToolCall { id, name, input, .. } => {
                        content_parts.push(ContentPart::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                    }
                    PartType::ToolResult { tool_call_id, content, is_error, .. } => {
                        content_parts.push(ContentPart::ToolResult {
                            tool_use_id: tool_call_id.clone(),
                            content: content.clone(),
                            is_error: *is_error,
                        });
                    }
                    _ => {}
                }
            }
            
            let content = if content_parts.is_empty() {
                Content::Text(String::new())
            } else if content_parts.len() == 1 {
                if let ContentPart::Text { text } = &content_parts[0] {
                    Content::Text(text.clone())
                } else {
                    Content::Parts(content_parts)
                }
            } else {
                Content::Parts(content_parts)
            };
            
            Message {
                role: match msg.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::System => "system",
                    MessageRole::Tool => "user", // Tool results go as user messages
                }.to_string(),
                content,
            }
        })
        .collect()
}
```

## 架构影响

### 符合 ROCode 宪法

- **第一条（唯一执行内核）**：工具调用循环由 `OrchestrationCore` 驱动
- **第四条（唯一工具调度）**：工具执行通过 `ToolRegistry` 统一调度
- **第五条（唯一状态所有权）**：`SessionManager` 管理包括工具调用在内的所有会话状态
- **第九条（副作用路径唯一）**：工具执行产生的副作用通过 `ToolRegistry` 统一管理

### 数据流

```
User Input
    ↓
OrchestrationCore.execute_prompt()
    ↓
Session.add_user_message()
    ↓
┌─────────────────────────────────────┐
│         LLM 循环                     │
│  ┌───────────────────────────────┐  │
│  │ Provider.chat(with tools)     │  │
│  └───────────┬───────────────────┘  │
│              │                       │
│              ▼                       │
│  ┌───────────────────────────────┐  │
│  │ Has tool_calls?               │  │
│  └───────┬───────────┬───────────┘  │
│          │ Yes       │ No            │
│          ▼           │               │
│  ┌───────────────┐  │               │
│  │ Execute tools │  │               │
│  └───────┬───────┘  │               │
│          │          │               │
│          ▼          │               │
│  ┌───────────────┐  │               │
│  │ Save results  │  │               │
│  └───────┬───────┘  │               │
│          │          │               │
│          └──────────┘               │
│              │                       │
│              ▼                       │
│         Continue loop                │
│              or                      │
│         Break (no tools)             │
└─────────────────────────────────────┘
    ↓
Session.add_assistant_message()
    ↓
Return PromptExecutionResult
```

## 测试策略

### 单元测试

```rust
#[tokio::test]
async fn test_tool_call_execution() {
    // 1. 创建 mock provider（返回 tool_calls）
    // 2. 创建 mock tool
    // 3. 执行 prompt
    // 4. 验证工具被调用
    // 5. 验证工具结果被保存到会话
    // 6. 验证 LLM 收到工具结果并继续
}

#[tokio::test]
async fn test_multiple_tool_calls() {
    // 测试单次响应中多个工具调用
}

#[tokio::test]
async fn test_tool_call_loop() {
    // 测试多轮工具调用（LLM → tool → LLM → tool → LLM）
}

#[tokio::test]
async fn test_tool_error_handling() {
    // 测试工具执行失败的情况
}
```

### 集成测试

1. **简单工具调用**
   - 用户请求需要工具的任务
   - LLM 返回 tool_call
   - 工具执行成功
   - LLM 使用工具结果生成最终响应

2. **多轮工具调用**
   - LLM 调用工具 A
   - 根据 A 的结果调用工具 B
   - 使用 A 和 B 的结果生成最终响应

3. **工具错误处理**
   - 工具执行失败
   - LLM 收到错误信息
   - LLM 尝试其他方法或报告错误

## 已知限制

1. **串行执行**：工具按顺序执行，不支持并发
   - 如果 LLM 返回多个独立的 tool_calls，仍然串行执行
   - 未来可以添加并发执行优化

2. **无超时控制**：工具执行没有超时限制
   - 长时间运行的工具可能阻塞整个循环
   - 未来应该添加工具级别的超时

3. **无循环检测**：没有检测无限工具调用循环
   - 如果 LLM 陷入循环（tool → LLM → same tool → ...）
   - 未来应该添加最大循环次数限制

4. **无流式输出**：工具调用期间没有中间状态反馈
   - 用户看不到工具执行进度
   - Phase 6.3 将添加流式输出支持

## 性能考虑

### 工具执行开销

- 每个工具调用都是一次异步操作
- 工具执行时间取决于具体工具（文件读取、网络请求等）
- 多轮工具调用会显著增加总响应时间

### 会话大小增长

- 每次工具调用都会添加 2 条消息（tool_call + tool_result）
- 长对话中的多次工具调用会快速增加会话大小
- 需要在 Phase 6.4 中实现上下文压缩

### 锁策略

```rust
// ❌ 错误：持有锁期间执行工具（可能阻塞很久）
let sessions_guard = sessions.lock().await;
let result = execute_tool(tools, &tool_call).await?; // 持有锁！
drop(sessions_guard);

// ✅ 正确：释放锁，执行工具，重新获取锁
drop(sessions_guard);
let result = execute_tool(tools, &tool_call).await?;
let mut sessions_guard = sessions.lock().await;
```

## 验收标准

- [ ] `execute_prompt_with_tools` 函数实现
- [ ] `extract_tool_calls` 函数实现
- [ ] `execute_tool` 函数实现
- [ ] `Session::add_tool_call` 方法实现
- [ ] `Session::add_tool_result` 方法实现
- [ ] `build_messages_with_tools` 函数实现
- [ ] `OrchestrationCore` 添加 `ToolRegistry` 字段
- [ ] `OrchestrationCore::execute_prompt` 调用新实现
- [ ] 单元测试通过
- [ ] 整个 workspace 编译通过
- [ ] 工具调用循环测试通过
- [ ] 多轮工具调用测试通过

## 下一步（Phase 6.3）

实现流式输出支持：
1. 流式接收 provider 响应
2. 实时发送文本片段给客户端
3. 流式处理工具调用
4. 保持会话状态一致性
5. 支持取消和错误恢复
