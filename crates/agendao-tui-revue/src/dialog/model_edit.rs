//! 金 — Provider Model Add/Edit Dialog.
//!
//! 字段:id / name / context_window / max_output_tokens / reasoning_effort
//! (‹ › ←/→ 循环,仅当模型 reasoning==true 时显示) / timeout_secs /
//! stream_stall_timeout_secs(后三者数字输入,可空=清除)。
//! 其余高级字段(cost/temperature 等)走 prefill,UI 不动——`set_prefill`
//! 保留 server 原 ModelConfig 副本,submit 时随 Submission 带出、由
//! `submit_model_edit` 合并回去(避免 PUT 半空 ModelConfig 覆写丢字段,
//! 土律·第十条·可观测性 + 完整性)。Prefill 由 AppHandler 在 open_edit 后
//! 先 GET 原 ModelConfig 再 `set_prefill` 存入。
//!
//! 渲染经 `backdrop::render_dialog`(实色底不透字)。字段块 4 行一块
//! (label 1 + rounded border 3),鼠标命中按块反查——Reasoning effort
//! 显隐会影响块序,故命中反查走 `field_at_block_index`(显隐同源),
//! 不再用固定 FIELDS 数组。
//!
//! 上游(panel_dispatch)调:
//! - `dialog.handle_key(&key)` → `Some(Action::Submit(...))` 时,AppHandler
//!   读 `ModelEditSubmission` 调 client `put_provider_model_config`;
//! - close 在 submit / Cancel / Esc 三路对称(道纪·第九条)。

use revue::event::Key;
use revue::prelude::*;
use revue::widget::Border;

use crate::dialog::backdrop;
use crate::theme::colors;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelEditMode {
    Add,
    Edit,
}

