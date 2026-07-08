//! 金 — Settings Screen:全屏三栏(分类 | Providers | Details)。
//!
//! 入口唯一(阳):`UiActionId::OpenSettings`(⚙ click + `/settings` slash);
//! 状态唯一(阴):`AppStore.providers` + `settings_*` signals(土律·单一权威)。
//!
//! 排版对照:`dev_docs/gemini-code-1781709371616.html`,改用 TUI 列宽缩放
//! (HTML 220/200/flex → TUI Categories 22 / VLine 1 / Providers 28 / VLine 1 / Details flex)。
//! 当前阶段(Plan 第 1 部分约定):只做 Model Settings 分类;其余 5 项灰显;
//! API key 永不下发(占位 `••••••••` + "edit in config" 提示)。
//!
//! 结构选型:不实现 `View`——revue `child_flex` 要 `+ 'static`,持 `&AppStore`
//! 引用会失败。改用 unit struct + 关联函数 `build()` 返回 `Stack`(同 SessionSidebar
//! 模式):snapshot signals 后纯组合;无生命周期、自动 `'static`(土律·单点编排)。

use std::collections::HashSet;

use revue::prelude::*;
use revue::widget::{Border, Input};

use crate::app::settings_edit_state::{
    SettingsEditField, SettingsEditState,
};
use crate::store::app_store::AppStore;
use crate::store::types::{SettingsCategory, SettingsFocusPane};
use crate::theme::colors;

/// Settings 三栏布局常量(单点权威,改一处全跟随)。
const CATEGORIES_W: u16 = 22;
const PROVIDERS_W: u16 = 28;
const VLINE_W: u16 = 1;

/// Settings 全屏构建器;由 `app/mod.rs` 在 `Route::Settings` 分支调用 `build()`。
pub struct SettingsScreen;

impl SettingsScreen {
    /// 从 `AppStore` snapshot 当前 settings 相关 signal,组装三栏布局并返回 `Stack`。
    /// 返回 `'static` Stack,直接喂给 `child_flex`/`child` 即可。
    ///
    /// `pane_height` 为整屏可用高度(`ctx.area.height`);Providers 栏用它推导
    /// 数据行可见行数,再交给 [`crate::dialog::backdrop::list_viewport_window`]
    /// 算 `(start, end)` 滑窗——选中超出视野时跟随移动(金律·成形语法唯一)。
    pub fn build(
        store: &AppStore,
        pane_height: u16,
        edit_state: Option<&SettingsEditState>,
    ) -> revue::widget::Stack {
        let category = store.settings_category.get();
        let focus = store.settings_focus_pane.get();
        let providers = store.providers.get();
        let connected = store.providers_connected.get();
        let selected = store.settings_selected_provider.get();

        // edit_state.active = true 时 Settings 的 keymap focus 概念被覆盖:
        //   - Edit:focus 强制按 Details(编辑发生在 Details pane);Providers 栏依旧
        //     显示 selected highlight,但视觉权重让位给 Details 的高亮边框
        //   - Add:虚拟追加一行 "(new provider)";selected 强制指向这行,Details
        //     从 edit_state 字段取空白可填值
        let editing_active = edit_state.is_some_and(|s| s.active);
        let is_add = edit_state.is_some_and(|s| s.is_add());

        // selected_idx:Edit 模式照常查 providers;Add 模式落在虚拟追加行
        // (providers.len()——0-based 下标正好等于追加位置)。
        let selected_idx = if is_add {
            Some(providers.len())
        } else {
            selected
                .as_deref()
                .and_then(|id| providers.iter().position(|p| p.id == id))
        };

        let selected_model = store.settings_selected_model.get();

        let cat_pane = build_categories_pane(category, focus == SettingsFocusPane::Categories);
        let prov_pane = build_providers_pane(
            &providers,
            &connected,
            selected.as_deref(),
            focus == SettingsFocusPane::Providers,
            pane_height,
            selected_idx,
            is_add,
        );
        let detail_pane = build_details_pane(
            &providers,
            &connected,
            selected.as_deref(),
            focus == SettingsFocusPane::Details || editing_active,
            selected_model.as_deref(),
            edit_state,
        );

        hstack().gap(0)
            .child_sized(cat_pane, CATEGORIES_W)
            .child_sized(vline(), VLINE_W)
            .child_sized(prov_pane, PROVIDERS_W)
            .child_sized(vline(), VLINE_W)
            .child_flex(detail_pane, 1.0)
    }
}

