# 性能基准报告 (修订版 — 2026-05-27)

## 测试环境

- **平台**: Linux 6.8.0-117-generic
- **测试日期**: 2026-05-27
- **Provider**: Mock（无真实 LLM 调用，数字仅反映框架开销）
- **测试文件**: `crates/rocode-orchestrator/tests/test_performance.rs`
- **测试数**: 10

> **重要**: 所有数据为 **mock provider** 下的框架开销，不含网络延迟和模型推理时间。
> 真实 LLM 场景的端到端延迟通常由 API 调用主导（100-500ms），框架开销在此场景下可忽略。

## 基准结果

### 1. 启动性能（框架层面）

| 测试 | 实测 (avg) | 说明 |
|------|-----------|------|
| `OrchestrationCore::new` 冷启动 | ~42 µs | 独立构造，含 SessionManager/ProviderRegistry/ToolRegistry 创建 |
| `new_with_shared_authorities` 冷启动 | 97 ns | 共享 authority 注入，无新建实例（生产路径） |
| 带 Provider 注册 | ~24 µs | 冷启动 + 注册 1 个 mock provider |

**结论**: 共享 authority 构造（生产路径）为纳秒级，因只做 Arc clone 无新建。

### 2. 执行性能（框架层面）

| 测试 | 实测 (avg) | 说明 |
|------|-----------|------|
| 单轮对话 | ~73 µs | prompt → mock LLM → response |
| 流式输出 TTFB | ~41 µs | 首个 StreamEvent 到达时间 |
| 3 轮对话 | ~98 µs | 3 次 prompt + 3 次 mock response |

**结论**: 执行路径在微秒级完成（mock 场景）。多轮线性增长。

### 3. 操作开销

| 测试 | 实测 | 说明 |
|------|------|------|
| Tool 注册 | 8.4 µs | 注册 1 个 mock tool |
| Session 查询 | ~5 µs | get_session（session 不存在路径） |

### 4. 并发

| 测试 | 实测 | 说明 |
|------|------|------|
| 100 并发 session 创建 | 3.1 ms (31,817/sec) | tokio::spawn 并发，mock provider |

### 5. 热重载

| 测试 | 结果 |
|------|------|
| Provider 注册后立即可见 | ✅ 验证通过（共享 Arc 路径） |
| Config 修改后无需重启 | ⚠️ 本报告未单独实测；能力来自共享 `Arc<ConfigStore>` 设计，功能验证归属主线 2 |

## 未实测项（诚实口径）

以下指标在当前报告中 **无真实数据支撑**，标记为未测试：

### ⚠️ 未测试

- [ ] **Unix Socket 端到端启动时间** — 含 socket bind、listener accept 的完整路径
- [ ] **HTTP 服务器启动时间** — 含 axum TcpListener::bind + router 构建
- [ ] **工具调用循环开销** — tool use → execute → tool result → LLM round-trip
- [ ] **100 并发会话内存占用** — 未做 RSS/Heap profiling
- [ ] **真实 Provider 端到端延迟** — 未接入真实 LLM API

### 无法以 mock 测量

- 端到端 LLM 延迟（由网络 + 模型推理主导，mock 场景无意义）
- 上下文压缩性能（需大量历史消息积累）
- 磁盘 I/O 持久化开销（当前为内存存储）

## 测试方法

每个测试使用 `std::time::Instant` 测量，含 3 次预热 + 多次采样取 min/max/avg。
并发测试使用 `tokio::spawn` 并发模型。

## 结论

1. **框架开销极低**: 共享 authority 构造路径为纳秒级，执行路径为微秒级（mock 场景）
2. **Provider 热重载已验证**: 共享 `Arc<RwLock<ProviderRegistry>>` 路径下注册后立即可见
3. **Config 热重载结构已就绪**: 共享 `Arc<ConfigStore>` 路径已落地，但不计入本报告的性能实测项
4. **不具外推性**: 以上数据仅反映框架本身开销，不能外推为端到端性能
5. **所有未测项已诚实标注**: 不再以推断或"应该能达成"替代实测数据
