//! 木 — Slash command popup: / triggered command palette.
//!
//! Uses agendao_command::CommandRegistry for real slash commands
//! with fuzzy matching, keyboard navigation, and declarative Revue layout.
//!
//! U3 交互契约（2026-08 重构，行为 Breaking）：
//! - **单点权威**：过滤 query 永远派生自输入框首 token（`slash_token`），
//!   popup 不再自维一份字符缓冲——打字/Backspace/粘贴/Ctrl 全落输入框，
//!   popup 只是输入框的视图。消除"输入框显示 /s 而 popup 筛 settings"脱节。
//! - **trigger 收窄**：仅当 `/` 是首 token（前仅空白）且命令名未完成
//!   （无参数空格）时触发；行中 `/xxx`、填回后的 `/compact focus` 不触发。
//! - **Enter/Tab = 填回不执行**：选中命令写回输入框（含尾部空格）；
//!   有参命令转 ArgHint 保持打开显示参数占位，无参命令关 popup——
//!   第二次 Enter 走正常 submit 才执行（VS Code 命令面板同口径）。
//! - **Esc = 恢复原文**：open 时暂存 `/` 之前的内容，Esc 写回输入框。

use crate::theme::colors;
use agendao_command::{CommandRegistry, UiCommandArgumentKind, UiCommandSpec};
use revue::event::Key;
use revue::prelude::*;
use revue::runtime::render::Cell;
// 截断唯一实现见 backdrop(水律:消灭第二处),Home 窄输入框下防止 positioned 裁半个 CJK。
use crate::dialog::backdrop::{list_viewport_window, truncate_to_width};

#[derive(Debug, Clone)]
enum SlashCompletion {
    Ui(UiCommandSpec),
    Prompt {
        name: String,
        title: String,
        description: String,
        argument_kind: UiCommandArgumentKind,
    },
}

impl SlashCompletion {
    fn slash_name(&self) -> &str {
        match self {
            Self::Ui(command) => command
                .slash
                .as_ref()
                .map(|slash| slash.name)
                .unwrap_or(command.title),
            Self::Prompt { name, .. } => name,
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::Ui(command) => command.title,
            Self::Prompt { title, .. } => title,
        }
    }

    fn description(&self) -> &str {
        match self {
            Self::Ui(command) => command.description,
            Self::Prompt { description, .. } => description,
        }
    }

    fn category_label(&self) -> &str {
        match self {
            Self::Ui(command) => command.category.label(),
            Self::Prompt { .. } => "Commands",
        }
    }

    fn keybind(&self) -> Option<&str> {
        match self {
            Self::Ui(command) => command.keybind,
            Self::Prompt { .. } => None,
        }
    }

    fn argument_kind(&self) -> UiCommandArgumentKind {
        match self {
            Self::Ui(command) => command.argument_kind(),
            Self::Prompt { argument_kind, .. } => *argument_kind,
        }
    }

    fn is_suggested(&self) -> bool {
        matches!(self, Self::Ui(command) if command.slash.as_ref().is_some_and(|slash| slash.suggested))
    }

    fn fuzzy_score(&self, query: &str) -> Option<i32> {
        let name_score = fuzzy_match(query, self.slash_name());
        let title_score = fuzzy_match(query, self.title());
        let alias_score = match self {
            Self::Ui(command) => command
                .slash
                .as_ref()
                .into_iter()
                .flat_map(|slash| slash.aliases.iter())
                .filter_map(|alias| fuzzy_match(query, alias))
                .max(),
            Self::Prompt { .. } => None,
        };
        name_score
            .into_iter()
            .chain(alias_score)
            .chain(title_score)
            .max()
    }
}

/// Simple fuzzy match: check if all chars of `query` appear in `target` in order.
pub(crate) fn fuzzy_match(query: &str, target: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = target.to_lowercase();
    let mut qi = q.chars();
    let mut current = qi.next();
    let mut score = 0i32;
    for (i, tc) in t.chars().enumerate() {
        if let Some(qc) = current {
            if qc == tc {
                score += 100 - (i as i32).min(50);
                current = qi.next();
            }
        } else {
            break;
        }
    }
    if current.is_none() {
        Some(score)
    } else {
        None
    }
}

