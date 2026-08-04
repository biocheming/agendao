//! 水 — AppOpOutcome: 非 prompt 异步操作回执 channel（U6）。
//!
//! 与 `DispatchOutcome`（prompt 发送回执）同构但语义分离：dispatch 管
//! "会话发送"，本 channel 管"设置/管理类操作"（provider 测连接、compact、
//! 会话打开、settings 写、弹窗拉取）——这些操作的路由终点是 store signal /
//! toast，不是 session 状态机，混进 DispatchOutcome 会让 session_id 路由
//! 守卫误杀（金：事件语义不可漂移）。
//!
//! 火（keymap/slash_action spawn 点火）→ 水（`Event::Tick` drain 回收）。
//! 后台 task 经 `sender()` 投递回执，主线程在 Tick 非阻塞 `drain()`。

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// provider 测连接结果（从 `TestProviderConnectionResponse` 剥离为 plain
/// data：回执要跨 task 边界，只带渲染 toast 需要的字段）。
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderTestData {
    pub ok: bool,
    pub status: Option<u16>,
    pub latency_ms: u128,
    pub error: Option<String>,
}

/// 一次异步操作的回流结果。
#[derive(Clone, Debug)]
pub enum AppOpOutcome {
    /// `settings_test_provider_connection` 的后台探测完成（server 只读
    /// 探测，最长 10s——U6 前是 UI 线程 block_on 的最长冻结点）。
    ProviderTested {
        provider_id: String,
        provider_name: String,
        /// Err = 传输层失败（网络/local 调用错误）；Ok 内 `ok=false` =
        /// server 探测到 provider 端不通（HTTP 状态/超时等）。
        result: Result<ProviderTestData, String>,
    },
    /// `/compact` 触发回执（触发调用本身可耗数秒——压缩本体在 server
    /// 侧异步跑，这里只是"已受理/受理失败"的通知）。
    CompactionTriggered {
        session_id: String,
        focus: Option<String>,
        result: Result<(), String>,
    },
    /// `open_session` 的后台拉取完成（U6③：原连续 5 个同步调用——
    /// get_session/get_messages/todos/questions/permissions，大会话下
    /// 冻结数秒）。主线程 Tick drain 时按字段逐个 apply（每个 fetch
    /// 独立成败：messages 挂了不拖垮 title/usage 播种）。
    SessionLoaded {
        session_id: String,
        data: Box<SessionOpenData>,
    },
    /// settings 写操作完成（U6④：connect/disconnect/toggle/save/delete
    /// 共 11 个写点原全部 UI 线程 block_on）。result 直接携带最终 toast
    /// 文案（Ok=成功文案，Err=失败文案）——文案在 spawn 点拼好（那里有
    /// 行的上下文：name/title/方向），drain 只负责 refresh + toast。
    SettingsWriteDone {
        refresh: SettingsRefresh,
        result: Result<String, String>,
    },
}

/// settings 写成功后回灌哪个 catalog（水律·回流，与旧同步路径同一
/// refresh 单点权威）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsRefresh {
    Mcp,
    Skills,
    Tools,
    Plugins,
}

/// open_session 后台拉取的载荷（plain data，跨 task 边界）。
#[derive(Clone, Debug)]
pub struct SessionOpenData {
    /// title + telemetry.usage 播种。
    pub info: Result<agendao_client::SessionInfo, String>,
    pub messages: Result<Vec<agendao_client::MessageInfo>, String>,
    pub todos: Result<Vec<agendao_client::ApiTodoItem>, String>,
    pub questions: Result<Vec<agendao_client::QuestionInfo>, String>,
    pub permissions: Result<Vec<agendao_client::PermissionRequestInfo>, String>,
}

/// 回流 channel。sender 交给后台 task，receiver 在 `Event::Tick` drain。
pub struct AppOps {
    tx: UnboundedSender<AppOpOutcome>,
    rx: UnboundedReceiver<AppOpOutcome>,
}

impl AppOps {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    /// 后台 task 用它投递回执（`UnboundedSender` 是 Clone+Send）。
    pub fn sender(&self) -> UnboundedSender<AppOpOutcome> {
        self.tx.clone()
    }

    /// 主线程 Tick 非阻塞消费全部积压回执。
    pub fn drain(&mut self) -> Vec<AppOpOutcome> {
        let mut out = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            out.push(d);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AppOpOutcome {
        AppOpOutcome::ProviderTested {
            provider_id: "openai".into(),
            provider_name: "OpenAI".into(),
            result: Ok(ProviderTestData {
                ok: true,
                status: Some(200),
                latency_ms: 42,
                error: None,
            }),
        }
    }

    #[test]
    fn drain_empty() {
        let mut ops = AppOps::new();
        assert!(ops.drain().is_empty());
    }

    #[test]
    fn send_and_drain() {
        let mut ops = AppOps::new();
        let tx = ops.sender();
        tx.send(sample()).unwrap();
        let out = ops.drain();
        assert_eq!(out.len(), 1);
        let AppOpOutcome::ProviderTested {
            provider_id,
            result,
            ..
        } = &out[0]
        else {
            panic!("expected ProviderTested");
        };
        assert_eq!(provider_id, "openai");
        assert_eq!(result.as_ref().unwrap().latency_ms, 42);
        assert!(ops.drain().is_empty(), "drain 后再取为空（已消费）");
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AppOps>();
        assert_send::<UnboundedSender<AppOpOutcome>>();
        assert_send::<AppOpOutcome>();
    }
}