// ── 公共小件 ──

/// 垂直分隔线(占 1 列,贯整列高 `│` 暗色)。revue stack 会按 child 区域裁切高度,
/// 这里返回一个 vstack 填满即可。
fn vline() -> revue::widget::Stack {
    vstack().gap(0).child_flex(
        Text::new("│".repeat(64)).fg(colors::SIDEBAR_DIVIDER),
        1.0,
    )
}

// ── 第一栏:Categories ──

fn build_categories_pane(active: SettingsCategory, focused: bool) -> revue::widget::Stack {
    // 标题 + 6 项 + flex 空白 + 底部 Esc 提示。Pane 不画边框(列已用 VLine 划界),
    // 这样视觉重量集中在 active 行的高亮,与 HTML 稿一致。
    let title_color = if focused { colors::E_TEAL } else { colors::FG_SECONDARY };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1) // 顶呼吸 1 行
        .child_sized(Text::new("  ⚙ Preferences").fg(title_color).bold(), 1)
        .child_sized(Text::new(""), 1);

    for cat in SettingsCategory::ALL.iter().copied() {
        s = s.child_sized(category_row(cat, active == cat), 1);
    }

    s.child_flex(Text::new(""), 1.0)
        .child_sized(Text::new("  ← Esc: Exit").fg(colors::FG_TRACE), 1)
        .child_sized(Text::new(""), 1)
}

fn category_row(cat: SettingsCategory, is_active: bool) -> revue::widget::Stack {
    // 视觉权重:active = ▸ + E_TEAL,implemented(非 active)= FG_SECONDARY,
    // 灰显占位 = FG_TRACE(刚好看清,与 HTML 的 disabled 一致)。
    let (icon, color) = if is_active {
        ("▸", colors::E_TEAL)
    } else if cat.is_implemented() {
        (" ", colors::FG_SECONDARY)
    } else {
        (" ", colors::FG_TRACE)
    };
    let line = format!(" {} {} {}", icon, cat.icon(), cat.label());
    let w = line.chars().count() as u16;
    hstack().gap(0)
        .child_sized(Text::new(line).fg(color), w)
        .child_flex(Text::new(""), 1.0)
}

// ── 第二栏:Providers ──