/// popup 两种形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashPopupMode {
    /// 命令列表补全（↑/↓ 导航，Enter/Tab 填回）。
    Completion,
    /// 填回有参命令后的参数提示条（Enter 执行，其余键全落输入框）。
    ArgHint,
}

/// `handle_key` 的返回：popup 只决定"语义"，文本改动归调用方
/// （panel_dispatch 持有 prompt，土律·单点权威）。
pub(crate) enum SlashKeyOutcome {
    /// 填回：把完整命令名（含尾部空格）写回输入框。takes_args=true 时
    /// popup 已转 ArgHint 保持打开，调用方保留 Panel::Slash。
    FillBack { command: String, takes_args: bool },
    /// ArgHint 下 Enter：调用方走正常 prompt submit 执行输入框文本。
    Submit,
    /// Esc：调用方用 `pre_slash_text` 覆盖输入框并关 panel。
    Restore,
    /// ↑/↓ 导航等已被消费，无需后续。
    Consumed,
    /// 未处理——调用方把键贯穿给 prompt 输入（单点权威的核心）。
    Pass,
}

pub struct SlashPopup {
    pub visible: bool,
    pub query: String,
    pub selected: usize,
    pub(crate) mode: SlashPopupMode,
    /// ArgHint 形态的提示文本（如 "/compact 〈text〉"）。
    arg_hint: Option<String>,
    /// Esc 恢复快照：popup 首次打开时输入框中 `/` 之前的内容
    /// （trigger 收窄后通常仅前导空白；Ctrl+P 手动打开为空串）。
    pub(crate) pre_slash_text: String,
    /// Built-in UI actions plus server-resolved Markdown commands.
    all_commands: Vec<SlashCompletion>,
    /// Filtered indices into all_commands
    filtered: Vec<usize>,
}

impl Default for SlashPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl SlashPopup {
    pub fn new() -> Self {
        Self::with_prompt_commands(Vec::new())
    }

