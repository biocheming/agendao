//! # M4.1 Command Gateway & Disposition Flow
//!
//! 命令网关职责：
//! 1. 唯一写入口：将 `SubmitIntent` / `InterruptIntent` 转换为服务端命令；
//! 2. 生成 `client_request_id`（UUID v4）确保提交与重试幂等；
//! 3. 严格的草稿与回执事务保护：
//!    - 发送前绝不清空 Prompt；
//!    - 只有当服务端返回接受回执（Started / Queued / SteeringPending），且当前 Session 与 DraftRevision 仍匹配时才清空草稿；
//!    - 若 Session 已切换或草稿已被修改（revision 变化），则保留新草稿，不静默覆盖；
//!    - 失败（Rejected / 网络异常）保留当前草稿并生成可观测错误；
//! 4. 纯函数回执结算：不直接产生 UI 副作用，生成 `GatewayReceiptOutcome` 供调用方确定性应用。

use crate::interaction_contract::TurnId;
pub use agendao_types::submission::{
    InterruptCommand, InterruptDisposition, SubmissionDisposition, SubmissionMode,
    SubmissionRejectionReason, SubmitInputCommand,
};
use serde::{Deserialize, Serialize};

pub type ClientRequestId = String;
pub type SessionId = String;

/// 网关命令枚举（Interrupt 在领域层独立于文本提交）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayCommand {
    Submit(SubmitInputCommand),
    Interrupt(InterruptCommand),
}

/// 提交请求在发起时捕获的上下文
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionContext {
    pub client_request_id: ClientRequestId,
    pub session_id: SessionId,
    pub draft_revision: u64,
    pub content: String,
    pub mode: SubmissionMode,
}

/// 中断请求在发起时捕获的上下文
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptContext {
    pub client_request_id: ClientRequestId,
    pub session_id: SessionId,
    pub expected_turn_id: TurnId,
}

/// Captured authority coordinates for a queue body edit. Unlike prompt
/// submission, acceptance never authorizes a client-side projection update:
/// the subsequent runtime snapshot remains the sole QueueSummary writer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEditContext {
    pub session_id: SessionId,
    pub item_id: String,
    pub expected_revision: u64,
    pub draft_revision: u64,
    pub content: String,
}

/// 网关返回的领域回执（Submit 返回 SubmissionDisposition，Interrupt 返回 InterruptDisposition）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayServerResponse {
    Submit(SubmissionDisposition),
    Interrupt(InterruptDisposition),
}

/// 网关回执结算结果
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayReceiptOutcome {
    /// 提交被服务端接受（Started / Queued / SteeringPending），且草稿版本匹配，应清空 Prompt
    SubmitAcceptedAndClearPrompt {
        client_request_id: ClientRequestId,
        disposition: SubmissionDisposition,
    },
    /// 提交被服务端接受，但草稿已被用户修改或 Session 已切换，保留新 Prompt
    SubmitAcceptedRetainModifiedPrompt {
        client_request_id: ClientRequestId,
        disposition: SubmissionDisposition,
        current_session: SessionId,
        current_revision: u64,
    },
    /// 提交被服务端拒绝，保留 Prompt
    SubmitRejected {
        client_request_id: ClientRequestId,
        reason: SubmissionRejectionReason,
        message: String,
    },
    /// 网络/传输层失败，保留 Prompt
    TransportFailed {
        client_request_id: ClientRequestId,
        error: String,
    },
    /// 中断成功
    InterruptAcknowledged {
        client_request_id: ClientRequestId,
        turn_id: TurnId,
    },
    /// 中断被拒绝
    InterruptRejected {
        client_request_id: ClientRequestId,
        reason: String,
    },
}

/// CommandGateway 核心逻辑（纯函数与事务状态结算）
pub struct CommandGateway;

impl CommandGateway {
    pub fn prepare_queue_edit(
        session_id: SessionId,
        item_id: String,
        expected_revision: u64,
        draft_revision: u64,
        content: String,
    ) -> (
        agendao_types::submission::QueueEditRequest,
        QueueEditContext,
    ) {
        let request = agendao_types::submission::QueueEditRequest {
            client_request_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.clone(),
            item_id: item_id.clone(),
            expected_revision,
            content: content.clone(),
        };
        let context = QueueEditContext {
            session_id,
            item_id,
            expected_revision,
            draft_revision,
            content,
        };
        (request, context)
    }
    /// 创建提交命令与追踪上下文
    pub fn prepare_submit(
        session_id: SessionId,
        draft_revision: u64,
        content: &str,
        mode: SubmissionMode,
    ) -> (SubmitInputCommand, SubmissionContext) {
        let client_request_id = uuid::Uuid::new_v4().to_string();
        let cmd = SubmitInputCommand {
            client_request_id: client_request_id.clone(),
            session_id: session_id.clone(),
            mode: mode.clone(),
            content: content.to_string(),
        };
        let ctx = SubmissionContext {
            client_request_id,
            session_id,
            draft_revision,
            content: content.to_string(),
            mode,
        };
        (cmd, ctx)
    }

