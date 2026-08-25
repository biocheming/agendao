//! # M4.0 Interaction Contract & Decision Table
//!
//! 纯函数交互决策系统：
//! - 严格不修改 Prompt，不修改 Projection，不调用 API，不生成状态，不弹 Toast。
//! - 相同的 (InteractionInput, InteractionContext) 永远产出唯一的 InteractionDecision。
//! - 复用 `agendao_types::submission::SubmissionMode` 契约，避免双重命名。

use agendao_types::submission::SubmissionMode;
use serde::{Deserialize, Serialize};

pub type TurnId = String;

/// 终端键盘能力特性（用于区分组合键是否独立可识别）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardCapabilities {
    pub supports_ctrl_enter: bool,
    pub supports_shift_enter: bool,
    pub kitty_keyboard_protocol: bool,
}

impl Default for KeyboardCapabilities {
    fn default() -> Self {
        Self {
            supports_ctrl_enter: false,
            supports_shift_enter: false,
            kitty_keyboard_protocol: false,
        }
    }
}

/// 执行阶段
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionPhase {
    Idle,
    ExecutingTool { tool_name: String },
    StreamingModelResponse,
    AwaitingApproval,
    Paused,
}

/// UI 焦点目标
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusTarget {
    PromptComposer,
    Transcript,
    Sidebar,
    ModalOrDialog,
    Settings,
}

/// 交互上下文（只读环境事实）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionContext {
    pub focus: FocusTarget,
    pub panel_open: bool,
    pub composer_empty: bool,
    pub active_turn_id: Option<TurnId>,
    pub execution_phase: Option<ExecutionPhase>,
    pub keyboard: KeyboardCapabilities,
}

/// 逻辑交互动作输入（非物理键盘码，避免绑定具体终端按键）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionInput {
    DefaultSubmit,
    ExplicitQueue,
    ExplicitSteer,
    Interrupt,
    InsertNewline,
}

/// 提交意图
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitIntent {
    pub mode: SubmissionMode,
    pub content: String,
}

/// 打断意图（不携带文本，直接针对目标 Turn）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptIntent {
    pub expected_turn_id: TurnId,
}

/// 上层交互意图
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionIntent {
    Submit(SubmitIntent),
    Interrupt(InterruptIntent),
    InsertNewline,
}

/// 本地确定性拒绝原因（无需网络请求）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalRejection {
    EmptyContent,
    NoActiveTurnToSteer,
    NoActiveTurnToInterrupt,
    NonPromptFocusIgnored,
    ModalOrPanelIntercepted,
}

/// 交互决策结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionDecision {
    Intent(InteractionIntent),
    Unhandled,
    RejectedLocally { reason: LocalRejection },
}

