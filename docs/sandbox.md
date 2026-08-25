# AgenDao Sandbox 系统

> **每个模型可达的进程启动点，都能回答：谁授权、实际使用哪一个 immutable plan、哪个 OS backend 强制执行、子孙进程如何退出、事件在哪里可见、测试如何证明越界失败。**

本文是 sandbox 系统当前事实的权威描述。它把 `permission`（逻辑层的 allow/ask/deny）与 `sandbox`（OS 层的强制隔离）分开讲清楚，并逐平台标注真实能力与未覆盖边界。

实现与平台细节以 `crates/agendao-sandbox` 为准；本文件的定位是语义契约（金），不是代码复述。

---

## 一、permission 与 sandbox 是两件事

- **permission** 是逻辑治理：谁（哪条 prompt/哪个 tool）被允许调用什么工具，answer 是 allow / ask / deny。它回答"该不该跑"，是 `火`（执行裁决）的阳面。
- **sandbox** 是物理治理：一个模型可达的子进程在不满足某项约束时**根本跑不起来**（fail-closed）。它回答"跑起来之后能碰到什么"，是 `土`（承载）+ `火`（执行限制）的阴面。

二者**不是**替代关系，也**不能合一**：permission 不知道 `bwrap` 参数、`Seatbelt` 规则、`WFP` 过滤器；sandbox 不做 prompt 语义判断。违反这条各自为五行的边界，正是计划明确排除的反模式。

session permission mode 只影响 **permission 层**，不改变 sandbox 的 fail-closed 底线：

| mode | permission 行为 | sandbox 底线 |
| --- | --- | --- |
| `default` | 逐请求治理 | 模型可达执行仍走 contained |
| `trusted_workspace` | 自动批准 `workspace:/` scope | 仍 contained（该 mode 不放开任何超出 default 的 sandbox 宽度） |
| `unsandboxed_yolo` | 本 session 对宿主机执行全开 | 可经 `SandboxExecutionBoundary` 获 `Native` 通道；是唯一能触达 native 的 session 级路径 |

**禁止把 `unsandboxed_yolo` 叫做 "sandbox YOLO"**：它不是 sandbox，而是显式退出 sandbox。`architecture.md` 已同步此表述。

---

## 二、信任类与 profile（谁授权，用哪个 plan）

每个模型可达的启动点都以一个 `SandboxExecutionRequest` 进入唯一权威 `SandboxExecutionBoundary`。请求携带：

1. **trust class**：`ModelReachable`（模型工具直接触发）、`UserConfiguredIntegration`（MCP/LSP/plugin host，二进制由用户配置但模型可达）、`HostManagement`（产品/实例管理，模型不可达）。
2. **profile kind**：`WorkspaceWrite` / `Check` / `InteractiveShell` / `Integration` / `Native`。

关键约束（土——唯一权威不能被工具绕过）：

- `ModelReachable` **永远不能**自己 resolve 到 `FilesystemMode::Unrestricted` 或 `ProcessMode::Native`。native 必须由 host 在 session 层显式授权。
- 平台 backend 只执行 `Contained` plan；`Native` plan 走显式 native 通道，且 native 通道**拒绝** contained plan（绝不做 fallback）。
- `IntegrationSandboxContext` 固定 `UserConfiguredIntegration` + `Integration` profile——integration 无法自行放宽。

---

## 三、backend 与 fail-closed（哪个 OS 强制、谁在不满足时拒绝）

`SandboxBackend` 的实现住在 `platform/`，通过 `BackendRegistry` 按注册顺序选择。**选择即 fail-closed 点**：`Contained` plan 找不到可用 backend 时，返回 `SandboxUnavailable` 并给出**第一个失败 backend 的可行动原因**（而不是静默缩窄候选列表）。

各平台真实状态：

| OS | backend | 状态 | 说明 |
| --- | --- | --- | --- |
| Linux | `bwrap` | **完整** | namespaces + seccomp(deny network/ptrace) + `<system>/etc/...` 只读 bind + workspace bind + protected-metadata（`.git` 等只读）+ 私有 `/tmp` tmpfs + process-group 生命周期梯 |
| macOS | `seatbelt` | **禁用、fail-closed** | SBPL 路径字面量会严格拒绝引号、反斜杠、控制字符和 NUL；固定共享 HOME 已移除。由于 `sandbox-exec` 已弃用且尚无 execution-scoped HOME/enforcement 组合，默认 registry 不注册该 backend，contained 启动明确不可用，不回退 native |
| Windows | `restricted-token` | **模型层 + fail-closed** | `token`(restricted-token plan) / `acl`(protected-metadata DACL) / `job`(job-object kill-on-close) / `wfp`(network sublayer) 四个纯模型已就位并被 contract-test；**kernel 执行路径未集成**（`CreateRestrictedToken`/`Job Object`/WFP），backend 恒定 probe 不可用，contained 启动 fail-closed 并给出可行动原因 |
| 其余 OS | — | 无 backend | contained 启动报 "no platform backend registered on this build" |

能力缺失时 **fail closed，不做静默降级**。没有哪个 backend 失败会自动改用 native；任何 native 运行都必须可追溯到显式 policy。

---

## 四、权威链与五行闭环

一条模型可达的执行（例如 `bash` / `apply_patch` 的 git 通道 / 外部 catalog 工具）走：