    /// 创建中断命令与追踪上下文
    pub fn prepare_interrupt(
        session_id: SessionId,
        expected_turn_id: TurnId,
    ) -> (InterruptCommand, InterruptContext) {
        let client_request_id = uuid::Uuid::new_v4().to_string();
        let cmd = InterruptCommand {
            client_request_id: client_request_id.clone(),
            session_id: session_id.clone(),
            expected_turn_id: expected_turn_id.clone(),
        };
        let ctx = InterruptContext {
            client_request_id,
            session_id,
            expected_turn_id,
        };
        (cmd, ctx)
    }

    /// 结算提交回执：
    /// 传入发起时的 `SubmissionContext`，以及当前实时的 `(current_session, current_revision)` 权威状态
    pub fn settle_submission_disposition(
        ctx: &SubmissionContext,
        disposition: Result<SubmissionDisposition, String>,
        current_session: &str,
        current_revision: u64,
    ) -> GatewayReceiptOutcome {
        match disposition {
            Ok(disp) => match &disp {
                SubmissionDisposition::Started { .. }
                | SubmissionDisposition::Queued { .. }
                | SubmissionDisposition::SteeringPending { .. } => {
                    // 仅当 Session 相同且草稿未发生任何二次编辑时，才清空 Prompt
                    if ctx.session_id == current_session && ctx.draft_revision == current_revision {
                        GatewayReceiptOutcome::SubmitAcceptedAndClearPrompt {
                            client_request_id: ctx.client_request_id.clone(),
                            disposition: disp,
                        }
                    } else {
                        GatewayReceiptOutcome::SubmitAcceptedRetainModifiedPrompt {
                            client_request_id: ctx.client_request_id.clone(),
                            disposition: disp,
                            current_session: current_session.to_string(),
                            current_revision,
                        }
                    }
                }
                SubmissionDisposition::Rejected { reason, message } => {
                    GatewayReceiptOutcome::SubmitRejected {
                        client_request_id: ctx.client_request_id.clone(),
                        reason: reason.clone(),
                        message: message.clone(),
                    }
                }
            },
            Err(err) => GatewayReceiptOutcome::TransportFailed {
                client_request_id: ctx.client_request_id.clone(),
                error: err,
            },
        }
    }

