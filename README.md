<p>
  <img src="icons/ROCODE_logo.svg" alt="ROCode" width="280" />
</p>

# RockyCode (ROCode)

一个真实的编码回合，往往不是从提问开始，也不是在回答出现时结束。  
你会先读仓库，会怀疑自己的判断，会临时分叉，会回滚，会改计划，会隔一天再回来继续。到了那时，真正决定体验好坏的，往往已经不是“模型会不会回答”，而是这个系统还记不记得你们到底在做什么。

有些编码智能体擅长回答问题。  
ROCode 更关心另一件事：当软件工作变成一段持续数小时、数天、甚至数周的对话时，系统有没有能力不丢失边界，不误解上下文，不把历史和临时噪音混成一锅。

ROCode 是一个面向本地仓库工作的 Rust 编码智能体系统。它提供 CLI、TUI、HTTP Server 和 Web，但这些界面不是四套分离的产品表皮。它们共享同一套 session、scheduler、tool、provider、skill、memory、telemetry authority。  
换句话说，ROCode 不是把“问答能力”套在代码库上，而是试着把一段真正的软件劳动组织起来。

如果一定要用一句话概括它的气质，我会这样说：

> ROCode 想做的不是更会说话，而是更会记住、更会收束、更会把一件事做完。

当前版本：`v2026.5.15`

## 它是从什么问题里长出来的

写代码并不是连续地向模型提问。真正的工作更像这样：

- 你先理解仓库，再试探边界
- 你会临时开分叉，会回滚，会验证，会改计划
- 你会遇到工具输出、provider 差异、权限、缓存、记忆、历史包袱
- 你不是只想让模型“回答”，你想让系统在复杂回合里保持秩序

ROCode 的设计，基本都来自这个判断：  
**软件开发不是单轮问答，而是一种需要连续性、责任归属和节制感的运行时。**

## ROCode 想守住什么

### 1. 提示面不能失主

system、tools、messages、reasoning continuation、cache hints 这些东西，不能由多个层各自偷偷改。  
谁拥有提示面，谁就必须对缓存、可解释性和回放一致性负责。

### 2. 状态不能流浪

session 不是临时文本框。  
memory 不是无边界笔记堆。  
provider profile 不是到处推断出来的影子配置。  

ROCode 一直在做的事情，就是把这些状态重新送回各自明确的 owner。

### 3. 长对话里，稳定比聪明更重要

一个系统如果每轮都临时拼接更多信息，短期看像是“更聪明”，长期看通常只会让上下文更脆弱。  
ROCode 更在意 stable prompt surface、source anchors、memory anchors、output projection、cache fingerprint 这些缓慢但可靠的东西。

### 4. 记忆不是堆积，而是沉淀

经验不是一句口号。  
它应该有 evidence、有边界、有冲突处理、有晋升路径。  
所以 ROCode 的 memory 不是“把历史都存起来”，而是让 lesson、pattern、methodology 这些东西有机会被真正沉淀下来。

### 5. 工具、provider、skill 应该一起工作

它们都属于同一段运行时劳动。  
ROCode 尽量避免让一个系统长成很多彼此不通气的“半框架”。

## 它是什么样的系统

ROCode 是一套完整的本地编码智能体运行时，而不是一个只有聊天入口的壳。

- 它可以在本地仓库里运行编码智能体，支持交互式 TUI、单次 `run`、HTTP Server、Web UI 和 ACP
- 它维护会话树、会话分叉、导入导出，以及统一的 session telemetry / usage / events 读模型
- 它把 provider、模型目录、认证状态、catalog 刷新收进统一运行面
- 它有正式的 skill hub、memory 系统、scheduler continuity、artifact 与 diagnostics sidecar
- 它会用稳定提示面、输出投影和 cache diagnostic 来保护长对话的上下文缓存命中
- 它已经不是“只读一份全局配置”的工具，而是围绕 workspace authority 组织配置与运行边界

如果只看三条最容易被忽略、但最影响长期体验的能力，可以先看这三件事：

### 1. 记忆系统不是收集历史，而是裁决什么值得留下

ROCode 不把 memory 当作一堆随取随塞的笔记。
它更像一条慢一些的整理线：

- 新材料先作为 candidate 进入系统
- 它要经过 validation、conflict 检查、consolidation，才会成为正式记录
- 检索时也不是把所有历史直接塞回 prompt，而是先给 retrieval preview，解释为什么要注入

所以它的重点不是“记住得更多”，而是“让留下来的东西更可信，也更容易复用”。

