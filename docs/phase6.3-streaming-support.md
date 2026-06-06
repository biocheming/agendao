# Phase 6.3: 流式输出支持（阶段记录）

这份文档记录流式输出能力接入时的阶段方案与拆分方式。当前如果要理解现行前端行为，应优先看产品文档；这里主要用来保留设计背景与实现思路。

## 目标

为 `OrchestrationCore` 添加流式输出支持，允许客户端实时接收 LLM 响应。

## 背景

当前实现（Phase 6.2）使用非流式 API：
- 调用 `provider.chat()` 等待完整响应
- 用户体验差（长时间等待无反馈）
- 无法实时显示生成的文本

流式输出的优势：
- 实时反馈（用户立即看到生成开始）
- 更好的用户体验
- 支持长响应的渐进式显示
- 支持工具调用的实时显示

## 架构设计

### 1. 流式 API 签名

```rust
pub async fn execute_prompt_streaming(
    &self,
    session_id: &str,
    text: &str,
    options: PromptExecutionOptions,
) -> Result<StreamResult, OrchestratorError>
```

返回 `StreamResult = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>`

### 2. StreamEvent 类型

Provider 层已定义完整的 `StreamEvent` 枚举：
- `Start` - 流开始
- `TextDelta(String)` - 增量文本
- `TextStart` / `TextEnd` - 文本块边界
- `ReasoningStart` / `ReasoningDelta` / `ReasoningEnd` - 推理块
- `ToolCallStart` / `ToolCallDelta` / `ToolCallEnd` - 工具调用
- `FinishStep` - 步骤完成（带 usage）
- `Done` - 流结束
- `Error(String)` - 错误

### 3. 工具调用循环的流式版本

流式工具调用循环的挑战：
1. LLM 流式输出 → 检测 `ToolCallEnd` 事件
2. 执行工具（非流式）
3. 将工具结果添加到会话
4. 继续 LLM 流式输出

流程：
```
User message → Session
  ↓
LLM Stream (round 1)
  ├─ TextDelta events → forward to client
  ├─ ToolCallStart/Delta/End → forward to client
  └─ Done → check if has tool calls
      ↓
      If has tool calls:
        ├─ Execute tools (non-streaming)
        ├─ Save tool results to session
        └─ LLM Stream (round 2) → repeat
      ↓
      If no tool calls:
        └─ Stream finished
```

### 4. 会话状态管理

流式输出时的会话更新策略：
- **实时更新**：每个 `TextDelta` 都追加到当前消息
- **批量更新**：在 `Done` 事件时一次性保存完整消息
- **选择**：批量更新（简单、一致）

原因：
- 实时更新会产生大量小写操作
- 批量更新在流结束时保存完整内容
- 与非流式版本保持一致的会话状态

### 5. Usage 统计

Usage 信息在 `FinishStep` 事件中：
```rust
StreamEvent::FinishStep {
    finish_reason: Option<String>,
    usage: StreamUsage,
    provider_metadata: Option<serde_json::Value>,
}
```

累积策略：
- 在流处理过程中累积 usage
- 在 `Done` 事件后更新会话 usage

## 实现计划

### Step 1: 添加流式 API 到 OrchestrationCore

```rust
// crates/agendao-orchestrator/src/core.rs
impl OrchestrationCore {
    pub async fn execute_prompt_streaming(
        &self,
        session_id: &str,
        text: &str,
        options: PromptExecutionOptions,
    ) -> Result<StreamResult, OrchestratorError> {
        execute_prompt_streaming_with_session(
            &self.sessions,
            &self.providers,
            &self.tools,
            session_id,
            text,
            &options,
        )
        .await
    }
}
```

### Step 2: 实现 execute_prompt_streaming_with_session

```rust
// crates/agendao-orchestrator/src/prompt_execution.rs
pub async fn execute_prompt_streaming_with_session(
    sessions: &Arc<tokio::sync::Mutex<SessionManager>>,
    providers: &Arc<tokio::sync::RwLock<ProviderRegistry>>,
    tools: &Arc<tokio::sync::RwLock<ToolRegistry>>,
    session_id: &str,
    text: &str,
    options: &PromptExecutionOptions,
) -> Result<StreamResult, OrchestratorError>
```

实现要点：
1. 添加用户消息到会话
2. 获取 provider 和 tool definitions
3. 构建初始请求
4. 调用 `provider.chat_stream()`
5. 包装流以处理工具调用循环

### Step 3: 流式工具调用循环

使用 `stream::unfold` 实现状态机：

```rust
struct StreamState {
    sessions: Arc<Mutex<SessionManager>>,
    providers: Arc<RwLock<ProviderRegistry>>,
    tools: Arc<RwLock<ToolRegistry>>,
    session_id: String,
    options: PromptExecutionOptions,
    current_stream: Option<StreamResult>,
    accumulated_text: String,
    accumulated_tool_calls: Vec<ToolCall>,
    accumulated_usage: UsageInfo,
    round: u32,
}
```

状态转换：
1. **Streaming** - 转发 StreamEvent，累积文本和工具调用
2. **ToolExecution** - 执行工具，保存结果
3. **NextRound** - 开始新的 LLM 流
4. **Finished** - 保存最终消息，结束

### Step 4: 会话状态更新

在流结束时（`Done` 事件后）：
1. 保存助手消息（累积的文本）
2. 更新 usage 统计
3. 如果有工具调用，保存工具结果并继续循环

## 限制和未来改进

### Phase 6.3 限制

- ⚠️ 工具执行是非流式的（工具调用期间流会暂停）
- ⚠️ 无超时控制
- ⚠️ 无循环检测（最大迭代次数限制）
- ⚠️ 串行执行工具（不支持并发）

### 未来改进（Phase 7+）

- 流式工具执行（工具输出也流式返回）
- 并发工具执行
- 超时和取消支持
- 循环检测和保护
- 流式错误恢复

## 测试策略

### 单元测试

1. 流式文本输出（无工具调用）
2. 流式工具调用（单轮）
3. 流式工具调用（多轮）
4. Usage 统计累积
5. 错误处理

### 集成测试

1. 端到端流式对话
2. 工具调用循环
3. 会话状态一致性

## 验收标准

- [ ] `OrchestrationCore::execute_prompt_streaming` API 实现
- [ ] 流式文本输出正常工作
- [ ] 流式工具调用循环正常工作
- [ ] 会话状态正确更新（消息、usage）
- [ ] 所有测试通过
- [ ] Workspace 编译通过

## 参考

- Provider 层流式实现：`crates/agendao-provider/src/stream.rs`
- StreamEvent 定义：`crates/agendao-provider/src/stream.rs:9-98`
- 现有流式使用示例：`crates/agendao-server/src/routes/session/scheduler.rs:693-743`
