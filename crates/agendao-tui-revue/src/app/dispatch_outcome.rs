//! 水 — DispatchOutcome: 本地发送回执 channel。
//!
//! 与服务端 `FrontendEvent`（`EventBus`）严格分离：`FrontendEvent` 是"服务端
//! 推送"语义（server → client 的 session 投影），而发送回执是"本地编排结果"
//! （dispatch 的后台 task 完成通知）。两类来源、生命周期、路由都不同，混入
//! 同一个 enum 会让 `apply_frontend_event` 错把本地回执当服务端事件处理
//! （金：事件语义不可漂移）。
//!
//! 火（dispatch spawn 点火）→ 水（`Event::Tick` drain 回收）。后台 task 经
//! `sender()` 投递回执，主线程在 Tick 非阻塞 `drain()`。

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// 一次 dispatch 的回流结果。携带 `(session_id, user_msg_id)` 用于路由与回收。
#[derive(Clone, Debug)]
pub enum DispatchOutcome {
    /// `send_prompt_with` 成功。`status` 来自 `PromptResponse`
    ///（queued / awaiting_user / ...）。成功时无需回收乐观消息。
    Sent {
        session_id: String,
        status: String,
    },
    /// `send_prompt_with` 失败。dispatch 已乐观 push 的 user message
    /// 需在 Tick drain 回收（生命周期对称：push ↔ remove）。
    Failed {
        session_id: String,
        user_msg_id: String,
        error: String,
    },
}

/// 回流 channel。sender 交给后台 task，receiver 在 `Event::Tick` drain。
pub struct DispatchOutcomes {
    tx: UnboundedSender<DispatchOutcome>,
    rx: UnboundedReceiver<DispatchOutcome>,
}

impl DispatchOutcome {
    /// 两个 variant 都携带 `session_id`，用于 Tick drain 的路由守卫
    ///（仅处理当前 active_session，过滤用户切走后的陈旧回执）。
    pub fn session_id(&self) -> &str {
        match self {
            DispatchOutcome::Sent { session_id, .. } => session_id,
            DispatchOutcome::Failed { session_id, .. } => session_id,
        }
    }
}

impl DispatchOutcomes {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }

    /// 后台 task 用它投递回执（`UnboundedSender` 是 Clone+Send）。
    pub fn sender(&self) -> UnboundedSender<DispatchOutcome> {
        self.tx.clone()
    }

    /// 主线程 Tick 非阻塞消费全部积压回执。
    pub fn drain(&mut self) -> Vec<DispatchOutcome> {
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

    #[test]
    fn drain_empty() {
        let mut o = DispatchOutcomes::new();
        assert!(o.drain().is_empty());
    }

    #[test]
    fn send_and_drain() {
        let mut o = DispatchOutcomes::new();
        let tx = o.sender();
        tx.send(DispatchOutcome::Sent {
            session_id: "s1".into(),
            status: "queued".into(),
        })
        .unwrap();
        let out = o.drain();
        assert_eq!(out.len(), 1);
        // drain 后再取为空（已消费）
        assert!(o.drain().is_empty());
    }

    #[test]
    fn is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<DispatchOutcomes>();
        assert_send::<UnboundedSender<DispatchOutcome>>();
        assert_send::<DispatchOutcome>();
    }
}