fn build_providers_pane(
    providers: &[agendao_client::ProviderInfo],
    connected: &HashSet<String>,
    selected_id: Option<&str>,
    focused: bool,
    pane_height: u16,
    selected_idx: Option<usize>,
    is_add: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL } else { colors::FG_SECONDARY };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(Text::new("  ◆ Providers").fg(title_color).bold(), 1)
        .child_sized(Text::new(""), 1);

    if providers.is_empty() && !is_add {
        // 空态:加载中或 server 无 provider 配置。
        s = s.child_sized(
            Text::new("   (no providers)").fg(colors::FG_TRACE),
            1,
        );
        return s
            .child_flex(Text::new(""), 1.0)
            .child_sized(
                Text::new("  + Add provider").fg(colors::FG_TRACE),
                1,
            )
            .child_sized(Text::new(""), 1);
    }

    // 数据行可见行数 = 整屏高 - 5(顶呼吸 1 + 标题 1 + 标题后 blank 1 + 底 "+ Add" 1 + 底呼吸 1)。
    // 不再保留 position 指示行(指示文案在 "+ Add" 行右侧/footer,不再单占行,避免再扣 1)。
    let visible_rows = pane_height.saturating_sub(5).max(1) as usize;
    // total 包含 Add 模式下的虚拟"(new provider)"行(土律:草稿不入 store.providers,
    // 但渲染层这里**虚拟追加**一行让用户看见编辑目标,close 后自然消失)。
    let total = providers.len() + if is_add { 1 } else { 0 };
    // selected_idx 缺省为 0(钉顶):列表非空但还没建立 selected,默认显示首屏。
    let sel = selected_idx.unwrap_or(0);
    let (start, end) = crate::dialog::backdrop::list_viewport_window(total, sel, visible_rows);

    // 渲染窗口内的真 provider 行。
    let real_end = end.min(providers.len());
    for p in &providers[start.min(providers.len())..real_end] {
        let is_selected = !is_add && selected_id == Some(p.id.as_str());
        let is_connected = connected.contains(&p.id);
        s = s.child_sized(provider_row(&p.name, is_connected, is_selected), 1);
    }
    // Add 模式 + 虚拟行落在窗口内 → 追加 "(new provider)" highlight 行
    // (selected_idx == providers.len(),只要 end > providers.len() 即在窗口内)。
    if is_add && end > providers.len() {
        s = s.child_sized(provider_row_draft(focused), 1);
    }

    // "+ Add provider" 行右侧叠位置指示 `{i}/{total}`(同 slash_popup 形态)。
    // selected_idx 缺省时显示 `-/total`,诚实标"无选中"(土律·第十条·可观测性)。
    let pos = match selected_idx {
        Some(i) => format!("{}/{}", i + 1, total),
        None => format!("-/{}", total),
    };
    let pos_w = pos.chars().count() as u16;
    let add_color = if focused { colors::FG_SECONDARY } else { colors::FG_TRACE };
    // Providers pane 通常窄(~24 列),hint 必须极简:`a/e/d` 三字母合订 +
    // 后边 `+ Add provider` 主入口文案。详细 hint("a: Add e: Edit d: Delete")
    // 由 Details pane footer_hint 承接(底部 1 行更宽,文字不挤),
    // 同一信号同时存在于两层窗口对**不同视野阶**用户友好:
    // 老用户看 a/e/d 即懂,新用户切 Details focus 后看长 hint(金律·成形递进)。
    let add_text = if focused { "  + Add  (a/e/d)" } else { "  + Add provider" };
    let add_w = add_text.chars().count() as u16;
    let footer_row = hstack().gap(0)
        .child_sized(Text::new(add_text).fg(add_color), add_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(pos).fg(colors::FG_TRACE), pos_w)
        .child_sized(Text::new("  "), 2);

    s.child_flex(Text::new(""), 1.0)
        .child_sized(footer_row, 1)
        .child_sized(Text::new(""), 1)
}

/// Add 模式下虚拟追加的"(new provider)"草稿行(永远 selected,标 highlight)。
/// 不进入 store.providers——纯渲染层占位,close 后随 edit_state 一起消失。
fn provider_row_draft(focused: bool) -> revue::widget::Stack {
    let name_color = if focused { colors::E_AMBER } else { colors::FG_SECONDARY };
    let prefix = " ▸ (new provider)";
    let prefix_w = prefix.chars().count() as u16;
    hstack().gap(0)
        .child_sized(Text::new(prefix).fg(name_color).italic(), prefix_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new("✎").fg(colors::E_AMBER), 1)
        .child_sized(Text::new(" "), 1)
}

fn provider_row(name: &str, is_connected: bool, is_selected: bool) -> revue::widget::Stack {
    // active provider 行(selected):▸ + E_TEAL;否则 ◇ + FG_SECONDARY。
    // 连接状态 dot:● ACCENT_GREEN(connected) / ─ FG_TRACE(disconnected)。
    let (marker, color) = if is_selected {
        ("▸", colors::E_TEAL)
    } else {
        ("◇", colors::FG_SECONDARY)
    };
    let dot = if is_connected { "●" } else { "─" };
    let dot_color = if is_connected { colors::ACCENT_GREEN } else { colors::FG_TRACE };
    let prefix = format!(" {} {}", marker, name);
    let prefix_w = prefix.chars().count() as u16;
    hstack().gap(0)
        .child_sized(Text::new(prefix).fg(color), prefix_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(dot).fg(dot_color), 1)
        .child_sized(Text::new(" "), 1)
}

