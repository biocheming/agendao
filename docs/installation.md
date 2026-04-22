# ROCode 安装指南

本文档介绍 ROCode 的系统要求、构建安装方式以及首次运行配置。

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

ROCode 目前以源码形式分发。克隆仓库并构建：

```bash
git clone <repo-url>
cd rocode

# Web 前端单独构建
npm --prefix apps/rocode-web install
npm --prefix apps/rocode-web run build

# 推荐：一次性构建并安装三个并列二进制
./scripts/install-local.sh release ~/.local
```

安装后固定布局为：

```
~/.local/bin/rocode
~/.local/bin/rocode-server
~/.local/bin/rocode-tui
~/.local/share/rocode/web
```

如果只想构建、不立即安装：

```bash
cargo build --release -p rocode-cli -p rocode-server -p rocode-tui
npm --prefix apps/rocode-web run build
```

### 方式二：cargo install

```bash
cargo install --path crates/rocode-cli --bin rocode --root ~/.local
cargo install --path crates/rocode-server --root ~/.local
cargo install --path crates/rocode-tui --root ~/.local
```

`cargo install` 也必须把这三个二进制安装到同一个 `--root` 下，否则 `rocode` 无法在运行时找到 `rocode-server` / `rocode-tui`。

### 方式三：生成 release 分发包

```bash
./scripts/package-release.sh release
```

该脚本会生成固定布局的发布目录和压缩包：

```
dist/release/rocode-<version>-<target>/bin/rocode
dist/release/rocode-<version>-<target>/bin/rocode-server
dist/release/rocode-<version>-<target>/bin/rocode-tui
dist/release/rocode-<version>-<target>/share/rocode/web
dist/release/rocode-<version>-<target>.tar.gz
```

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
rocode version
which rocode
which rocode-server
which rocode-tui
```

成功安装后输出类似：

```
ROCode 2026.4.22
```

查看完整构建信息：

```bash
rocode info
```

输出包括编译器版本、目标平台、构建配置和数据路径：

```
ROCode 2026.4.22

Build Info:
  Compiler:   rustc 1.xx.x
  Profile:    release
  Target:     x86_64-unknown-linux-gnu
  Host:       x86_64-unknown-linux-gnu
  Built at:   2026-04-22T...

Paths:
  Data:       ~/.local/share/rocode
  Config:     ~/.config/rocode
  Cache:      ~/.cache/rocode
```

确认二进制文件位置：

```bash
which rocode          # Linux / macOS
where rocode          # Windows (Command Prompt)
```

---

## 首次运行配置

### 1. 设置 API 密钥

ROCode 需要至少一个 LLM Provider 的凭证才能工作。最简单的方式是设置环境变量：

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

ROCode 在首次运行时会自动使用默认配置。如需自定义，创建项目级或全局配置文件：

**项目级配置**（推荐）：

```bash
# 在项目根目录创建
touch rocode.jsonc
```

**全局配置**：

```bash
mkdir -p ~/.config/rocode
touch ~/.config/rocode/rocode.jsonc
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

### 3. 启动 ROCode

```bash
# 在项目目录中启动 TUI
cd my-project
rocode

# 或直接执行单次任务
rocode run "explain the project structure"
```

---

## 重要目录

ROCode 使用以下标准目录（遵循 XDG 规范）：

| 目录 | 路径 | 用途 |
|------|------|------|
| 数据目录 | `~/.local/share/rocode` | 日志、数据库、认证信息 |
| 配置目录 | `~/.config/rocode` | 全局配置 |
| 缓存目录 | `~/.cache/rocode` | 模型目录缓存、其他缓存 |
| 项目配置 | `<project>/.rocode/` | 项目级配置、agent、command |
| 项目根配置 | `<project>/rocode.jsonc` | 项目根配置文件 |

使用 `rocode debug paths` 查看当前系统中的实际路径。

---

## 可选 Cargo 特性

| 特性 | 说明 |
|------|------|
| 默认 | 核心功能集 |

如需启用额外功能，修改 `crates/rocode-cli/Cargo.toml` 中的 feature 标志。

---

## 环境变量

| 变量 | 说明 |
|------|------|
| `ZHIPUAI_API_KEY` | 智谱 BigModel API 密钥 |
| `ALIBABA_CN_API_KEY` | 阿里云百炼 API 密钥 |
| `KIMI_FOR_CODING_API_KEY` | Moonshot Kimi API 密钥 |
| `ROCODE_SERVER_URL` | 服务器 URL（默认 `http://127.0.0.1:3000`） |
| `ROCODE_SERVER_BIN` | 覆盖 CLI 启动 `rocode-server` 时使用的可执行文件路径 |
| `ROCODE_TUI_BIN` | 覆盖 CLI 启动 `rocode-tui` 时使用的可执行文件路径 |
| `ROCODE_WEB_DIST` | 覆盖 `rocode-server` 加载外部 Web 前端 `dist/` 目录的位置 |
| `ROCODE_CONFIG_DIR` | 覆盖配置目录路径 |
| `RUST_LOG` | 日志级别过滤（如 `debug`、`rocode_provider=trace`） |

完整的 Provider 环境变量列表参见 [认证](auth)。

---

## 卸载

```bash
# 移除三个并列二进制
rm ~/.local/bin/rocode
rm ~/.local/bin/rocode-server
rm ~/.local/bin/rocode-tui
rm -rf ~/.local/share/rocode/web
# 或
sudo rm /usr/local/bin/rocode
sudo rm /usr/local/bin/rocode-server
sudo rm /usr/local/bin/rocode-tui
sudo rm -rf /usr/local/share/rocode/web

# 移除配置和数据（可选）
rm -rf ~/.config/rocode
rm -rf ~/.local/share/rocode
rm -rf ~/.cache/rocode
```

或使用内置卸载命令：

```bash
rocode uninstall
rocode uninstall --keep-config --keep-data   # 保留配置和数据
rocode uninstall --dry-run                   # 仅预览将删除的文件
```

---

## 升级

```bash
rocode upgrade
rocode upgrade v2026.4.22           # 升级到指定版本
rocode upgrade --method brew       # 显式指定包管理器方式
```

如果你是从源码 / 本地安装脚本安装的，请整体重装三件套，而不是只替换 `rocode`：

```bash
cd rocode
git pull
npm --prefix apps/rocode-web install
npm --prefix apps/rocode-web run build
./scripts/install-local.sh release ~/.local
```

---

## 常见问题

### 编译错误：OpenSSL

如果遇到 OpenSSL 相关编译错误，确保安装了 `libssl-dev`（Debian/Ubuntu）或 `openssl-devel`（Fedora/RHEL）。

### 首次运行无响应

首次运行时 ROCode 需要从 `models.dev` 获取模型目录。如果网络超时（10 秒限制），Provider 列表可能不完整。设置环境变量 `RUST_LOG=debug` 查看详细日志。

### macOS Gatekeeper 警告

从源码构建的二进制可能触发 macOS 安全警告。右键点击二进制并选择"打开"，或运行：

```bash
xattr -dr com.apple.quarantine /usr/local/bin/rocode
```
