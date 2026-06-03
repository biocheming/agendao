# AgenDao 安装指南

本文档介绍 AgenDao 的系统要求、构建安装方式以及首次运行配置。

---

## 系统要求

| 平台 | 架构 | 最低要求 |
|------|------|---------|
| Linux | x86_64 | glibc 2.17+（2014 年后的大多数发行版） |
| Linux | aarch64 | glibc 2.17+ |
| macOS | x86_64 | macOS 11 Big Sur |
| macOS | aarch64 | macOS 11 Big Sur（Apple Silicon） |
| Windows | x86_64 | Windows 10 / Server 2019 |

### Rust 工具链

从源码构建需要 Rust 稳定版（1.75 或更高）。通过 [rustup](https://rustup.rs/) 安装：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

验证安装：

```bash
rustc --version
cargo --version
```

---

## 安装方式

### 方式一：从源码构建（推荐）

AgenDao 目前以源码形式分发。克隆仓库并构建：

```bash
git clone <repo-url>
cd agendao

# 首次构建前安装 Web 前端依赖
npm --prefix apps/agendao-web install

# 推荐：构建并安装单一分发入口
./scripts/install-local.sh release ~/.local
```

安装后固定布局为：

```
~/.local/bin/agendao
~/.local/share/agendao/web
```

默认运行时会优先使用内嵌进 `agendao` 二进制的 Web 资源。`share/agendao/web` 仍会被安装，用作显式外部覆盖与兼容性回退。

如果只想构建、不立即安装：

```bash
cargo build --release -p agendao
```

说明：

- `crates/agendao-server/build.rs` 会自动检查 `apps/agendao-web/dist` 是否缺失或过期
- 只有当前端源码变化时，才会增量触发 `npm --prefix apps/agendao-web run build`
- 如果 Web 没有变化，不会在每次 release 构建时重复打包前端

### 方式二：cargo install

```bash
cargo install --path crates/agendao --bin agendao --root ~/.local
```

### 方式三：生成 release 分发包

```bash
./scripts/package-release.sh release
```

该脚本会生成固定布局的发布目录和压缩包：

```
dist/release/agendao-<version>-<target>/bin/agendao
dist/release/agendao-<version>-<target>/share/agendao/web
dist/release/agendao-<version>-<target>.tar.gz
```

### 版本同步脚本

本仓库使用日期版本。发布或本地版本推进时，优先使用脚本维护版本信息：

```bash
./scripts/release-date.sh 2026-05-17
```

该脚本会更新 workspace 版本并调用：

```bash
./scripts/sync_version.sh
```

`sync_version.sh` 会同步受管版本文件，包括：

- `Cargo.lock`
- `apps/agendao-web/package.json`
- `apps/agendao-web/package-lock.json`
- `docs/examples/plugins_example/rust/Cargo.lock`

README、安装指南、文档首页和示例文档中的展示版本需要作为发布说明的一部分同步更新。

---

## Linux 系统依赖

在 Linux 上构建可能需要以下开发库：

```bash
# Debian / Ubuntu
sudo apt-get install -y build-essential libssl-dev pkg-config

# Fedora / RHEL
sudo dnf install -y gcc openssl-devel

# Arch
sudo pacman -S base-devel openssl
```

---

## 验证安装

```bash
agendao version
which agendao
```

成功安装后输出类似：

```
AgenDao 2026.5.17
```

查看完整构建信息：

```bash
agendao info
```

输出包括编译器版本、目标平台、构建配置和数据路径：

```
AgenDao 2026.5.17

Build Info:
  Compiler:   rustc 1.xx.x
  Profile:    release
  Target:     x86_64-unknown-linux-gnu
  Host:       x86_64-unknown-linux-gnu
  Built at:   2026-05-17T...

Paths:
  Data:       ~/.local/share/agendao
  Config:     ~/.config/agendao
  Cache:      ~/.cache/agendao
```

确认二进制文件位置：

```bash
which agendao          # Linux / macOS
where agendao          # Windows (Command Prompt)
```

---

## 首次运行配置

### 1. 设置 API 密钥

AgenDao 需要至少一个 LLM Provider 的凭证才能工作。最简单的方式是设置环境变量：

```bash
# 智谱 BigModel（推荐）
export ZHIPUAI_API_KEY="zhipu-..."

# 或阿里云百炼
export ALIBABA_CN_API_KEY="dashscope-..."

# 或 Moonshot Kimi
export KIMI_FOR_CODING_API_KEY="kimi-..."

# 或使用本地 Ollama（无需 API 密钥）
# 先安装并启动 Ollama: https://ollama.ai
ollama pull llama3.2
```

将环境变量写入 shell profile 使其持久化：

```bash
# 添加到 ~/.bashrc 或 ~/.zshrc
echo 'export ZHIPUAI_API_KEY="zhipu-..."' >> ~/.bashrc
source ~/.bashrc
```

参见 [认证](auth) 了解所有支持的 Provider 及其配置方式。

### 2. 创建配置文件（可选）

AgenDao 在首次运行时会自动使用默认配置。如需自定义，创建项目级或全局配置文件：

**项目级配置**（推荐）：

```bash
# 在项目根目录创建
touch agendao.jsonc   # 或 agendao.json
```

**全局配置**：

```bash
mkdir -p ~/.config/agendao
touch ~/.config/agendao/agendao.jsonc   # 或 ~/.config/agendao/agendao.json
```

最小配置示例：

```jsonc
{
  "model": "glm-5.1",
  "provider": {
    "zhipuai": {
      "name": "Zhipu AI"
    }
  }
}
```

参见 [配置参考](configuration) 了解完整配置选项。

如果你还想一起启用外部 scheduler 配置，不要再去找旧的平铺示例文件名。当前示例入口在：

- `docs/examples/scheduler/presets/`
- `docs/examples/scheduler/verifier/`
- `docs/examples/scheduler/pso/`
- `docs/examples/scheduler/autoresearch/`

更稳妥的做法是把对应示例复制到你的项目里，再让 `schedulerPath` 指向项目内副本，这样示例里的 `workflowPath`、`agentTree` 和 `trees/` 相对路径不会丢。

### 3. 启动 AgenDao

```bash
# 在项目目录中启动 TUI
cd my-project
agendao

# 或直接执行单次任务
agendao run "explain the project structure"
```

---

## 重要目录

AgenDao 使用以下标准目录（遵循 XDG 规范）：

| 目录 | 路径 | 用途 |
|------|------|------|
| 数据目录 | `~/.local/share/agendao` | 日志、数据库、认证信息 |
| 配置目录 | `~/.config/agendao` | 全局配置 |
| 缓存目录 | `~/.cache/agendao` | 模型目录缓存、其他缓存 |
| 项目配置 | `<project>/.agendao/` | 项目级配置、agent、command |
| 项目根配置 | `<project>/agendao.jsonc` / `<project>/agendao.json` | 项目根配置文件 |

使用 `agendao debug paths` 查看当前系统中的实际路径。

---

## 可选 Cargo 特性

| 特性 | 说明 |
|------|------|
| 默认 | 核心功能集 |

如需调整产品装配层或发布入口，检查 `crates/agendao/Cargo.toml`；如需调整命令前端行为，检查 `crates/agendao-cli/Cargo.toml`。

---

## 环境变量

| 变量 | 说明 |
|------|------|
| `ZHIPUAI_API_KEY` | 智谱 BigModel API 密钥 |
| `ALIBABA_CN_API_KEY` | 阿里云百炼 API 密钥 |
| `KIMI_FOR_CODING_API_KEY` | Moonshot Kimi API 密钥 |
| `AGENDAO_SERVER_URL` | 服务器 URL（默认 `http://127.0.0.1:3000`） |
| `AGENDAO_WEB_DIST` | 显式覆盖默认内嵌 Web 资源，改为加载外部 `dist/` 目录 |
| `AGENDAO_CONFIG_DIR` | 覆盖配置目录路径 |
| `RUST_LOG` | 日志级别过滤（如 `debug`、`agendao_provider=trace`） |

完整的 Provider 环境变量列表参见 [认证](auth)。

---

## 卸载

```bash
# 移除单一分发入口
rm ~/.local/bin/agendao
rm -rf ~/.local/share/agendao/web
# 或
sudo rm /usr/local/bin/agendao
sudo rm -rf /usr/local/share/agendao/web

# 移除配置和数据（可选）
rm -rf ~/.config/agendao
rm -rf ~/.local/share/agendao
rm -rf ~/.cache/agendao
```

或使用内置卸载命令：

```bash
agendao uninstall
agendao uninstall --keep-config --keep-data   # 保留配置和数据
agendao uninstall --dry-run                   # 仅预览将删除的文件
```

---

## 升级

```bash
agendao upgrade
agendao upgrade v2026.6.3           # 升级到指定版本
agendao upgrade --method brew       # 显式指定包管理器方式
```

如果你是从源码 / 本地安装脚本安装的，推荐整体重装；`agendao` 会自动决定是否需要重新构建内嵌 Web：

```bash
cd agendao
git pull
npm --prefix apps/agendao-web install
./scripts/install-local.sh release ~/.local
```

---

## 常见问题

### 编译错误：OpenSSL

如果遇到 OpenSSL 相关编译错误，确保安装了 `libssl-dev`（Debian/Ubuntu）或 `openssl-devel`（Fedora/RHEL）。

### 首次运行无响应

首次运行时 AgenDao 需要从 `models.dev` 获取模型目录。如果网络超时（10 秒限制），Provider 列表可能不完整。设置环境变量 `RUST_LOG=debug` 查看详细日志。

### macOS Gatekeeper 警告

从源码构建的二进制可能触发 macOS 安全警告。右键点击二进制并选择"打开"，或运行：

```bash
xattr -dr com.apple.quarantine /usr/local/bin/agendao
```
