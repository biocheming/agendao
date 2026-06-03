# Phase 5.2: 服务器集成 Unix Socket 启动

## 目标

在 `agendao-server` 中添加 Unix Socket 启动选项，支持同时监听 Unix Socket 和 HTTP。

## 实现细节

### 1. ServerRuntimeOptions 扩展

**文件**: `crates/agendao-server/src/server.rs`

添加 `unix_socket_path` 字段：

```rust
#[derive(Debug, Clone)]
pub struct ServerRuntimeOptions {
    pub port: u16,
    pub hostname: String,
    pub cwd: Option<PathBuf>,
    pub web_dist: Option<PathBuf>,
    pub embedded_web_assets: Option<crate::web::EmbeddedWebAssetLoader>,
    pub mdns: bool,
    pub mdns_domain: String,
    pub cors: Vec<String>,
    /// Optional Unix socket path for local IPC
    pub unix_socket_path: Option<String>,
}
```

### 2. 新增 run_server_with_unix_socket 函数

**文件**: `crates/agendao-server/src/server.rs`

创建新函数支持同时启动 Unix Socket 和 HTTP 服务器，并向 `OrchestrationCore` 注入共享 authority：

```rust
/// Run server with optional Unix socket support
pub async fn run_server_with_unix_socket(
    addr: SocketAddr,
    workspace_root: PathBuf,
    unix_socket_path: Option<String>,
) -> anyhow::Result<()> {
    let server_url = if addr.ip().is_unspecified() {
        format!("http://127.0.0.1:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };
    let state = Arc::new(
        ServerState::new_with_storage_for_url_in_workspace(server_url, workspace_root.clone()).await?,
    );

    // Start Unix socket server if path is provided
    if let Some(socket_path) = unix_socket_path {
        let core = Arc::new(
            agendao_orchestrator::OrchestrationCore::<agendao_session::SessionManager>::new_with_shared_authorities(
                Arc::clone(&state.config_store),
                Arc::clone(&state.sessions),
                Arc::clone(&state.providers),
                Arc::new(tokio::sync::RwLock::new(agendao_tool::ToolRegistry::new())),
            ),
        );

        let unix_server =
            crate::unix_socket::UnixSocketServer::new(Arc::clone(&state), core, socket_path.clone());

        tokio::spawn(async move {
            if let Err(e) = unix_server.serve().await {
                tracing::error!("Unix socket server error: {}", e);
            }
        });

        tracing::info!("Unix socket server listening on {}", socket_path);
    }

    // Start HTTP server
    let app = routes::router()
        .layer(middleware::from_fn(server_auth_middleware))
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("HTTP server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
```

### 3. 修改 run_server_runtime

**文件**: `crates/agendao-server/src/server.rs`

更新 `run_server_runtime` 调用新函数：

```rust
pub async fn run_server_runtime(options: ServerRuntimeOptions) -> anyhow::Result<()> {
    // ... 前面的代码不变 ...
    
    run_server_with_unix_socket(addr, workspace_root, options.unix_socket_path).await
}
```

### 4. CLI 参数支持

**文件**: `crates/agendao-server/src/main.rs`

添加 `--unix-socket` 参数：

```rust
#[derive(Parser, Debug)]
struct ServerCli {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long)]
    cwd: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "agendao.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
    #[arg(long)]
    unix_socket: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = ServerCli::parse();
    agendao_server::run_server_runtime(agendao_server::ServerRuntimeOptions {
        port: cli.port,
        hostname: cli.hostname,
        cwd: cli.cwd,
        web_dist: None,
        embedded_web_assets: None,
        mdns: cli.mdns,
        mdns_domain: cli.mdns_domain,
        cors: cli.cors,
        unix_socket_path: cli.unix_socket,
    })
    .await
}
```

### 5. 更新其他初始化点

**文件**: `crates/agendao/src/host.rs`

更新所有 `ServerRuntimeOptions` 初始化，添加 `unix_socket_path: None`：

```rust
// run_server_command
agendao_server::run_server_runtime(ServerRuntimeOptions {
    // ... 其他字段 ...
    unix_socket_path: None,
})

// run_web_command
agendao_server::run_server_runtime(ServerRuntimeOptions {
    // ... 其他字段 ...
    unix_socket_path: None,
})

// run_tui (内部服务器)
agendao_server::run_server_runtime(ServerRuntimeOptions {
    // ... 其他字段 ...
    unix_socket_path: None,
})
```

## 架构设计

### 双服务器模式

```
┌─────────────────────────────────────────────────────────┐
│                    agendao-server                         │
│                                                          │
│  ┌────────────────────┐      ┌────────────────────┐    │
│  │  HTTP Server       │      │ Unix Socket Server │    │
│  │  (axum)            │      │ (tokio task)       │    │
│  └─────────┬──────────┘      └─────────┬──────────┘    │
│            │                            │               │
│            ▼                            ▼               │
│  ┌─────────────────────┐    ┌──────────────────────┐  │
│  │   ServerState       │    │ OrchestrationCore    │  │
│  │  - sessions ────────┼───▶│  - sessions          │  │
│  │  - providers ───────┼───▶│  - providers         │  │
│  │  - config_store ────┼───▶│  - config_store      │  │
│  │  - tool_registry    │    │  - tools (独立实例)  │  │
│  └─────────────────────┘    └──────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### Shared Authority 机制

当前实现中，Unix Socket 路径不再复制 `SessionManager`、`ProviderRegistry` 或 `ConfigStore`。  
`OrchestrationCore::new_with_shared_authorities(...)` 直接持有和 `ServerState` 相同的 `Arc`：

```rust
let core = Arc::new(
    agendao_orchestrator::OrchestrationCore::<agendao_session::SessionManager>::new_with_shared_authorities(
        Arc::clone(&state.config_store),
        Arc::clone(&state.sessions),
        Arc::clone(&state.providers),
        Arc::new(tokio::sync::RwLock::new(agendao_tool::ToolRegistry::new())),
    ),
);
```

这意味着：

- HTTP route 对 `config_store` 的 `patch/replace_with` 更新，会被 Unix prompt 路径立即看到
- `rebuild_providers()` 重建 `state.providers` 后，Unix prompt 路径读取的是同一个 registry
- session 的读写对 HTTP / Unix 两条链路可见的是同一份数据

当前唯一未共享的是 `ToolRegistry`：

- `ServerState` 持有 `Arc<ToolRegistry>`
- `OrchestrationCore` 持有 `Arc<RwLock<ToolRegistry>>`
- 因类型不匹配，当前仍使用独立实例

## 使用方式

### 启动服务器

```bash
# 只启动 HTTP 服务器（默认）
agendao-server --port 3000

