//! 金 — Provider Model Add/Edit Dialog.
//!
//! 字段:id / name / context_window / max_output_tokens(四字段 form);
//! 高级字段(cost/reasoning/temperature)走 prefill,UI 不动——`set_prefill`
//! 保留 server 原 ModelConfig 副本,submit 时随 Submission 带出、由
//! `submit_model_edit` 合并回去(避免 PUT 半空 ModelConfig 覆写丢字段,
//! 土律·第十条·可观测性 + 完整性)。Prefill 由 AppHandler 在 open_edit 后
//! 先 GET 原 ModelConfig 再 `set_prefill` 存入。
//!
//! 上游(Part 5)调:
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelEditField {
    Id,
    Name,
    ContextWindow,
    MaxOutputTokens,
}

impl ModelEditField {
    fn next(self) -> Self {
        match self {
            Self::Id => Self::Name,
            Self::Name => Self::ContextWindow,
            Self::ContextWindow => Self::MaxOutputTokens,
            Self::MaxOutputTokens => Self::Id,
        }
    }
    fn prev(self) -> Self {
        match self {
            Self::Id => Self::MaxOutputTokens,
            Self::Name => Self::Id,
            Self::ContextWindow => Self::Name,
            Self::MaxOutputTokens => Self::ContextWindow,
        }
    }
}

pub enum ModelEditAction {
    Submit(ModelEditSubmission),
    Cancel,
}

/// 提交载荷。`context_window` / `max_output_tokens` 字段值 None 表示用户清空
/// → AppHandler 在 prefill ModelConfig 上把 limit.context/output 置 None。
pub struct ModelEditSubmission {
    pub mode: ModelEditMode,
    /// 该 model 所属 provider 的 id(server endpoint key)。
    pub provider_id: String,
    /// model key:Edit 模式 = 原 model.id;Add 模式 = 用户填的新 id。
    pub model_key: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
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
    focus: ModelEditField,
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
            focus: ModelEditField::Id,
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
        self.focus = ModelEditField::Id;
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
        // max_output 字段在 ProviderModelInfo 不直接暴露;Edit prefill 留空,
        // AppHandler 拿到原 ModelConfig 后经 `set_prefill` 回填(土律·第十条·完整 prefill)。
        self.max_output_input = revue::widget::Input::new()
            .placeholder("e.g. 16384 (optional)");
        self.focus = ModelEditField::Id;
        self.prefill = None;
        self.visible = true;
    }

    /// 存入 GET 到的原 ModelConfig 全量副本(Edit 模式,open_edit 之后立刻调用)。
    /// 双重作用:补全 ProviderModelInfo 不暴露的 max_output 输入框预填;
    /// submit 时随 `ModelEditSubmission.prefill` 带出,作为 PUT 合并基底。
    pub fn set_prefill(&mut self, cfg: agendao_config::ModelConfig) {
        let max_output = cfg.limit.as_ref().and_then(|l| l.output);
        let s = max_output.map(|n| n.to_string()).unwrap_or_default();
        self.max_output_input = revue::widget::Input::new()
            .placeholder("e.g. 16384 (optional)")
            .value(s);
        self.prefill = Some(cfg);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.id_input.clear();
        self.name_input.clear();
        self.context_input.clear();
        self.max_output_input.clear();
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
                    prefill: self.prefill.take(),
                };
                self.close();
                Some(ModelEditAction::Submit(submission))
            }
            Key::Tab => {
                self.focus = self.focus.next();
                None
            }
            Key::BackTab => {
                self.focus = self.focus.prev();
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
                }
                None
            }
        }
    }

    pub fn render(&self, ctx: &mut RenderContext) {
        if !self.visible {
            return;
        }
        let title = match self.mode {
            ModelEditMode::Add => " Add Model ",
            ModelEditMode::Edit => " Edit Model ",
        };

        let id_field = field_input(
            "Model ID",
            self.id_input.clone(),
            self.focus == ModelEditField::Id,
            self.mode == ModelEditMode::Edit, // Edit 时 readonly hint
        );
        let name_field = field_input(
            "Display Name",
            self.name_input.clone(),
            self.focus == ModelEditField::Name,
            false,
        );
        let ctx_field = field_input(
            "Context Window (tokens)",
            self.context_input.clone(),
            self.focus == ModelEditField::ContextWindow,
            false,
        );
        let max_field = field_input(
            "Max Output Tokens (optional)",
            self.max_output_input.clone(),
            self.focus == ModelEditField::MaxOutputTokens,
            false,
        );

        let content = vstack()
            .gap(0)
            .child_sized(id_field, 4)
            .child_sized(name_field, 4)
            .child_sized(ctx_field, 4)
            .child_sized(max_field, 4);

        backdrop::render_dialog(
            title,
            colors::ACCENT_CYAN,
            content,
            "Tab: next   Enter: save   Esc: cancel",
            ctx,
            70,
            22,
        );
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
        colors::FG_MUTED
    } else if focused {
        colors::E_AMBER
    } else {
        colors::FG_SECONDARY
    };
    let border_color = if readonly {
        colors::BORDER
    } else if focused {
        colors::E_AMBER
    } else {
        colors::BORDER
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
        assert!(matches!(d.handle_key(&Key::Enter), None));
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
    }

    fn sample_prefill() -> agendao_config::ModelConfig {
        use agendao_config::{ModelConfig, ModelLimitConfig};
        ModelConfig {
            name: Some("GPT-4o".into()),
            model: Some("gpt-4o".into()),
            reasoning: Some(true),
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
    fn set_prefill_after_edit_fills_max_output() {
        let mut d = ModelEditDialog::new();
        d.open_edit("openai", &sample_model());
        assert_eq!(d.max_output_input.text(), "");
        d.set_prefill(sample_prefill());
        assert_eq!(d.max_output_input.text(), "16384");
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