    /// Add Markdown commands from the already merged workspace config. Their
    /// execution remains server-authoritative; this list is discovery only.
    pub fn with_prompt_commands(commands: Vec<(String, String, String)>) -> Self {
        let registry = CommandRegistry::new();
        let mut all_commands: Vec<SlashCompletion> = registry
            .ui_all_slash_commands()
            .into_iter()
            .cloned()
            .map(SlashCompletion::Ui)
            .collect();

        // Configured commands are server-merged overrides, so list them
        // before local built-ins when names collide.
        for (name, title, description) in commands {
            let name = format!("/{}", name.trim().trim_start_matches('/'));
            if name == "/"
                || all_commands
                    .iter()
                    .any(|command| command.slash_name() == name)
            {
                continue;
            }
            all_commands.push(SlashCompletion::Prompt {
                name,
                title,
                description,
                argument_kind: UiCommandArgumentKind::None,
            });
        }

        // `/goal` is a server-executed prompt command, not a TUI-only action.
        // Keep it discoverable locally even when the workspace has no custom
        // command config. Other prompt-command discovery remains unchanged.
        if let Some(command) = registry.get("goal") {
            let name = format!("/{}", command.name.trim().trim_start_matches('/'));
            if !all_commands
                .iter()
                .any(|candidate| candidate.slash_name() == name)
            {
                let argument_kind = command
                    .invocation
                    .as_ref()
                    .filter(|invocation| invocation.allow_inline_arguments)
                    .map(|_| UiCommandArgumentKind::Text)
                    .unwrap_or(UiCommandArgumentKind::None);
                all_commands.push(SlashCompletion::Prompt {
                    name,
                    title: command.name.clone(),
                    description: command.description.clone(),
                    argument_kind,
                });
            }
        }
        Self {
            visible: false,
            query: String::new(),
            selected: 0,
            mode: SlashPopupMode::Completion,
            arg_hint: None,
            pre_slash_text: String::new(),
            all_commands,
            filtered: Vec::new(),
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
        self.query.clear();
        self.mode = SlashPopupMode::Completion;
        self.arg_hint = None;
        self.pre_slash_text.clear();
        self.refresh_filter();
    }

    /// query 同步入口（单点权威）：每次输入框文本变化由调用方用
    /// `slash_token` 的派生值调这里。只改过滤，不动 pre_slash_text
    /// （快照由调用方在 closed→open 转变时写入）。
    pub fn open_with_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.mode = SlashPopupMode::Completion;
        self.arg_hint = None;
        self.refresh_filter();
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.filtered.clear();
        self.selected = 0;
        self.mode = SlashPopupMode::Completion;
        self.arg_hint = None;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    /// U3·trigger 收窄：仅当 `/` 是**首 token**（前仅空白）且命令名
    /// 尚未完成（单 token、无参数空格）时返回 `/` 后的 query。
    /// - "/mo" → Some("mo")；"/" → Some("")（suggested 列表）；
    /// - "please fix /main" → None（行中 slash 是普通文本）；
    /// - "/compact focus" / "/compact " → None（命令名已完成，
    ///   进入参数阶段——补全不再打扰，ArgHint 由 mode 保持）。
    pub fn slash_token(text: &str) -> Option<String> {
        let trimmed = text.trim_start();
        let token = trimmed.split_whitespace().next()?;
        if !token.starts_with('/') {
            return None;
        }
        // token 之后还有内容 → 命令名已完成（在敲参数），不触发补全
        if !trimmed[token.len()..].is_empty() {
            return None;
        }
        Some(token.trim_start_matches('/').to_string())
    }

    /// Number of filtered results (for sizing the popup).
    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    /// popup 渲染高度（app/mod.rs 的布局预算唯一口径）。
    pub fn display_height(&self) -> u16 {
        match self.mode {
            SlashPopupMode::Completion => self.filtered.len().min(8) as u16 + 4,
            SlashPopupMode::ArgHint => 3,
        }
    }

    /// prompt 条 hint 行文案（app/mod.rs 的 is_slash 分支唯一口径）。
    /// U20：Enter/Tab 实为「填回不执行」（U3 Breaking），二次 Enter 才运行——
    /// 旧文案 "complete" 只说了半步，补全两步语义避免用户以为选中即执行。
    pub fn hint_line(&self) -> &'static str {
        match self.mode {
            SlashPopupMode::Completion => " ↑↓:select Enter/Tab:fill → Enter:run Esc:cancel",
            SlashPopupMode::ArgHint => " Enter: run · Esc: cancel",
        }
    }

