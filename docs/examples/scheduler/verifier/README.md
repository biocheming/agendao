# Verifier Preset Guide

这份指南面向第一次接触 `verifier` 的读者：先解释它要解决什么问题，再解释
LLM-as-a-Verifier 的核心原理和公式，最后说明 ROCode 现在如何把它落到工程实现、
示例配置、产物和前端展示里。

## Verifier 要解决什么问题

普通 autonomous coding 流程很容易默认使用“最后一次成功迭代”作为最终结果。
这个假设并不总是成立。

例如，一个更晚的候选方案可能：

- 通过了同一组测试，但实现更冒险；
- 提升了某个指标，却破坏了隐含约束；
- 最终回答看起来更完整，但执行轨迹里的证据更弱；
- 改动更多，维护风险更高；
- 只是在局部验证中成功，并不代表它是多个候选里最优的。

`verifier` 的目标是把“生成候选”和“选择候选”分开。ROCode 仍然用原有
autonomous workflow 生成、执行和验证候选；当有多个可接受候选时，再让 verifier
judge 基于轨迹证据进行显式比较，选出最终结果。

适合使用 verifier 的场景：

- 高影响代码修改，有多个看起来都可行的实现方案；
- 重构、迁移、工程增强这类需要权衡正确性、风险和可维护性的任务；
- 测试是必要条件，但测试本身不足以表达全部质量要求；
- 需要留下可审计的选择报告；
- 希望区分“能跑通”和“更值得采用”。

不适合使用 verifier 的场景：

- 很小的单点修改，一个候选已经足够；
- 选择标准完全机械化，例如固定 benchmark 最大值；
- 一次性低成本修复，额外 judge 成本不划算；
- 没有足够候选差异，比较本身不会提供价值。

## 核心思想

Verifier mode 有两个职责边界：

- **候选生成**：由 ROCode 已有 autonomous workflow 完成，包括执行、验证、guard、
  metric、artifact 记录等。
- **候选选择**：由 verifier judge 对保留下来的候选进行比较，使用明确 criterion
  和候选轨迹 `tau` 作为证据。

因此 verifier 不是测试替代品。它依赖测试输出、guard 输出、metric、workspace
diff 摘要、结构化 artifact 和最终回答，把这些证据汇总成 candidate trajectory，
再交给 judge 做选择。

## 原始 LLM-as-a-Verifier 算法

核心算法不是让模型随便写一段 JSON 说“我选 A”。它要求 judge 在固定 score token
集合上输出概率，然后用这些 token 的 logprob 计算期望奖励。

规范化公式如下：

```text
R(t, tau) =
  1/(C*K) * sum_c sum_k sum_g p_theta(v_g | t, c, tau) * phi(v_g)
```

含义：

- `t`：任务目标，也就是用户原始请求和当前 workflow 要解决的问题；
- `tau`：候选方案的执行轨迹，包括最终答案、验证输出、工具调用、metric、diff 等；
- `C`：评价 criteria 数量；
- `K`：每个 criterion 重复 judge 的次数，用于降低 judge 方差；
- `V_score = {v_1, ..., v_G}`：离散 score token 集合；
- `p_theta(v_g | t, c, tau)`：judge 模型在当前任务、criterion 和轨迹条件下给
  score token `v_g` 的概率；
- `phi(v_g)`：把 score token 映射为数值奖励。

ROCode 使用 `A` 到 `T` 作为 score token：

- `A` 表示最高分；
- `T` 表示最低分；
- token 会映射到 `[0, 1]` 区间的 reward；
- `granularity: 20` 对应 `A..T` 这 20 个 score token。

直观地说，judge 不只是输出“我觉得是 A 档”。ROCode 会读取模型在 score token
位置上的 top-logprobs，把每个候选的分数看作概率分布上的期望值。

## Score Job 分解

ROCode 的 canonical verifier path 把一次候选比较拆成多个 score job：

```text
ScoreJob = task objective + candidate pair + criterion + repetition
```

每个 score job 只处理一个候选对、一个 criterion、一次 repetition。

Judge prompt 要求模型为两个候选分别输出固定标签：

```text
<score_A>A</score_A>
<score_B>A</score_B>
```

