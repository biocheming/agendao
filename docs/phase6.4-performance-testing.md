# Phase 6.4: 性能测试（计划归档）

这是一份阶段测试计划稿，保留的是当时希望验证的性能目标、指标和方法。它不等同于当前已经持续维护的正式性能基线。

## 目标

验证混合传输层架构的核心目标：**TUI 启动时间从 500-1000ms 降低到 50-100ms（10x 提升）**

## 测试维度

### 1. 启动性能

**测试场景**：
- Direct Transport（TUI/CLI 本地使用）
- Unix Socket Transport（本地进程间通信）
- HTTP Transport（远程访问）

**测量指标**：
- 冷启动时间（首次初始化）
- 热启动时间（已有 provider 缓存）
- 内存占用
- 依赖加载时间

### 2. 执行性能

**测试场景**：
- 单轮对话（无工具调用）
- 多轮对话（3-5 轮）
- 工具调用（1-3 个工具）
- 流式输出 vs 非流式输出

**测量指标**：
- 首字节时间（TTFB）
- 总执行时间
- 吞吐量（tokens/s）
- 延迟分布（p50, p95, p99）

### 3. 传输层性能

**测试场景**：
- Direct vs Unix Socket vs HTTP
- 小消息（< 1KB）
- 中等消息（1-10KB）
- 大消息（> 10KB）

**测量指标**：
- 往返时间（RTT）
- 序列化/反序列化开销
- 传输开销

### 4. 并发性能

**测试场景**：
- 单会话顺序请求
- 多会话并发请求
- 高负载场景（100+ 并发）

**测量指标**：
- 吞吐量（requests/s）
- 响应时间分布
- 资源利用率（CPU、内存）

## 实现计划

### Step 1: 基准测试框架

创建 `crates/agendao-orchestrator/benches/` 目录，使用 Criterion.rs：

```rust
// benches/startup_benchmark.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use agendao_orchestrator::OrchestrationCore;

fn benchmark_cold_start(c: &mut Criterion) {
    c.bench_function("cold_start", |b| {
        b.iter(|| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let config = agendao_config::Config::default();
                let core = OrchestrationCore::new(&config).await.unwrap();
                black_box(core);
            });
        });
    });
}

criterion_group!(benches, benchmark_cold_start);
criterion_main!(benches);
```

### Step 2: 启动性能测试

测试内容：
1. `OrchestrationCore::new()` 时间
2. Provider 注册时间
3. Tool 注册时间
4. 总启动时间

对比：
- 旧架构（HTTP 服务器启动）
- 新架构（Direct Transport）

### Step 3: 执行性能测试

测试内容：
1. `execute_prompt()` 端到端时间
2. `execute_prompt_streaming()` 首字节时间
3. 工具调用开销
4. 会话状态更新开销

### Step 4: 传输层性能测试

测试内容：
1. Direct Transport（基准）
2. Unix Socket Transport（vs Direct）
3. HTTP Transport（vs Unix Socket）

### Step 5: 并发性能测试

测试内容：
1. 单会话顺序请求（基准）
2. 10 并发会话
3. 100 并发会话
4. 资源利用率监控

## 验收标准

### 核心目标

- [ ] TUI 启动时间 < 100ms（Direct Transport）
- [ ] Unix Socket 启动时间 < 150ms
- [ ] HTTP 启动时间 < 200ms（可接受，因为需要网络连接）

### 性能指标

- [ ] Direct Transport 比 HTTP 快至少 5x
- [ ] Unix Socket 比 HTTP 快至少 3x
- [ ] 流式输出首字节时间 < 50ms
- [ ] 工具调用开销 < 10ms（不含工具执行时间）

### 资源使用

- [ ] 内存占用 < 50MB（Direct Transport）
- [ ] CPU 使用率 < 10%（空闲时）
- [ ] 并发 100 会话时内存 < 500MB

## 测试工具

### Criterion.rs

用于微基准测试：
- 精确的时间测量
- 统计分析（均值、标准差、p95/p99）
- 性能回归检测
- HTML 报告生成

### 自定义性能测试

用于端到端测试：
- 真实场景模拟
- 多维度指标收集
- 对比分析

## 输出

### 性能报告

生成 `docs/performance-report.md`：
- 测试环境描述
- 各场景性能数据
- 对比分析（旧 vs 新）
- 瓶颈分析
- 优化建议

### 基准数据

保存到 `benches/baseline/`：
- 各场景的基准数据
- 用于未来的性能回归检测

## 参考

- Criterion.rs: https://github.com/bheisler/criterion.rs
- Rust Performance Book: https://nnethercote.github.io/perf-book/