    /// 参数类型的人类可读占位（UiCommandArgumentKind → hint 文案）。
    fn arg_kind_label(kind: UiCommandArgumentKind) -> &'static str {
        match kind {
            UiCommandArgumentKind::None => "",
            UiCommandArgumentKind::Text => "text",
            UiCommandArgumentKind::SessionTarget => "session id",
            UiCommandArgumentKind::ModelRef => "provider/model",
            UiCommandArgumentKind::ThemeId => "theme id",
            UiCommandArgumentKind::ModeRef => "mode",
            UiCommandArgumentKind::AgentRef => "agent",
        }
    }

    pub(crate) fn handle_key(&mut self, key: &Key) -> SlashKeyOutcome {
        if !self.visible {
            return SlashKeyOutcome::Pass;
        }
        match self.mode {
            SlashPopupMode::Completion => match key {
                Key::Escape => {
                    self.close();
                    SlashKeyOutcome::Restore
                }
                // Enter/Tab = 填回不执行（U3 行为 Breaking：老"选中即执行"
                // 让位给带参命令可补参。第二次 Enter 走正常 submit 执行）。
                Key::Enter | Key::Tab => {
                    if let Some(idx) = self.filtered.get(self.selected) {
                        let cmd = &self.all_commands[*idx];
                        let name = cmd.slash_name().to_string();
                        let kind = cmd.argument_kind();
                        let takes_args = kind != UiCommandArgumentKind::None;
                        if takes_args {
                            self.mode = SlashPopupMode::ArgHint;
                            self.arg_hint =
                                Some(format!("{} 〈{}〉", name, Self::arg_kind_label(kind)));
                            // visible 保持——转参数提示条
                        } else {
                            self.close();
                        }
                        return SlashKeyOutcome::FillBack {
                            command: format!("{name} "),
                            takes_args,
                        };
                    }
                    SlashKeyOutcome::Consumed
                }
                Key::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    SlashKeyOutcome::Consumed
                }
                Key::Down => {
                    let max = self.filtered.len().saturating_sub(1);
                    if self.selected < max {
                        self.selected += 1;
                    }
                    SlashKeyOutcome::Consumed
                }
                // 其余键（字符/Backspace/Paste/Ctrl）全部贯穿给 prompt——
                // query 由调用方从输入框文本重新派生，popup 不碰字符。
                _ => SlashKeyOutcome::Pass,
            },
            SlashPopupMode::ArgHint => match key {
                Key::Escape => {
                    self.close();
                    SlashKeyOutcome::Restore
                }
                Key::Enter => {
                    self.close();
                    SlashKeyOutcome::Submit
                }
                // ↑/↓ 也让路（prompt 历史导航），参数阶段 popup 只是提示条。
                _ => SlashKeyOutcome::Pass,
            },
        }
    }

    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = self
                .all_commands
                .iter()
                .enumerate()
                .filter(|(_, command)| command.is_suggested())
                .map(|(i, _)| i)
                .collect();
        } else {
            let mut scored: Vec<(usize, i32)> = self
                .all_commands
                .iter()
                .enumerate()
                .filter_map(|(i, command)| command.fuzzy_score(&self.query).map(|score| (i, score)))
                .collect();
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.selected = 0;
    }

    /// Render popup — 无框实色面板(背景由调用方 [`fill_background`] 预填整片)。
    /// ❯ pointer + 分类标题 + 底部 hint。去 border:revue 边框线格经 ctx.set 覆盖,
    /// 默认 bg=None(Cell::new),边框线要么实色要么发黑;用户要"border 不要背景",
    /// 故去框,靠实色面板与下层 BG_PRIMARY 区分。每行 Text 再 .bg(BG_SURFACE) 补文字格
    /// (revue 的 Text 渲染 ctx.set 也是覆盖写,不补 bg 则文字格发黑/透字)。
    pub fn render_popup(&self, w: u16) -> impl View {
        let mut stack = vstack();
        if !self.visible {
            return stack;
        }

        // ArgHint 形态：填回有参命令后的参数占位提示条（无列表）。
        if self.mode == SlashPopupMode::ArgHint {
            let mut list = vstack().gap(0);
            let hint = self.arg_hint.clone().unwrap_or_default();
            let hint = truncate_to_width(&hint, w.saturating_sub(1).max(8) as usize);
            list = list.child(
                Text::new(format!(" {}", hint))
                    .fg(colors::ACCENT_CYAN())
                    .bg(colors::BG_SURFACE()),
            );
            list = list.child(
                Text::new(" Enter: run · Esc: cancel")
                    .fg(colors::FG_MUTED())
                    .bg(colors::BG_SURFACE()),
            );
            stack = stack.child(list);
            return stack;
        }

        // 空状态:背景由 fill_background 预填,这里只显示提示(文字格补 .bg)
        if self.filtered.is_empty() {
            stack = stack.child(
                Text::new("  No results ")
                    .fg(colors::FG_MUTED())
                    .bg(colors::BG_SURFACE()),
            );
            return stack;
        }

        // 与 app/mod.rs 的 ph = filtered_count.min(8) + 4 高度预算对齐,
        // 数据行至多 8 条,但靠 sliding viewport 滚动(窗口随 selected 跟随),
        // 而不是死 .take(8)——后者会让选到第 9 项以后视野不跟随(金律违例:
        // 截断了真实选择的输入)。窗口计算收归 backdrop::list_viewport_window
        // 唯一权威(成形语法单点)。
        let max_visible = 8usize;
        let total = self.filtered.len();
        let (start, end) = list_viewport_window(total, self.selected, max_visible);
        let rows = end - start;

        let mut list = vstack().gap(0);
        let mut last_category: Option<&str> = None;

        for (rel_idx, &cmd_idx) in self.filtered[start..end].iter().enumerate() {
            let abs_idx = start + rel_idx;
            let cmd = &self.all_commands[cmd_idx];
            let is_selected = abs_idx == self.selected;

            // 分类分隔
            let cat = cmd.category_label();
            if last_category.map(|c| c != cat).unwrap_or(true) {
                if last_category.is_some() {
                    list = list.child(Text::new("").bg(colors::BG_SURFACE()));
                }
                list = list.child(
                    Text::new(format!(" {}:", cat))
                        .fg(colors::ACCENT_BLUE())
                        .bg(colors::BG_SURFACE()),
                );
                last_category = Some(cat);
            }

            let slash_name = cmd.slash_name().trim_start_matches('/');

            // ❯ pointer + 文字色;.bg(BG_SURFACE) 补文字格,否则 ctx.set 默认 bg=None 发黑
            let pointer = if is_selected { "❯ " } else { "  " };
            let keybind_str = cmd
                .keybind()
                .map(|keybind| format!(" ({keybind})"))
                .unwrap_or_default();
            let desc = format!(
                "{} /{}{}  {}",
                pointer,
                slash_name,
                keybind_str,
                cmd.description()
            );
            // 窄宽（Home 64）截断 + …，留 1 列右边距，避免 positioned 裁半个 CJK。
            let desc = truncate_to_width(&desc, w.saturating_sub(1).max(8) as usize);

            let text = if is_selected {
                Text::new(&desc)
                    .fg(colors::ACCENT_CYAN())
                    .bg(colors::BG_SURFACE())
            } else {
                Text::new(&desc)
                    .fg(colors::FG_SECONDARY())
                    .bg(colors::BG_SURFACE())
            };
            list = list.child(text);
        }

        // 位置指示:滚动模式下显示「selected/total」,让用户感知"我在 47/100 处"
        // (backdrop dialog 把这放标题里;这里无框无标题,放底部 hint 旁)。
        // 列表 ≤ 窗口时不必显示,留出干净视觉。
        let position_hint = if total > rows {
            format!(" {}/{}", self.selected + 1, total)
        } else {
            String::new()
        };

        // 底部 hint
        list = list.child(
            Text::new(format!(
                " ↑/↓ navigate · Enter/Tab complete · Esc cancel{}",
                position_hint
            ))
            .fg(colors::FG_MUTED())
            .bg(colors::BG_SURFACE()),
        );

        stack = stack.child(list);
        stack
    }

    /// 实色填充 popup 区域,挡住下层 transcript。由调用方(app/mod.rs)在
    /// render_popup + positioned 渲染前调用。
    /// 根因——revue positioned 浮层不清背景(positioned.rs 只通过 sub_area 划区),
    /// 且内部 Stack/Text 渲染虽补了文字格 .bg,但 list 之外的浮层边缘/空白格仍透明。
    /// 故先实色预填整片,再让 render_popup 在其上绘制。守住"实色不透字"契约。
    pub fn fill_background(&self, buf: &mut Buffer, x: u16, y: u16, w: u16, h: u16) {
        buf.fill(x, y, w, h, Cell::new(' ').bg(colors::BG_SURFACE()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::colors;

    /// U20：hint 文案与 handle_key 行为同步——Completion 的 Enter/Tab 是
    /// 「填回不执行」（U3 Breaking），hint 必须说全两步（fill → run），
    /// 不能只写 "complete" 半步。
    #[test]
    fn hint_line_matches_fill_then_run_semantics() {
        let mut popup = SlashPopup::new();
        popup.open();
        assert_eq!(popup.mode, SlashPopupMode::Completion);
        let hint = popup.hint_line();
        assert!(hint.contains("fill"), "{hint}");
        assert!(hint.contains("run"), "{hint}");
        assert!(hint.contains("Esc:cancel"), "{hint}");
    }

    /// U3·trigger 收窄：仅首 token 且命令名未完成时触发。
    #[test]
    fn slash_token_narrowed_trigger() {
        assert_eq!(SlashPopup::slash_token("/mo"), Some("mo".to_string()));
        assert_eq!(SlashPopup::slash_token("/"), Some(String::new()));
        assert_eq!(SlashPopup::slash_token("  /mo"), Some("mo".to_string()));
        // 行中 slash 是普通文本
        assert_eq!(SlashPopup::slash_token("please fix /main"), None);
        // 命令名已完成（有参数空格）→ 不触发补全
        assert_eq!(SlashPopup::slash_token("/compact focus"), None);
        assert_eq!(SlashPopup::slash_token("/compact "), None);
        assert_eq!(SlashPopup::slash_token("hello"), None);
        assert_eq!(SlashPopup::slash_token(""), None);
    }

    /// U3·Enter 填回：无参命令填回后关 popup，不直接执行。
    #[test]
    fn enter_fills_back_noarg_command() {
        let mut popup = SlashPopup::new();
        popup.open_with_query("settings");
        assert!(popup.filtered_count() > 0);
        match popup.handle_key(&Key::Enter) {
            SlashKeyOutcome::FillBack {
                command,
                takes_args,
            } => {
                assert_eq!(command, "/settings ");
                assert!(!takes_args);
            }
            _ => panic!("expected FillBack"),
        }
        assert!(!popup.is_open(), "无参命令填回后 popup 应关闭");
    }

    /// U3·Enter 填回：有参命令填回后转 ArgHint 保持打开。
    #[test]
    fn enter_fills_back_arg_command_into_hint_mode() {
        let mut popup = SlashPopup::new();
        popup.open_with_query("compact");
        match popup.handle_key(&Key::Enter) {
            SlashKeyOutcome::FillBack {
                command,
                takes_args,
            } => {
                assert_eq!(command, "/compact ");
                assert!(takes_args);
            }
            _ => panic!("expected FillBack"),
        }
        assert!(popup.is_open(), "有参命令填回后 popup 保持打开");
        assert_eq!(popup.mode, SlashPopupMode::ArgHint);
        // ArgHint 下 Enter → Submit；字符 → Pass（贯穿输入框）
        match popup.handle_key(&Key::Char('f')) {
            SlashKeyOutcome::Pass => {}
            _ => panic!("ArgHint 字符键应 Pass"),
        }
        match popup.handle_key(&Key::Enter) {
            SlashKeyOutcome::Submit => {}
            _ => panic!("ArgHint Enter 应 Submit"),
        }
        assert!(!popup.is_open());
    }

    #[test]
    fn configured_prompt_command_is_discoverable_and_fills_back() {
        let mut popup = SlashPopup::with_prompt_commands(vec![(
            "global-only".to_string(),
            "Global only".to_string(),
            "Inherited Markdown command".to_string(),
        )]);

        popup.open_with_query("global-only");
        assert_eq!(popup.filtered_count(), 1);
        match popup.handle_key(&Key::Enter) {
            SlashKeyOutcome::FillBack {
                command,
                takes_args,
            } => {
                assert_eq!(command, "/global-only ");
                assert!(!takes_args);
            }
            _ => panic!("expected configured command fill-back"),
        }
    }

    #[test]
    fn builtin_goal_is_discoverable_and_requests_plain_text() {
        let mut popup = SlashPopup::new();

        popup.open_with_query("goal");
        assert_eq!(popup.filtered_count(), 1);
        match popup.handle_key(&Key::Enter) {
            SlashKeyOutcome::FillBack {
                command,
                takes_args,
            } => {
                assert_eq!(command, "/goal ");
                assert!(takes_args);
            }
            _ => panic!("expected goal command fill-back"),
        }
    }

    /// U3·Esc → Restore（调用方据 pre_slash_text 恢复输入框）。
    #[test]
    fn esc_yields_restore() {
        let mut popup = SlashPopup::new();
        popup.open_with_query("mo");
        match popup.handle_key(&Key::Escape) {
            SlashKeyOutcome::Restore => {}
            _ => panic!("Esc 应 Restore"),
        }
        assert!(!popup.is_open());
    }

    /// U3·Completion 下字符/Backspace 全部 Pass（popup 不碰字符）。
    #[test]
    fn completion_passes_typing_keys_through() {
        let mut popup = SlashPopup::new();
        popup.open_with_query("m");
        for key in [Key::Char('o'), Key::Backspace, Key::Char(' ')] {
            match popup.handle_key(&key) {
                SlashKeyOutcome::Pass => {}
                _ => panic!("{:?} 应 Pass 贯穿输入框", key),
            }
        }
        // query 不被 popup 改动（单点权威在输入框）
        assert_eq!(popup.query, "m");
    }

    /// fill_background 必须把指定矩形填成 BG_SURFACE 实色,且不污染区域外。
    /// 守住"实色不透字"契约——positioned 浮层不清背景,全靠这一步预填整片。
    #[test]
    fn fill_background_fills_region_solid() {
        let popup = SlashPopup::new();
        let mut buf = Buffer::new(20, 10);
        popup.fill_background(&mut buf, 2, 2, 10, 5);
        // 区域内实色
        assert_eq!(buf.get(5, 4).and_then(|c| c.bg), Some(colors::BG_SURFACE()));
        assert_eq!(
            buf.get(10, 6).and_then(|c| c.bg),
            Some(colors::BG_SURFACE())
        );
        // 区域外未被填充,保持 None
        assert_eq!(buf.get(0, 0).and_then(|c| c.bg), None);
        assert_eq!(buf.get(19, 9).and_then(|c| c.bg), None);
    }

    /// 完整流程:fill_background 预填后 render_popup 绘制,内部仍保持实色
    /// (每行 Text 已 .bg(BG_SURFACE) 补文字格)。守住"渲染后不透字"。
    #[test]
    fn render_popup_keeps_solid_after_fill() {
        let mut popup = SlashPopup::new();
        popup.open();
        let view = popup.render_popup(60);
        let mut buf = Buffer::new(60, 20);
        popup.fill_background(&mut buf, 0, 0, 60, 20);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 20));
        view.render(&mut ctx);
        let mut has_solid = false;
        for x in (1u16..59).step_by(5) {
            if has_solid {
                break;
            }
            for y in (1u16..19).step_by(2) {
                if buf.get(x, y).and_then(|c| c.bg) == Some(colors::BG_SURFACE()) {
                    has_solid = true;
                    break;
                }
            }
        }
        assert!(has_solid, "popup 渲染后内部必须保持实色,否则透字");
    }

    /// 空状态分支(无匹配命令)同样要实色背景,不透字。
    #[test]
    fn render_popup_empty_state_keeps_solid() {
        let mut popup = SlashPopup::new();
        popup.open();
        popup.query = "zzz_no_match".to_string();
        popup.refresh_filter();
        assert_eq!(popup.filtered_count(), 0);
        let view = popup.render_popup(60);
        let mut buf = Buffer::new(60, 6);
        popup.fill_background(&mut buf, 0, 0, 60, 6);
        let mut ctx = RenderContext::new(&mut buf, Rect::new(0, 0, 60, 6));
        view.render(&mut ctx);
        let mut has_solid = false;
        for x in (1u16..59).step_by(5) {
            if has_solid {
                break;
            }
            for y in (1u16..5).step_by(2) {
                if buf.get(x, y).and_then(|c| c.bg) == Some(colors::BG_SURFACE()) {
                    has_solid = true;
                    break;
                }
            }
        }
        assert!(has_solid, "空状态 popup 也要实色背景");
    }
}
