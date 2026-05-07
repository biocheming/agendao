# 生命周期钩子（Lifecycle Hooks）完整参考

ROCode 的生命周期钩子系统允许在 orchestrator 执行流程的关键节点注入自定义逻辑。它通过 `LifecycleHook` trait 定义，被 scheduler、agent tree、skill graph 等执行引擎在运行时调用。

---

## 目录

- [架构概览](#架构概览)
- [LifecycleHook Trait](#lifecyclehook-trait)
- [Orchestration 钩子](#orchestration-钩子)
- [Step 钩子](#step-钩子)
- [Tool 钩子](#tool-钩子)
- [Scheduler Stage 钩子](#scheduler-stage-钩子)
- [钩子在执行引擎中的调用时机](#钩子在执行引擎中的调用时机)
- [ExecutionContext 传播](#executioncontext-传播)
- [NoopLifecycleHook](#nooplifecyclehook)
- [插件注册钩子](#插件注册钩子)

---

## 架构概览

```
用户请求
  |
  v
Scheduler Stage Loop
  |
  +-- [on_scheduler_stage_start]  <- stage 开始
  |
  +-- AgentTreeOrchestrator / SkillGraphOrchestrator / SkillListOrchestrator
  |     |
  |     +-- [on_orchestration_start]  <- agent 执行开始
  |     |
  |     +-- LLM Loop (step 1..N)
  |     |     |
  |     |     +-- [on_step_start]  <- 单步开始
  |     |     |
  |     |     +-- Tool Call (如果有)
  |     |     |     |
  |     |     |     +-- [on_tool_start]  <- 工具调用开始
  |     |     |     +-- tool execution
  |     |     |     +-- [on_tool_end]    <- 工具调用结束
  |     |     |
  |     |     +-- (下一步 或 结束)
  |     |
  |     +-- [on_orchestration_end]  <- agent 执行结束
  |
  +-- [on_scheduler_stage_content]  <- stage 内容增量
  +-- [on_scheduler_stage_reasoning] <- stage 推理增量
  +-- [on_scheduler_stage_usage]    <- stage token 使用
  +-- [on_scheduler_stage_end]      <- stage 结束
  |
  v
下一个 Stage 或 完成
```

---

## LifecycleHook Trait

`LifecycleHook` 是所有生命周期钩子的核心 trait，定义在 `rocode-orchestrator/src/traits.rs` 中。

```rust
#[async_trait]
pub trait LifecycleHook: Send + Sync {
    async fn on_orchestration_start(
        &self,
        agent_name: &str,
        max_steps: Option<u32>,
        exec_ctx: &ExecutionContext,
    );

    async fn on_step_start(
        &self,
        agent_name: &str,
        model_id: &str,
        step: u32,
        exec_ctx: &ExecutionContext,
    );

    async fn on_tool_start(
        &self,
        agent_name: &str,
        tool_call_id: &str,
        tool_name: &str,
        tool_args: &serde_json::Value,
        exec_ctx: &ExecutionContext,
    );

    async fn on_tool_end(
        &self,
        agent_name: &str,
        tool_call_id: &str,
        tool_name: &str,
        tool_output: &ToolOutput,
        exec_ctx: &ExecutionContext,
    );

    async fn on_orchestration_end(
        &self,
        agent_name: &str,
        steps: u32,
        exec_ctx: &ExecutionContext,
    );

    async fn on_scheduler_stage_start(
        &self,
        agent_name: &str,
        stage_name: &str,
        stage_index: u32,
        capabilities: Option<&SchedulerStageCapabilities>,
        exec_ctx: &ExecutionContext,
    );

    async fn on_scheduler_stage_end(
        &self,
        agent_name: &str,
        stage_name: &str,
        stage_index: u32,
        stage_total: u32,
        content: &str,
        exec_ctx: &ExecutionContext,
    );

    async fn on_scheduler_stage_content(
        &self,
        stage_name: &str,
        stage_index: u32,
        content_delta: &str,
        exec_ctx: &ExecutionContext,
    );

    async fn on_scheduler_stage_reasoning(
        &self,
        stage_name: &str,
        stage_index: u32,
        reasoning_delta: &str,
        exec_ctx: &ExecutionContext,
    );

    async fn on_scheduler_stage_usage(
        &self,
        stage_name: &str,
        stage_index: u32,
        usage: &StepUsage,
        finalized: bool,
        exec_ctx: &ExecutionContext,
    );
}
```

钩子方法分为以下几类：

| 类别 | 钩子 | 说明 |
|------|------|------|
| Orchestration | `on_orchestration_start` / `on_orchestration_end` | Agent 执行生命周期 |
| Step | `on_step_start` | LLM 单步执行 |
| Tool | `on_tool_start` / `on_tool_end` | 工具调用生命周期 |
| Scheduler Stage | `on_scheduler_stage_start` / `on_scheduler_stage_end` | Scheduler stage 生命周期 |
| Scheduler Stream | `on_scheduler_stage_content` / `on_scheduler_stage_reasoning` / `on_scheduler_stage_usage` | 流式输出与使用量 |

---

## Orchestration 钩子

### on_orchestration_start

在 agent 开始执行（进入 LLM 循环）时触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 当前 agent 的名称 |
| `max_steps` | `Option<u32>` | 最大执行步数限制 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**调用时机：** 在 `SkillListOrchestrator::execute()` 的入口处，LLM 循环开始前。

**典型用途：**

- 记录 agent 开始执行的日志
- 初始化 agent 级别的追踪指标
- 通知 UI 显示 agent 活动状态

### on_orchestration_end

在 agent 完成所有执行步骤后触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 当前 agent 的名称 |
| `steps` | `u32` | 实际执行的总步数 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**调用时机：** 在 `SkillListOrchestrator::execute()` 的出口处，LLM 循环结束后。

**典型用途：**

- 记录 agent 完成执行的日志和步数统计
- 聚合 agent 执行指标
- 通知 UI 清除 agent 活动状态

---

## Step 钩子

### on_step_start

在 LLM 的每一步（一次 model 调用）开始前触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 当前 agent 的名称 |
| `model_id` | `&str` | 正在使用的模型 ID |
| `step` | `u32` | 当前步数（从 1 开始） |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**调用时机：** LLM 循环的每次迭代开始时，发送消息给 model 之前。

**典型用途：**

- 跟踪 LLM 调用步数
- 监控模型使用情况
- 实现步数限制的观察者逻辑

---

## Tool 钩子

### on_tool_start

在工具执行前触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 发起工具调用的 agent |
| `tool_call_id` | `&str` | 工具调用 ID |
| `tool_name` | `&str` | 工具名称 |
| `tool_args` | `&Value` | 工具调用参数 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**默认实现：** 空操作（noop），即不实现此方法时不会有任何行为。

**调用时机：** `ToolRunner` 执行工具前。

**典型用途：**

- 工具调用审计日志
- 参数校验或前置检查
- 通知 UI 显示工具调用状态

### on_tool_end

在工具执行完成后触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 发起工具调用的 agent |
| `tool_call_id` | `&str` | 工具调用 ID |
| `tool_name` | `&str` | 工具名称 |
| `tool_output` | `&ToolOutput` | 工具执行结果 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**ToolOutput 结构：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `output` | `String` | 工具输出文本 |
| `is_error` | `bool` | 是否为错误 |
| `title` | `Option<String>` | 输出标题 |
| `metadata` | `Option<Value>` | 工具返回的元数据 |

**默认实现：** 空操作（noop）。

**调用时机：** `ToolRunner` 收到工具执行结果后。

**典型用途：**

- 记录工具执行结果
- 统计工具使用频率
- 捕获工具元数据（session_id、agent_task_id 等）

---

## Scheduler Stage 钩子

### on_scheduler_stage_start

在 scheduler stage 开始执行时触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 当前 agent 名称 |
| `stage_name` | `&str` | stage 名称（如 `"execution-orchestration"`） |
| `stage_index` | `u32` | stage 在序列中的索引（从 0 开始） |
| `capabilities` | `Option<&SchedulerStageCapabilities>` | 该 stage 的能力配置 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**SchedulerStageCapabilities 结构：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `skill_list` | `Vec<SchedulerSkillRef>` | 可用的 skill 列表 |
| `agents` | `Vec<String>` | 可用的 agent 名称列表 |
| `categories` | `Vec<String>` | 可用的能力分类 |
| `attached_session` | `bool` | 是否创建子会话 |

**默认实现：** 空操作。

**调用时机：** scheduler 开始执行某个 stage 的逻辑前。

**典型用途：**

- 通知 UI 显示当前 stage 进度
- 记录 stage 能力配置到审计日志
- 初始化 stage 级别的追踪指标

### on_scheduler_stage_end

在 scheduler stage 执行完成后触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `agent_name` | `&str` | 当前 agent 名称 |
| `stage_name` | `&str` | stage 名称 |
| `stage_index` | `u32` | stage 索引 |
| `stage_total` | `u32` | 总 stage 数量 |
| `content` | `&str` | stage 的输出内容 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**默认实现：** 空操作。

**调用时机：** scheduler 完成某个 stage 的执行后。

**典型用途：**

- 记录 stage 执行结果
- 计算 stage 执行耗时
- 通知 UI 更新进度条

### on_scheduler_stage_content

在 stage 输出内容增量时触发（流式）。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `stage_name` | `&str` | stage 名称 |
| `stage_index` | `u32` | stage 索引 |
| `content_delta` | `&str` | 内容增量 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**默认实现：** 空操作。

**调用时机：** 每次 LLM 输出文本增量时。

**典型用途：**

- 实时流式显示 stage 输出
- 转发到 WebSocket 客户端

### on_scheduler_stage_reasoning

在 stage 输出推理增量时触发（扩展思考模型的思维过程）。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `stage_name` | `&str` | stage 名称 |
| `stage_index` | `u32` | stage 索引 |
| `reasoning_delta` | `&str` | 推理增量 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**默认实现：** 空操作。

**调用时机：** 支持 extended thinking 的模型输出推理内容时。

**典型用途：**

- 展示模型推理过程
- 调试和审计模型思维链

### on_scheduler_stage_usage

在报告 stage 的 token 使用量时触发。

**参数：**

| 参数 | 类型 | 说明 |
|------|------|------|
| `stage_name` | `&str` | stage 名称 |
| `stage_index` | `u32` | stage 索引 |
| `usage` | `&StepUsage` | token 使用量 |
| `finalized` | `bool` | 是否为最终使用量 |
| `exec_ctx` | `&ExecutionContext` | 执行上下文 |

**默认实现：** 空操作。

**调用时机：** 每次 LLM 返回 token 使用量信息时。`finalized` 为 true 时表示该步的最终使用量。

**典型用途：**

- 跟踪 token 消耗
- 实现预算控制
- 成本估算

---

## 钩子在执行引擎中的调用时机

### SkillListOrchestrator（单 agent 执行）

```
execute(input, ctx)
  |
  +-- [on_orchestration_start(agent_name, max_steps)]
  |
  +-- loop {
  |     +-- [on_step_start(agent_name, model_id, step)]
  |     +-- model.chat_stream(...)
  |     +-- foreach tool_call {
  |     |     +-- [on_tool_start(agent_name, call_id, tool_name, args)]
  |     |     +-- tool_executor.execute(...)
  |     |     +-- [on_tool_end(agent_name, call_id, tool_name, output)]
  |     |   }
  |     +-- (continue or break)
  |   }
  |
  +-- [on_orchestration_end(agent_name, total_steps)]
```

### AgentTreeOrchestrator（树形执行）

```
execute_node(root, input, ctx)
  |
  +-- SkillListOrchestrator(root.agent).execute(...)  // 触发完整 orchestration 钩子
  |
  +-- if has children:
  |     +-- execute_children(parallel or sequential)
  |     |     +-- for each child:
  |     |          +-- execute_node(child, ...)  // 递归，触发完整钩子
  |     |
  |     +-- SkillListOrchestrator(root.agent).execute(aggregation_task)
  |        // 再次触发 orchestration 钩子（聚合轮）
```

### Scheduler（Stage 循环）

```
for each stage in stages:
  |
  +-- [on_scheduler_stage_start(agent_name, stage_name, index, capabilities)]
  |
  +-- execute_stage(...)  // 内部触发 AgentTree/SkillGraph 的钩子
  |
  +-- [on_scheduler_stage_content(reasoning)]  // 流式内容
  +-- [on_scheduler_stage_content(content)]     // 流式内容
  +-- [on_scheduler_stage_usage(usage, finalized)]
  |
  +-- [on_scheduler_stage_end(agent_name, stage_name, index, total, content)]
```

---

## ExecutionContext 传播

每个钩子都接收 `&ExecutionContext`，包含以下信息：

```rust
pub struct ExecutionContext {
    pub session_id: String,           // 会话 ID
    pub workdir: String,              // 工作目录
    pub agent_name: String,           // 当前 agent 名称
    pub metadata: HashMap<String, Value>,  // 扩展元数据
}
```

`metadata` 字段在多层执行中会逐层扩展：

- Scheduler 注入 stage 信息
- AgentTree 注入 tree 层级信息
- Tool 输出注入 session 和 task 标识

钩子实现可以通过 `metadata` 获取执行链路的完整上下文。

---

## NoopLifecycleHook

`NoopLifecycleHook` 是 `LifecycleHook` 的空实现，用于不需要任何钩子行为的场景：

```rust
pub struct NoopLifecycleHook;

#[async_trait]
impl LifecycleHook for NoopLifecycleHook {
    async fn on_orchestration_start(&self, _: &str, _: Option<u32>, _: &ExecutionContext) {}
    async fn on_step_start(&self, _: &str, _: &str, _: u32, _: &ExecutionContext) {}
    async fn on_orchestration_end(&self, _: &str, _: u32, _: &ExecutionContext) {}
}
```

注意：`NoopLifecycleHook` 只实现了三个必需方法（`on_orchestration_start`、`on_step_start`、`on_orchestration_end`），其他方法使用 trait 的默认空实现。

在测试中通常使用自定义的 `TestLifecycleHook`（空实现所有方法），而不是 `NoopLifecycleHook`。

---

## 插件注册钩子

`LifecycleHook` 通过 `OrchestratorContext` 注入到所有执行引擎：

```rust
pub struct OrchestratorContext {
    pub lifecycle_hook: Arc<dyn LifecycleHook>,  // 钩子实例
    // ... 其他字段
}
```

### 集成点

宿主应用（如 `rocode` 产品壳，或测试/工具宿主）在创建 `OrchestratorContext` 时注入具体的 `LifecycleHook` 实现。该实现可以：

1. **直接实现 trait** -- 在宿主代码中写一个 struct 实现 `LifecycleHook`
2. **组合多个钩子** -- 通过内部 `Vec<Arc<dyn LifecycleHook>>` 实现多播
3. **桥接到外部系统** -- 在钩子中发送 HTTP 请求、写入日志文件、更新 UI 状态

### 在 Scheduler 中的传播

Scheduler 在执行每个 stage 时，将 `LifecycleHook` 传递给底层执行引擎：

```
Scheduler
  -> AgentTreeOrchestrator (或 SkillGraphOrchestrator)
    -> SkillListOrchestrator
      -> LifecycleHook.on_step_start / on_tool_start / on_tool_end
```

所有层级的钩子调用都通过同一个 `LifecycleHook` 实例，确保事件不丢失。

### Stage Context 传播

`AgentTreeOrchestrator` 通过 `set_stage_context(stage_name, stage_index)` 将当前 stage 信息传播到每个子 orchestrator，使 `on_step_start` 等钩子中的 `ExecutionContext` 包含正确的 stage 上下文。

---

## 钩子方法总结

| 钩子 | 触发时机 | 必须实现 | 关键参数 |
|------|---------|---------|---------|
| `on_orchestration_start` | agent 执行开始 | 是 | agent_name, max_steps |
| `on_step_start` | LLM 单步开始 | 是 | agent_name, model_id, step |
| `on_orchestration_end` | agent 执行结束 | 是 | agent_name, steps |
| `on_tool_start` | 工具调用开始 | 否（默认 noop） | tool_call_id, tool_name, tool_args |
| `on_tool_end` | 工具调用结束 | 否（默认 noop） | tool_call_id, tool_name, tool_output |
| `on_scheduler_stage_start` | Stage 开始 | 否（默认 noop） | stage_name, stage_index, capabilities |
| `on_scheduler_stage_end` | Stage 结束 | 否（默认 noop） | stage_name, stage_index, stage_total, content |
| `on_scheduler_stage_content` | 流式内容 | 否（默认 noop） | content_delta |
| `on_scheduler_stage_reasoning` | 流式推理 | 否（默认 noop） | reasoning_delta |
| `on_scheduler_stage_usage` | Token 使用量 | 否（默认 noop） | usage, finalized |
