# Phase 5.1: Unix Socket 传输层实现完成

## 目标

实现 Unix Socket 传输层，支持本地进程间通信（IPC），作为 Direct 和 HTTP 之间的中间选项。

## 架构设计

### 三种传输模式对比

| 传输模式 | 开销 | 适用场景 | 特点 |
|---------|------|---------|------|
| **Direct** | 零开销 | TUI/CLI 单进程 | 直接调用 OrchestrationCore，无序列化 |
| **Unix Socket** | 最小开销 | 本地多进程 | JSON-RPC over Unix socket，本地 IPC |
| **HTTP** | 网络开销 | 远程访问/Web | HTTP/JSON，支持远程连接 |

### Unix Socket 的价值

1. **进程隔离**：允许多个客户端进程连接到同一个服务器进程
2. **资源共享**：多个客户端共享同一个 OrchestrationCore 实例
3. **性能优势**：比 HTTP 快，比 Direct 更灵活
4. **本地安全**：Unix socket 文件权限控制访问

## 实现细节

### 1. 客户端：UnixSocketTransport

**文件**: `crates/agendao-client/src/transport/unix.rs`

**协议**: JSON-RPC 2.0 over Unix socket
- 每个请求/响应是一行 JSON，以 `\n` 结尾
- 支持的方法：`prompt`, `list_sessions`, `get_session`

**核心实现**:
```rust
pub struct UnixSocketTransport {
    socket_path: String,
}

impl UnixSocketTransport {
    async fn send_request<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: T,
    ) -> Result<R> {
        // 1. 连接到 Unix socket
        let mut stream = UnixStream::connect(&self.socket_path).await?;
        
        // 2. 构造 JSON-RPC 请求
        let request = JsonRpcRequest { jsonrpc: "2.0", method, params, id: 1 };
        
        // 3. 发送请求
        let request_json = serde_json::to_string(&request)?;
        stream.write_all(request_json.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        
        // 4. 读取响应
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;
        
        // 5. 解析响应
        let response: JsonRpcResponse<R> = serde_json::from_str(&response_line)?;
        response.result.ok_or_else(|| anyhow!("Missing result"))
    }
}
```

### 2. 服务器端：UnixSocketServer

**文件**: `crates/agendao-server/src/unix_socket.rs`

**核心实现**:
```rust
pub struct UnixSocketServer {
    core: Arc<OrchestrationCore>,
    socket_path: String,
}

impl UnixSocketServer {
    pub async fn serve(&self) -> Result<()> {
        // 1. 绑定 Unix socket
        let listener = UnixListener::bind(&self.socket_path)?;
        
        // 2. 接受连接
        loop {
            let (stream, _) = listener.accept().await?;
            let core = Arc::clone(&self.core);
            
            // 3. 为每个连接启动独立任务
            tokio::spawn(async move {
                handle_connection(stream, core).await
            });
        }
    }
}

async fn handle_connection(stream: UnixStream, core: Arc<OrchestrationCore>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    
    // 逐行读取请求
    let mut line = String::new();
    while reader.read_line(&mut line).await? > 0 {
        let request: JsonRpcRequest = serde_json::from_str(&line)?;
        let response = handle_request(request, &core).await;
        
        // 发送响应
        let response_json = serde_json::to_string(&response)?;
        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        
        line.clear();
    }
    Ok(())
}
```

### 3. 传输层集成

**文件**: `crates/agendao-client/src/transport/mod.rs`

**更新的 FrontendTransport 枚举**:
```rust
pub enum FrontendTransport {
    Direct(DirectTransport),
    Unix(UnixSocketTransport),
    Http(HttpTransport),
}

impl FrontendTransport {
    pub async fn direct(config: &Config) -> Result<Self> {
        Ok(Self::Direct(DirectTransport::new(config).await?))
    }
    
    pub fn unix(socket_path: String) -> Self {
        Self::Unix(UnixSocketTransport::new(socket_path))
    }
    
    pub fn http(base_url: String, password: Option<String>) -> Self {
        Self::Http(HttpTransport::new(base_url, password))
    }
    
    // 所有方法都支持三种传输模式
    pub async fn prompt(&self, ...) -> Result<PromptResponse> {
        match self {
            Self::Direct(t) => t.prompt(...).await,
            Self::Unix(t) => t.prompt(...).await,
            Self::Http(t) => t.prompt(...).await,
        }
    }
}
```

### 4. 类型系统更新