// ── 第三栏:Details ──

fn build_details_pane(
    providers: &[agendao_client::ProviderInfo],
    connected: &HashSet<String>,
    selected_id: Option<&str>,
    focused: bool,
    selected_model: Option<&str>,
    edit_state: Option<&SettingsEditState>,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL } else { colors::FG_SECONDARY };
    let editing_active = edit_state.is_some_and(|s| s.active);
    let is_add = edit_state.is_some_and(|s| s.is_add());

    // editing 时 Details pane 取数策略:
    //   - Add:store.providers 中无此条,合成"空白"展示;name 从 edit_state.name_input 取
    //   - Edit:走 store.providers[selected_id],字段值取自 edit_state.{base/protocol/api_key}
    //   - 非 editing:走 store.providers[selected_id] 现有只读路径
    let provider_opt: Option<&agendao_client::ProviderInfo> = if is_add {
        None
    } else {
        selected_id.and_then(|id| providers.iter().find(|p| p.id == id))
    };

    // 没 provider 且非 Add 草稿:空态。
    if provider_opt.is_none() && !is_add {
        return vstack().gap(0)
            .child_sized(Text::new(""), 1)
            .child_sized(Text::new("  Select a provider").fg(colors::FG_MUTED), 1)
            .child_flex(Text::new(""), 1.0);
    }

    // ── Header 行 ──
    // 非 editing:◆ name + [Enabled] pill;Add:◆ (new provider) + [Drafting] pill;
    // Edit:照常显示 provider.name(name 不可改)
    let (header_label, pill, pill_color) = if is_add {
        ("  ◆ (new provider)".to_string(), " Drafting ", colors::E_AMBER)
    } else {
        let p = provider_opt.expect("provider_opt is Some when !is_add");
        let is_connected = connected.contains(&p.id);
        let lbl = format!("  ◆ {}", p.name);
        let pl = if is_connected { " Enabled " } else { " Disabled " };
        let pc = if is_connected { colors::ACCENT_GREEN } else { colors::FG_MUTED };
        (lbl, pl, pc)
    };
    let header_label_w = header_label.chars().count() as u16;
    let pill_w = pill.chars().count() as u16;
    let mode_text = "Connection mode <API key>";
    let mode_w = mode_text.chars().count() as u16;
    let header = hstack().gap(1)
        .child_sized(Text::new(header_label).fg(title_color).bold(), header_label_w)
        .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(mode_text).fg(colors::FG_TRACE), mode_w)
        .child_sized(Text::new("  "), 2);

    // ── Name 字段(仅 Add 模式 editing 显示;Edit 模式 name 不允许改,不渲染单独字段)──
    let name_field_opt = if is_add {
        let st = edit_state.expect("is_add => edit_state Some");
        let focused_field = st.focus == SettingsEditField::Name;
        Some(field_block_editing(
            "Name (also used as ID)",
            st.name_input.clone(),
            "",
            focused_field,
        ))
    } else {
        None
    };

    // ── BaseURL 字段 ──
    let base_field = if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::BaseUrl;
        field_block_editing("Base URL", st.base_url_input.clone(), "", focused_field)
    } else {
        let p = provider_opt.expect("non-editing => provider_opt Some");
        let base_value = p
            .base_url
            .clone()
            .unwrap_or_else(|| "(not set)".to_string());
        let base_color = if p.base_url.is_some() { colors::FG_PRIMARY } else { colors::FG_TRACE };
        field_block("Base URL", &base_value, base_color, "")
    };

    // ── Protocol 字段 ──
    let protocol_field = if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::Protocol;
        field_block_choice("Protocol", st.protocol_label(), focused_field)
    } else {
        let p = provider_opt.expect("non-editing => provider_opt Some");
        // 与 base_url 配对(同 server 唯一权威),决定 HTTP 实际打哪条契约
        // (`openai`/`anthropic`/`google`/...:openai-compatible /v1/chat/completions
        // vs anthropic-compatible /v1/messages 等)。`None` = catalog/config 都没记 npm,
        // 显示 `(unknown)` 诚实标注,不假装"openai"(土律·第十条·可观测性权利)。
        let protocol_value = p
            .protocol
            .as_deref()
            .map(protocol_display_label)
            .unwrap_or("(unknown)");
        let protocol_color = if p.protocol.is_some() { colors::FG_PRIMARY } else { colors::FG_TRACE };
        field_block("Protocol", protocol_value, protocol_color, "")
    };

    // ── APIKey 字段:editing 时 Input.password 显示 `•`,buffer 明文(submit 时取);
    // 非 editing 时 placeholder 永远 `••••••` 永不下发;focused 时 hint "e: Edit"
    // 让用户知道改 key 走 e 进入 in-place 编辑(Part 7c 接线)。
    let api_key_field = if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::ApiKey;
        field_block_editing("API key", st.api_key_input.clone(), "", focused_field)
    } else {
        let api_key_hint = if focused { "e: Edit" } else { "" };
        field_block(
            "API key",
            "••••••••••••••••••••",
            colors::FG_MUTED,
            api_key_hint,
        )
    };

    // ── Models 列表 ──
    // Models header 一行平铺:左侧 "Models",右侧(focused 时)keymap hint。
    // 与 Providers footer 同形态(金律·成形语法权威):每个区段的"可用动作"
    // 都贴在该区段的边缘,而非藏在屏幕底栏一并展示。
    let models_label = "  Models";
    let models_label_w = models_label.chars().count() as u16;
    let models_hint = if focused && !editing_active { "m: Add  e: Edit  d: Delete  " } else { "" };
    let models_hint_w = models_hint.chars().count() as u16;
    let models_header_row = hstack().gap(0)
        .child_sized(
            Text::new(models_label).fg(colors::FG_SECONDARY).bold(),
            models_label_w,
        )
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(models_hint).fg(colors::FG_TRACE), models_hint_w);
    let mut models_block = vstack().gap(0)
        .child_sized(models_header_row, 1)
        .child_sized(Text::new(""), 1);
    // editing 中或 Add 草稿:不展示 models 列表(避免对未落盘 provider 谈 models);
    // 否则按现有逻辑画 models 列表。
    let show_models = !editing_active;
    if show_models {
        let provider = provider_opt.expect("show_models => provider_opt Some");
        if provider.models.is_empty() {
            models_block = models_block.child_sized(
                Text::new("   (no models configured)").fg(colors::FG_TRACE),
                1,
            );
        } else {
            for m in &provider.models {
                let is_selected = selected_model.is_some_and(|k| k == m.id);
                models_block = models_block.child_sized(model_row(m, is_selected, focused), 1);
            }
        }
    } else if is_add {
        models_block = models_block.child_sized(
            Text::new("   (add models after saving)").fg(colors::FG_TRACE).italic(),
            1,
        );
    } else {
        models_block = models_block.child_sized(
            Text::new("   (models hidden while editing)").fg(colors::FG_TRACE).italic(),
            1,
        );
    }

    // 组装总 stack:editing 时 Name 字段只在 Add 模式出现;其余字段一律渲染。
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(header, 1)
        .child_sized(Text::new(""), 1);
    if let Some(name_field) = name_field_opt {
        s = s.child_sized(name_field, 3).child_sized(Text::new(""), 1);
    }
    s.child_sized(base_field, 3)
        .child_sized(Text::new(""), 1)
        .child_sized(protocol_field, 3)
        .child_sized(Text::new(""), 1)
        .child_sized(api_key_field, 3)
        .child_sized(Text::new(""), 1)
        .child_flex(models_block, 1.0)
        .child_sized(footer_hint_editing(focused, selected_model.is_some(), editing_active), 1)
        .child_sized(Text::new(""), 1)
}

