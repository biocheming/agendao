<p>
  <img src="icons/logo.svg" alt="AgenDao" width="280" />
</p>

# AgenDao

一个真实的编码回合，往往不是从提问开始，也不是在回答出现时结束。

你会先读仓库，会怀疑自己的判断，会临时分叉，会回滚，会改计划，会隔一天再回来继续。到了那时，真正决定体验好坏的，往往已经不是"模型会不会回答"，而是这个系统还记不记得你们到底在做什么。

有些编码智能体擅长回答问题。
AgenDao 更关心另一件事：当软件工作变成一段持续数小时、数天、甚至数周的对话时，系统有没有能力不丢失边界，不误解上下文，不把历史和临时噪音混成一锅。

> AgenDao 想做的不是更会说话，而是更会记住、更会收束、更会把一件事做完。

当前版本：`v2026.6.3`

---

## 理念

智能体开发，真不能执，道不可满，大道至简。

AgenDao 想守住的，不是功能越多越好，而是边界清楚、状态有主、上下文可收束。

软件开发不是单轮问答，而是一种需要连续性、责任归属和节制感的运行时。如果一套系统不能在长回合里持续澄清、压缩、回收和完成，那么它再聪明，也只会越来越乱。

所以 AgenDao 关注的不是"更会回答"，而是几条更慢、也更难被替代的事：

**提示面不能失主。**
system、tools、messages、reasoning continuation、cache hints——这些东西不能由多个层各自偷偷修改。谁拥有提示面，谁就必须对缓存、可解释性和回放一致性负责。

**状态不能流浪。**
session 不是临时文本框。memory 不是无边界笔记堆。provider profile 不是到处推断出来的影子配置。AgenDao 一直在做的事情，就是把这些状态重新送回各自明确的 owner。

**长对话里，稳定比聪明更重要。**
一个系统如果每轮都临时拼接更多信息，短期看像是"更聪明"，长期看通常只会让上下文更脆弱。AgenDao 更在意 stable prompt surface、source anchors、memory anchors、output projection、cache fingerprint 这些缓慢但可靠的东西。

**记忆不是堆积，而是沉淀。**
经验不是一句口号。它应该有 evidence、有边界、有冲突处理、有晋升路径。所以 AgenDao 的 memory 不是"把历史都存起来"，而是让 lesson、pattern、methodology 这些东西有机会被真正沉淀下来。

**工具、provider、skill 应该一起工作，而不是彼此越界。**
它们都属于同一段运行时劳动。AgenDao 尽量避免让一个系统长成很多彼此不通气的"半框架"。

这些原则不是贴在 README 里的装饰。它们直接决定了下面的每一条设计决策。

---

## 三条长期能力

如果你第一次了解 AgenDao，最值得先看的是这三条线。它们不热闹，但决定了系统能不能在长周期工作里保持稳定。

### 记忆系统：裁决什么值得留下，而不是尽量多存

AgenDao 不把 memory 当作一堆随取随塞的笔记。它更像一条慢一些的整理线：

- 新材料先作为 candidate 进入系统
- 经过 validation、conflict 检查、consolidation，才会成为正式记录
- 检索时也不是把所有历史直接塞回 prompt，而是先给 retrieval preview，解释为什么要注入
- 正式 memory record 和临时会话材料分开，不让草稿直接污染长期上下文

所以它的重点不是"记住得更多"，而是"让留下来的东西更可信，也更容易复用"。

### Skill 系统：既要增长，也要能收束

AgenDao 的 skill 系统已经不是一个静态目录：

- usage ledger 知道哪些 skill 真正在被使用
- negative entropy 识别长期闲置、需要复查或应该退出运行面的 skill
- semantic conflict 和 composition relationship 避免同一类能力不断重复长出
- runtime gate 和 proposal review 分开：inspection 可见不等于运行时一定可用
- proposal inbox 让新方法先进入审阅，而不是直接覆盖现有 skill

这套设计的目标很朴素：skill 既要增长，也要能收束，不能只增不减。它更像一个会治理的方法目录，而不是一个不断变大的仓库。

### 上下文缓存：保护稳定提示面，而不是多塞几个字段

很多系统把缓存理解成"多塞几个 cache 字段"。AgenDao 不这么看。

它更关心的是：

- system prompt 哪一部分是真正稳定的
- tools 的 schema 和来源是否保持可复现
- scheduler continuity 该留下什么锚点，而不是整段原文
- memory 注入应该是稳定背景，还是本轮临时检索
- compact、child handoff、output projection 会不会把 prefix 弄乱
- 缓存命中下降时，系统能不能解释到底是 tool surface 变了、boundary 被改了，还是 live context 已经不该继续带下去

所以 AgenDao 的缓存优化，本质上是在做一件更慢的事：把 prompt surface 变得长期稳定、可解释、可诊断。它会记录 prompt surface fingerprint、cache evidence、context closure contract，也会把 request/live/workflow 三本账拆开。

如果你深入这部分，直接看 [docs/context-caching.md](docs/context-caching.md)。

---

这三条线其实是同一类问题。如果你不知道什么该留下、什么该退出、什么该只做摘要，长会话最后一定会失稳。记忆、skill 和缓存命中——它们共同回答一个问题：**一段持续数天的软件工作，系统怎么帮你保持清醒。**

---

## 运行时边界

AgenDao 是一套完整的本地编码智能体运行时，CLI、TUI、HTTP Server 和 Web 共享同一套 session、scheduler、tool、provider、skill、memory、telemetry authority。换句话说，它不是把"问答能力"套在代码库上，而是试着把一段真正的软件劳动组织起来。

如果只看几条最容易被忽略、但最影响长期体验的治理特性：