为了支持 JSON 序列化，所有传输类型都添加了 `Serialize` 和 `Deserialize` 派生：

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromptResponse {
    pub session_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionListItem { ... }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionDetail { ... }
```

## JSON-RPC 协议

### 请求格式

```json
{
  "jsonrpc": "2.0",
  "method": "prompt",
  "params": {
    "session_id": "abc123",
    "text": "Hello",
    "agent_id": null,
    "model": "anthropic/claude-opus-4"
  },
  "id": 1
}
```

### 响应格式（成功）

```json
{
  "jsonrpc": "2.0",
  "result": {
    "session_id": "abc123",
    "message_id": "msg456",
    "text": "Hello! How can I help you?"
  },
  "id": 1
}
```

### 响应格式（错误）

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32000,
    "message": "Execution error: Provider not found"
  },
  "id": 1
}
```

### 错误码

| 错误码 | 含义 |
|-------|------|
| -32700 | Parse error（JSON 解析失败）|
| -32601 | Method not found（方法不存在）|
| -32602 | Invalid params（参数无效）|
| -32603 | Internal error（内部错误）|
| -32000 | Execution error（执行错误）|

## 使用示例

### 启动服务器

```rust
use agendao_orchestrator::OrchestrationCore;
use agendao_server::UnixSocketServer;

let config = agendao_config::Config::load()?;
let core = Arc::new(OrchestrationCore::new(&config).await?);
let server = UnixSocketServer::new(core, "/tmp/agendao.sock".to_string());

// 启动服务器（阻塞）
server.serve().await?;
```

### 客户端连接

```rust
use agendao_client::transport::{FrontendTransport, PromptOptions};

// 创建 Unix Socket 传输
let transport = FrontendTransport::unix("/tmp/agendao.sock".to_string());

// 发送请求
let response = transport.prompt(
    "session-123",
    "Hello, world!",
    PromptOptions::default()
).await?;

println!("Response: {}", response.text);
```

## 验证结果

### 编译成功

```bash
$ cargo check --workspace
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 10.66s
```

### 新增文件

- ✅ `crates/agendao-client/src/transport/unix.rs` - 客户端实现
- ✅ `crates/agendao-server/src/unix_socket.rs` - 服务器端实现

### 修改文件

- ✅ `crates/agendao-client/src/transport/mod.rs` - 添加 Unix 变体
- ✅ `crates/agendao-client/Cargo.toml` - 添加 tokio 依赖
- ✅ `crates/agendao-server/src/lib.rs` - 导出 unix_socket 模块

## 架构影响

### 符合 AgenDao 宪法

- **第一条（唯一执行内核）**：所有传输模式都调用同一个 `OrchestrationCore`
- **第九条（副作用路径唯一）**：Unix Socket 服务器只是传输层，不直接操作领域服务

### 分层清晰

```
┌─────────────────────────────────────────────────────────┐
│                   Adapter Layer                          │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐             │
│  │   TUI    │  │   CLI    │  │   Web    │             │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘             │
└───────┼─────────────┼─────────────┼────────────────────┘
        │             │             │
        │ Direct      │ Unix Socket │ HTTP
        │             │             │
        └─────────────┴─────────────┘
                      │
        ┌─────────────▼─────────────┐
        │   UnixSocketServer         │
        │   (agendao-server)          │
        └─────────────┬─────────────┘
                      │
        ┌─────────────▼─────────────┐
        │   OrchestrationCore        │
        │   (唯一执行内核)            │
        └───────────────────────────┘
```

## 性能特性

### 预期性能

| 操作 | Direct | Unix Socket | HTTP |
|-----|--------|-------------|------|
| 连接建立 | 0ms | <1ms | 1-5ms |
| 请求延迟 | 0ms | <1ms | 5-20ms |
| 序列化开销 | 无 | JSON | JSON + HTTP |
| 适用场景 | 单进程 | 本地多进程 | 远程访问 |

### 使用场景

1. **Direct**：TUI/CLI 单进程模式，最快
2. **Unix Socket**：
   - 多个 CLI 实例共享同一个服务器
   - 长期运行的后台服务
   - 需要进程隔离但不需要网络访问
3. **HTTP**：
   - Web 前端
   - 远程访问
   - 跨网络通信

## 已知限制

1. **平台限制**：Unix Socket 仅支持 Unix-like 系统（Linux, macOS）
   - Windows 需要使用 Named Pipes 或 HTTP
   
2. **单向通信**：当前实现是请求-响应模式
   - 不支持服务器主动推送
   - 流式输出需要额外实现

3. **错误处理**：连接断开时客户端需要重连
   - 没有自动重连机制
   - 需要上层处理连接失败

4. **并发限制**：每个连接一个 tokio 任务
   - 大量并发连接可能消耗资源
   - 需要连接池管理（未实现）

## 下一步（Phase 5.2）

1. **集成到 TUI/CLI**
   - 添加 `--socket` 参数支持
   - 实现传输模式自动选择

2. **服务器启动集成**
   - 在 `agendao-server` 中添加 Unix Socket 启动选项
   - 支持同时监听 Unix Socket 和 HTTP

3. **测试**
   - 端到端测试
   - 并发连接测试
   - 错误恢复测试

4. **文档**
   - 用户文档：如何使用 Unix Socket 模式
   - 开发者文档：如何扩展协议

## 总结

Phase 5.1 成功完成：

✅ 实现了 UnixSocketTransport 客户端  
✅ 实现了 UnixSocketServer 服务器端  
✅ 定义了 JSON-RPC 协议  
✅ 集成到 FrontendTransport 枚举  
✅ 添加了 Serialize/Deserialize 支持  
✅ 整个 workspace 编译通过  
✅ 符合 AgenDao 宪法的架构原则  

关键成就：提供了第三种传输模式，在性能和灵活性之间取得平衡，为本地多进程场景提供了高效的 IPC 方案。