/// 字段块:label(1 行 FG_SECONDARY)+ bordered value(2 行,rounded border)+
/// 右上角辅助文本(如 "⌕ View")。返回总高 3 行的 vstack。
fn field_block(label: &str, value: &str, value_color: Color, hint: &str) -> revue::widget::Stack {
    let label_line = if hint.is_empty() {
        hstack().gap(0).child_flex(
            Text::new(format!("  {}", label)).fg(colors::FG_SECONDARY),
            1.0,
        )
    } else {
        let lab = format!("  {}", label);
        let lab_w = lab.chars().count() as u16;
        let hint_w = hint.chars().count() as u16;
        hstack().gap(0)
            .child_sized(Text::new(lab).fg(colors::FG_SECONDARY), lab_w)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new(hint).fg(colors::FG_TRACE), hint_w)
            .child_sized(Text::new("  "), 2)
    };
    let bordered = Border::only_bottom()
        .fg(colors::BORDER)
        .child(
            hstack().gap(0)
                .child_sized(Text::new("  "), 2)
                .child_flex(Text::new(value.to_string()).fg(value_color), 1.0),
        );
    vstack().gap(0)
        .child_sized(label_line, 1)
        .child_sized(bordered, 2) // value 行(1) + 下边框(1)
}

/// 字段块·可编辑形态:label + Input(承载光标 + 文字 buffer)+ 下边框。
/// editing 进行中且当前字段 focused = true 时:
///   - label 颜色 E_AMBER(高亮)
///   - 下边框颜色 E_AMBER + Input.focused(true) 让光标闪
/// focused = false(其他字段在编辑但不是当前焦点):
///   - label / 边框 用 BORDER 暗色
///   - Input.focused(false)
///
/// 返回总高 3 行(与 field_block 等高,便于 Add 模式插入 Name 字段不引起跳层)。
fn field_block_editing(
    label: &str,
    input: Input,
    hint: &str,
    focused: bool,
) -> revue::widget::Stack {
    let label_color = if focused { colors::E_AMBER } else { colors::FG_SECONDARY };
    let border_color = if focused { colors::E_AMBER } else { colors::BORDER };
    let label_line = if hint.is_empty() {
        hstack().gap(0).child_flex(
            Text::new(format!("  {}", label)).fg(label_color),
            1.0,
        )
    } else {
        let lab = format!("  {}", label);
        let lab_w = lab.chars().count() as u16;
        let hint_w = hint.chars().count() as u16;
        hstack().gap(0)
            .child_sized(Text::new(lab).fg(label_color), lab_w)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new(hint).fg(colors::FG_TRACE), hint_w)
            .child_sized(Text::new("  "), 2)
    };
    // Input 是 Clone:此处的 owned 副本只为渲染存在,渲染完即销毁;
    // SettingsEditState 保留原 Input 等下一次 handle_key 修改 buffer/cursor。
    let input_view = input
        .focused(focused)
        .fg(colors::FG_PRIMARY);
    let bordered = Border::only_bottom()
        .fg(border_color)
        .child(
            hstack().gap(0)
                .child_sized(Text::new("  "), 2)
                .child_flex(input_view, 1.0),
        );
    vstack().gap(0)
        .child_sized(label_line, 1)
        .child_sized(bordered, 2)
}