- provider 不是靠 npm 名、历史别名或请求选项临时猜测出来的；`ProviderProfile`、descriptor、validation 和 runtime profile 共享同一条 authority 语义
- session 不把 live context、child workflow 花费和累计消耗混成一笔账；CLI、TUI、Web 都能读到 request/live/workflow 三本账和 context closure contract
- external adapter 不能自行猜测 session id；必须先完成 owner-local provisioning，再绑定 replay / verify / run
- fork 和 subsession 有明确边界：child 只接收显式 packet，parent 只吸收 result / summary；显式 full-history fork 会冻结策略，并将 imported history 设为只读
- config、provider、scheduler、skill tree 的"当前到底生效了什么"有统一的只读解释面，前端不需要各自侧取大响应或重推配置
- Web / TUI / Server 的最终消息同步链路以 authoritative `session.updated` 对齐；流式 `output_block` 和最终持久化 message 不再长期分叉
- skill hub search 是正式产品面的一部分；搜索结果会返回 source、entry、trust、maintenance、staleness 与 refresh 建议，便于 agent 直接走 search → install

内置 scheduler presets：`sisyphus` · `prometheus` · `atlas` · `hephaestus` · `verifier`

---

## 使用方式

AgenDao 有几种常用入口，对应不同的工作姿态。

### `agendao tui`

最完整、也最适合长期使用的界面。如果你想在一个仓库里持续工作、追踪 session tree、看 stage、看 memory、看 telemetry，用它。

### `agendao run`

适合把一件事交代清楚，然后让系统单次执行。脚本、CI、批量任务都更适合这个入口。

### `agendao serve` / `agendao web`

当你希望一个运行时被多个界面消费，或者想保留更长期的可观测面，就走 server / web。

### `agendao attach`

当 server 已经在跑，而你只是想接上它正在维护的那段工作。

### `agendao acp`

当你需要 Agent Client Protocol server，而不是终端或浏览器。

---

## 快速开始

### 环境要求

- Rust stable
- Cargo
- Git

### 构建

AgenDao 的产品分发入口固定为单一可执行文件 `agendao`。

```bash
cargo build -p agendao
```

如果你需要 Web 前端：

```bash
npm --prefix apps/agendao-web install
cargo build -p agendao
```

### 启动

默认进入 TUI：

```bash
cargo run -p agendao --
```

当前默认传输策略：

- `agendao tui` 默认走 Direct（in-process）
- `agendao run` / `agendao cli` 默认走 Direct（in-process）
- `--socket` 显式覆盖为本地 Unix socket
- `--attach-url` / `--attach` 显式覆盖为 HTTP
- `agendao web` 保持 HTTP-first

```bash
cargo run -p agendao -- tui                  # 显式 TUI
cargo run -p agendao -- tui --socket         # 走 Unix socket
cargo run -p agendao -- tui --attach-url http://127.0.0.1:3000  # 走 HTTP attach
cargo run -p agendao -- run "审查当前仓库里最危险的改动"   # 单次运行
cargo run -p agendao -- serve --hostname 127.0.0.1 --port 3000  # HTTP Server
cargo run -p agendao -- web --hostname 127.0.0.1 --port 3000    # Web
```

### 本地安装

```bash
./scripts/install-local.sh release ~/.local
```

安装后主要布局：

- `~/.local/bin/agendao`
- `~/.local/share/agendao/web`

---

## 内部世界：不是一堆模块，而是一组责任边界

如果你从仓库结构理解 AgenDao，比较好的看法不是"有哪些 crate"，而是"哪些责任被谁拥有"。

- `crates/agendao` — 产品分发壳，唯一正式分发入口
- `crates/agendao-cli` / `crates/agendao-tui` / `apps/agendao-web` — 三个前端层，各自表达不同的交互姿态，但共享同一运行时
- `crates/agendao-server` — HTTP、SSE、runtime control、读模型路由
- `crates/agendao-session` — session 领域模型、提示面组织、上下文连续性
- `crates/agendao-orchestrator` — scheduler / orchestration authority
- `crates/agendao-provider` — provider profile、transport、descriptor、cache、usage normalization
- `crates/agendao-skill` — skill authority、hub、distribution、guard、lifecycle
- `crates/agendao-memory` — 记忆的验证、检索、冲突与晋升
- `crates/agendao-types` — 跨端共享的读写模型

更多细目请看 [docs/README.md](docs/README.md)。

---

## 如果你是开发者

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

## 接下来可以去哪里

- 用户使用指南：[USER_GUIDE.md](USER_GUIDE.md)
- 文档索引：[docs/README.md](docs/README.md)
- 上下文缓存：[docs/context-caching.md](docs/context-caching.md)
- Scheduler 示例：[docs/examples/scheduler/README.md](docs/examples/scheduler/README.md)
- Context Docs：[docs/examples/context_docs/README.md](docs/examples/context_docs/README.md)
- 插件 / skill 示例：[docs/examples/plugins_example/README.md](docs/examples/plugins_example/README.md)
- 发布说明：[CHANGELOG.md](CHANGELOG.md)

常用帮助入口：

```bash
agendao tui --help
agendao run --help
agendao models --help
agendao session --help
agendao skill hub --help
agendao debug --help
```

---

AgenDao 想守住的方向很明确：
让一段真正的软件工作，拥有连续性、边界感、记忆力和收束能力。

---

**致谢**

AgenDao 的架构设计受到开源 AI agent 社区的广泛启发，特别感谢 [OpenCode](https://github.com/anomalyco/opencode)、[Hermes Agent](https://github.com/stitionai/hermes-agent)、[Codex](https://github.com/openai/codex)、[Oh-My-OpenCode](https://github.com/oh-my-opencode)、[Holon](https://github.com/holon-run/holon) 以及 [LLM-as-a-Verifier](https://github.com/llm-as-a-verifier) 等项目的先行探索。
