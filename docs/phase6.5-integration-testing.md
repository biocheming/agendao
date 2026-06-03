# Phase 6.5: 集成测试

## 目标

创建端到端集成测试，验证混合传输层架构的完整功能：
- 端到端对话流程
- 多轮对话与会话状态
- 工具调用循环
- 流式输出完整性
- 传输层互操作性

## 测试维度

### 1. 端到端对话流程

**测试场景**：
- 创建会话 → 发送消息 → 接收响应 → 验证会话状态
- 单轮对话（无工具）
- 多轮对话（保持上下文）
- 会话列表和查询

**验证点**：
- 会话正确创建和持久化
- 消息正确保存到会话历史
- Usage 统计正确累积
- 会话元数据正确更新

### 2. 工具调用集成

**测试场景**：
- LLM 请求工具调用
- 工具执行并返回结果
- LLM 处理工具结果并继续
- 多轮工具调用循环

**验证点**：
- 工具定义正确传递给 provider
- 工具调用正确解析和执行
- 工具结果正确保存到会话
- 工具调用循环正确终止

### 3. 流式输出集成

**测试场景**：
- 流式文本输出
- 流式工具调用
- 流式多轮对话
- 流式错误处理

**验证点**：
- 所有事件正确发出（Start, TextDelta, ToolCall*, FinishStep, Done）
- 文本正确累积
- 工具调用正确累积
- 会话状态在 Done 后正确更新

### 4. 错误处理

**测试场景**：
- Provider 错误
- 工具执行错误
- 无效的 model spec
- 会话不存在

**验证点**：
- 错误正确传播
- 会话状态保持一致
- 资源正确清理

### 5. 并发场景

**测试场景**：
- 多个会话并发执行
- 同一会话的并发请求
- Provider 并发访问

**验证点**：
- 无数据竞争
- 会话状态隔离
- 性能无明显退化

## 实现计划

### Step 1: 端到端对话测试

创建 `crates/agendao-orchestrator/tests/test_integration_e2e.rs`：

```rust
#[tokio::test]
async fn test_e2e_single_turn_dialogue() {
    // 1. 创建 OrchestrationCore
    // 2. 注册 mock provider
    // 3. 执行单轮对话
    // 4. 验证响应和会话状态
}

#[tokio::test]
async fn test_e2e_multi_turn_dialogue() {
    // 1. 创建 OrchestrationCore
    // 2. 注册 mock provider
    // 3. 执行 3 轮对话
    // 4. 验证每轮的响应和会话状态
    // 5. 验证上下文正确传递
}

#[tokio::test]
async fn test_e2e_session_management() {
    // 1. 创建多个会话
    // 2. 列出会话
    // 3. 查询特定会话
    // 4. 验证会话元数据
}
```

### Step 2: 工具调用集成测试

创建 `crates/agendao-orchestrator/tests/test_integration_tools.rs`：

```rust
#[tokio::test]
async fn test_tool_calling_single_tool() {
    // 1. 注册 mock provider（返回工具调用）
    // 2. 注册 mock tool
    // 3. 执行对话
    // 4. 验证工具被调用
    // 5. 验证工具结果保存到会话
}

#[tokio::test]
async fn test_tool_calling_multiple_rounds() {
    // 1. 注册 mock provider（多轮工具调用）
    // 2. 注册 mock tools
    // 3. 执行对话
    // 4. 验证多轮工具调用循环
    // 5. 验证最终响应
}

#[tokio::test]
async fn test_tool_execution_error() {
    // 1. 注册会失败的 mock tool
    // 2. 执行对话
    // 3. 验证错误处理
    // 4. 验证会话状态一致性
}
```

### Step 3: 流式输出集成测试

创建 `crates/agendao-orchestrator/tests/test_integration_streaming.rs`：