    /// 结算中断回执
    pub fn settle_interrupt_disposition(
        ctx: &InterruptContext,
        disposition: Result<InterruptDisposition, String>,
    ) -> GatewayReceiptOutcome {
        match disposition {
            Ok(InterruptDisposition::Interrupted { turn_id, .. }) => {
                GatewayReceiptOutcome::InterruptAcknowledged {
                    client_request_id: ctx.client_request_id.clone(),
                    turn_id,
                }
            }
            Ok(InterruptDisposition::Rejected { reason, .. }) => {
                GatewayReceiptOutcome::InterruptRejected {
                    client_request_id: ctx.client_request_id.clone(),
                    reason,
                }
            }
            Err(err) => GatewayReceiptOutcome::TransportFailed {
                client_request_id: ctx.client_request_id.clone(),
                error: err,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepare_submit_generates_unique_client_request_id() {
        let (cmd1, ctx1) =
            CommandGateway::prepare_submit("s1".into(), 1, "msg1", SubmissionMode::Auto);
        let (cmd2, ctx2) =
            CommandGateway::prepare_submit("s1".into(), 1, "msg1", SubmissionMode::Auto);

        assert_ne!(cmd1.client_request_id, cmd2.client_request_id);
        assert_eq!(cmd1.client_request_id, ctx1.client_request_id);
        assert_eq!(cmd1.mode, SubmissionMode::Auto);
        assert_eq!(cmd2.client_request_id, ctx2.client_request_id);
    }

    #[test]
    fn queue_edit_prepare_generates_unique_id_and_captures_cas() {
        let (a, ca) =
            CommandGateway::prepare_queue_edit("s".into(), "q".into(), 7, 11, "one".into());
        let (b, cb) =
            CommandGateway::prepare_queue_edit("s".into(), "q".into(), 7, 11, "one".into());
        assert_ne!(a.client_request_id, b.client_request_id);
        assert_eq!(ca.expected_revision, 7);
        assert_eq!(cb.item_id, "q");
    }

    #[test]
    fn test_settle_submission_accepted_and_clear_when_revision_matches() {
        let (_cmd, ctx) =
            CommandGateway::prepare_submit("s1".into(), 5, "Hello", SubmissionMode::Auto);
        let disp = Ok(SubmissionDisposition::Started {
            turn_id: "t1".into(),
            session_id: "s1".into(),
        });

        // session 相同，revision 相同 -> 允许清空已发送草稿
        let outcome = CommandGateway::settle_submission_disposition(&ctx, disp, "s1", 5);
        assert!(matches!(
            outcome,
            GatewayReceiptOutcome::SubmitAcceptedAndClearPrompt { .. }
        ));
    }

    #[test]
    fn test_settle_submission_retains_prompt_when_draft_modified() {
        let (_cmd, ctx) =
            CommandGateway::prepare_submit("s1".into(), 5, "Hello", SubmissionMode::Auto);
        let disp = Ok(SubmissionDisposition::Queued {
            item_id: "item_1".into(),
            session_id: "s1".into(),
            position: 1,
            queue_revision: 1,
        });

        // 用户在等待回执期间键入了新内容（revision 从 5 变成 6） -> 保留新草稿
        let outcome = CommandGateway::settle_submission_disposition(&ctx, disp, "s1", 6);
        match outcome {
            GatewayReceiptOutcome::SubmitAcceptedRetainModifiedPrompt {
                current_revision, ..
            } => {
                assert_eq!(current_revision, 6);
            }
            _ => panic!("Expected SubmitAcceptedRetainModifiedPrompt"),
        }
    }

    #[test]
    fn test_settle_submission_retains_prompt_when_session_switched() {
        let (_cmd, ctx) =
            CommandGateway::prepare_submit("s1".into(), 5, "Hello", SubmissionMode::Auto);
        let disp = Ok(SubmissionDisposition::Started {
            turn_id: "t1".into(),
            session_id: "s1".into(),
        });

        // 用户在等待回执期间切换了 Session (s1 -> s2) -> 保留 s2 的草稿，不得跨 Session 清空
        let outcome = CommandGateway::settle_submission_disposition(&ctx, disp, "s2", 5);
        assert!(matches!(
            outcome,
            GatewayReceiptOutcome::SubmitAcceptedRetainModifiedPrompt { .. }
        ));
    }

    #[test]
    fn test_settle_submission_rejected_retains_prompt() {
        let (_cmd, ctx) =
            CommandGateway::prepare_submit("s1".into(), 5, "Hello", SubmissionMode::Auto);
        let disp = Ok(SubmissionDisposition::Rejected {
            reason: SubmissionRejectionReason::EmptyContent,
            message: "Content cannot be empty".into(),
        });

        let outcome = CommandGateway::settle_submission_disposition(&ctx, disp, "s1", 5);
        match outcome {
            GatewayReceiptOutcome::SubmitRejected { reason, .. } => {
                assert_eq!(reason, SubmissionRejectionReason::EmptyContent);
            }
            _ => panic!("Expected SubmitRejected"),
        }
    }

    #[test]
    fn test_settle_submission_transport_failure_retains_prompt() {
        let (_cmd, ctx) =
            CommandGateway::prepare_submit("s1".into(), 5, "Hello", SubmissionMode::Auto);
        let disp = Err("Network connection timeout".into());

        let outcome = CommandGateway::settle_submission_disposition(&ctx, disp, "s1", 5);
        match outcome {
            GatewayReceiptOutcome::TransportFailed { error, .. } => {
                assert!(error.contains("timeout"));
            }
            _ => panic!("Expected TransportFailed"),
        }
    }

    #[test]
    fn test_interrupt_command_flow() {
        let (cmd, ctx) = CommandGateway::prepare_interrupt("s1".into(), "turn_active".into());
        assert_eq!(cmd.session_id, "s1");
        assert_eq!(cmd.expected_turn_id, "turn_active");
        assert_eq!(cmd.client_request_id, ctx.client_request_id);

        let disp = Ok(InterruptDisposition::Interrupted {
            turn_id: "turn_active".into(),
            session_id: "s1".into(),
        });

        let outcome = CommandGateway::settle_interrupt_disposition(&ctx, disp);
        assert_eq!(
            outcome,
            GatewayReceiptOutcome::InterruptAcknowledged {
                client_request_id: ctx.client_request_id,
                turn_id: "turn_active".into(),
            }
        );
    }
}