### 2. skill 系统不只是安装和调用，它还要处理增生、重叠和漂移

ROCode 现在的 skill 系统已经不是一个静态目录：

- 它有 usage ledger，知道哪些 skill 真正在被使用
- 它会做 negative entropy，识别长期闲置、需要复查或应该退出运行面的 skill
- 它会做 semantic conflict 和 composition relationship，避免同一类能力不断重复长出
- 它把 runtime gate 和 proposal review 分开，inspection 可见不等于运行时一定可用

这套设计的目标很朴素：skill 既要增长，也要能收束，不能只增不减。

### 3. 上下文缓存优化不是补几个字段，而是保护稳定提示面

ROCode 对缓存敏感，不是因为它想显示更多 hit/miss 数字。
它真正想保护的是：

- 什么内容属于稳定前缀
- 什么内容只是这一轮的动态尾部
- compact、child handoff、output projection 会不会把 prefix 弄乱

所以它会记录 prompt surface fingerprint、cache evidence、context closure contract，也会把 request/live/workflow 三本账拆开。这样缓存命中下降时，系统至少能说明到底是 tool surface 变了、boundary 被改了，还是 live context 已经不该继续带下去。

如果继续往下看 ROCode 的具体能力，可以把下面这些理解为它的运行时边界与治理特性：

- provider 不是靠 npm 名、历史别名或请求选项临时猜测出来的；`ProviderProfile`、descriptor、validation 和 runtime profile 共享同一条 authority 语义
- session 不把 live context、child workflow 花费和累计消耗混成一笔账；CLI、TUI、Web 都能读到 request/live/workflow 三本账和 context closure contract
- external adapter 不能自行猜测 session id；必须先完成 owner-local provisioning，再绑定 replay / verify / run
- fork 和 subsession 有明确边界：child 只接收显式 packet，parent 只吸收 result / summary；显式 full-history fork 会冻结策略，并将 imported history 设为只读
- skill hub 不只是安装和删除；它还包含 usage ledger、negative entropy、semantic conflict、composition relationship、runtime gate 和 proposal review
- config、provider、scheduler、skill tree 的“当前到底生效了什么”有统一的只读解释面，前端不需要各自侧取大响应或重推配置
- Web / TUI / Server 的最终消息同步链路以 authoritative `session.updated` 对齐；流式 `output_block` 和最终持久化 message 不再长期分叉
- skill hub search 是正式产品面的一部分；搜索结果会返回 source、entry、trust、maintenance、staleness 与 refresh 建议，便于 agent 直接走 search → install

内置的 scheduler presets：

- `sisyphus`
- `prometheus`
- `atlas`
- `hephaestus`
- `verifier`

## 你可以怎样使用它

ROCode 有几种常用入口，但它们不是为了炫技，而是对应不同的工作姿态。

### `rocode tui`

这是最完整、也最适合长期使用的界面。  
如果你想在一个仓库里持续工作、追踪 session tree、看 stage、看 memory、看 telemetry，用它。

### `rocode run`

适合把一件事交代清楚，然后让系统单次执行。  
脚本、CI、批量任务都更适合这个入口。

### `rocode serve` / `rocode web`

当你希望一个运行时被多个界面消费，或者想保留更长期的可观测面，就走 server / web。

### `rocode attach`

当 server 已经在跑，而你只是想接上它正在维护的那段工作。

### `rocode acp`

当你需要 Agent Client Protocol server，而不是终端或浏览器。

## 快速开始

### 环境要求

- Rust stable
- Cargo
- Git

### 构建

ROCode 的产品分发入口固定为单一可执行文件 `rocode`。

```bash
cargo build -p rocode
```

如果你需要 Web 前端:

```bash
npm --prefix apps/rocode-web install
cargo build -p rocode
```

### 查看帮助

```bash
cargo run -p rocode -- --help
```

### 启动

默认进入 TUI:

```bash
cargo run -p rocode --
```

显式指定 TUI:

```bash
cargo run -p rocode -- tui
```

单次运行:

```bash
cargo run -p rocode -- run "请审查当前仓库里最危险的改动"
```

启动 HTTP Server:

```bash
cargo run -p rocode -- serve --hostname 127.0.0.1 --port 3000
```

启动 Web:

```bash
cargo run -p rocode -- web --hostname 127.0.0.1 --port 3000
```

显式指定 workspace 打开 Web:

```bash
cargo run -p rocode -- web --dir /path/to/workspace
```

### 本地安装

```bash
./scripts/install-local.sh release ~/.local
```

安装后主要布局是:

- `~/.local/bin/rocode`
- `~/.local/share/rocode/web`