/// 字段块·横向选择器(Protocol 字段专用):`‹ openai ›` 形态,focused 时
/// 高亮箭头 + 加宽视觉权重,告诉用户"按 ←/→ 切",其他键无效。
fn field_block_choice(label: &str, choice_label: &str, focused: bool) -> revue::widget::Stack {
    let label_color = if focused { colors::E_AMBER } else { colors::FG_SECONDARY };
    let border_color = if focused { colors::E_AMBER } else { colors::BORDER };
    let arrow_color = if focused { colors::E_AMBER } else { colors::FG_TRACE };
    let value_color = if focused { colors::FG_PRIMARY } else { colors::FG_SECONDARY };
    let label_line = hstack().gap(0).child_flex(
        Text::new(format!("  {}", label)).fg(label_color),
        1.0,
    );
    let value_text = format!("  ‹ {} ›", choice_label);
    let arrow_hint = if focused { "  ←/→ to change  " } else { "" };
    let arrow_hint_w = arrow_hint.chars().count() as u16;
    let value_w = value_text.chars().count() as u16;
    let value_row = hstack().gap(0)
        .child_sized(Text::new(value_text).fg(value_color), value_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(arrow_hint).fg(arrow_color), arrow_hint_w);
    let bordered = Border::only_bottom().fg(border_color).child(value_row);
    vstack().gap(0)
        .child_sized(label_line, 1)
        .child_sized(bordered, 2)
}