1. **木→火**：工具代码构造 `SandboxExecutionRequest`（只读权威授权，工具不自行放宽 trust class / profile）。
2. **火→土**：`SandboxExecutionBoundary`（server `SandboxAuthority` / CLI `CliSandboxAuthority`）做 policy 校验、`build_plan`（canonical 路径 + fingerprint）、minter 事件。
3. **土→金**：`SandboxLauncher` 选 backend → `start()` 得到 `SandboxExecutionHandle`（pi pid、stdio、cancel/ wait）。
4. **金→水**：生命周期通过 `SandboxHandleDriver` 完成 TERM→grace→KILL 梯（contained 的进程组/Job Object），退出状态回到调用方；事件进 `EventLog` 供 projection。
5. **水→木**：denial/timeout/exit 回流为工具错误信息，进入下一轮输入。

`integration_request` / `integration_launch` 对 MCP/LSP/plugin host 固定 Integration profile，走同一权威链。

---

## 五、可观测与守护

- **spawn inventory**（`agendao-server/tests/fixtures/sandbox-spawn-inventory.tsv` + `agendao-server/tests/sandbox_spawn_inventory.rs`）是**双向精确**的守护：每个 production spawn 必须登记（源码→TSV），每条登记必须在源码中存在（TSV→源码），model-reachable 的唯一合法终局是 `boundary`。manifest 位于可版本化的非 `docs/plans/` 路径，因为 `docs/plans/` 永久被忽略。这是“无第二套模型可达执行 authority”的机制保证。
- backend 的纯参数构造（`build_bwrap_args` / `build_seatbelt_profile` / Windows 四模型）全部被 contract-test，能在任意宿主运行（即使无对应 OS）。
- 每个平台 backend 至少有一个不可用/失败测试（质量门禁 §10.5）。

---

## 六、安装前提与测试

各平台 backend 依赖：

- **Linux**：`bwrap` 在 PATH；未启用 user namespace 时 fail-closed。`bwrap` 缺失或 `unprivileged_userns_clone=0` 时，contained 启动失败并解释原因。
- **macOS**：系统自带 `sandbox-exec`（可能需显式路径）。缺失时 fail-closed。
- **Windows**：无需安装（模型层 + fail-closed）。

测试命令（hub 侧，产物与 fixture 一律用 `../target`，不落宿主 `/tmp`）：

```bash
export CARGO_TARGET_DIR=../target
cargo test --locked -p agendao-sandbox            # 全量 contract + 各 backend
cargo test --locked -p agendao-server --test sandbox_spawn_inventory   # 双向守护
cargo check --workspace --locked --tests           # 全仓库编译门禁
cargo fmt --all -- --check                         # 格式化门禁
```

跨平台 compile 门禁（纯 cfg 验证，无需链接器）：

```bash
cargo check --locked -p agendao-sandbox --target aarch64-apple-darwin
cargo check --locked -p agendao-sandbox --target x86_64-pc-windows-msvc
```

多 OS 的 repository build artifacts 位于 `../target`（仓库 `<repo>/.cargo/config.toml` 已设 target-dir；或显式 `CARGO_TARGET_DIR=../target`），绝不建仓库内 `target/` 或用 `/tmp` 当 Cargo target。

---

## 七、尚保留的 host-management path（诚实边界）

以下两类**不是**模型可达执行，已逐项分类、审计、显式授权，不纳入 sandbox（计划 §11 非目标）：

- **Product/launcher host-management**：`agendao-launcher`、`agendao/src/host.rs`、`product_cli.rs` 的 install / launcher / osascript / powershell；`HostManagement` trust class，`model_reachable=false`。
- **Server git/helper**：`agendao-server/src/routes/{file,project}.rs`、`worktree.rs`、`agendao-session/snapshot.rs`；`HostManagement`，audit-before-boundary。
- **Plugin install command**：`agendao-plugin/subprocess/loader.rs` 的 `npm install`——需网络（Integration 禁网会坏），`HostManagement` 独立事件；`--version` probe 只读 stdout。
- **CLI user-initiated**：`agendao-cli` 的 git/gh/sqlite3 等人类配置动作。

Windows criterion 已统一通过 `SandboxAuthority.launch_check`；缺少 contained backend 时 fail-closed，不再保留任何 model-reachable 直连路径。

`ProxyOnly` 当前是保留的策略语义，不是已交付的网络 backend。Linux
Bubblewrap、macOS Seatbelt 和 Windows restricted-token backend 都不会选择
`ProxyOnly` plan；在 authority-managed proxy transport 完成前，这类请求明确
fail-closed，绝不回退到宿主网络。

运行期 violation 的身份只能由 launcher/lifecycle 从不可变 plan 生成。Linux
对 `SIGSYS`（或 shell 表示的 `128 + SIGSYS`）记录 `syscall_denied` 的
best-effort evidence；它证明命中了 syscall boundary，不证明具体 syscall、URL
或文件路径。普通非零退出不会被升级为 violation。

未被覆盖的资源维度（计划 §11 非目标）：CPU / 内存 / 磁盘 / 进程数的完整 cgroup/workingset quota 属于后续 resource governance 计划，不冒充 sandbox 已解决。

---

## 八、权威与五行

- **唯一权威**：所有 model-reachable 执行经 `SandboxExecutionBoundary`（server `SandboxAuthority` / CLI `CliSandboxAuthority`）；工具/适配层只读查询，绝不直连 OS。
- **阴阳闭环**：每个启动点都同时答"谁生发执行"（阳：工具构造请求 + authority prepare）与"谁收束记账"（阴：plan fingerprint + EventLog + 生命周期梯 + spawn inventory）。
- **相生**：输入(木)→执行(火)→承载(土)→成形(金)→回流(水)→再输入(木)，闭环由守护测试兜底。
