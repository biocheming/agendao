//! 木 — PromptInput: single authority for all user text input.
//!
//! Multi-line composer: editing/rendering is delegated to the
//! [`WrapEditor`] widget (editing authority: revue `TextArea` — cursor,
//! selection, word navigation; rendering: `EditorView` with the ❯ arrow
//! prefix, continuation-line indent, adaptive height capped at
//! `MAX_VISIBLE_LINES` with a scrollbar past that, and a blinking block
//! cursor driven by the app-level blink tick). This layer keeps the
//! prompt semantics: history/draft, snapshot undo/redo stacks,
//! InputMode(Shell), submit semantics, placeholder, status_hint and
//! paste CRLF normalization.
//!
//! Key contract: `Enter` submits; `Shift+Enter` / `Ctrl+Enter` insert a
//! newline (routed by `app::keymap` via [`PromptInput::insert_newline`]
//! before the bare-Enter path, so the two never collide).

use revue::event::Key;

use crate::widget::wrap_editor::{EditorView, VisualMove, WrapEditor, PROMPT_INDENT};

#[derive(Clone, Debug)]
pub enum PromptAction {
    None,
    Consumed,
    Submit(String),
    SubmitShell(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode {
    Normal,
    Shell,
}

pub struct PromptInput {
    /// 编辑控件（编辑权威 + 渲染 + 命中几何回流）。聚焦闸门由自有
    /// `focused` 字段承担，editor 只负责编辑语义。
    editor: WrapEditor,
    mode: InputMode,
    focused: bool,
    history: Vec<String>,
    history_idx: Option<usize>,
    draft: Option<String>,
    normal_placeholders: Vec<String>,
    shell_placeholders: Vec<String>,
    /// Optional path for persisting history to disk.
    history_path: Option<std::path::PathBuf>,
    /// 当前 placeholder（mode/随机选择后固定，避免每帧重建 editor）。
    placeholder: String,
    /// 快照 undo/redo 栈（内容 + 光标）。自有权威——revue TextArea::undo
    /// 有 CJK char/byte 混算缺陷，而 revue 是第三方库不可改（土律·边界），
    /// 故 prompt 的 undo 语义由本层以快照持有（每次变更一记，粒度可预期）。
    undo_stack: Vec<(String, (usize, usize))>,
    redo_stack: Vec<(String, (usize, usize))>,
}

/// undo 栈上限（防长会话内存无界；到达上限丢最老快照）。
const UNDO_STACK_CAP: usize = 128;

fn default_history_path() -> std::path::PathBuf {
    // 输入历史统一收在 agendao_home（~/.agendao,土律·单点权威）。
    agendao_util::agendao_home().join("prompt-history.json")
}

fn load_history(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_history(path: &std::path::Path, history: &[String]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(history) {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
            {
                let _ = f.write_all(json.as_bytes());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::write(path, &json);
        }
    }
}

impl Default for PromptInput {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptInput {
    pub fn new() -> Self {
        Self {
            editor: WrapEditor::new(),
            mode: InputMode::Normal,
            focused: false,
            history: Vec::new(),
            history_idx: None,
            draft: None,
            normal_placeholders: vec!["Ask anything...".into()],
            shell_placeholders: vec!["Run a command...".into()],
            history_path: None,
            placeholder: "Ask anything...".into(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Load history from default path.
    pub fn with_persistence(mut self) -> Self {
        let path = default_history_path();
        self.history = load_history(&path);
        self.history_path = Some(path);
        self
    }

    pub fn with_placeholders(mut self, normal: &[&str], shell: &[&str]) -> Self {
        self.normal_placeholders = normal.iter().map(|s| s.to_string()).collect();
        self.shell_placeholders = shell.iter().map(|s| s.to_string()).collect();
        // Pick a random one
        let idx = (std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as usize)
            % self.normal_placeholders.len();
        self.placeholder = self.normal_placeholders[idx].clone();
        self
    }

    /// 换 placeholder 并清空内容（mode 切换 / submit 后复位共用）。
    fn reset_editor(&mut self, placeholder: &str) {
        self.editor.set_content("");
        self.placeholder = placeholder.to_string();
        // 提交/模式切换是语义边界：旧草稿的 undo 历史一并作废。
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// 变更前快照（自有 undo 权威）。与栈顶去重、清空 redo、封顶
    /// UNDO_STACK_CAP——所有变更入口（键入/删除/换行/粘贴/kill/历史导航/
    /// set_text）统一经此一记（土律·单点口径）。
    fn snapshot_undo(&mut self) {
        let snap = self.editor.snapshot();
        push_undo_snapshot(&mut self.undo_stack, &mut self.redo_stack, snap);
    }

    fn undo(&mut self) -> bool {
        let Some(snap) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.editor.snapshot());
        self.editor.restore(&snap);
        true
    }

    fn redo(&mut self) -> bool {
        let Some(snap) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.editor.snapshot());
        self.editor.restore(&snap);
        true
    }

    fn normal_placeholder(&self) -> &str {
        self.normal_placeholders
            .first()
            .map(|s| s.as_str())
            .unwrap_or("Ask anything...")
    }

    fn shell_placeholder(&self) -> &str {
        self.shell_placeholders
            .first()
            .map(|s| s.as_str())
            .unwrap_or("Run a command...")
    }

    pub fn handle_key(&mut self, key: &Key) -> PromptAction {
        // Shell mode toggle / U26① `!!` 转义。
        if let Key::Char('!') = key {
            if self.editor.text().trim().is_empty() {
                match self.mode {
                    InputMode::Normal => {
                        self.mode = InputMode::Shell;
                        self.focused = true;
                        let ph = self.shell_placeholder().to_string();
                        self.reset_editor(&ph);
                        return PromptAction::None;
                    }
                    InputMode::Shell => {
                        // 第二个 `!` = 字面 `!` 退回 Normal——"!important"
                        // 这类普通消息原本无法发出（首 `!` 被模式切换吞掉，
                        // `!` 永远无法作消息首字符）。Esc 仍是纯退出。
                        self.mode = InputMode::Normal;
                        self.focused = true;
                        let ph = self.normal_placeholder().to_string();
                        self.reset_editor(&ph);
                        self.set_text("!");
                        return PromptAction::Consumed;
                    }
                }
            }
        }
        if matches!(key, Key::Escape) && self.mode == InputMode::Shell {
            self.mode = InputMode::Normal;
            self.focused = false;
            let ph = self.normal_placeholder().to_string();
            self.reset_editor(&ph);
            return PromptAction::None;
        }

        match key {
            Key::Enter => {
                let text = self.editor.text().trim().to_string();
                if !text.is_empty() {
                    self.history.push(text.clone());
                    if let Some(ref path) = self.history_path {
                        save_history(path, &self.history);
                    }
                    self.history_idx = None;
                    self.draft = None;
                    self.focused = false;
                    if self.mode == InputMode::Shell {
                        self.mode = InputMode::Normal;
                        let ph = self.normal_placeholder().to_string();
                        self.reset_editor(&ph);
                        return PromptAction::SubmitShell(text);
                    }
                    let ph = self.placeholder.clone();
                    self.reset_editor(&ph);
                    return PromptAction::Submit(text);
                }
                PromptAction::None
            }
            // Up/Down：视觉行化（折行布局口径）——光标在折行中段先回
            // 上一视觉行；到首/末视觉行（AtTop/AtBottom）才进历史。
            Key::Up => {
                if self.editor.move_visual_up() == VisualMove::AtTop {
                    self.history_up();
                } else {
                    self.focused = true;
                }
                PromptAction::Consumed
            }
            Key::Down => {
                if self.editor.move_visual_down() == VisualMove::AtBottom {
                    self.history_down();
                } else {
                    self.focused = true;
                }
                PromptAction::Consumed
            }
            // Tab 不插入（对齐旧单行行为：Tab 归 keymap 的 transcript 导航）。
            Key::Tab => PromptAction::None,
            _ => {
                self.focused = true;
                // 仅变更类按键快照（导航键不进 undo 历史）。
                if matches!(key, Key::Char(_) | Key::Backspace | Key::Delete) {
                    self.snapshot_undo();
                }
                let changed = self.editor.handle_key(key);
                if changed {
                    PromptAction::Consumed
                } else {
                    PromptAction::None
                }
            }
        }
    }

    /// Shift+Enter / Ctrl+Enter 换行（Enter 发送语义不变——keymap 单点路由）。
    pub fn insert_newline(&mut self) {
        self.focused = true;
        self.snapshot_undo();
        self.editor.insert_newline();
    }

    /// Ctrl 组合键（readline 集，U2）：^Z/^Y 走本层快照栈（自有 undo
    /// 权威）；其余 chord（A/E/W/U/K、词跳）委托 [`WrapEditor`]，kill
    /// 类变更前经回调回记快照（口径同 snapshot_undo）。
    /// 返回 true=已消费；未绑定 chord 返回 false（调用方吞掉，
    /// 绝不剥修饰键退化成插入字面字母）。
    pub fn handle_ctrl_key(&mut self, event: &revue::event::KeyEvent) -> bool {
        use revue::event::Key as K;
        self.focused = true;
        match event.key {
            K::Char('z') => {
                self.undo();
            }
            K::Char('y') => {
                self.redo();
            }
            _ => {
                let undo_stack = &mut self.undo_stack;
                let redo_stack = &mut self.redo_stack;
                return self.editor.handle_ctrl_key(event, |ed| {
                    push_undo_snapshot(undo_stack, redo_stack, ed.snapshot());
                });
            }
        }
        true
    }

    /// 粘贴（U1）：bracketed paste 文本原样进编辑器（多行保留）。
    /// \r\n / \r 归一为 \n（Windows 剪贴板口径）。
    pub fn paste(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        self.focused = true;
        self.snapshot_undo();
        self.editor.insert_str(&normalized);
    }

    fn history_up(&mut self) -> PromptAction {
        if self.history.is_empty() {
            return PromptAction::None;
        }
        if self.history_idx.is_none() {
            self.draft = Some(self.editor.text());
            self.history_idx = Some(self.history.len().saturating_sub(1));
        } else if let Some(idx) = self.history_idx {
            if idx > 0 {
                self.history_idx = Some(idx - 1);
            }
        }
        if let Some(idx) = self.history_idx {
            if let Some(entry) = self.history.get(idx).cloned() {
                self.snapshot_undo();
                self.editor.set_content(&entry);
                self.editor.move_document_end();
            }
        }
        PromptAction::None
    }

    fn history_down(&mut self) -> PromptAction {
        if self.history_idx.is_none() {
            return PromptAction::None;
        }
        if let Some(idx) = self.history_idx {
            if idx + 1 < self.history.len() {
                self.history_idx = Some(idx + 1);
                if let Some(entry) = self.history.get(idx + 1).cloned() {
                    self.snapshot_undo();
                    self.editor.set_content(&entry);
                    self.editor.move_document_end();
                }
            } else {
                self.history_idx = None;
                let draft = self.draft.take().unwrap_or_default();
                self.snapshot_undo();
                self.editor.set_content(&draft);
                self.editor.move_document_end();
            }
        }
        PromptAction::None
    }

    pub fn text(&self) -> String {
        self.editor.text()
    }

    /// Replace the input text wholesale (e.g. restoring a stashed draft).
    /// 喂回输入框权威 —— 水生木闭环（stash 恢复项回灌下一轮输入）。
    pub fn set_text(&mut self, text: &str) {
        self.snapshot_undo();
        self.editor.set_content(text);
        self.editor.move_document_end();
    }
    pub fn clear(&mut self) {
        self.editor.set_content("");
        self.focused = false;
        // 清空是语义边界（同 reset_editor）：undo 历史一并作废。
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Focus the input — shows the block cursor. Used when entering a route
    /// that is "ready to type" (e.g. Home), so the cursor is visible on entry
    /// rather than only after the first keystroke/click.
    pub fn focus(&mut self) {
        self.focused = true;
    }

    /// 布局期高度：内容按 `content_w`（含 ❯ 缩进的整宽）soft-wrap
    /// 折行后的可见行数（封顶 MAX_VISIBLE_LINES，超出出滚动条）。
    /// 宽度权威在 app 层 prompt_geometry（Home 居中宽 / Session 主区-PAD）。
    pub fn visible_height_for(&self, content_w: u16) -> u16 {
        self.editor
            .wrapped_height(content_w.saturating_sub(PROMPT_INDENT))
    }

    /// Handle a mouse click at (x, y) — absolute screen coords.
    /// 命中区来自 render 发布的真实几何（替代旧 y>=35 硬编码）：
    /// 命中 → 聚焦并把光标定位到点击的字符位置；未命中 → 失焦。
    pub fn handle_click(&mut self, x: u16, y: u16) -> bool {
        let hit = self.editor.handle_click(x, y);
        self.focused = hit;
        hit
    }
    pub fn mode(&self) -> &InputMode {
        &self.mode
    }

    /// Show status hint above the prompt bar.
    /// U20：只宣传真实可用的键（与 keymap/handle_ctrl_key 双向核对）——
    /// Enter 发送；Alt/Shift/Ctrl+Enter 换行（keymap 1185 三修饰同闸）；
    /// ↑/↓ 到顶/底行才进历史（多行内是行间移动，不宣传为纯历史键）；
    /// ^Z/^Y undo/redo（U2 readline 集）；^P 命令面板（keymap 全局）。
    pub fn status_hint(&self, is_running: bool) -> String {
        if is_running {
            return "Running... Esc: stop".into();
        }
        // U26①：shell 模式有自己的宣传口径（Esc 退出 / !! 字面转义）——
        // 不宣传则用户不知道为何 Enter 变成"运行命令"。
        if self.mode == InputMode::Shell {
            return "Shell mode | Enter:run | Esc:normal mode | !!: literal !".into();
        }
        let len = self.editor.text().trim().len();
        if self.focused && len > 0 {
            // 宣传 Alt 放首位：Shift/Ctrl+Enter 在无 kitty 键盘协议的终端
            // 是死键（与 keymap 三修饰同闸不矛盾——只宣传最可靠的一个）。
            format!(
                "{} chars | Enter:send Alt+Enter:newline | ^Z/^Y:undo ^P:commands",
                len
            )
        } else if self.focused {
            "Type or /command | Enter:send | ↑/↓:history ^P:commands ?:help".into()
        } else {
            "Click below to type, or just start typing...".into()
        }
    }

    /// Snapshot a renderable view of the composer.
    /// `cursor_on` = 闪烁相（app 层 blink tick 推导）&& 希望画光标。
    pub fn view(&self, cursor_on: bool) -> EditorView {
        self.editor.view(
            self.focused && cursor_on,
            self.focused,
            self.placeholder.clone(),
        )
    }
}

/// 快照入栈口径单点：与栈顶去重、封顶 UNDO_STACK_CAP、清空 redo。
fn push_undo_snapshot(
    undo_stack: &mut Vec<(String, (usize, usize))>,
    redo_stack: &mut Vec<(String, (usize, usize))>,
    snap: (String, (usize, usize)),
) {
    if undo_stack.last() == Some(&snap) {
        return;
    }
    undo_stack.push(snap);
    if undo_stack.len() > UNDO_STACK_CAP {
        undo_stack.remove(0);
    }
    redo_stack.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::wrap_editor::MAX_VISIBLE_LINES;

    /// U26①：`!!` 转义——首 `!` 进 Shell 模式，第二个 `!` 退回 Normal
    /// 并留下字面 `!`（"!important" 这类普通消息可发）；Shell 模式
    /// hint 宣传 Esc 退出与 !! 转义。
    #[test]
    fn bang_bang_escapes_to_literal_normal_mode() {
        let mut p = PromptInput::new();
        assert!(matches!(p.handle_key(&Key::Char('!')), PromptAction::None));
        assert!(matches!(p.mode(), InputMode::Shell), "首 ! 进 Shell 模式");
        // Shell 模式 hint 宣传口径。
        let hint = p.status_hint(false);
        assert!(hint.contains("Shell mode"), "{hint}");
        assert!(hint.contains("Esc:normal mode"), "{hint}");
        assert!(hint.contains("!!: literal !"), "{hint}");
        // 第二个 `!`：退回 Normal + 字面 `!`。
        assert!(matches!(
            p.handle_key(&Key::Char('!')),
            PromptAction::Consumed
        ));
        assert!(matches!(p.mode(), InputMode::Normal), "!! 退回 Normal");
        assert_eq!(p.text(), "!", "留下字面 ! 作首字符");
        // 继续输入成普通消息，Enter 走 Submit 而非 SubmitShell。
        p.handle_key(&Key::Char('i'));
        match p.handle_key(&Key::Enter) {
            PromptAction::Submit(t) => assert_eq!(t, "!i"),
            other => panic!("!! 转义后 Enter 应是普通 Submit，实际 {other:?}"),
        }
    }

    /// U20：hint 只宣传真实可用的键（正向半：宣传→实现；反向半由 keymap
    /// 侧已存的绑键测试兜住——^P palette / q quit / ? help / Esc interrupt /
    /// 每个 transcript 绑键均有独立测试）。
    #[test]
    fn status_hint_advertises_only_real_keys() {
        let mut p = PromptInput::new();
        // 运行态：Esc stop（keymap interrupt 真实存在）。
        assert!(p.status_hint(true).contains("Esc: stop"));
        // 空 prompt + focused：历史/面板/help 入口。
        p.focused = true;
        let empty_hint = p.status_hint(false);
        assert!(empty_hint.contains("↑/↓:history"), "{empty_hint}");
        assert!(empty_hint.contains("^P:commands"), "{empty_hint}");
        assert!(empty_hint.contains("?:help"), "{empty_hint}");
        // 有文本：发送/换行/undo。
        p.set_text("hello");
        let text_hint = p.status_hint(false);
        assert!(text_hint.contains("Enter:send"), "{text_hint}");
        assert!(text_hint.contains("Alt+Enter:newline"), "{text_hint}");
        assert!(text_hint.contains("^Z/^Y:undo"), "{text_hint}");
    }

    #[test]
    fn multiline_content_adapts_height_capped_at_max() {
        let mut p = PromptInput::new();
        assert_eq!(p.visible_height_for(80), 1);
        p.set_text("a\nb\nc");
        assert_eq!(p.visible_height_for(80), 3);
        // 长逻辑行按宽折行也撑高：24 列内容、文本宽 10 → 3 视觉行。
        p.set_text(&"a".repeat(24));
        assert_eq!(
            p.visible_height_for(12),
            3,
            "整宽 12 - ❯ 缩进 2 = 文本宽 10"
        );
        let long = (0..20)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        p.set_text(&long);
        assert_eq!(p.visible_height_for(80), MAX_VISIBLE_LINES);
    }

    #[test]
    fn insert_newline_keeps_enter_submit_semantics() {
        let mut p = PromptInput::new();
        p.set_text("hello");
        p.insert_newline();
        p.handle_key(&Key::Char('!'));
        // '!' 非空文本 → 不切 shell，落进编辑器
        assert_eq!(p.text(), "hello\n!");
        match p.handle_key(&Key::Enter) {
            PromptAction::Submit(t) => assert_eq!(t, "hello\n!"),
            other => panic!("expected Submit, got {:?}", other),
        }
        assert_eq!(p.text(), "");
    }

    #[test]
    fn up_down_move_within_multiline_before_history() {
        let mut p = PromptInput::new();
        p.set_text("l1\nl2");
        // cursor 在末行（set_text → move_document_end）
        assert_eq!(p.editor.cursor_position().0, 1);
        p.handle_key(&Key::Up);
        assert_eq!(p.editor.cursor_position().0, 0, "Up 在多行内先行间移动");
        p.handle_key(&Key::Down);
        assert_eq!(p.editor.cursor_position().0, 1);
    }

    /// 折行历史闸：光标在折行中段按 Up 必须先回上一视觉行，不得进
    /// 历史；到首视觉行（AtTop）才进。Down 对称（末视觉行才出草稿）。
    #[test]
    fn up_in_wrapped_middle_moves_visual_row_before_history() {
        let mut p = PromptInput::new();
        // 造一条历史。
        p.set_text("history-entry");
        p.handle_key(&Key::Enter);
        // 长逻辑行折成 3 视觉行（文本宽 12-2=10 → 24 个 a 硬折 10/10/4）。
        let long = "a".repeat(24);
        p.set_text(&long);
        // 布局期 prime 折行缓存（render 前 Up/Down 同源口径）。
        assert_eq!(p.visible_height_for(12), 3);
        // 光标在文档末（视觉行 2）：Up → 回视觉行 1，不进历史。
        p.handle_key(&Key::Up);
        assert_eq!(p.text(), long, "折行中段 Up 不得进历史");
        assert_eq!(p.editor.cursor_position(), (0, 14));
        p.handle_key(&Key::Up);
        assert_eq!(p.text(), long);
        assert_eq!(p.editor.cursor_position(), (0, 4));
        // 已到首视觉行：Up → 进历史。
        p.handle_key(&Key::Up);
        assert_eq!(p.text(), "history-entry");
    }

    #[test]
    fn click_without_published_geometry_unfocuses() {
        let mut p = PromptInput::new();
        p.focus();
        // 尚无 render 发布几何 → 未命中，失焦（命中下钻的回归在
        // widget::wrap_editor 的 EditorGeom 测试侧）。
        assert!(!p.handle_click(5, 5));
        assert!(!p.is_focused());
    }

    #[test]
    fn paste_inserts_multiline_and_normalizes_crlf() {
        let mut p = PromptInput::new();
        p.paste("hello\r\nworld\r!");
        assert_eq!(p.text(), "hello\nworld\n!");
        assert!(p.is_focused());
        assert_eq!(p.visible_height_for(80), 3);
    }

    #[test]
    fn ctrl_keys_edit_without_inserting_letters() {
        let mut p = PromptInput::new();
        p.set_text("foo bar");
        let ctrl = |key: Key| revue::event::KeyEvent {
            key,
            ctrl: true,
            alt: false,
            shift: false,
        };
        // Ctrl+W 删词（不插入 'w'）
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('w'))));
        assert_eq!(p.text(), "foo ");
        // Ctrl+U 删到行首
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('u'))));
        assert_eq!(p.text(), "");
        // 未绑定 chord（Ctrl+G）→ false（调用方吞掉），文本不变
        assert!(!p.handle_ctrl_key(&ctrl(Key::Char('g'))));
        assert_eq!(p.text(), "");
    }

    #[test]
    fn ctrl_z_snapshot_undo_redo_roundtrip_cjk() {
        let mut p = PromptInput::new();
        let ctrl = |key: Key| revue::event::KeyEvent {
            key,
            ctrl: true,
            alt: false,
            shift: false,
        };
        // CJK 逐字键入：自有快照 undo（绕开 revue TextArea undo 的 CJK 缺陷）。
        p.handle_key(&Key::Char('你'));
        p.handle_key(&Key::Char('好'));
        p.handle_key(&Key::Char('a'));
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('z'))));
        assert_eq!(p.text(), "你好");
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('z'))));
        assert_eq!(p.text(), "你");
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('y'))));
        assert_eq!(p.text(), "你好");
        assert_eq!(p.editor.cursor_position(), (0, 2), "undo/redo 恢复光标位");
    }

    #[test]
    fn kills_on_multiline_only_touch_current_line() {
        let mut p = PromptInput::new();
        let ctrl = |key: Key| revue::event::KeyEvent {
            key,
            ctrl: true,
            alt: false,
            shift: false,
        };
        p.set_text("hello world\nfoo bar");
        // set_text → 光标在末行行尾 (1, 7)
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('w'))));
        assert_eq!(p.text(), "hello world\nfoo ");
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('u'))));
        // 只删当前行行首；revue TextArea 会归一掉文档末尾的空尾行（语义等价）。
        assert_eq!(p.text(), "hello world");
        // Ctrl+K 在中部行尾截断：挪到行中再 k。
        p.editor.set_cursor(0, 5);
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('k'))));
        assert_eq!(p.text(), "hello");
        // kill 进快照 undo：z 逐级回滚。
        assert!(p.handle_ctrl_key(&ctrl(Key::Char('z'))));
        assert_eq!(p.text(), "hello world");
    }
}