fn model_row(
    m: &agendao_client::ProviderModelInfo,
    selected: bool,
    pane_focused: bool,
) -> revue::widget::Stack {
    // 行:`{marker} {name}    {ctx}K context   🔗 ✎ ✕`
    // selected = true:`▸ ` 前缀 + name 高亮(focused 时 ACCENT_AMBER,否则 FG_SECONDARY)
    // selected = false:`  ` 前缀对齐(占位等宽)+ name 默认色
    //
    // pane_focused 区分"Details 在焦点 → 选中色显眼"与"Details 失焦 → 选中淡显"
    // (金律·成形语法:focus 信号通过颜色权威而非位置变化传达,避免视觉跳动)。
    let name = if m.name.is_empty() { m.id.clone() } else { m.name.clone() };
    let (marker, name_color) = if selected {
        let color = if pane_focused { colors::E_AMBER } else { colors::FG_SECONDARY };
        ("▸ ", color)
    } else {
        ("  ", colors::FG_PRIMARY)
    };
    let ctx_label = match m.context_window {
        Some(n) if n > 0 => format!("{}K context", n / 1000),
        _ => "—".to_string(),
    };
    let name_str = format!("{}{}", marker, name);
    let name_w = name_str.chars().count() as u16;
    let ctx_w = ctx_label.chars().count() as u16;
    hstack().gap(0)
        .child_sized(Text::new(name_str).fg(name_color), name_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(ctx_label).fg(colors::FG_MUTED), ctx_w)
        .child_sized(Text::new("   "), 3)
        .child_sized(Text::new("🔗").fg(colors::FG_TRACE), 2)
        .child_sized(Text::new(" "), 1)
        .child_sized(Text::new("✎").fg(colors::FG_TRACE), 1)
        .child_sized(Text::new(" "), 1)
        .child_sized(Text::new("✕").fg(colors::FG_TRACE), 1)
        .child_sized(Text::new("  "), 2)
}

/// 把 server 端 protocol 标识(`openai` / `anthropic` / ...)映射成用户能读的
/// "协议族 + 契约提示"标签。语义对应 server 端 `protocol_to_npm` 反向映射
/// (provider.rs:582),保持单点权威(土律·第四条)。
///
/// 未识别值原样返回(不假装),便于未来加新 protocol 时灰度上线。
fn protocol_display_label(protocol: &str) -> &str {
    match protocol {
        "openai" => "openai-compatible (/v1/chat/completions)",
        "anthropic" => "anthropic (/v1/messages)",
        "google" => "google generative-ai",
        "bedrock" => "amazon bedrock",
        "vertex" => "google vertex-ai",
        "openrouter" => "openrouter (openai-compatible)",
        "perplexity" => "perplexity",
        "github-copilot" => "github copilot",
        "gitlab" => "gitlab duo",
        other => other,
    }
}