```rust
#[tokio::test]
async fn test_streaming_e2e() {
    // 1. 创建 OrchestrationCore
    // 2. 注册 streaming mock provider
    // 3. 执行流式对话
    // 4. 收集所有事件
    // 5. 验证事件顺序和内容
    // 6. 验证会话状态
}

#[tokio::test]
async fn test_streaming_with_tools() {
    // 1. 注册 streaming provider（返回工具调用）
    // 2. 注册 mock tool
    // 3. 执行流式对话
    // 4. 验证流式工具调用循环
    // 5. 验证最终会话状态
}

#[tokio::test]
async fn test_streaming_multi_turn() {
    // 1. 执行多轮流式对话
    // 2. 验证每轮的流式输出
    // 3. 验证上下文正确传递
}
```

### Step 4: 错误处理测试

创建 `crates/agendao-orchestrator/tests/test_integration_errors.rs`：

```rust
#[tokio::test]
async fn test_provider_error_handling() {
    // 1. 注册会失败的 mock provider
    // 2. 执行对话
    // 3. 验证错误正确返回
    // 4. 验证会话状态一致性
}

#[tokio::test]
async fn test_invalid_model_spec() {
    // 1. 使用无效的 model spec
    // 2. 验证错误消息
}

#[tokio::test]
async fn test_session_not_found() {
    // 1. 查询不存在的会话
    // 2. 验证错误处理
}
```

### Step 5: 并发测试

创建 `crates/agendao-orchestrator/tests/test_integration_concurrent.rs`：

```rust
#[tokio::test]
async fn test_concurrent_sessions() {
    // 1. 创建 10 个并发会话
    // 2. 并发执行对话
    // 3. 验证所有会话状态正确
    // 4. 验证无数据竞争
}

#[tokio::test]
async fn test_concurrent_requests_same_session() {
    // 1. 对同一会话发起并发请求
    // 2. 验证请求顺序处理
    // 3. 验证会话状态一致性
}
```

## 验收标准

### 端到端对话

- [ ] 单轮对话正确执行
- [ ] 多轮对话保持上下文
- [ ] 会话正确创建和持久化
- [ ] 会话列表和查询正确

### 工具调用

- [ ] 单个工具调用正确执行
- [ ] 多轮工具调用循环正确
- [ ] 工具结果正确保存到会话
- [ ] 工具执行错误正确处理

### 流式输出

- [ ] 流式文本输出正确
- [ ] 流式工具调用正确
- [ ] 流式多轮对话正确
- [ ] 会话状态在流结束后正确更新

### 错误处理

- [ ] Provider 错误正确传播
- [ ] 工具错误正确处理
- [ ] 无效输入正确拒绝
- [ ] 会话状态保持一致

### 并发

- [ ] 多会话并发执行正确
- [ ] 无数据竞争
- [ ] 会话状态隔离

## Mock 组件

### MockProvider

需要支持多种行为模式：
- 简单文本响应
- 工具调用响应
- 多轮工具调用
- 错误响应
- 流式输出

### MockTool

需要支持：
- 成功执行
- 失败执行
- 可配置的执行时间
- 可配置的返回值

## 测试数据

### 对话场景

1. **简单问答**：
   - User: "Hello"
   - Assistant: "Hi there!"

2. **多轮对话**：
   - User: "What's the weather?"
   - Assistant: "I need your location."
   - User: "San Francisco"
   - Assistant: "It's sunny in San Francisco."

3. **工具调用**：
   - User: "What's 2+2?"
   - Assistant: [calls calculator tool]
   - Tool: "4"
   - Assistant: "The answer is 4."

## 输出

### 测试报告

生成 `docs/integration-test-report.md`：
- 测试覆盖率
- 测试结果汇总
- 发现的问题
- 修复建议

### 测试统计

- 总测试数
- 通过/失败/跳过
- 测试覆盖的场景
- 未覆盖的场景

## 参考

- Phase 6.1: 完整的 execute_prompt
- Phase 6.2: 工具调用支持
- Phase 6.3: 流式输出支持
- Phase 6.4: 性能测试