如果 provider 返回 score token 位置的 top-logprobs，ROCode 就按概率计算两个候选
的 expected reward。如果 provider 没有返回可用 logprobs，ROCode 会退回到解析
文本标签，并在 artifact 里记录 fallback 原因。

pairwise 结果由该候选对下所有 score jobs 聚合得到：

```text
pair_score(candidate) = mean(expected_reward over criteria and repetitions)
```

最终选择策略支持：

- `round-robin`：所有候选两两比较，每个 pair 产生一个胜者，按胜场选择最终候选；
- `tournament`：候选按路径逐轮淘汰，保留 champion。

## ROCode 的实现路线

ROCode 没有为 verifier 另造一套执行系统，而是把它实现为 scheduler preset 加
workflow mode：

```text
scheduler preset: verifier
workflow mode: verify
execution kernel: existing autoresearch / autonomous workflow runtime
scoring engine: verifier_engine.rs
trajectory projection: verifier_trace.rs
artifact authority: WorkflowArtifactWriter
```

这个结构符合 ROCode 宪法里的几个关键约束：

- **单一执行内核**：verifier 复用已有 autonomous workflow，不绕过主执行路径；
- **单一配置真源**：scheduler profile 和 workflow config 是配置入口；
- **单一状态所有权**：workflow runtime 拥有 candidate state 和 final selection；
- **适配器边界清晰**：provider adapter 只负责模型调用，不拥有 verifier 算法；
- **可观察工作流**：score job、pairwise matrix、selection report 都写入 artifact；
- **前端不掌握算法权力**：CLI、TUI、Web 只选择 preset 和展示 artifact，不计算分数。

这也解释了为什么 verifier 的实现看起来像一个 workflow 能力，而不是某个 UI 或
provider 的特殊分支。

## 一次运行如何发生

一次 verifier run 的典型流程如下：

1. ROCode 运行正常 autonomous execution loop。
2. 每个满足保留条件的 iteration 被记录为候选。
3. workflow 从 session telemetry 中投影出候选轨迹 `tau`。
4. 当候选数至少为 2 时，进入 verifier selection。
5. canonical mode 将比较拆成 `(pair, criterion, repetition)` score jobs。
6. 每个未命中 cache 的 score job 通过 scheduler model authority 调用 judge。
7. 如果可用，score-token logprobs 被转换为 expected reward。
8. 每个 pair 按 criteria 和 repetitions 聚合平均分。
9. `round-robin` 或 `tournament` 产生最终候选。
10. 被选中的候选成为 workflow final result。

注意：如果最终候选不是最后一次 iteration，这是 verifier 的预期行为。它返回的是
被 judge 选中的候选，而不是时间上最新的候选。

## Candidate Trajectory Evidence

原算法中的 `tau` 是候选轨迹。ROCode 会把已有 workflow/session 证据投影为稳定、
可缓存、可展示的 trace。

一个 verifier trace 通常包含：

- 候选最终回答；
- execution step count；
- tool call count；
- execution metadata summary；
- verify command 输出；
- guard command 输出，如果配置了 guard；
- metric value 和优化方向；
- workspace change summary；
- structured mode artifacts；
- stable trajectory fingerprint。

这些证据会进入 judge prompt，也会参与 cache key。也就是说，如果候选代码或验证
证据变了，即使最终回答文本类似，trajectory fingerprint 也会变化，cache 不会错误
复用旧分数。

## 示例配置

`docs/examples/scheduler/verifier/profile.example.jsonc` 提供了两个入口：

- `verifier-default`：内联 workflow 配置；
- `verifier-custom`：通过 `workflowPath` 引用外部 workflow 文件。

核心 verifier 配置形态如下：

```jsonc
"verifier": {
  "model": {
    "providerId": "openai",
    "modelId": "gpt-5"
  },
  "criteria": [
    {
      "id": "spec",
      "name": "Spec adherence",
      "description": "Prefer the candidate that most directly satisfies the request.",
      "weight": 1.0,
      "aggregation": "score-margin"
    }
  ],
  "repetitions": 3,
  "maxCandidates": 3,
  "selection": "round-robin",
  "useLogprobs": true,
  "granularity": 20,
  "traceFormat": "compact"
}
```

字段解释：

