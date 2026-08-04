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
