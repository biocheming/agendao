<p>
  <img src="icons/logo.svg" alt="AgenDao" width="280" />
</p>

# AgenDao

> **"完成"不是一个感觉，而是一组可以逐条核对的条件。**

大多数 AI 编码工具解决的是"怎么做"的问题——怎么生成代码、怎么调用工具、怎么多步推理。AgenDao 解决的是另一个问题：**当一段软件工作持续数小时、跨越多个 session、涉及多个前端、经过多次 fork 和回滚之后，系统凭什么还能说"这件事做完了"？**

答案不在模型能力上，在治理上。

当前版本：`v2026.6.6`

---

## 一句话理解 AgenDao

AgenDao 和其他 AI Agent工具属于同一类：本地编码智能体运行时。但它的设计重心不在"让模型更聪明"，而在**让系统更可信**。

具体来说，AgenDao 不信任"代码编译通过 + 测试通过 = 完成"。它在整个运行时里强制了 11 条宪法约束，并要求每次改进声明"完成"之前通过 8 步检查清单——包括全链路 trace、多 crate 同名类型一致性扫描、可观测读写双路径验证、E2E 测试走真实入口而非手拼中间对象。编译通过被当作"完成"的替代品，是 AgenDao 设计中最明确要杜绝的行为偏差。

---

## 11 条宪法（不是原则，是约束）

AgenDao 的设计不是从功能列表倒推出来的，而是从一条宗旨推出来的：

> **每个语义域有且仅有一个权威；适配层引用权威，不复制权威。**

这条宗旨展开为 11 条可执行的宪法：

| 宪法 | 约束 | 违规信号 |
|------|------|---------|
| 唯一执行内核 | 所有 LLM 循环由唯一内核驱动，适配层不得自建循环 | 适配层中出现独立的 `while model→tool→model` |
| 唯一配置真相 | 配置只有一份活真相，读取返回引用而非副本 | `clone()` 后独立缓存的配置副本 |
| 唯一权限裁决 | 权限判定只在一个地方发生，外部只提交请求 | 多个模块各自实现 `allow/ask/deny` 匹配 |
| 唯一工具调度 | 工具通过统一调度抽象，不绕过抽象直调注册表 | `registry.get()` 出现在非调度层 |
| 唯一状态所有权 | 每个状态域有且仅有一个 owner | 外部模块直接写入其他模块的内部字段 |
| 唯一插件契约 | 插件通过单一协议通信，副作用不绕过前五条 | 插件自建循环、独立判定权限 |
| 生命周期对称性 | 注册即承诺注销，创建即承诺销毁 | 不对称的 init/cleanup 配对 |
| 可观测性权利 | 每个活跃执行体在权威注册表中可观测 | 运行中的执行体不在任何注册表中 |
| 副作用路径唯一 | 副作用必须经由编排层，适配层只做只读查询 | 适配层直接调用领域服务写操作 |
| 唯一提示面权威 | serialized prompt surface 由唯一权威构造 | 多个层各自拼接 system/tools/messages |
| 完成即验证 | patch 完成前必须通过 8 步检查清单 | `cargo check` 通过就声明完成 |

这 11 条不是贴在 README 里的装饰。它们是 `AGENTS.md` 中实际生效的指令，每次编码 session 都会被加载。

---

## 完成即验证：把"做完"从主观判断变成可审计流程

第十一条（完成即验证）是 AgenDao 最独特的设计。它要求每个 patch 声明"完成"之前必须：

1. **数据流图** — 画出 event → who writes what → who reads it → observation surface
2. **Producer→Consumer 全链路 Trace** — `grep -rn` 确认每个新增字段至少一个非测试点写入非默认值
3. **Authority Consumer Verification** — 确认至少一个 consumer 从 authority 读取该值，而非使用本地硬编码常量副本
4. **All-Sites Scan** — 扫描同名类型在所有 crate 中的定义，全部更新
5. **E2E 测试走真实入口** — 测试必须调用生产代码的真实入口函数
6. **验收标准逐条核对** — 逐条打 ✅❌
7. **可观测双路径** — 验证写路径和读路径同时存在且接通
8. **Serde/Constructor 兼容** — 新增字段必须是 `Option` 或带 `#[serde(default)]`

这套流程源于一次系统性复盘：30+ 个缺口不是计划缺陷或宪法冲突，而是检查清单从未被真正执行。

---

## 三条长期能力

### 记忆系统：裁决什么值得留下，而不是尽量多存

- 新材料先作为 candidate 进入，经过 validation、conflict 检查、consolidation 才成为正式记录
- 检索时先给 retrieval preview，解释为什么要注入
- 正式 memory record 和临时会话材料分开，草稿不直接污染长期上下文

### Skill 系统：既要增长，也要能收束