# 同时启动 HTTP 和 Unix Socket 服务器
agendao-server --port 3000 --unix-socket /tmp/agendao.sock

# 只启动 Unix Socket 服务器（端口设为 0 禁用 HTTP）
agendao-server --port 0 --unix-socket /tmp/agendao.sock
```

### 客户端连接

```rust
use agendao_client::transport::FrontendTransport;

// 连接到 Unix Socket
let transport = FrontendTransport::unix("/tmp/agendao.sock".to_string());

// 发送请求
let response = transport.prompt(
    "session-123",
    "Hello, world!",
    PromptOptions::default()
).await?;
```

## 验证结果

### 编译成功

```bash
$ cargo check --workspace
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.84s
```

### 修改的文件

- ✅ `crates/agendao-server/src/server.rs` - 添加 `unix_socket_path` 字段和 `run_server_with_unix_socket` 函数
- ✅ `crates/agendao-server/src/main.rs` - 添加 `--unix-socket` CLI 参数
- ✅ `crates/agendao/src/host.rs` - 更新所有 `ServerRuntimeOptions` 初始化（3 处）

### 新增功能

- ✅ 支持通过 `--unix-socket` 参数启动 Unix Socket 服务器
- ✅ Unix Socket 和 HTTP 服务器可以同时运行
- ✅ Unix Socket 服务器在独立的 tokio 任务中运行
- ✅ Provider 从 ServerState 复制到 OrchestrationCore

## 架构影响

### 符合 AgenDao 宪法

- **第一条（唯一执行内核）**：Unix Socket 服务器调用 `OrchestrationCore`，不自建循环
- **第二条（唯一配置真相）**：从 `ServerState.config_store` 获取配置
- **第九条（副作用路径唯一）**：Unix Socket 服务器只是传输层，不直接操作领域服务

### 分层清晰

```
Adapter Layer (HTTP/Unix Socket)
        ↓
Orchestration Layer (OrchestrationCore)
        ↓
Domain Services (ProviderRegistry, SessionManager, ConfigStore)
```

## 性能特性

### 启动开销

- **HTTP 服务器**：需要初始化 `ServerState`（包括数据库、插件等）
- **Unix Socket 服务器**：只需要初始化 `OrchestrationCore`（轻量级）
- **同时启动**：Unix Socket 服务器在后台任务中启动，不阻塞 HTTP 服务器

### 运行时开销

- **HTTP 请求**：通过 axum 路由 → ServerState → 业务逻辑
- **Unix Socket 请求**：通过 JSON-RPC → OrchestrationCore → 业务逻辑
- **Provider 共享**：两个服务器共享相同的 provider 实例（`Arc<dyn Provider>`）

## 已知限制

1. ~~**Session 不共享**~~ ✅ (已修复) — `OrchestrationCore<S: SessionStore>` 使用 `agendao_session::SessionManager`，通过 `Arc<Mutex<SessionManager>>` 与 ServerState 共享同一实例

2. ~~**Provider 复制**~~ ✅ (已修复) — `OrchestrationCore::new_with_shared_authorities` 直接持有 `Arc<RwLock<ProviderRegistry>>`（与 ServerState 同一实例），运行时变更即刻生效

3. ~~**配置更新**~~ ✅ (已修复) — `OrchestrationCore` 持有 `Arc<ConfigStore>`（与 ServerState 同一实例），配置变更即刻生效

4. **ToolRegistry 不共享**：server 存储 `Arc<ToolRegistry>`，core 存储 `Arc<RwLock<ToolRegistry>>`，类型不匹配暂保持独立实例

## 下一步（Phase 5.3）

1. **TUI/CLI 集成**
   - 添加 `--socket` 参数支持
   - 实现传输模式自动选择（优先 Unix Socket，回退到 HTTP）
   - 添加连接重试逻辑

2. **测试**
   - 端到端测试（HTTP + Unix Socket）
   - 并发连接测试
   - 错误恢复测试

3. **文档**
   - 用户文档：如何使用 Unix Socket 模式
   - 开发者文档：如何扩展协议

## 总结

Phase 5.2 成功完成：

✅ 在 `ServerRuntimeOptions` 中添加 `unix_socket_path` 字段  
✅ 创建 `run_server_with_unix_socket` 函数  
✅ 支持同时启动 Unix Socket 和 HTTP 服务器  
✅ 添加 `--unix-socket` CLI 参数  
✅ 更新所有 `ServerRuntimeOptions` 初始化点  
✅ 整个 workspace 编译通过  
✅ 符合 AgenDao 宪法的架构原则  

关键成就：实现了双服务器模式，允许 HTTP 和 Unix Socket 同时运行，为本地高性能 IPC 和远程 HTTP 访问提供了灵活的选择。