- `model`：verifier judge 使用的模型；
- `criteria`：评价维度；
- `repetitions`：每个 criterion 重复 score job 的次数；
- `maxCandidates`：最多保留多少候选参与最终比较；
- `selection`：候选选择策略，支持 `round-robin` 和 `tournament`；
- `useLogprobs`：启用 canonical score-token expected reward 路径；
- `granularity`：score token 数量，目前有效范围是 `1..20`；
- `traceFormat`：候选轨迹渲染方式，通常先用 `compact` 控制成本。

如果目标是真正的 logprob-based LLM-as-a-Verifier，应使用：

```jsonc
"useLogprobs": true,
"granularity": 20
```

## Criteria 如何写

好的 criterion 应该具体、可由轨迹证据支撑。

推荐写法：

- “候选是否直接满足用户明确提出的需求？”
- “验证输出是否支撑候选声称的行为？”
- “候选是否降低回归风险，而不是只修好表面问题？”
- “改动是否保持了项目既有架构和边界？”

不推荐写法：

- “这个实现好吗？”
- “代码是否优雅？”
- “感觉哪个更对？”

ROCode 还支持 criterion metadata：

- `weight`
- `aggregation = "score-margin" | "winner-vote"`

原始公式默认对 criteria 做均匀平均。ROCode 保留权重和 aggregation 是为了兼容工程
策略表达，尤其是 `useLogprobs=false` 的 JSON judge path。canonical score-job path
仍然以 expected reward 和 score job matrix 为核心。

## 运行后看什么

Verifier 的价值不只在最终选了谁，还在它把“为什么选”落成了 artifact。

重点 artifact：

### `score-job-matrix`

canonical mode 下最重要的 artifact，一行对应一个 score job。

它记录：

- candidate pair；
- criterion；
- repetition；
- score A / score B；
- 是否请求 logprobs；
- 是否实际使用 logprobs；
- `logprob_status`；
- fallback kind；
- model/provider；
- latency；
- error，如果有；
- trajectory fingerprint；
- criterion fingerprint；
- cache status。

常见 `logprob_status`：

- `requested-usable`：provider 返回了可用 top-logprobs，并且已用于 expected reward；
- `requested-missing-provider-metadata`：provider 没有返回 metadata；
- `requested-missing-logprobs-field`：metadata 存在，但没有 logprobs 字段；
- `requested-empty-logprobs`：logprobs 字段存在但为空；
- `requested-unusable-score-token-logprobs`：有 logprobs，但不能用于 score token。

### `pairwise-score-matrix`

一行对应一个候选对的聚合比较结果。

它记录：

- 两个候选的 pair mean；
- pair winner；
- selection strategy；
- 该 pair 使用了哪些 score job keys。

### `round-robin-win-counts`

`round-robin` 策略下的最终胜场表。它能帮助你判断最终选择是否稳定，例如第一名是否
明显领先，还是多个候选胜场接近。

### `selection-report`

最终选择报告，通常包括：

- selected candidate；
- pairwise comparison 数量；
- judge call 数量；
- cache hit 数量；
- 是否实际用到了 logprobs；
- fallback 统计。

## Cache 语义

canonical mode 的 cache key 会包含：

- judge provider/model；
- score-token request shape；
- task objective fingerprint；
- criterion fingerprint；
- candidate fingerprints；
- stable pair trajectory hash；
- repetition index。

这意味着 cache 是按“证据等价”复用，而不是只看候选编号或文本。候选轨迹变了，分数
也需要重新计算。

## 成本模型

Verifier 成本主要由候选数、criteria 数量和 repetitions 决定。

`round-robin` 下：

```text
pair_count = N * (N - 1) / 2
score_jobs = pair_count * C * K
```

例子：

```text
N = 3 candidates
C = 2 criteria
K = 3 repetitions

pair_count = 3
score_jobs = 18
```

调成本的主要旋钮：

- `maxCandidates`：减少候选数会显著减少 pair 数；
- `criteria`：保持少而准；
- `repetitions`：方差高时再增加；
- `traceFormat`：优先用 `compact`，证据不足时再用 `full`。

## Fallback 行为

`useLogprobs=true` 表示 ROCode 会请求 logprobs，但 provider 不一定真的返回可用
logprobs。

ROCode 的处理方式是显式降级：

- logprobs 可用：使用 expected reward；
- logprobs 不可用但文本标签可解析：解析 `<score_A>` / `<score_B>`；
- 标签也不可解析：记录 score job error fallback；
- artifact 中保留 fallback reason 和 `logprob_status`。