- usage ledger 知道哪些 skill 真正在被使用
- negative entropy 识别长期闲置、需要复查或退出的 skill
- semantic conflict 和 composition relationship 避免同一类能力重复长出
- runtime gate 和 proposal review 分开：inspection 可见不等于运行时可用

### 上下文缓存：保护稳定提示面

AgenDao 的缓存优化不是"多塞几个 cache 字段"，而是把 prompt surface 变得长期稳定、可解释、可诊断。它会记录 prompt surface fingerprint、cache evidence、context closure contract，也会把 request/live/workflow 三本账拆开。详见 [docs/context-caching.md](docs/context-caching.md)。

---

## 治理 KPI（不是装饰）

```
语义重复点数：目标 0
新增事件改动触点数：目标 1
跨端行为一致性：目标 100%
```

---

## 运行时边界

AgenDao 是一套完整的本地编码智能体运行时。CLI、TUI 和 Web 共享同一套 session、scheduler、tool、provider、skill、memory、telemetry authority。

- provider 不靠 npm 名或历史别名猜测；`ProviderProfile`、descriptor、validation 和 runtime profile 共享同一条 authority 语义
- session 不把 live context、child workflow 花费和累计消耗混成一笔账；CLI、TUI、Web 都能读到 request/live/workflow 三本账和 context closure contract
- fork 和 subsession 有明确边界：child 只接收显式 packet，parent 只吸收 result/summary
- config、provider、scheduler、skill tree 的"当前到底生效了什么"有统一的只读解释面
- Web / TUI / Server 的消息同步以 authoritative `session.updated` 对齐；流式 `output_block` 和最终持久化 message 不长期分叉
- 三前端统一事件契约

内置 scheduler presets：`sisyphus` · `prometheus` · `atlas` · `hephaestus` · `verifier`

---

## 使用方式

### `agendao`

最完整、也最适合长期使用的 tui 界面。

### `agendao run`

脚本、CI、批量任务的单次执行入口。

### `agendao serve` / `agendao web`

server / web 入口，适合需要长期可观测面的场景。

### `agendao attach`

接上 server 正在维护的会话。

### `agendao acp`

Agent Client Protocol server 入口。

---

## 快速开始

### 环境要求

- Rust stable
- Cargo
- Git

### 构建

```bash
cargo build -p agendao
```

需要 Web 前端：

```bash
npm --prefix apps/agendao-web install
cargo build -p agendao
```

### 启动

```bash
cargo run -p agendao --                      # 默认 TUI
cargo run -p agendao -- tui --socket         # Unix socket
cargo run -p agendao -- tui --attach-url http://127.0.0.1:3000  # HTTP attach
cargo run -p agendao -- run "审查当前仓库里最危险的改动"
cargo run -p agendao -- serve --hostname 127.0.0.1 --port 3000
cargo run -p agendao -- web --hostname 127.0.0.1 --port 3000
```

### 本地安装

```bash
./scripts/install-local.sh release ~/.local
```

---

## 内部世界

- `crates/agendao` — 产品分发壳，唯一正式分发入口
- `crates/agendao-cli` / `crates/agendao-tui` / `apps/agendao-web` — 三个前端，共享同一运行时与 authority
- `crates/agendao-server` — HTTP、SSE、runtime control
- `crates/agendao-session` — session 领域模型、提示面组织、上下文连续性
- `crates/agendao-orchestrator` — scheduler / orchestration authority
- `crates/agendao-provider` — provider profile、transport、descriptor、cache
- `crates/agendao-skill` — skill authority、hub、distribution、guard
- `crates/agendao-memory` — 记忆的验证、检索、冲突与晋升

更多细目：[docs/README.md](docs/README.md)

---

## 开发者

```bash
cargo fmt --all
cargo check
cargo check -p agendao -p agendao-cli -p agendao-server -p agendao-tui
```

版本发布：

```bash
./scripts/release-date.sh 2026-05-17
./scripts/sync_version.sh
```

---

## 接下来

- 用户使用指南：[USER_GUIDE.md](USER_GUIDE.md)
- 文档索引：[docs/README.md](docs/README.md)
- 上下文缓存：[docs/context-caching.md](docs/context-caching.md)
- 发布说明：[CHANGELOG.md](CHANGELOG.md)

---

## 致谢

AgenDao 的架构设计受到开源 AI agent 社区的广泛启发，特别感谢 [OpenCode](https://github.com/anomalyco/opencode)、[Hermes Agent](https://github.com/stitionai/hermes-agent)、[Codex](https://github.com/openai/codex)、[Holon](https://github.com/holon-run/holon) 以及 [LLM-as-a-Verifier](https://github.com/llm-as-a-verifier) 等项目的先行探索。