/// reasoning effort 循环选项。`default` = 不写(ModelConfig.reasoning_effort = None,
/// 继承 server/全局默认);其余原样写入。与 agendao-config schema 注释
/// (none/minimal/low/medium/high)同源,`default` 是 UI 层的"不设置"哨兵。
pub(crate) const EFFORT_OPTIONS: &[&str] =
    &["default", "none", "minimal", "low", "medium", "high"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelEditField {
    Id,
    Name,
    ContextWindow,
    MaxOutputTokens,
    ReasoningEffort,
    TimeoutSecs,
    StreamStallSecs,
}

impl ModelEditField {
    /// 全字段渲染顺序(ReasoningEffort 显隐由调用方按 `reasoning_visible` 过滤)。
    const ORDER: [ModelEditField; 7] = [
        Self::Id,
        Self::Name,
        Self::ContextWindow,
        Self::MaxOutputTokens,
        Self::ReasoningEffort,
        Self::TimeoutSecs,
        Self::StreamStallSecs,
    ];

    fn next(self, reasoning_visible: bool) -> Self {
        let fields = visible_fields(reasoning_visible);
        let i = fields.iter().position(|f| *f == self).unwrap_or(0);
        fields[(i + 1) % fields.len()]
    }
    fn prev(self, reasoning_visible: bool) -> Self {
        let fields = visible_fields(reasoning_visible);
        let i = fields.iter().position(|f| *f == self).unwrap_or(0);
        fields[(i + fields.len() - 1) % fields.len()]
    }
}

/// 当前可见字段序列(渲染顺序)。ReasoningEffort 仅在 reasoning_visible 时出现。
/// 渲染/Tab 循环/鼠标命中三处共用同一序列(金律·顺序唯一权威)。
fn visible_fields(reasoning_visible: bool) -> Vec<ModelEditField> {
    ModelEditField::ORDER
        .into_iter()
        .filter(|f| reasoning_visible || *f != ModelEditField::ReasoningEffort)
        .collect()
}

pub enum ModelEditAction {
    Submit(Box<ModelEditSubmission>),
    Cancel,
}

/// 提交载荷。`context_window` / `max_output_tokens` / `timeout_secs` /
/// `stream_stall_timeout_secs` 字段值 None 表示用户清空 → AppHandler 在
/// prefill ModelConfig 上把对应字段置 None(清除)。
/// `reasoning_effort` None = 用户选了 `default`;`reasoning_effort_visible`
/// 记录提交时该字段是否展示——未展示(non-reasoning 模型)时 submit_model_edit
/// 不动 prefill 原值(防误清),展示时按表单值覆写。
pub struct ModelEditSubmission {
    pub mode: ModelEditMode,
    /// 该 model 所属 provider 的 id(server endpoint key)。
    pub provider_id: String,
    /// model key:Edit 模式 = 原 model.id;Add 模式 = 用户填的新 id。
    pub model_key: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    /// 用户选中的 reasoning effort;None = `default`(写入 None,清除显式设置)。
    pub reasoning_effort: Option<String>,
    /// 提交时 Reasoning effort 字段是否可见(可见才覆写,不可见保留 prefill)。
    pub reasoning_effort_visible: bool,
    pub timeout_secs: Option<u64>,
    pub stream_stall_timeout_secs: Option<u64>,
    /// Edit 模式的原 server ModelConfig 全量副本(`set_prefill` 存入)。
    /// submit_model_edit 以此为基底合并 form 字段——server PUT 是整体覆写,
    /// 没有它 cost/reasoning/temperature 等高级字段会被半空 config 抹掉。
    pub prefill: Option<agendao_config::ModelConfig>,
}

pub struct ModelEditDialog {
    pub visible: bool,
    pub mode: ModelEditMode,
    pub provider_id: String,
    pub origin_model_key: String,
    id_input: revue::widget::Input,
    name_input: revue::widget::Input,
    context_input: revue::widget::Input,
    max_output_input: revue::widget::Input,
    /// reasoning effort 选项下标(EFFORT_OPTIONS)。仅 reasoning_visible 时可编辑。
    effort_idx: usize,
    timeout_input: revue::widget::Input,
    stall_input: revue::widget::Input,
    focus: ModelEditField,
    /// Reasoning effort 字段是否展示。Add 模式恒 true(新模型允许直接设置);
    /// Edit 模式由 `set_prefill` 按原 ModelConfig.reasoning==true 决定
    /// (prefill GET 失败 → false,字段隐藏且 submit 不动原值,土律·第十条)。
    reasoning_visible: bool,
    /// 原 ModelConfig 全量副本,open_edit 后由 AppHandler GET 到再 `set_prefill`;
    /// close() 时清除(道纪·第九条·配对销毁)。
    prefill: Option<agendao_config::ModelConfig>,
}

impl ModelEditDialog {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: ModelEditMode::Add,
            provider_id: String::new(),
            origin_model_key: String::new(),
            id_input: revue::widget::Input::new().placeholder("e.g. gpt-4o-mini"),
            name_input: revue::widget::Input::new().placeholder("Display name"),
            context_input: revue::widget::Input::new().placeholder("e.g. 128000"),
            max_output_input: revue::widget::Input::new()
                .placeholder("e.g. 16384 (optional)"),
            effort_idx: 0,
            timeout_input: revue::widget::Input::new()
                .placeholder("e.g. 120 (optional)"),
            stall_input: revue::widget::Input::new()
                .placeholder("e.g. 30 (optional)"),
            focus: ModelEditField::Id,
            reasoning_visible: true,
            prefill: None,
        }
    }

    pub fn open_add(&mut self, provider_id: &str) {
        self.mode = ModelEditMode::Add;
        self.provider_id = provider_id.to_string();
        self.origin_model_key.clear();
        self.id_input = revue::widget::Input::new().placeholder("e.g. gpt-4o-mini");
        self.name_input = revue::widget::Input::new().placeholder("Display name");
        self.context_input = revue::widget::Input::new().placeholder("e.g. 128000");
        self.max_output_input = revue::widget::Input::new()
            .placeholder("e.g. 16384 (optional)");
        self.effort_idx = 0;
        self.timeout_input = revue::widget::Input::new()
            .placeholder("e.g. 120 (optional)");
        self.stall_input = revue::widget::Input::new()
            .placeholder("e.g. 30 (optional)");
        self.focus = ModelEditField::Id;
        self.reasoning_visible = true;
        self.prefill = None;
        self.visible = true;
    }

    pub fn open_edit(
        &mut self,
        provider_id: &str,
        model: &agendao_client::ProviderModelInfo,
    ) {
        self.mode = ModelEditMode::Edit;
        self.provider_id = provider_id.to_string();
        self.origin_model_key = model.id.clone();
        self.id_input = revue::widget::Input::new()
            .placeholder("Model ID")
            .value(model.id.clone());
        self.name_input = revue::widget::Input::new()
            .placeholder("Display name")
            .value(if model.name.is_empty() {
                model.id.clone()
            } else {
                model.name.clone()
            });
        let ctx_str = model
            .context_window
            .filter(|n| *n > 0)
            .map(|n| n.to_string())
            .unwrap_or_default();
        self.context_input = revue::widget::Input::new()
            .placeholder("e.g. 128000")
            .value(ctx_str);
        // max_output/timeout/effort 字段在 ProviderModelInfo 不直接暴露;Edit prefill
        // 留空,AppHandler 拿到原 ModelConfig 后经 `set_prefill` 回填(土律·第十条·完整 prefill)。
        self.max_output_input = revue::widget::Input::new()
            .placeholder("e.g. 16384 (optional)");
        self.effort_idx = 0;
        self.timeout_input = revue::widget::Input::new()
            .placeholder("e.g. 120 (optional)");
        self.stall_input = revue::widget::Input::new()
            .placeholder("e.g. 30 (optional)");
        self.focus = ModelEditField::Id;
        // 未知 reasoning 能力前先隐藏 effort 字段;set_prefill 到达后按原 config 决定。
        self.reasoning_visible = false;
        self.prefill = None;
        self.visible = true;
    }

    /// 存入 GET 到的原 ModelConfig 全量副本(Edit 模式,open_edit 之后立刻调用)。
    /// 双重作用:补全 ProviderModelInfo 不暴露的 max_output/timeout/effort 输入框预填
    /// (+ 按 cfg.reasoning 决定 effort 字段显隐);
    /// submit 时随 `ModelEditSubmission.prefill` 带出,作为 PUT 合并基底。
    pub fn set_prefill(&mut self, cfg: agendao_config::ModelConfig) {
        let max_output = cfg.limit.as_ref().and_then(|l| l.output);
        let s = max_output.map(|n| n.to_string()).unwrap_or_default();
        self.max_output_input = revue::widget::Input::new()
            .placeholder("e.g. 16384 (optional)")
            .value(s);
        let timeout_s = cfg.timeout_secs.map(|n| n.to_string()).unwrap_or_default();
        self.timeout_input = revue::widget::Input::new()
            .placeholder("e.g. 120 (optional)")
            .value(timeout_s);
        let stall_s = cfg
            .stream_stall_timeout_secs
            .map(|n| n.to_string())
            .unwrap_or_default();
        self.stall_input = revue::widget::Input::new()
            .placeholder("e.g. 30 (optional)")
            .value(stall_s);
        self.reasoning_visible = cfg.reasoning == Some(true);
        self.effort_idx = cfg
            .reasoning_effort
            .as_deref()
            .and_then(|v| EFFORT_OPTIONS.iter().position(|o| *o == v))
            .unwrap_or(0);
        // effort 字段若刚变为可见且焦点在其后字段,焦点下标无需调整
        // (focus 以字段枚举为权威,非下标)。
        self.prefill = Some(cfg);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.id_input.clear();
        self.name_input.clear();
        self.context_input.clear();
        self.max_output_input.clear();
        self.timeout_input.clear();
        self.stall_input.clear();
        self.provider_id.clear();
        self.origin_model_key.clear();
        self.prefill = None;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn handle_key(&mut self, key: &Key) -> Option<ModelEditAction> {
        if !self.visible {
            return None;
        }
        match key {
            Key::Escape => {
                self.close();
                Some(ModelEditAction::Cancel)
            }
            Key::Enter => {
                let id = self.id_input.text().trim().to_string();
                let name = self.name_input.text().trim().to_string();
                if id.is_empty() {
                    return None; // id 必填;静默不提交。
                }
                let context_window = parse_optional_u64(self.context_input.text());
                let max_output_tokens = parse_optional_u64(self.max_output_input.text());
                let timeout_secs = parse_optional_u64(self.timeout_input.text());
                let stream_stall_timeout_secs = parse_optional_u64(self.stall_input.text());
                let reasoning_effort = EFFORT_OPTIONS
                    .get(self.effort_idx)
                    .and_then(|v| if *v == "default" { None } else { Some(v.to_string()) });
                let model_key = if self.mode == ModelEditMode::Edit {
                    self.origin_model_key.clone()
                } else {
                    id.clone()
                };
                let submission = ModelEditSubmission {
                    mode: self.mode,
                    provider_id: self.provider_id.clone(),
                    model_key,
                    name: if name.is_empty() { id.clone() } else { name },
                    context_window,
                    max_output_tokens,
                    reasoning_effort,
                    reasoning_effort_visible: self.reasoning_visible,
                    timeout_secs,
                    stream_stall_timeout_secs,
                    prefill: self.prefill.take(),
                };
                self.close();
                Some(ModelEditAction::Submit(Box::new(submission)))
            }
            Key::Tab => {
                self.focus = self.focus.next(self.reasoning_visible);
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev(self.reasoning_visible);
                None
            }
            // Reasoning effort 字段下 ←/→ 切选项(其他字段走 Input 光标移动)。
            Key::Left | Key::Right
                if self.focus == ModelEditField::ReasoningEffort
                    && self.reasoning_visible =>
            {
                let n = EFFORT_OPTIONS.len();
                self.effort_idx = match key {
                    Key::Left => (self.effort_idx + n - 1) % n,
                    _ => (self.effort_idx + 1) % n,
                };
                None
            }
            _ => {
                match self.focus {
                    ModelEditField::Id => {
                        // Edit 模式 id 只读(改 id 会变 model_key 二义,走删旧+加新)。
                        if self.mode == ModelEditMode::Add {
                            let _ = self.id_input.handle_key(key);
                        }
                    }
                    ModelEditField::Name => {
                        let _ = self.name_input.handle_key(key);
                    }
                    ModelEditField::ContextWindow => {
                        let _ = self.context_input.handle_key(key);
                    }
                    ModelEditField::MaxOutputTokens => {
                        let _ = self.max_output_input.handle_key(key);
                    }
                    ModelEditField::ReasoningEffort => {
                        // 吞掉文字键——effort 只接 ←/→。
                    }
                    ModelEditField::TimeoutSecs => {
                        let _ = self.timeout_input.handle_key(key);
                    }
                    ModelEditField::StreamStallSecs => {
                        let _ = self.stall_input.handle_key(key);
                    }
                }
                None
            }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) -> Option<revue::prelude::Rect> {
        if !self.visible {
            return None;
        }
        let title = match self.mode {
            ModelEditMode::Add => " Add Model ",
            ModelEditMode::Edit => " Edit Model ",
        };

        let mut content = vstack().gap(0);
        let mut n_fields = 0u16;
        for field in visible_fields(self.reasoning_visible) {
            let focused = self.focus == field;
            let block = match field {
                ModelEditField::Id => field_input(
                    "Model ID",
                    self.id_input.clone(),
                    focused,
                    self.mode == ModelEditMode::Edit, // Edit 时 readonly hint
                ),
                ModelEditField::Name => {
                    field_input("Display Name", self.name_input.clone(), focused, false)
                }
                ModelEditField::ContextWindow => field_input(
                    "Context Window (tokens)",
                    self.context_input.clone(),
                    focused,
                    false,
                ),
                ModelEditField::MaxOutputTokens => field_input(
                    "Max Output Tokens (optional)",
                    self.max_output_input.clone(),
                    focused,
                    false,
                ),
                ModelEditField::ReasoningEffort => field_choice(
                    "Reasoning effort",
                    EFFORT_OPTIONS.get(self.effort_idx).copied().unwrap_or("default"),
                    focused,
                ),
                ModelEditField::TimeoutSecs => field_input(
                    "Timeout (secs, optional)",
                    self.timeout_input.clone(),
                    focused,
                    false,
                ),
                ModelEditField::StreamStallSecs => field_input(
                    "Stream stall timeout (secs, optional)",
                    self.stall_input.clone(),
                    focused,
                    false,
                ),
            };
            content = content.child_sized(block, 4);
            n_fields += 1;
        }

        // 返回外框 Rect（绝对坐标）：发布给 keymap 做鼠标字段命中（金律·几何同源）。
        Some(backdrop::render_dialog(
            title,
            colors::ACCENT_CYAN(),
            content,
            "Tab: next   ←/→: effort   Enter: save   Esc: cancel",
            ctx,
            76,
            n_fields * 4 + 6, // border 2 + gap 1 + footer 1 + 呼吸 2
        ))
    }
}

impl ModelEditDialog {
    /// 鼠标点击设置当前字段（与 Tab 切换同一 `focus` 权威）。
    pub(crate) fn set_focus(&mut self, field: ModelEditField) {
        self.focus = field;
    }

    /// 当前焦点字段（测试/命中校验用）。
    #[cfg(test)]
    pub(crate) fn focus(&self) -> ModelEditField {
        self.focus
    }

    /// 按渲染块序(4 行一块)反查字段——ReasoningEffort 显隐同源,
    /// 鼠标命中不再依赖固定字段数组(金律·顺序唯一权威 `visible_fields`)。
    pub(crate) fn field_at_block_index(&self, idx: usize) -> Option<ModelEditField> {
        visible_fields(self.reasoning_visible).get(idx).copied()
    }
}

impl Default for ModelEditDialog {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_optional_u64(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<u64>().ok()
    }
}

fn field_input(
    label: &str,
    mut input: revue::widget::Input,
    focused: bool,
    readonly: bool,
) -> revue::widget::Stack {
    let label_color = if readonly {
        colors::FG_MUTED()
    } else if focused {
        colors::E_AMBER()
    } else {
        colors::FG_SECONDARY()
    };
    let border_color = if readonly {
        colors::BORDER()
    } else if focused {
        colors::E_AMBER()
    } else {
        colors::BORDER()
    };
    input = input.focused(focused && !readonly);
    let label_text = if readonly {
        format!(" {} (read-only)", label)
    } else {
        format!(" {}", label)
    };
    vstack()
        .gap(0)
        .child_sized(Text::new(label_text).fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(input), 3)
}

/// effort 横向选择器:`‹ medium ›` 形态,focused 时高亮(与 mcp_edit
/// field_choice 同语义,dialog 几何内复刻)。
fn field_choice(label: &str, choice_label: &str, focused: bool) -> revue::widget::Stack {
    let label_color = if focused {
        colors::E_AMBER()
    } else {
        colors::FG_SECONDARY()
    };
    let border_color = if focused {
        colors::E_AMBER()
    } else {
        colors::BORDER()
    };
    let value_color = if focused {
        colors::FG_PRIMARY()
    } else {
        colors::FG_SECONDARY()
    };
    let value = Text::new(format!("‹ {} ›  (←/→ to change)", choice_label)).fg(value_color);
    vstack()
        .gap(0)
        .child_sized(Text::new(format!(" {}", label)).fg(label_color), 1)
        .child_sized(Border::rounded().fg(border_color).child(value), 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> agendao_client::ProviderModelInfo {
        agendao_client::ProviderModelInfo {
            id: "gpt-4o".into(),
            name: "GPT-4o".into(),
            provider: "openai".into(),
            variants: vec![],
            context_window: Some(128_000),
            max_output_tokens: None,
            cost_per_million_input: None,
            cost_per_million_output: None,
        }
    }

    #[test]
    fn open_add_sets_provider_id_and_focus() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        assert!(d.is_open());
        assert_eq!(d.provider_id, "openai");
        assert_eq!(d.mode, ModelEditMode::Add);
        assert_eq!(d.focus, ModelEditField::Id);
        assert!(d.reasoning_visible, "Add 模式恒展示 effort 字段");
    }

    #[test]
    fn open_edit_prefills_id_name_context() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        assert_eq!(d.mode, ModelEditMode::Edit);
        assert_eq!(d.id_input.text(), "gpt-4o");
        assert_eq!(d.name_input.text(), "GPT-4o");
        assert_eq!(d.context_input.text(), "128000");
        assert_eq!(d.origin_model_key, "gpt-4o");
        assert!(!d.reasoning_visible, "set_prefill 前 effort 字段隐藏");
    }

    #[test]
    fn edit_mode_id_is_readonly() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        d.focus = ModelEditField::Id;
        // 试图打字应被吞,Input 内容不变。
        d.handle_key(&Key::Char('x'));
        assert_eq!(d.id_input.text(), "gpt-4o");
    }

    #[test]
    fn add_mode_id_accepts_typing() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        for c in "mymodel".chars() {
            d.handle_key(&Key::Char(c));
        }
        assert_eq!(d.id_input.text(), "mymodel");
    }

    #[test]
    fn enter_with_empty_id_does_not_submit() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        assert!(d.handle_key(&Key::Enter).is_none());
        assert!(d.is_open());
    }

    #[test]
    fn esc_returns_cancel_and_closes() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        let action = d.handle_key(&Key::Escape);
        assert!(matches!(action, Some(ModelEditAction::Cancel)));
        assert!(!d.is_open());
    }

    #[test]
    fn submit_parses_numbers() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        for c in "x".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → Name(空,会回退用 id "x")
        d.handle_key(&Key::Tab); // → ContextWindow
        for c in "65536".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.handle_key(&Key::Tab); // → MaxOutputTokens
        for c in "8192".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.mode, ModelEditMode::Add);
        assert_eq!(s.provider_id, "openai");
        assert_eq!(s.model_key, "x");
        assert_eq!(s.name, "x"); // 空 name 兜底用 id
        assert_eq!(s.context_window, Some(65536));
        assert_eq!(s.max_output_tokens, Some(8192));
        assert!(!d.is_open());
    }

    #[test]
    fn submit_with_empty_max_output_yields_none() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        for c in "m".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.context_window, None);
        assert_eq!(s.max_output_tokens, None);
        assert_eq!(s.timeout_secs, None);
        assert_eq!(s.stream_stall_timeout_secs, None);
        assert_eq!(s.reasoning_effort, None, "默认 effort=default → None");
        assert!(s.reasoning_effort_visible);
    }

    fn sample_prefill() -> agendao_config::ModelConfig {
        use agendao_config::{ModelConfig, ModelLimitConfig};
        ModelConfig {
            name: Some("GPT-4o".into()),
            model: Some("gpt-4o".into()),
            reasoning: Some(true),
            reasoning_effort: Some("medium".into()),
            timeout_secs: Some(120),
            stream_stall_timeout_secs: Some(30),
            temperature: Some(true),
            limit: Some(ModelLimitConfig {
                context: Some(128_000),
                input: Some(100_000),
                output: Some(16_384),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn set_prefill_after_edit_fills_max_output_and_timeouts() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        assert_eq!(d.max_output_input.text(), "");
        d.set_prefill(sample_prefill());
        assert_eq!(d.max_output_input.text(), "16384");
        assert_eq!(d.timeout_input.text(), "120");
        assert_eq!(d.stall_input.text(), "30");
        assert!(d.reasoning_visible, "reasoning==true → effort 字段展示");
        assert_eq!(d.effort_idx, 4, "medium 在 EFFORT_OPTIONS 下标 4");
    }

    #[test]
    fn set_prefill_non_reasoning_model_hides_effort() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        let mut cfg = sample_prefill();
        cfg.reasoning = Some(false);
        d.set_prefill(cfg);
        assert!(!d.reasoning_visible);
        // Tab 循环不含 ReasoningEffort:Id→Name→Context→MaxOutput→Timeout→Stall→Id
        let mut f = ModelEditField::Id;
        for _ in 0..6 {
            f = f.next(d.reasoning_visible);
        }
        assert_eq!(f, ModelEditField::Id);
        // 隐藏时 submit 标 visible=false,submit_model_edit 不动 prefill 原值。
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert!(!s.reasoning_effort_visible);
    }

    #[test]
    fn effort_left_right_cycles_and_submits() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        for c in "m".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = ModelEditField::ReasoningEffort;
        d.handle_key(&Key::Right); // default → none
        d.handle_key(&Key::Right); // none → minimal
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.reasoning_effort.as_deref(), Some("minimal"));
    }

    #[test]
    fn submit_parses_timeouts() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai");
        for c in "m".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = ModelEditField::TimeoutSecs;
        for c in "300".chars() {
            d.handle_key(&Key::Char(c));
        }
        d.focus = ModelEditField::StreamStallSecs;
        for c in "45".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert_eq!(s.timeout_secs, Some(300));
        assert_eq!(s.stream_stall_timeout_secs, Some(45));
    }

    #[test]
    fn field_at_block_index_respects_effort_visibility() {
        let mut d = ModelEditDialog::new();
        d.open_add("openai"); // reasoning_visible=true → 7 块
        assert_eq!(d.field_at_block_index(4), Some(ModelEditField::ReasoningEffort));
        assert_eq!(d.field_at_block_index(5), Some(ModelEditField::TimeoutSecs));
        assert_eq!(d.field_at_block_index(6), Some(ModelEditField::StreamStallSecs));
        assert_eq!(d.field_at_block_index(7), None);
        d.open_edit("openai", &sample_model()); // 隐藏 → 6 块,下标 4 = Timeout
        assert_eq!(d.field_at_block_index(4), Some(ModelEditField::TimeoutSecs));
        assert_eq!(d.field_at_block_index(6), None);
    }

    #[test]
    fn submit_carries_prefill_through() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        d.set_prefill(sample_prefill());
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        let prefill = s.prefill.expect("prefill should ride along on submit");
        assert_eq!(prefill.reasoning, Some(true));
        assert_eq!(prefill.limit.as_ref().and_then(|l| l.input), Some(100_000));
        // 表单值(prefill 回填,未改):effort=medium、timeout=120 随提交带出。
        assert_eq!(s.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(s.timeout_secs, Some(120));
        assert_eq!(s.stream_stall_timeout_secs, Some(30));
        // close() 后 dialog 内不再驻留 prefill(配对销毁)。
        assert!(d.prefill.is_none());
    }

    #[test]
    fn add_mode_has_no_prefill() {
        let mut d = ModelEditDialog::new();
        d.set_prefill(sample_prefill());
        d.open_add("openai");
        for c in "m".chars() {
            d.handle_key(&Key::Char(c));
        }
        let Some(ModelEditAction::Submit(s)) = d.handle_key(&Key::Enter) else {
            panic!("expected Submit");
        };
        assert!(s.prefill.is_none());
    }
}
