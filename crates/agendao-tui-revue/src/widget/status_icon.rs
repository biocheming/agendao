//! StatusIcon — 状态图标+颜色的单一权威。
//!
//! 治理目标：消除 session.rs(◌◐●/▶✓⏳⊘↻) + sidebar.rs(○◉●) 的口径分裂。
//! 所有"ToolPhase / TodoStatus / StageUpdate status / Result"→(icon, color)
//! 只经此单点。
//!
//! 对 plan Task 5 的修正：plan 原设计 `Status::Stage(bool, bool)` 只能表达
//! Running/Done/Other 三态，会丢失 Waiting(⏳)/Blocked(⊘)/Retrying(↻)。
//! 这里改为 `StageState` 枚举，完整保留 StageUpdate 的 7 态语义，且把
//! status:String 的归一也收口到本模块的 `stage_state()`。

use revue::prelude::Color;
use crate::theme::colors;
use crate::store::types::{ToolPhase, TodoStatus};

/// StageUpdate status 字段（原为 String）的归一态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageState {
    Running,
    Done,
    Waiting,
    Cancelled,
    Blocked,
    Retrying,
    Idle,  // default / unknown
}

/// 统一状态枚举，覆盖所有需要图标的地方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Tool(ToolPhase),
    Todo(TodoStatus),
    Stage(StageState),
    ResultOk,
    ResultError,
}

/// StageUpdate 的 `status:String` → `StageState` 归一（大小写不敏感）。
pub fn stage_state(s: &str) -> StageState {
    match s.to_ascii_lowercase().as_str() {
        "running" => StageState::Running,
        "done" => StageState::Done,
        "waiting" => StageState::Waiting,
        "cancelled" | "cancelling" => StageState::Cancelled,
        "blocked" => StageState::Blocked,
        "retrying" => StageState::Retrying,
        _ => StageState::Idle,
    }
}

/// 单点：Status → (icon 字符, 颜色)。
pub fn status_icon(s: Status) -> (&'static str, Color) {
    match s {
        // ToolPhase — 统一 ◌ ◐ ●（修正 sidebar 曾用的 ○ ◉）
        Status::Tool(ToolPhase::Starting) => ("◌", colors::ACCENT_BLUE),
        Status::Tool(ToolPhase::Running)  => ("◐", colors::E_AMBER),
        Status::Tool(ToolPhase::Done)     => ("●", colors::E_TEAL),
        // Todo — ✔ ◼ ✕ ◻
        Status::Todo(TodoStatus::Completed)   => ("✔", colors::ACCENT_GREEN),
        Status::Todo(TodoStatus::InProgress)  => ("◼", colors::E_AMBER),
        Status::Todo(TodoStatus::Cancelled)   => ("✕", colors::FG_MUTED),
        Status::Todo(TodoStatus::Pending)     => ("◻", colors::FG_MUTED),
        // Stage — ▶ ✓ ⏳ ✕ ⊘ ↻ ●（完整 7 态）
        Status::Stage(StageState::Running)   => ("▶", colors::ACCENT_CYAN),
        Status::Stage(StageState::Done)      => ("✓", colors::ACCENT_GREEN),
        Status::Stage(StageState::Waiting)   => ("⏳", colors::ACCENT_YELLOW),
        Status::Stage(StageState::Cancelled) => ("✕", colors::FG_MUTED),
        Status::Stage(StageState::Blocked)   => ("⊘", colors::ACCENT_RED),
        Status::Stage(StageState::Retrying)  => ("↻", colors::ACCENT_YELLOW),
        Status::Stage(StageState::Idle)      => ("●", colors::FG_MUTED),
        // Result
        Status::ResultOk    => ("✓", colors::E_TEAL),
        Status::ResultError => ("✕", colors::ACCENT_RED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_phase_icons_consistent() {
        // 关键：session 和 sidebar 必须拿到同一组 icon
        assert_eq!(status_icon(Status::Tool(ToolPhase::Starting)).0, "◌");
        assert_eq!(status_icon(Status::Tool(ToolPhase::Running)).0, "◐");
        assert_eq!(status_icon(Status::Tool(ToolPhase::Done)).0, "●");
    }

    #[test]
    fn todo_all_variants_mapped() {
        for t in [TodoStatus::Pending, TodoStatus::InProgress,
                  TodoStatus::Completed, TodoStatus::Cancelled] {
            let _ = status_icon(Status::Todo(t)); // 不 panic
        }
    }

    #[test]
    fn result_ok_distinct_from_error() {
        assert_ne!(status_icon(Status::ResultOk).1, status_icon(Status::ResultError).1);
    }

    #[test]
    fn stage_state_normalizes_case_and_variants() {
        assert_eq!(stage_state("Running"), StageState::Running);
        assert_eq!(stage_state("cancelled"), StageState::Cancelled);
        assert_eq!(stage_state("Cancelling"), StageState::Cancelled);
        assert_eq!(stage_state("???"), StageState::Idle);
    }

    #[test]
    fn stage_all_states_have_icon() {
        for s in [StageState::Running, StageState::Done, StageState::Waiting,
                  StageState::Cancelled, StageState::Blocked, StageState::Retrying,
                  StageState::Idle] {
            let _ = status_icon(Status::Stage(s)); // 不 panic
        }
    }
}