所以一次 run 成功并不一定代表它真的走了 canonical logprob path。要确认这一点，
需要看 `score-job-matrix` 里的 `used_logprobs` 和 `logprob_status`。

## JSON Judge 兼容路径

当 `useLogprobs=false` 时，ROCode 使用 JSON pairwise judge path。judge 直接返回
winner 和可选 criterion scores。

这条路径适合：

- provider 不支持 logprobs；
- 想要较低成本的 pairwise judge；
- 需要兼容早期 verifier 配置。

但它不是原始 LLM-as-a-Verifier 的核心算法。真正的 canonical path 应使用
`useLogprobs=true`，并检查 artifact 确认 logprobs 确实被使用。

## CLI、TUI、Web 前端里的位置

`verifier` 是公共 scheduler preset。CLI、TUI、Web 的职责是：

- 允许用户选择 `verifier` preset；
- 传递或加载 scheduler profile / workflow config；
- 展示 workflow summary 和 verifier artifacts；
- 展示 provider logprob capability / fallback 状态。

它们不应该：

- 自己计算 expected reward；
- 自己决定 candidate winner；
- 绕过 workflow artifact writer 写 verifier 结果；
- 在前端维护一份独立 verifier 状态。

这个边界保证了三个前端看到的是同一套后端事实，而不是三个 UI 各自实现一遍算法。

## 如何阅读示例

先看这三个文件：

- `docs/examples/scheduler/README.md`
- `docs/examples/scheduler/verifier/profile.example.jsonc`
- `docs/examples/scheduler/verifier/workflow.example.jsonc`

`profile.example.jsonc` 展示 scheduler profile 如何选择 `verifier` preset。

`workflow.example.jsonc` 展示外置 workflow 配置如何表达：

- `workflow.mode: "verify"`；
- verifier judge model；
- criteria；
- repetitions；
- candidate limit；
- logprob path；
- trace rendering。

配置加载后，最终应该解析成：

```text
orchestrator: verifier
workflow.mode: verify
verifier.useLogprobs: true
```

运行结束后，按这个顺序读结果：

1. `summary.json`：先看最终 selected candidate 和 counters；
2. `run-manifest.json`：确认 workflow summary 和持久化状态；
3. `mode-artifacts.json`：重点看 `score-job-matrix`、`pairwise-score-matrix`、
   `round-robin-win-counts`、`selection-report`；
4. verifier cache：确认 score job cache hit 是否符合预期。

## 实用调参建议

从保守配置开始：

```jsonc
"maxCandidates": 3,
"repetitions": 2,
"traceFormat": "compact"
```

什么时候增加候选数：

- 生成阶段确实能产生不同实现路线；
- 任务本身需要探索多个架构选择；
- 你希望 verifier 在更多候选中做选择。

什么时候增加 repetitions：

- judge 结果不稳定；
- 候选质量接近；
- criterion 比较主观，需要降低单次采样影响。

什么时候增加 criteria：

- 有清晰独立的评价维度；
- 每个 criterion 都能从轨迹证据中判断；
- 增加 criterion 不会把 prompt 变得含糊。

什么时候用 `traceFormat: "full"`：

- compact trace 没有暴露足够证据；
- 需要 judge 看到更完整工具输出；
- 候选差异主要体现在执行过程而不是最终回答。

## ROCode Verifier 的特色

ROCode 的 verifier 不是一个 prompt 模板，而是一套可观察的 runtime 能力：

- score job 是显式运行时记录；
- logprob 可用时按 score-token probability 计算 expected reward；
- logprob 不可用时显式记录 fallback；
- candidate trajectory 来自 workflow telemetry；
- cache key 包含稳定轨迹 fingerprint；
- lifecycle 和 cancellation 走 scheduler authority；
- artifacts 暴露完整 scoring surface；
- CLI、TUI、Web 只展示事实，不拥有选择算法；
- 实现路线符合 ROCode 单内核、单配置、单状态所有权的架构约束。

最终效果是：verifier 不只回答“选了谁”，还回答“基于哪些候选证据、哪些 criteria、
哪些 score jobs、是否真正使用 logprobs、有没有 fallback、成本是多少”。