/// 核心决策纯函数：
/// 1. 焦点或弹窗打开时，快捷键不得泄漏到 Prompt；
/// 2. 文本提交类意图（DefaultSubmit, ExplicitQueue, ExplicitSteer）在 Prompt 为空时本地拒绝；
/// 3. DefaultSubmit 统一为 Submit(Auto)，由服务端权威做 Started / Queued 原子分流；
/// 4. ExplicitSteer / Interrupt 依赖活跃 TurnId，无活跃 Turn 时本地拒绝；
/// 5. InsertNewline 产出 InsertNewline 意图。
pub fn decide_interaction(
    input: InteractionInput,
    content: &str,
    context: &InteractionContext,
) -> InteractionDecision {
    // 1. 弹窗打开或焦点不在 Prompt 输入框时，不触发 Prompt 意图
    if context.panel_open {
        return InteractionDecision::RejectedLocally {
            reason: LocalRejection::ModalOrPanelIntercepted,
        };
    }
    if context.focus != FocusTarget::PromptComposer {
        return InteractionDecision::RejectedLocally {
            reason: LocalRejection::NonPromptFocusIgnored,
        };
    }

    match input {
        InteractionInput::InsertNewline => {
            InteractionDecision::Intent(InteractionIntent::InsertNewline)
        }
        InteractionInput::DefaultSubmit => {
            if context.composer_empty || content.trim().is_empty() {
                return InteractionDecision::RejectedLocally {
                    reason: LocalRejection::EmptyContent,
                };
            }
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Auto,
                content: content.to_string(),
            }))
        }
        InteractionInput::ExplicitQueue => {
            if context.composer_empty || content.trim().is_empty() {
                return InteractionDecision::RejectedLocally {
                    reason: LocalRejection::EmptyContent,
                };
            }
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Queue,
                content: content.to_string(),
            }))
        }
        InteractionInput::ExplicitSteer => {
            if context.composer_empty || content.trim().is_empty() {
                return InteractionDecision::RejectedLocally {
                    reason: LocalRejection::EmptyContent,
                };
            }
            match &context.active_turn_id {
                Some(turn_id) => {
                    InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                        mode: SubmissionMode::Steer {
                            expected_turn_id: turn_id.clone(),
                        },
                        content: content.to_string(),
                    }))
                }
                None => InteractionDecision::RejectedLocally {
                    reason: LocalRejection::NoActiveTurnToSteer,
                },
            }
        }
        InteractionInput::Interrupt => match &context.active_turn_id {
            Some(turn_id) => {
                InteractionDecision::Intent(InteractionIntent::Interrupt(InterruptIntent {
                    expected_turn_id: turn_id.clone(),
                }))
            }
            None => InteractionDecision::RejectedLocally {
                reason: LocalRejection::NoActiveTurnToInterrupt,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_context() -> InteractionContext {
        InteractionContext {
            focus: FocusTarget::PromptComposer,
            panel_open: false,
            composer_empty: false,
            active_turn_id: None,
            execution_phase: Some(ExecutionPhase::Idle),
            keyboard: KeyboardCapabilities::default(),
        }
    }

    #[test]
    fn test_default_submit_when_idle_produces_auto_mode() {
        let ctx = base_context();
        let decision = decide_interaction(InteractionInput::DefaultSubmit, "Hello", &ctx);
        assert_eq!(
            decision,
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Auto,
                content: "Hello".into(),
            }))
        );
    }

    #[test]
    fn test_default_submit_when_busy_still_produces_auto_mode() {
        let mut ctx = base_context();
        ctx.active_turn_id = Some("turn_123".into());
        ctx.execution_phase = Some(ExecutionPhase::StreamingModelResponse);

        // 核心规范：TUI 绝对不因本地 Busy 猜测转成 Queue，统一提交 Auto 给服务端权威原子分流
        let decision = decide_interaction(InteractionInput::DefaultSubmit, "Steer or queue", &ctx);
        assert_eq!(
            decision,
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Auto,
                content: "Steer or queue".into(),
            }))
        );
    }

    #[test]
    fn test_explicit_queue_produces_queue_mode_regardless_of_turn() {
        let ctx = base_context();
        let decision = decide_interaction(InteractionInput::ExplicitQueue, "Queued msg", &ctx);
        assert_eq!(
            decision,
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Queue,
                content: "Queued msg".into(),
            }))
        );
    }

    #[test]
    fn test_explicit_steer_requires_active_turn() {
        let mut ctx = base_context();
        ctx.active_turn_id = None;

        // 无活跃 Turn 时本地拒绝
        let err_decision = decide_interaction(InteractionInput::ExplicitSteer, "Steer msg", &ctx);
        assert_eq!(
            err_decision,
            InteractionDecision::RejectedLocally {
                reason: LocalRejection::NoActiveTurnToSteer
            }
        );

        // 有活跃 Turn 时带上 expected_turn_id
        ctx.active_turn_id = Some("turn_active".into());
        let ok_decision = decide_interaction(InteractionInput::ExplicitSteer, "Steer msg", &ctx);
        assert_eq!(
            ok_decision,
            InteractionDecision::Intent(InteractionIntent::Submit(SubmitIntent {
                mode: SubmissionMode::Steer {
                    expected_turn_id: "turn_active".into()
                },
                content: "Steer msg".into(),
            }))
        );
    }

    #[test]
    fn test_interrupt_requires_active_turn_and_no_content() {
        let mut ctx = base_context();
        ctx.active_turn_id = None;

        let err_decision = decide_interaction(InteractionInput::Interrupt, "", &ctx);
        assert_eq!(
            err_decision,
            InteractionDecision::RejectedLocally {
                reason: LocalRejection::NoActiveTurnToInterrupt
            }
        );

        ctx.active_turn_id = Some("turn_running".into());
        let ok_decision = decide_interaction(InteractionInput::Interrupt, "", &ctx);
        assert_eq!(
            ok_decision,
            InteractionDecision::Intent(InteractionIntent::Interrupt(InterruptIntent {
                expected_turn_id: "turn_running".into(),
            }))
        );
    }

    #[test]
    fn test_empty_content_rejected_for_submits() {
        let ctx = base_context();
        for input in [
            InteractionInput::DefaultSubmit,
            InteractionInput::ExplicitQueue,
            InteractionInput::ExplicitSteer,
        ] {
            let res = decide_interaction(input, "   ", &ctx);
            assert_eq!(
                res,
                InteractionDecision::RejectedLocally {
                    reason: LocalRejection::EmptyContent
                }
            );
        }
    }

    #[test]
    fn test_focus_and_panel_protection() {
        let mut ctx = base_context();
        ctx.panel_open = true;

        let res = decide_interaction(InteractionInput::DefaultSubmit, "Hello", &ctx);
        assert_eq!(
            res,
            InteractionDecision::RejectedLocally {
                reason: LocalRejection::ModalOrPanelIntercepted
            }
        );

        ctx.panel_open = false;
        ctx.focus = FocusTarget::ModalOrDialog;
        let res2 = decide_interaction(InteractionInput::DefaultSubmit, "Hello", &ctx);
        assert_eq!(
            res2,
            InteractionDecision::RejectedLocally {
                reason: LocalRejection::NonPromptFocusIgnored
            }
        );
    }

    #[test]
    fn test_newline_always_allowed_when_focused() {
        let ctx = base_context();
        let res = decide_interaction(InteractionInput::InsertNewline, "", &ctx);
        assert_eq!(
            res,
            InteractionDecision::Intent(InteractionIntent::InsertNewline)
        );
    }
}