fn footer_hint_editing(
    focused: bool,
    has_selected_model: bool,
    editing_active: bool,
) -> revue::widget::Stack {
    // 三状态:editing 中显示 Tab/Enter/Esc 编辑流;Details focused + 有 model 选中
    // → m/e/d;其他 → 通用 Tab/↑/↓。单点权威:每个状态对应唯一一行 hint。
    let active_color = if focused { colors::FG_SECONDARY } else { colors::FG_TRACE };
    let line = if editing_active {
        "  Tab: Next field   ←/→: Protocol   Enter: Save   Esc: Cancel"
    } else if focused && has_selected_model {
        "  ↑/↓: Model   m: Add  e: Edit  d: Delete   Tab: Pane   Esc: Back"
    } else if focused {
        "  ↑/↓: Browse models   m: Add model   Tab: Pane   Esc: Back"
    } else {
        "  Tab: Cycle Panes   ↑/↓: Navigate   Enter: Select   Esc: Back"
    };
    let w = line.chars().count() as u16;
    hstack().gap(0)
        .child_sized(Text::new(line).fg(active_color), w)
        .child_flex(Text::new(""), 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SettingsScreen build + render 不 panic,空 providers 也能画(空态)。
    #[test]
    fn render_empty_does_not_panic() {
        let store = AppStore::new();
        store.navigate_settings();
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None);
        stack.render(&mut ctx);
        // 标题行应能命中(顶呼吸 1 行 + 标题在 y=1):"⚙ Preferences" 第一个非空字符。
        let cell = buf.get(2, 1).unwrap();
        assert!(cell.symbol == ' ' || cell.symbol == '⚙');
    }

    /// SettingsScreen build + render 在有 providers 时正常,buffer 含内容。
    #[test]
    fn render_with_providers_shows_marker() {
        let store = AppStore::new();
        store.navigate_settings();
        store.providers.set(vec![agendao_client::ProviderInfo {
            id: "p1".to_string(),
            name: "Provider One".to_string(),
            models: vec![],
            base_url: Some("https://api.example.com".to_string()),
            protocol: Some("openai".to_string()),
        }]);
        store.settings_selected_provider.set(Some("p1".to_string()));
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None);
        stack.render(&mut ctx);
        let any_content = (0..120).any(|x| buf.get(x, 1).map(|c| c.symbol != ' ').unwrap_or(false));
        assert!(any_content, "expected at least some text on title row");
    }

    /// 关键回归测试:14 个 provider + pane_height=12 + selected=13(最后一个)。
    /// 不接滑窗会被 revue 静默裁切,只显示前 7 项;接了滑窗后选中钉底,
    /// 末尾的 "P14" 必须出现在可见区域内。
    fn collect_row(buf: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .filter_map(|x| buf.get(x, y).map(|c| c.symbol))
            .collect()
    }

    #[test]
    fn providers_pane_viewport_follows_selection_to_bottom() {
        let store = AppStore::new();
        store.navigate_settings();
        let mut providers = Vec::with_capacity(14);
        for i in 1..=14 {
            providers.push(agendao_client::ProviderInfo {
                id: format!("p{}", i),
                name: format!("P{}", i),
                models: vec![],
                base_url: None,
                protocol: None,
            });
        }
        store.providers.set(providers);
        store.settings_selected_provider.set(Some("p14".to_string()));
        // pane_height=12 → visible_rows = 12 - 5 = 7;total=14 > 7,
        // selected_idx=13(最后一个,selected+1>=total) → 钉底 start = 14-7 = 7,
        // 数据行可见 = P8..P14。
        let mut buf = Buffer::new(120, 12);
        let area = Rect::new(0, 0, 120, 12);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 12, None);
        stack.render(&mut ctx);

        // 收 6 行内容(从 y=3 起是数据行起点:顶呼吸 1 + 标题 1 + blank 1)。
        let rows: Vec<String> = (0..12).map(|y| collect_row(&buf, y, 120)).collect();
        let merged = rows.join("\n");
        // 末尾 P14 必须可见(被选中,钉底跟随)。
        assert!(merged.contains("P14"), "P14 (selected) must be visible:\n{}", merged);
        // P13 也必须可见(钉底后 selected 上一行)。
        assert!(merged.contains("P13"), "P13 must be visible:\n{}", merged);
        // P1 不应可见(已滑出视野)。
        // 用 word-boundary 兜底:P10/P11/P12... 都包含 "P1" substring。
        let has_p1_token = rows.iter().any(|r| {
            r.contains(" P1 ") || r.contains("▸ P1 ") || r.contains("◇ P1 ")
        });
        assert!(!has_p1_token, "P1 must be scrolled off when selecting P14:\n{}", merged);
        // 位置指示 "14/14" 应在 footer "+ Add provider" 行右侧。
        assert!(merged.contains("14/14"), "position indicator 14/14 missing:\n{}", merged);
    }
}