## 它为什么会对上下文缓存这么敏感

很多系统把缓存理解成“多塞几个 cache 字段”。  
ROCode 不这么看。

它更关心的是：

- system prompt 哪一部分是真正稳定的
- tools 的 schema 和来源是否保持可复现
- scheduler continuity 该留下什么锚点，而不是整段原文
- memory 注入应该是稳定背景，还是本轮临时检索
- 用户可见长输出，怎样投影成下一轮还能承受的上下文

所以 ROCode 的缓存优化，本质上是在做一件更慢的事：  
**把 prompt surface 变得长期稳定、可解释、可诊断。**

如果你想深入这部分，直接看 [docs/context-caching.md](docs/context-caching.md)。

## 它怎样看待记忆和 skill

这两件事在很多系统里容易被做成两个极端：

- 要么全靠临时对话，什么也留不下
- 要么什么都往 memory 和 skill 里塞，最后系统自己也分不清哪些还可信

ROCode 在这件事上的做法更保守一些。

对 memory：

- 它强调 validation、conflict、consolidation 和 promotion
- 它希望正式记录是经过裁决的，而不是把未定稿的会话碎片直接永久化

对 skill：

- 它强调 usage、negative entropy、semantic conflict、composition 和 runtime gate
- 它希望 skill 是会被治理的方法，而不是一个只会越长越多的目录

这也是为什么 memory、skill 和上下文缓存命中其实是同一类问题。
如果你不知道什么该留下、什么该退出、什么该只做摘要，长会话最后一定会失稳。

## 它的内部世界，不是一堆模块，而是一组责任边界

如果你从仓库结构理解 ROCode，比较好的看法不是“有哪些 crate”，而是“哪些责任被谁拥有”。

- `crates/rocode`
  - 产品分发壳，唯一正式分发入口 `rocode`
- `crates/rocode-cli` / `crates/rocode-tui` / `apps/rocode-web`
  - 三个前端层，各自表达不同的交互姿态，但共享同一运行时
- `crates/rocode-server`
  - HTTP、SSE、runtime control、读模型路由
- `crates/rocode-session`
  - session 领域模型、提示面组织、上下文连续性
- `crates/rocode-orchestrator`
  - scheduler / orchestration authority
- `crates/rocode-provider`
  - provider profile、transport、descriptor、cache、usage normalization
- `crates/rocode-skill`
  - skill authority、hub、distribution、guard、lifecycle
- `crates/rocode-memory`
  - 记忆的验证、检索、冲突与晋升
- `crates/rocode-types`
  - 跨端共享的读写模型

更多细目请看 [docs/README.md](docs/README.md)。

## 如果你是开发者

常用验证命令:

```bash
cargo fmt --all
cargo check
```

前后端主线常用:

```bash
cargo check -p rocode -p rocode-cli -p rocode-server -p rocode-tui
```

版本发布相关脚本:

```bash
./scripts/release-date.sh 2026-05-15
./scripts/sync_version.sh
```

当前版本:

- 软件名: `RockyCode` / `ROCode`
- 版本: `v2026.5.15`
- 可执行命令: `rocode`

## 你接下来可以去哪里

- 用户使用指南: [USER_GUIDE.md](USER_GUIDE.md)
- 文档索引: [docs/README.md](docs/README.md)
- 上下文缓存: [docs/context-caching.md](docs/context-caching.md)
- Scheduler 示例: [docs/examples/scheduler/README.md](docs/examples/scheduler/README.md)
- Context Docs: [docs/examples/context_docs/README.md](docs/examples/context_docs/README.md)
- 插件 / skill 示例: [docs/examples/plugins_example/README.md](docs/examples/plugins_example/README.md)
- 当前版本发布说明: [CHANGELOG.md](CHANGELOG.md)

常用帮助入口:

```bash
rocode tui --help
rocode run --help
rocode models --help
rocode session --help
rocode skill hub --help
rocode debug --help
```

---

ROCode 想守住的方向很明确：  
让一段真正的软件工作，拥有连续性、边界感、记忆力和收束能力。

---

**致谢**

ROCode 的架构设计受到开源 AI agent 社区的广泛启发，特别感谢 [OpenCode](https://github.com/anomalyco/opencode)、[Hermes Agent](https://github.com/stitionai/hermes-agent)、[Codex](https://github.com/openai/codex)、[Oh-My-OpenCode](https://github.com/oh-my-opencode) 以及 [LLM-as-a-Verifier](https://github.com/llm-as-a-verifier) 等项目的先行探索。
