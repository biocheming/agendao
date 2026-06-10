<p align="right">
  <strong>中文</strong> | <a href="./README.md">English</a>
</p>

<p>
  <img src="icons/logo.svg" alt="AgenDao" width="280" />
</p>

# AgenDao

> **让输入、执行、承载、输出、回流成为一条气脉，而不是五块拼起来的功能。**

大多数 AI 编码工具解决的是“怎么做”的问题：怎么生成代码、怎么调用工具、怎么多步推理。AgenDao 解决的是另一个问题：**当一段软件工作持续数小时、跨越多个 session、涉及多个前端、经过多次 fork、回滚、压缩与重放之后，系统凭什么还能保持同一条工作气脉？**

答案不只在模型能力上，更在治理上。

当前版本：`v2026.6.10`

---

## 一句话理解 AgenDao

AgenDao 和其他 AI Agent 工具属于同一类：本地编码智能体运行时。但它的设计重心不在“让模型更聪明”，而在**让系统更通、也更可信**。

AgenDao 的核心判断是：**系统失真，不是因为模型不够强，而是因为输入、执行、状态、输出、回流分裂成了多份权威。**

所以 AgenDao 的重点不是再加一层“更聪明的 agent”，而是让同一条链路从 prompt input 到下一轮 prompt 之间始终服从同一套治理法则。

---

## AgenDao 的道纪

AgenDao 的设计不是从功能列表倒推出来的，而是从一条总纲推出来的：

> **每个语义域有且仅有一个权威；权威必须在阴阳之间闭环，在五行之间流转。**

这里的“阴阳”不是装饰性比喻，而是运行时约束：

- **阳**：输入、触发、执行、展开、显示
- **阴**：收束、归一、稳定、记账、回收、验证

孤阳则躁：只有输入和执行，没有承载与回流，系统会碎。

孤阴则滞：只有权威和规则，没有真实触发和交付，系统会死。

AgenDao 的“道纪”因此不只是“别重复造轮子”，而是要求每一条产品链路都满足三件事：

1. 有唯一权威
2. 有阴阳对位
3. 有五行流转

---

## 五行视角下的 AgenDao

### 木：输入之木

`prompt`、附件、slash 命令、history、引用、模式切换，都属于 `木`。

木的规则是：**贵在生发，忌多头生长。**

所以 AgenDao 不接受：

- 文本、附件、提示语、待发 payload 分裂成多份输入真相
- 输入组件只显示文本，真正待发内容藏在外部状态
- 不同前端各自维护不同的 prompt surface 语义

### 火：执行之火

LLM loop、权限裁决、工具调度、乐观提交、取消、中断、重试，都属于 `火`。

火的规则是：**贵在点燃，忌多炉并起。**

所以 AgenDao 要求：

- 所有 `model -> tool -> model` 迭代由唯一执行内核驱动
- 权限判定只在一个裁决点发生
- 工具调度、名称修复、回退、归一化只写一次

### 土：承载之土

配置、会话状态、上下文管理、serialized prompt surface、跨端副作用中转，都属于 `土`。

土的规则是：**贵在归一，忌地脉分裂。**

这是 AgenDao 的中枢。木火金水皆可强，土若不稳，整条链路必碎。

所以 AgenDao 要求：

- 配置只有一份活真相
- 状态域有唯一 owner
- 一切副作用经由编排层中转
- 模型请求的 serialized prompt surface 由唯一权威构造

### 金：输出之金

assistant response、tool output、scheduler stage、reasoning 呈现、message projection、事件语法，都属于 `金`。

金的规则是：**贵在成形，忌多法争刃。**

所以 AgenDao 不把“能跑出来很多东西”当作成功；它要求输出有主次、有结构、有唯一成形语法。

### 水：回流之水

telemetry、cache、memory、compaction、replay、resend、workflow usage、session usage，都属于 `水`。

水的规则是：**贵在归藏，忌有显无藏。**

所以 AgenDao 不接受：

- 有展示，没有回灌
- 有 telemetry 写路径，没有热路径消费
- cache / usage 在不同前端各说各话

---

## 相生，不是拼装

AgenDao 的目标不是把更多功能塞到一个 agent 里，而是恢复这条相生链：

1. **木生火**：输入能被唯一执行内核直接点燃
2. **火生土**：执行状态能回收到唯一编排承载
3. **土生金**：会话、上下文、prompt surface 生成唯一输出成形语法
4. **金生水**：输出沉淀为 telemetry、cache、memory、usage、replay
5. **水生木**：上一轮沉淀反哺下一轮输入，而不是只躺在侧栏和日志里

如果一套系统“能输入、能运行、能显示”，却不能自然回到下一轮输入，它就还没有真正闭环。

---

## 相克，不是敌对

五行在 AgenDao 里也是治理边界：

- **金克木**：规则、建议、提示不能压住输入本体
- **木克土**：输入变体不能无限增殖，冲垮唯一权威
- **土克水**：治理可以约束回流，但不能把回流压成只展示不消费
- **水克火**：telemetry / cache / memory 可以反制无节制执行，但不能替代真实执行语义
- **火克金**：运行事件可以丰富输出，但不能冲散最终交付的成形权

因此，AgenDao 的很多设计选择都不是“更复杂”，而是为了避免相克失衡后出现那种：

> 功能更多了，系统反而更乱了。

---

## 这套说法如何落地

这不是一套只用于写宣言的比喻。它会直接影响代码怎么拆、状态归谁、前端怎么读、回流怎么接。

- 新能力先问 `土`：它的唯一 owner 在哪个 crate、哪个状态域、哪个 authority
- 新交互再问 `木`：输入是不是仍然回到同一份 prompt authority，而不是偷偷长出第二份草稿
- 新执行链路必问 `火`：是谁点燃、谁能取消、谁负责权限裁决、谁对运行状态记账
- 新展示统一问 `金`：这是不是沿用现有事件语法和 message projection，还是又发明了一套“看起来差不多”的输出结构
- 新 telemetry / cache / memory 必问 `水`：除了写出来，谁会在下一轮真正消费它

对 AgenDao 来说，真正的坏味道不是“代码不够优雅”，而是：

- 一个语义域出现两份真相
- 一个运行结果只有展示没有回流
- 一个前端为了方便，开始偷偷复制中层权威

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

## 运行时边界

AgenDao 是一套完整的本地编码智能体运行时。CLI、TUI 和 Web 不是三套产品，而是三张读同一条地脉的面：共享同一套 session、scheduler、tool、provider、skill、memory、telemetry authority。

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
- `crates/agendao-cli` / `crates/agendao-tui` / `apps/agendao-web` — 三张前端读面，共享同一运行时 authority
- `crates/agendao-server` — HTTP、SSE、runtime control，承担跨端观测与调度读面
- `crates/agendao-session` — session 领域模型、提示面组织、上下文连续性，是土与水最重的一层
- `crates/agendao-orchestrator` — scheduler / orchestration authority，是火与土的中枢
- `crates/agendao-provider` — provider profile、transport、descriptor、cache，负责 prompt surface 与 usage 语义的边界
- `crates/agendao-skill` — skill authority、hub、distribution、guard
- `crates/agendao-memory` — 记忆的验证、检索、冲突与晋升，负责把输出沉淀成可回流之水

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
