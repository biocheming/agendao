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
/// pub(crate)：keymap 鼠标命中与渲染共用同一组几何（金律·渲染/命中同源）。
pub(crate) const CATEGORIES_W: u16 = 22;
pub(crate) const PROVIDERS_W: u16 = 28;
pub(crate) const VLINE_W: u16 = 1;
/// MCP/Skills 列表栏宽（build_named_list_pane 用）。
pub(crate) const LIST_COL_W: u16 = 28;

/// 所有 pane 统一骨架的数据首行 y（顶呼吸 1 + 标题 1 + 空 1）。
pub(crate) const PANE_FIRST_ROW_Y: u16 = 3;
/// General body 首个 toggle 行 y（数据首行 + working dir 1 + 空 1），行距 2（行+desc）。
pub(crate) const GENERAL_FIRST_ROW_Y: u16 = 5;
pub(crate) const GENERAL_ROW_STRIDE: u16 = 2;
/// Details 编辑表单字段块 y（块高 3 + 块间空 1，Edit 模式从 BaseUrl 起）：
/// Add 模式 Name@3 / BaseUrl@7 / Protocol@11 / ApiKey@15；Edit 模式顺移 -4。
pub(crate) const EDIT_FIELD_BLOCK_Y: u16 = 3;
pub(crate) const EDIT_FIELD_BLOCK_STRIDE: u16 = 4;

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
        cursor_on: bool,
    ) -> revue::widget::Stack {
        let category = store.settings_category.get();
        let focus = store.settings_focus_pane.get();
        let cat_pane = build_categories_pane(category, focus == SettingsFocusPane::Categories);

        // 分类分派(土律·唯一编排):Categories 栏恒在左;右侧 body 按分类切换成形语法。
        //   - ModelSettings:三栏(Providers 28 + Details flex),provider/model CRUD
        //   - General:两栏,body = UI 偏好 toggle 列表
        //   - About:两栏,body = 只读版本/信息
        //   - 其余占位分类:两栏,body = "coming soon"(诚实标注,土律·第十条)
        match category {
            SettingsCategory::ModelSettings => {
                let body = build_model_settings_body(store, pane_height, edit_state, focus, cursor_on);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::General => {
                let body_focused = focus != SettingsFocusPane::Categories;
                let selected = store.settings_general_selected.get();
                let body = build_general_pane(store, body_focused, selected);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::Keybindings => {
                let body_focused = focus != SettingsFocusPane::Categories;
                let scroll = store.settings_keybindings_scroll.get();
                let body = build_keybindings_pane(body_focused, scroll, pane_height);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::McpServers => {
                let body = build_mcp_body(store, pane_height, focus);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::Skills => {
                let body = build_skills_body(store, pane_height, focus);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::Tools => {
                let body = build_tools_body(store, pane_height, focus);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::Plugins => {
                let body = build_plugins_body(store, pane_height, focus);
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(body, 1.0)
            }
            SettingsCategory::About => {
                hstack().gap(0)
                    .child_sized(cat_pane, CATEGORIES_W)
                    .child_sized(vline(), VLINE_W)
                    .child_flex(build_about_pane(), 1.0)
            }
        }
    }
}

/// Model Settings body:两栏(Providers 28 | Details flex)。从 `build()` 抽出,
/// 保持原三栏语义不变——外层再拼上 Categories 栏即成完整三栏。
fn build_model_settings_body(
    store: &AppStore,
    pane_height: u16,
    edit_state: Option<&SettingsEditState>,
    focus: SettingsFocusPane,
    cursor_on: bool,
) -> revue::widget::Stack {
    let providers = store.providers.get();
    let connected = store.providers_connected.get();
    let selected = store.settings_selected_provider.get();

    let editing_active = edit_state.is_some_and(|s| s.active);
    let is_add = edit_state.is_some_and(|s| s.is_add());

    let selected_idx = if is_add {
        Some(providers.len())
    } else {
        selected
            .as_deref()
            .and_then(|id| providers.iter().position(|p| p.id == id))
    };

    let selected_model = store.settings_selected_model.get();

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
        cursor_on,
    );

    hstack().gap(0)
        .child_sized(prov_pane, PROVIDERS_W)
        .child_sized(vline(), VLINE_W)
        .child_flex(detail_pane, 1.0)
}

// ── General 分类 body ──

/// General body:UI 偏好 toggle 列表。每行读对应 signal 显示当前值(阴阳同源:
/// 与 `execute_slash_action` 的 toggle 写路径读写同一 signal)。`focused` = body
/// 在焦点(非 Categories 栏);`selected` = 当前高亮行(`GeneralRow::ALL` 下标)。
fn build_general_pane(
    store: &AppStore,
    focused: bool,
    selected: usize,
) -> revue::widget::Stack {
    use crate::store::types::GeneralRow;

    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(title_row("  ☯ General", title_color), 1)
        .child_sized(Text::new(""), 1);

    // Working dir 只读展示(改 cwd 是更大范围,暂只读)。
    let wd = store.working_dir.get();
    let wd_line = format!("  Working dir: {}", wd);
    let wd_w = cell_w(&wd_line);
    s = s
        .child_sized(
            hstack().gap(0)
                .child_sized(Text::new(wd_line).fg(colors::FG_TRACE()), wd_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1);

    let sel = selected.min(GeneralRow::ALL.len() - 1);
    for (i, row) in GeneralRow::ALL.iter().copied().enumerate() {
        let is_sel = focused && i == sel;
        let value = general_row_value(store, row);
        s = s.child_sized(general_toggle_row(row, &value, is_sel), 1);
        s = s.child_sized(general_row_desc(row, is_sel), 1);
    }

    s.child_flex(Text::new(""), 1.0)
        .child_sized(general_footer_hint(focused), 1)
        .child_sized(Text::new(""), 1)
}

/// 读某行当前值的显示文案(bool → On/Off;Theme → 主题 label)。单点权威:
/// 值真相全在 store signal,这里只做展示映射。
fn general_row_value(store: &AppStore, row: crate::store::types::GeneralRow) -> String {
    use crate::store::types::GeneralRow;
    match row {
        GeneralRow::ShowThinking => on_off(store.show_thinking.get()),
        GeneralRow::ShowScrollbar => on_off(store.show_scrollbar.get()),
        GeneralRow::ShowHeader => on_off(store.show_header.get()),
        GeneralRow::ShowTips => on_off(store.show_tips.get()),
        GeneralRow::CompactDensity => on_off(store.compact_density.get()),
        GeneralRow::Theme => store.theme_id.get().label().to_string()
    }
}

fn on_off(v: bool) -> String {
    if v { "On".to_string() } else { "Off".to_string() }
}

/// 单个 toggle 行:`{marker} {label} ........ [{value}]`。
/// selected 行 ▸ + E_TEAL,value pill 用 On=ACCENT_GREEN / Off=FG_MUTED。
fn general_toggle_row(
    row: crate::store::types::GeneralRow,
    value: &str,
    selected: bool,
) -> revue::widget::Stack {
    let (marker, label_color) = if selected {
        ("▸", colors::E_TEAL())
    } else {
        (" ", colors::FG_PRIMARY())
    };
    let label = format!("  {} {}", marker, row.label());
    let label_w = cell_w(&label);
    // On 用绿色,dark/light 等非布尔值用青色,Off 用暗色。
    let value_color = match value {
        "On" => colors::ACCENT_GREEN(),
        "Off" => colors::FG_MUTED(),
        _ => colors::E_TEAL(),
    };
    let pill = format!("[ {} ]", value);
    let pill_w = cell_w(&pill);
    hstack().gap(0)
        .child_sized(Text::new(label).fg(label_color), label_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(pill).fg(value_color), pill_w)
        .child_sized(Text::new("  "), 2)
}

fn general_row_desc(
    row: crate::store::types::GeneralRow,
    selected: bool,
) -> revue::widget::Stack {
    let color = if selected { colors::FG_SECONDARY() } else { colors::FG_TRACE() };
    let line = format!("      {}", row.description());
    let w = cell_w(&line);
    hstack().gap(0)
        .child_sized(Text::new(line).fg(color), w)
        .child_flex(Text::new(""), 1.0)
}

fn general_footer_hint(focused: bool) -> revue::widget::Stack {
    let color = if focused { colors::FG_SECONDARY() } else { colors::FG_TRACE() };
    let line = if focused {
        "  ↑/↓: Row   Enter/Space: Toggle   Tab: Categories   Esc: Back"
    } else {
        "  Tab/Enter: Enter General   Esc: Back"
    };
    let w = cell_w(&line);
    hstack().gap(0)
        .child_sized(Text::new(line).fg(color), w)
        .child_flex(Text::new(""), 1.0)
}

// ── About 分类 body ──

fn build_about_pane() -> revue::widget::Stack {
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("  ℹ AgenDao TUI  v{}", version);
    let title_w = cell_w(&title);
    let lines: [(&str, Color); 5] = [
        ("  道纪 — Canon of Flow and Governance", colors::FG_SECONDARY()),
        ("  A terminal UI for the AgenDao agent runtime.", colors::FG_PRIMARY()),
        ("", colors::FG_PRIMARY()),
        ("  Press Ctrl+P or / for the command palette.", colors::FG_TRACE()),
        ("  Press ? for keyboard shortcuts.", colors::FG_TRACE()),
    ];
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(
            hstack().gap(0)
                .child_sized(Text::new(title).fg(colors::E_TEAL()).bold(), title_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1);
    for (line, color) in lines {
        let w = cell_w(line).max(1);
        s = s.child_sized(
            hstack().gap(0)
                .child_sized(Text::new(line).fg(color), w)
                .child_flex(Text::new(""), 1.0),
            1,
        );
    }
    s.child_flex(Text::new(""), 1.0)
        .child_sized(Text::new("  Esc: Back").fg(colors::FG_TRACE()), 1)
        .child_sized(Text::new(""), 1)
}

// ── Keybindings 分类 body ──

/// Keybindings body 的可见数据行数(整屏高 - 顶呼吸1 - 标题1 - 标题后空1 -
/// 底 hint1 - 底呼吸1 = 5)。keymap 滚动 clamp 与 screen 渲染同用此口径
/// (金律·成形语法唯一),避免"滚到看不见的行"。
pub fn keybindings_visible_rows(pane_height: u16) -> usize {
    pane_height.saturating_sub(5).max(1) as usize
}

/// Keybindings body:只读快捷键参考,数据源唯一 = `dialog::help::KEYBINDINGS`。
/// `scroll` = 首个可见 entry 下标(视窗起点),超长时 ↑/↓/PgUp/PgDn 滚动。
fn build_keybindings_pane(
    focused: bool,
    scroll: usize,
    pane_height: u16,
) -> revue::widget::Stack {
    use crate::dialog::help::{HelpEntry, KEYBINDINGS};

    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(title_row("  ⌨ Keybindings", title_color), 1)
        .child_sized(Text::new(""), 1);

    let total = KEYBINDINGS.len();
    let visible = keybindings_visible_rows(pane_height);
    let start = scroll.min(total.saturating_sub(1));
    let end = (start + visible).min(total);
    for entry in &KEYBINDINGS[start..end] {
        let row = match entry {
            HelpEntry::Section(title) => hstack().gap(0)
                .child_flex(Text::new(format!("  {}", title)).fg(colors::ACCENT_BLUE()), 1.0),
            HelpEntry::Binding(key, desc) => {
                let key_str = format!("  {:>12}", key);
                let key_w = cell_w(&key_str);
                hstack().gap(2)
                    .child_sized(Text::new(key_str).fg(colors::ACCENT_CYAN()), key_w)
                    .child_flex(Text::new((*desc).to_string()).fg(colors::FG_SECONDARY()), 1.0)
            }
        };
        s = s.child_sized(row, 1);
    }

    let more = total.saturating_sub(end);
    let hint = if focused {
        if more > 0 {
            format!("  ↑/↓/PgUp/PgDn: Scroll  (+{} below)   Tab: Categories", more)
        } else {
            "  ↑/↓: Scroll   Tab: Categories   Esc: Back".to_string()
        }
    } else {
        "  Tab/Enter: Enter Keybindings   Esc: Back".to_string()
    };
    let hint_color = if focused { colors::FG_SECONDARY() } else { colors::FG_TRACE() };
    let hint_w = cell_w(&hint);
    s.child_flex(Text::new(""), 1.0)
        .child_sized(
            hstack().gap(0)
                .child_sized(Text::new(hint).fg(hint_color), hint_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1)
}

// ── MCP Servers 分类 body ──

fn build_mcp_body(
    store: &AppStore,
    pane_height: u16,
    focus: SettingsFocusPane,
) -> revue::widget::Stack {
    let rows = store.settings_mcp.get();
    let selected = store.settings_mcp_selected.get().min(rows.len().saturating_sub(1));
    let list_focused = focus == SettingsFocusPane::Providers;
    let detail_focused = focus == SettingsFocusPane::Details;

    let list = build_named_list_pane(
        "⚔ MCP Servers",
        list_focused,
        pane_height,
        rows.len(),
        selected,
        |i| {
            let r = &rows[i];
            let marker = if i == selected { "▸" } else { "◇" };
            // 被禁行暗色（FG_TRACE）——视觉与 [ Off ] pill 同源。
            let color = if i == selected {
                colors::E_TEAL()
            } else if !r.enabled {
                colors::FG_TRACE()
            } else {
                colors::FG_PRIMARY()
            };
            let dot = if r.is_connected() { "●" } else { "─" };
            let dot_color = if r.is_connected() {
                colors::ACCENT_GREEN()
            } else {
                colors::FG_TRACE()
            };
            let prefix = format!(" {} {}", marker, r.name);
            let prefix_w = cell_w(&prefix);
            let (pill, pill_color) = on_off_pill(r.enabled);
            hstack().gap(0)
                .child_sized(Text::new(prefix).fg(color), prefix_w)
                .child_flex(Text::new(""), 1.0)
                .child_sized(Text::new(pill).fg(pill_color), cell_w(pill))
                .child_sized(Text::new(" "), 1)
                .child_sized(Text::new(dot).fg(dot_color), 1)
                .child_sized(Text::new(" "), 1)
        },
        if list_focused {
            "  ↑/↓  a/e  t: On/Off  x: Del  c/d"
        } else {
            "  Tab: Enter list"
        },
    );

    let detail = if let Some(r) = rows.get(selected) {
        build_mcp_detail(r, detail_focused)
    } else {
        build_empty_detail("No MCP servers configured — press a to add", detail_focused)
    };

    hstack().gap(0)
        .child_sized(list, LIST_COL_W)
        .child_sized(vline(), VLINE_W)
        .child_flex(detail, 1.0)
}

fn build_mcp_detail(r: &crate::store::types::SettingsMcpRow, focused: bool) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let pill = if r.is_connected() { " Connected " } else { " Disconnected " };
    let pill_color = if r.is_connected() {
        colors::ACCENT_GREEN()
    } else {
        colors::FG_MUTED()
    };
    let header = format!("  ⚔ {}", r.name);
    let header_w = cell_w(&header);
    let pill_w = cell_w(&pill);
    let (on_pill, on_pill_color) = on_off_pill(r.enabled);
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(
            hstack().gap(1)
                .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
                .child_sized(Text::new(on_pill).fg(on_pill_color).bold(), cell_w(on_pill))
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(field_block("Status", &r.status, colors::FG_PRIMARY(), ""), 3)
        .child_sized(Text::new(""), 1)
        .child_sized(field_block("Transport", &r.transport, colors::FG_PRIMARY(), ""), 3);
    // transport 对应端点字段：local → command；remote → url；unknown → 两者皆无。
    if let Some(ref cmd) = r.command {
        s = s
            .child_sized(Text::new(""), 1)
            .child_sized(field_block("Command", cmd, colors::FG_PRIMARY(), ""), 3);
    }
    if let Some(ref url) = r.url {
        s = s
            .child_sized(Text::new(""), 1)
            .child_sized(field_block("URL", url, colors::FG_PRIMARY(), ""), 3);
    }
    s = s
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block(
                "Enabled",
                if r.enabled { "on (config.mcp)" } else { "off (config.mcp)" },
                if r.enabled { colors::FG_PRIMARY() } else { colors::FG_MUTED() },
                "t: toggle",
            ),
            3,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Tools", &r.tools.to_string(), colors::FG_PRIMARY(), ""),
            3,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Resources", &r.resources.to_string(), colors::FG_PRIMARY(), ""),
            3,
        );
    if let Some(ref err) = r.error {
        s = s
            .child_sized(Text::new(""), 1)
            .child_sized(field_block("Error", err, colors::ACCENT_RED(), ""), 3);
    }
    let hint = if focused {
        "  c: Connect   d: Disconnect   t: On/Off   e: Edit   x: Delete   Tab: List   Esc: Back"
    } else {
        "  Tab: Detail pane"
    };
    s.child_flex(Text::new(""), 1.0)
        .child_sized(
            Text::new(hint).fg(if focused {
                colors::FG_SECONDARY()
            } else {
                colors::FG_TRACE()
            }),
            1,
        )
        .child_sized(Text::new(""), 1)
}

// ── Plugins 分类 body ──

fn build_plugins_body(
    store: &AppStore,
    pane_height: u16,
    focus: SettingsFocusPane,
) -> revue::widget::Stack {
    let rows = store.settings_plugins.get();
    let selected = store
        .settings_plugins_selected
        .get()
        .min(rows.len().saturating_sub(1));
    let list_focused = focus == SettingsFocusPane::Providers;
    let detail_focused = focus == SettingsFocusPane::Details;

    let list = build_named_list_pane(
        "⧉ Plugins",
        list_focused,
        pane_height,
        rows.len(),
        selected,
        |i| {
            let r = &rows[i];
            let is_sel = i == selected;
            let marker = if is_sel { "▸" } else { " " };
            let (tag, tag_color) = if r.managed {
                ("[M]", colors::ACCENT_CYAN())
            } else {
                ("[D]", colors::E_AMBER())
            };
            // 被禁行暗色（FG_TRACE）——视觉与 [ Off ] pill 同源。
            let name_color = if is_sel {
                colors::E_TEAL()
            } else if r.disabled {
                colors::FG_TRACE()
            } else {
                colors::FG_PRIMARY()
            };
            let prefix = format!(" {} {} ", marker, tag);
            let prefix_w = cell_w(&prefix);
            let name_w = cell_w(&r.name);
            let (pill, pill_color) = on_off_pill(!r.disabled);
            hstack().gap(0)
                .child_sized(Text::new(prefix).fg(tag_color), prefix_w)
                .child_sized(Text::new(r.name.clone()).fg(name_color), name_w)
                .child_flex(Text::new(""), 1.0)
                .child_sized(Text::new(pill).fg(pill_color), cell_w(pill))
                .child_sized(Text::new("  "), 2)
        },
        if list_focused {
            "  ↑/↓  a: Install  t: On/Off  x: Del"
        } else {
            "  Tab: Enter list"
        },
    );

    let detail = if let Some(r) = rows.get(selected) {
        build_plugin_detail(r, detail_focused)
    } else {
        build_empty_detail("No plugins installed — press a to install", detail_focused)
    };

    hstack().gap(0)
        .child_sized(list, LIST_COL_W)
        .child_sized(vline(), VLINE_W)
        .child_flex(detail, 1.0)
}

fn build_plugin_detail(
    r: &crate::store::types::SettingsPluginRow,
    focused: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let header = format!("  ⧉ {}", r.name);
    let header_w = cell_w(&header);
    let (pill, pill_color) = on_off_pill(!r.disabled);
    let pill_w = cell_w(pill);
    let source_label = if r.managed {
        "managed (config.plugin)"
    } else {
        "discovered (directory scan)"
    };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(
            hstack().gap(1)
                .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(field_block("Type", &r.plugin_type, colors::FG_PRIMARY(), ""), 3)
        .child_sized(Text::new(""), 1)
        .child_sized(field_block("Source", source_label, colors::FG_PRIMARY(), ""), 3)
        .child_sized(Text::new(""), 1)
        // 安装途径：server `build_plugin_list_entries` 单点权威标签。
        .child_sized(field_block("Origin", &r.origin, colors::FG_PRIMARY(), ""), 3);
    if let Some(ref path) = r.path {
        s = s
            .child_sized(Text::new(""), 1)
            .child_sized(field_block("Path", path, colors::FG_PRIMARY(), ""), 3);
    }
    s = s.child_sized(Text::new(""), 1).child_sized(
        field_block(
            "Version",
            r.version.as_deref().unwrap_or("(unspecified)"),
            colors::FG_PRIMARY(),
            "",
        ),
        3,
    );
    let hint = if focused {
        if r.managed {
            "  t: On/Off   x/d: Delete   Tab: List   Esc: Back"
        } else {
            "  t: On/Off   discovered: delete files from origin dir   Tab: List   Esc: Back"
        }
    } else {
        "  Tab: Detail pane"
    };
    s.child_flex(Text::new(""), 1.0)
        .child_sized(
            Text::new(hint).fg(if focused {
                colors::FG_SECONDARY()
            } else {
                colors::FG_TRACE()
            }),
            1,
        )
        .child_sized(Text::new(""), 1)
}

// ── Skills 分类 body ──

fn build_skills_body(
    store: &AppStore,
    pane_height: u16,
    focus: SettingsFocusPane,
) -> revue::widget::Stack {
    use crate::store::types::{flatten_settings_skill_rows, SettingsSkillLine, SettingsSkillRow};

    let rows = store.settings_skills.get();
    let collapsed = store.settings_skills_collapsed.get();
    let lines = flatten_settings_skill_rows(&rows, &collapsed);
    let selected = store
        .settings_skills_selected
        .get()
        .min(lines.len().saturating_sub(1));
    let list_focused = focus == SettingsFocusPane::Providers;
    let detail_focused = focus == SettingsFocusPane::Details;

    let list = build_named_list_pane(
        "✧ Skills",
        list_focused,
        pane_height,
        lines.len(),
        selected,
        |i| match &lines[i] {
            SettingsSkillLine::Category {
                name,
                count,
                collapsed,
                disabled_count,
            } => {
                // 类目头：选中 ▸ + E_TEAL；未选中 FG_SECONDARY。
                // 折叠glyph ▶/▼ 独立于选中 marker，避免双 ▸ 歧义。
                let is_sel = i == selected;
                let marker = if is_sel { "▸" } else { " " };
                let fold = if *collapsed { "▶" } else { "▼" };
                let color = if is_sel {
                    colors::E_TEAL()
                } else {
                    colors::ACCENT_BLUE()
                };
                let line = format!(" {} {} {} ({})", marker, fold, name, count);
                let line_w = cell_w(&line);
                let (pill, pill_color) = group_switch_pill(*count, *disabled_count);
                let pill_w = cell_w(&pill);
                hstack().gap(0)
                    .child_sized(Text::new(line).fg(color), line_w)
                    .child_flex(Text::new(""), 1.0)
                    .child_sized(Text::new(pill).fg(pill_color), pill_w)
                    .child_sized(Text::new("  "), 2)
            }
            SettingsSkillLine::Row(src) => {
                let r = &rows[*src];
                let marker = if i == selected { "▸" } else { " " };
                let (tag, tag_color) = match r {
                    SettingsSkillRow::Proposal { .. } => ("[P]", colors::E_AMBER()),
                    SettingsSkillRow::Catalog { .. } => ("[S]", colors::ACCENT_CYAN()),
                };
                let disabled = r.is_disabled();
                // 被禁行暗色（FG_TRACE）——视觉与 Off pill 同源。
                let name_color = if i == selected {
                    colors::E_TEAL()
                } else if disabled {
                    colors::FG_TRACE()
                } else {
                    colors::FG_PRIMARY()
                };
                // 数据行缩进 2 列，挂在类目头之下（树形层级）。
                let prefix = format!("   {} {} ", marker, tag);
                let prefix_w = cell_w(&prefix);
                let name = r.label().to_string();
                let name_w = cell_w(&name);
                let mut row = hstack().gap(0)
                    .child_sized(Text::new(prefix).fg(tag_color), prefix_w)
                    .child_sized(Text::new(name).fg(name_color), name_w)
                    .child_flex(Text::new(""), 1.0);
                // proposal 行无开关（由 a/r 裁决）；catalog 行带 On/Off pill。
                if !r.is_proposal() {
                    let (pill, pill_color) = on_off_pill(!disabled);
                    row = row
                        .child_sized(Text::new(pill).fg(pill_color), cell_w(pill))
                        .child_sized(Text::new("  "), 2);
                }
                row
            }
        },
        if list_focused {
            "  ↑/↓  Enter: Fold  t: On/Off  x: Del"
        } else {
            "  Tab: Enter list"
        },
    );

    let detail = match lines.get(selected) {
        Some(SettingsSkillLine::Category {
            name,
            count,
            collapsed,
            disabled_count,
        }) => build_skill_category_detail(name, *count, *collapsed, *disabled_count, detail_focused),
        Some(SettingsSkillLine::Row(src)) => build_skill_detail(&rows[*src], detail_focused),
        None => build_empty_detail("No skills or proposals", detail_focused),
    };

    hstack().gap(0)
        .child_sized(list, LIST_COL_W)
        .child_sized(vline(), VLINE_W)
        .child_flex(detail, 1.0)
}

/// 行尾 On/Off 开关 pill 文案+颜色（鼠标命中区与渲染同源：keymap 按行尾命中）。
/// enabled → 绿 `[ On ]`；disabled → 暗 `[ Off ]`。
fn on_off_pill(enabled: bool) -> (&'static str, Color) {
    if enabled {
        ("[ On ]", colors::ACCENT_GREEN())
    } else {
        ("[ Off ]", colors::FG_MUTED())
    }
}

/// 类目头聚合开关 pill：全禁 `[ Off ]`；全启 `[ On ]`；部分禁 `[n/m]`（琥珀）。
fn group_switch_pill(count: usize, disabled_count: usize) -> (String, Color) {
    if count > 0 && disabled_count == count {
        ("[ Off ]".to_string(), colors::FG_MUTED())
    } else if disabled_count == 0 {
        ("[ On ]".to_string(), colors::ACCENT_GREEN())
    } else {
        (
            format!("[{}/{}]", count - disabled_count, count),
            colors::E_AMBER(),
        )
    }
}

/// 类目头 detail：组名 + 行数 + 聚合启停态（`t` 整组启停 = `name/*` 通配）。
fn build_skill_category_detail(
    name: &str,
    count: usize,
    collapsed: bool,
    disabled_count: usize,
    focused: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let header = format!("  ✧ {}", name);
    let header_w = cell_w(&header);
    let state = if collapsed { " Collapsed " } else { " Expanded " };
    let state_w = cell_w(state);
    let (pill, pill_color) = group_switch_pill(count, disabled_count);
    let pill_w = cell_w(&pill);
    let switch_state = if count > 0 && disabled_count == count {
        format!("All off ({}/*)", name)
    } else if disabled_count == 0 {
        "On".to_string()
    } else {
        format!("{}/{} off", disabled_count, count)
    };
    let hint = if focused {
        "  Enter/Space: Fold   t: Toggle all   ↑/↓: Move   Tab: List   Esc: Back"
    } else {
        "  Tab: Detail pane"
    };
    vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(
            hstack().gap(1)
                .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                .child_sized(Text::new(state).fg(colors::FG_MUTED()).bold(), state_w)
                .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Skills", &count.to_string(), colors::FG_PRIMARY(), ""),
            3,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Switch", &switch_state, colors::FG_PRIMARY(), "t toggles name/*"),
            3,
        )
        .child_flex(Text::new(""), 1.0)
        .child_sized(
            Text::new(hint).fg(if focused {
                colors::FG_SECONDARY()
            } else {
                colors::FG_TRACE()
            }),
            1,
        )
        .child_sized(Text::new(""), 1)
}

/// 安装途径推断：`SkillCatalogEntry` 没有 source/root 字段，按 location 前缀/
/// 路径组件推断（诚实标注 inferred——土律·第十条）。
///
/// 推断顺序（先绝对前缀后组件匹配）：
///   1. `$AGENDAO_HOME/skill(s)`（默认 ~/.agendao/skills）→ user 级
///   2. `~/.agents/skills`（跨工具共享目录）→ shared 级
///   3. 路径组件含 `.agendao/skills` 或 `.agents/skills` → 项目级
///   4. 其余 → config `skill_paths` 外部目录（含 hub 安装）
fn skill_install_source(location: &str) -> String {
    let loc = std::path::Path::new(location);
    let agendao_home = agendao_util::agendao_home();
    if loc.starts_with(agendao_home.join("skills")) || loc.starts_with(agendao_home.join("skill")) {
        return "user (~/.agendao/skills)".to_string();
    }
    if let Some(home) = dirs::home_dir() {
        if loc.starts_with(home.join(".agents/skills")) {
            return "shared (~/.agents/skills)".to_string();
        }
    }
    let components: Vec<&str> = loc
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    for pair in components.windows(2) {
        match (pair[0], pair[1]) {
            (".agendao", "skills") | (".agendao", "skill") => {
                return "project (.agendao/skills)".to_string();
            }
            (".agents", "skills") => {
                return "project (.agents/skills)".to_string();
            }
            _ => {}
        }
    }
    "external (config skill_paths)".to_string()
}

fn build_skill_detail(
    r: &crate::store::types::SettingsSkillRow,
    focused: bool,
) -> revue::widget::Stack {
    use crate::store::types::SettingsSkillRow;
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0).child_sized(Text::new(""), 1);
    match r {
        SettingsSkillRow::Catalog {
            name,
            description,
            location,
            category,
            writable,
            disabled,
        } => {
            let header = format!("  ✧ {}", name);
            let header_w = cell_w(&header);
            let (pill, pill_color) = on_off_pill(!disabled);
            let pill_w = cell_w(pill);
            s = s
                .child_sized(
                    hstack().gap(1)
                        .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                        .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
                        .child_flex(Text::new(""), 1.0),
                    1,
                )
                .child_sized(Text::new(""), 1)
                .child_sized(
                    field_block(
                        "Description",
                        if description.is_empty() {
                            "(none)"
                        } else {
                            description
                        },
                        colors::FG_PRIMARY(),
                        "",
                    ),
                    3,
                )
                .child_sized(Text::new(""), 1)
                .child_sized(field_block("Location", location, colors::FG_PRIMARY(), ""), 3)
                .child_sized(Text::new(""), 1)
                // 安装途径：entry 无 source/root 字段，按 location 前缀推断并注明。
                .child_sized(
                    field_block(
                        "Source",
                        &skill_install_source(location),
                        colors::FG_PRIMARY(),
                        "inferred from location",
                    ),
                    3,
                )
                .child_sized(Text::new(""), 1)
                .child_sized(
                    field_block(
                        "Category",
                        category.as_deref().unwrap_or("(none)"),
                        colors::FG_PRIMARY(),
                        "",
                    ),
                    3,
                )
                .child_sized(Text::new(""), 1)
                .child_sized(
                    field_block(
                        "Writable",
                        if *writable {
                            "yes"
                        } else {
                            "no — delete disabled (not in project .agendao/skills)"
                        },
                        colors::FG_PRIMARY(),
                        "",
                    ),
                    3,
                );
            let hint = if focused {
                if *writable {
                    "  t: On/Off   x/d: Delete   Tab: List   Esc: Back"
                } else {
                    "  t: On/Off   read-only: delete via install source   Tab: List   Esc: Back"
                }
            } else {
                "  Tab: Detail pane"
            };
            s.child_flex(Text::new(""), 1.0)
                .child_sized(
                    Text::new(hint).fg(if focused {
                        colors::FG_SECONDARY()
                    } else {
                        colors::FG_TRACE()
                    }),
                    1,
                )
                .child_sized(Text::new(""), 1)
        }
        SettingsSkillRow::Proposal {
            id,
            title,
            status,
            kind,
        } => {
            let header = format!("  ✧ {}", title);
            let header_w = cell_w(&header);
            s = s
                .child_sized(
                    hstack().gap(0)
                        .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                        .child_flex(Text::new(""), 1.0)
                        .child_sized(Text::new(" Proposal ").fg(colors::E_AMBER()).bold(), 11),
                    1,
                )
                .child_sized(Text::new(""), 1)
                .child_sized(field_block("Status", status, colors::FG_PRIMARY(), ""), 3)
                .child_sized(Text::new(""), 1)
                .child_sized(field_block("Kind", kind, colors::FG_PRIMARY(), ""), 3)
                .child_sized(Text::new(""), 1)
                .child_sized(field_block("Id", id, colors::FG_MUTED(), ""), 3);
            // Proposal 动作 hint 常显（不再仅 focused 时展示）：鼠标命中区
            // 需要可见目标（金律·成形/命中同源）；颜色仍随 focus 分阶。
            let hint = "  a: Approve   r: Reject   Tab: List   Esc: Back";
            s.child_flex(Text::new(""), 1.0)
                .child_sized(
                    Text::new(hint).fg(if focused {
                        colors::FG_SECONDARY()
                    } else {
                        colors::FG_TRACE()
                    }),
                    1,
                )
                .child_sized(Text::new(""), 1)
        }
    }
}

// ── Tools 分类 body ──

fn build_tools_body(
    store: &AppStore,
    pane_height: u16,
    focus: SettingsFocusPane,
) -> revue::widget::Stack {
    use crate::store::types::{flatten_settings_tool_rows, SettingsToolLine};

    let rows = store.settings_tools.get();
    let collapsed = store.settings_tools_collapsed.get();
    let lines = flatten_settings_tool_rows(&rows, &collapsed);
    let selected = store
        .settings_tools_selected
        .get()
        .min(lines.len().saturating_sub(1));
    let list_focused = focus == SettingsFocusPane::Providers;
    let detail_focused = focus == SettingsFocusPane::Details;

    let list = build_named_list_pane(
        "⛏ Tools",
        list_focused,
        pane_height,
        lines.len(),
        selected,
        |i| match &lines[i] {
            SettingsToolLine::Category {
                name,
                count,
                collapsed,
                disabled_count,
            } => {
                let is_sel = i == selected;
                let marker = if is_sel { "▸" } else { " " };
                let fold = if *collapsed { "▶" } else { "▼" };
                let color = if is_sel {
                    colors::E_TEAL()
                } else {
                    colors::ACCENT_BLUE()
                };
                let line = format!(" {} {} {} ({})", marker, fold, name, count);
                let line_w = cell_w(&line);
                let (pill, pill_color) = group_switch_pill(*count, *disabled_count);
                let pill_w = cell_w(&pill);
                hstack().gap(0)
                    .child_sized(Text::new(line).fg(color), line_w)
                    .child_flex(Text::new(""), 1.0)
                    .child_sized(Text::new(pill).fg(pill_color), pill_w)
                    .child_sized(Text::new("  "), 2)
            }
            SettingsToolLine::Row(src) => {
                let r = &rows[*src];
                let is_sel = i == selected;
                let marker = if is_sel { "▸" } else { " " };
                // protected（facade/bridge）行：锁定标记 + 无开关。
                let (tag, tag_color) = if r.protected {
                    ("[*]", colors::FG_MUTED())
                } else {
                    ("[T]", colors::ACCENT_CYAN())
                };
                let name_color = if is_sel {
                    colors::E_TEAL()
                } else if r.disabled {
                    colors::FG_TRACE()
                } else {
                    colors::FG_PRIMARY()
                };
                let prefix = format!("   {} {} ", marker, tag);
                let prefix_w = cell_w(&prefix);
                let name_w = cell_w(&r.id);
                let mut row = hstack().gap(0)
                    .child_sized(Text::new(prefix).fg(tag_color), prefix_w)
                    .child_sized(Text::new(r.id.clone()).fg(name_color), name_w)
                    .child_flex(Text::new(""), 1.0);
                if !r.protected {
                    let (pill, pill_color) = on_off_pill(!r.disabled);
                    row = row
                        .child_sized(Text::new(pill).fg(pill_color), cell_w(pill))
                        .child_sized(Text::new("  "), 2);
                }
                row
            }
        },
        if list_focused {
            "  ↑/↓  Enter: Fold  t: On/Off"
        } else {
            "  Tab: Enter list"
        },
    );

    let detail = match lines.get(selected) {
        Some(SettingsToolLine::Category {
            name,
            count,
            collapsed,
            disabled_count,
        }) => build_tool_category_detail(name, *count, *collapsed, *disabled_count, detail_focused),
        Some(SettingsToolLine::Row(src)) => build_tool_detail(&rows[*src], detail_focused),
        None => build_empty_detail("No tools", detail_focused),
    };

    hstack().gap(0)
        .child_sized(list, LIST_COL_W)
        .child_sized(vline(), VLINE_W)
        .child_flex(detail, 1.0)
}

/// family 类目头 detail：组名 + 行数 + 聚合启停态（`t` 整组启停 = `family/*`）。
fn build_tool_category_detail(
    name: &str,
    count: usize,
    collapsed: bool,
    disabled_count: usize,
    focused: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let header = format!("  ⛏ {}", name);
    let header_w = cell_w(&header);
    let state = if collapsed { " Collapsed " } else { " Expanded " };
    let state_w = cell_w(state);
    let (pill, pill_color) = group_switch_pill(count, disabled_count);
    let pill_w = cell_w(&pill);
    let switch_state = if count > 0 && disabled_count == count {
        format!("All off ({}/*)", name)
    } else if disabled_count == 0 {
        "On".to_string()
    } else {
        format!("{}/{} off", disabled_count, count)
    };
    let hint = if focused {
        "  Enter/Space: Fold   t: Toggle all   ↑/↓: Move   Tab: List   Esc: Back"
    } else {
        "  Tab: Detail pane"
    };
    vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(
            hstack().gap(1)
                .child_sized(Text::new(header).fg(title_color).bold(), header_w)
                .child_sized(Text::new(state).fg(colors::FG_MUTED()).bold(), state_w)
                .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
                .child_flex(Text::new(""), 1.0),
            1,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Tools", &count.to_string(), colors::FG_PRIMARY(), ""),
            3,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block("Switch", &switch_state, colors::FG_PRIMARY(), "t toggles family/*"),
            3,
        )
        .child_flex(Text::new(""), 1.0)
        .child_sized(
            Text::new(hint).fg(if focused {
                colors::FG_SECONDARY()
            } else {
                colors::FG_TRACE()
            }),
            1,
        )
        .child_sized(Text::new(""), 1)
}

fn build_tool_detail(
    r: &crate::store::types::SettingsToolRow,
    focused: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let header = format!("  ⛏ {}", r.id);
    let header_w = cell_w(&header);
    let mut header_row = hstack().gap(1)
        .child_sized(Text::new(header).fg(title_color).bold(), header_w);
    if r.protected {
        header_row = header_row.child_sized(
            Text::new(" protected ").fg(colors::E_AMBER()).bold(),
            11,
        );
    } else {
        let (pill, pill_color) = on_off_pill(!r.disabled);
        header_row = header_row.child_sized(
            Text::new(pill).fg(pill_color).bold(),
            cell_w(pill),
        );
    }
    header_row = header_row.child_flex(Text::new(""), 1.0);

    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(header_row, 1)
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block(
                "Description",
                if r.description.is_empty() {
                    "(none)"
                } else {
                    r.description.as_str()
                },
                colors::FG_PRIMARY(),
                "",
            ),
            3,
        )
        .child_sized(Text::new(""), 1)
        .child_sized(
            field_block(
                "Family",
                r.family.as_deref().unwrap_or("(none)"),
                colors::FG_PRIMARY(),
                "",
            ),
            3,
        );
    if r.protected {
        s = s
            .child_sized(Text::new(""), 1)
            .child_sized(
                field_block(
                    "Protected",
                    "facade/bridge tool — the model reaches other tools and skill content through it; disabling is rejected by the registry filter",
                    colors::E_AMBER(),
                    "",
                ),
                3,
            );
    }
    let hint = if focused {
        if r.protected {
            "  protected: cannot be disabled   Tab: List   Esc: Back"
        } else {
            "  t: On/Off (registry rebuilds, effective immediately)   Tab: List   Esc: Back"
        }
    } else {
        "  Tab: Detail pane"
    };
    s.child_flex(Text::new(""), 1.0)
        .child_sized(
            Text::new(hint).fg(if focused {
                colors::FG_SECONDARY()
            } else {
                colors::FG_TRACE()
            }),
            1,
        )
        .child_sized(Text::new(""), 1)
}

fn build_empty_detail(msg: &str, focused: bool) -> revue::widget::Stack {
    let color = if focused {
        colors::FG_SECONDARY()
    } else {
        colors::FG_MUTED()
    };
    vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(Text::new(format!("  {}", msg)).fg(color), 1)
        .child_flex(Text::new(""), 1.0)
}

/// 通用左列表栏:标题 + 滑窗行 + footer。`row_builder(i)` 返回该行 Stack。
fn build_named_list_pane<F>(
    title: &str,
    focused: bool,
    pane_height: u16,
    total: usize,
    selected: usize,
    mut row_builder: F,
    footer: &str,
) -> revue::widget::Stack
where
    F: FnMut(usize) -> revue::widget::Stack,
{
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(title_row(&format!("  {}", title), title_color), 1)
        .child_sized(Text::new(""), 1);

    if total == 0 {
        return s
            .child_sized(Text::new("   (empty)").fg(colors::FG_TRACE()), 1)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new(footer).fg(colors::FG_TRACE()), 1)
            .child_sized(Text::new(""), 1);
    }

    let visible_rows = pane_height.saturating_sub(5).max(1) as usize;
    let (start, end) =
        crate::dialog::backdrop::list_viewport_window(total, selected, visible_rows);
    for i in start..end {
        s = s.child_sized(row_builder(i), 1);
    }
    let pos = format!("{}/{}", selected + 1, total);
    let pos_w = cell_w(&pos);
    let footer_color = if focused {
        colors::FG_SECONDARY()
    } else {
        colors::FG_TRACE()
    };
    let footer_w = cell_w(&footer);
    let footer_row = hstack().gap(0)
        .child_sized(Text::new(footer).fg(footer_color), footer_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(pos).fg(colors::FG_TRACE()), pos_w)
        .child_sized(Text::new("  "), 2);
    s.child_flex(Text::new(""), 1.0)
        .child_sized(footer_row, 1)
        .child_sized(Text::new(""), 1)
}

// ── 公共小件 ──

/// 单元格显示宽度(unicode-width,与 revue RichText 逐字宽度计量同源)。
/// `chars().count()` 把宽字形(CJK/emoji)按 1 列计,child_sized 宽度不足
/// 会导致后续 child 错位(内容区右侧多出一列);改用显示宽度对齐计量口径。
fn cell_w(s: &str) -> u16 {
    unicode_width::UnicodeWidthStr::width(s) as u16
}

/// Pane 标题行:标题 + 尾随 flex 铺满整行宽。
/// 必须铺满:revue 局部 dirty 重渲染只清 dirty rect(copy_from 旧 buffer 后
/// 按区域清),裸 `child_sized(Text, 1)` 的行只占内容宽,新标题比旧标题短时
/// 旧字符残留在行尾(切换分类残影,如 "MCP Servergs")。
fn title_row(title: &str, color: Color) -> revue::widget::Stack {
    hstack().gap(0)
        .child_sized(Text::new(title.to_string()).fg(color).bold(), cell_w(title))
        .child_flex(Text::new(""), 1.0)
}

/// 垂直分隔线(占 1 列,贯整列高 `│` 暗色)。
/// 委托 [`crate::widget::VLine`]:render 时逐 cell 写 symbol,任意高度不断线
/// (旧实现 `"│".repeat(64)` 塞单个 Text 靠裁切当竖线,高度 >64 断线)。
fn vline() -> crate::widget::VLine {
    crate::widget::VLine::new(colors::SIDEBAR_DIVIDER())
}

// ── 第一栏:Categories ──

fn build_categories_pane(active: SettingsCategory, focused: bool) -> revue::widget::Stack {
    // 标题 + 6 项 + flex 空白 + 底部 Esc 提示。Pane 不画边框(列已用 VLine 划界),
    // 这样视觉重量集中在 active 行的高亮,与 HTML 稿一致。
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1) // 顶呼吸 1 行
        .child_sized(title_row("  ⚙ Preferences", title_color), 1)
        .child_sized(Text::new(""), 1);

    for cat in SettingsCategory::ALL.iter().copied() {
        s = s.child_sized(category_row(cat, active == cat), 1);
    }

    s.child_flex(Text::new(""), 1.0)
        .child_sized(Text::new("  ← Esc: Exit").fg(colors::FG_TRACE()), 1)
        .child_sized(Text::new(""), 1)
}

fn category_row(cat: SettingsCategory, is_active: bool) -> revue::widget::Stack {
    // 视觉权重:active = ▸ + E_TEAL,implemented(非 active)= FG_SECONDARY,
    // 灰显占位 = FG_TRACE(刚好看清,与 HTML 的 disabled 一致)。
    let (icon, color) = if is_active {
        ("▸", colors::E_TEAL())
    } else if cat.is_implemented() {
        (" ", colors::FG_SECONDARY())
    } else {
        (" ", colors::FG_TRACE())
    };
    let line = format!(" {} {} {}", icon, cat.icon(), cat.label());
    let w = cell_w(&line);
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
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
    let mut s = vstack().gap(0)
        .child_sized(Text::new(""), 1)
        .child_sized(title_row("  ◆ Providers", title_color), 1)
        .child_sized(Text::new(""), 1);

    if providers.is_empty() && !is_add {
        // 空态:加载中或 server 无 provider 配置。
        s = s.child_sized(
            Text::new("   (no providers)").fg(colors::FG_TRACE()),
            1,
        );
        return s
            .child_flex(Text::new(""), 1.0)
            .child_sized(
                Text::new("  + Add provider").fg(colors::FG_TRACE()),
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
    let pos_w = cell_w(&pos);
    let add_color = if focused { colors::FG_SECONDARY() } else { colors::FG_TRACE() };
    // Providers pane 通常窄(~24 列),hint 必须极简:`a/e/d` 三字母合订 +
    // 后边 `+ Add provider` 主入口文案。详细 hint("a: Add e: Edit d: Delete")
    // 由 Details pane footer_hint 承接(底部 1 行更宽,文字不挤),
    // 同一信号同时存在于两层窗口对**不同视野阶**用户友好:
    // 老用户看 a/e/d 即懂,新用户切 Details focus 后看长 hint(金律·成形递进)。
    let add_text = if focused { "  + Add  (a/e/d)" } else { "  + Add provider" };
    let add_w = cell_w(&add_text);
    let footer_row = hstack().gap(0)
        .child_sized(Text::new(add_text).fg(add_color), add_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(pos).fg(colors::FG_TRACE()), pos_w)
        .child_sized(Text::new("  "), 2);

    s.child_flex(Text::new(""), 1.0)
        .child_sized(footer_row, 1)
        .child_sized(Text::new(""), 1)
}

/// Add 模式下虚拟追加的"(new provider)"草稿行(永远 selected,标 highlight)。
/// 不进入 store.providers——纯渲染层占位,close 后随 edit_state 一起消失。
fn provider_row_draft(focused: bool) -> revue::widget::Stack {
    let name_color = if focused { colors::E_AMBER() } else { colors::FG_SECONDARY() };
    let prefix = " ▸ (new provider)";
    let prefix_w = cell_w(&prefix);
    hstack().gap(0)
        .child_sized(Text::new(prefix).fg(name_color).italic(), prefix_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new("✎").fg(colors::E_AMBER()), 1)
        .child_sized(Text::new(" "), 1)
}

fn provider_row(name: &str, is_connected: bool, is_selected: bool) -> revue::widget::Stack {
    // active provider 行(selected):▸ + E_TEAL;否则 ◇ + FG_SECONDARY。
    // 连接状态 dot:● ACCENT_GREEN(connected) / ─ FG_TRACE(disconnected)。
    let (marker, color) = if is_selected {
        ("▸", colors::E_TEAL())
    } else {
        ("◇", colors::FG_SECONDARY())
    };
    let dot = if is_connected { "●" } else { "─" };
    let dot_color = if is_connected { colors::ACCENT_GREEN() } else { colors::FG_TRACE() };
    let prefix = format!(" {} {}", marker, name);
    let prefix_w = cell_w(&prefix);
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
    cursor_on: bool,
) -> revue::widget::Stack {
    let title_color = if focused { colors::E_TEAL() } else { colors::FG_SECONDARY() };
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
            .child_sized(Text::new("  Select a provider").fg(colors::FG_MUTED()), 1)
            .child_flex(Text::new(""), 1.0);
    }

    // ── Header 行 ──
    // 非 editing:◆ name + 状态 pill（可点击 toggle disabled，见 keymap 命中）;
    // Add:◆ (new provider) + [Drafting] pill;Edit:照常显示 provider.name
    let (header_label, pill, pill_color) = if is_add {
        ("  ◆ (new provider)".to_string(), " Drafting ", colors::E_AMBER())
    } else {
        let p = provider_opt.expect("provider_opt is Some when !is_add");
        let is_connected = connected.contains(&p.id);
        let lbl = format!("  ◆ {}", p.name);
        // 三态：disabled(config.disabled_providers) > connected(有 auth) > 无 key。
        // disabled 与 connected 是两个独立维度,此前 pill 拿 connected 冒充 enabled(伪权威)。
        let (pl, pc) = if p.disabled {
            (" Disabled ", colors::FG_MUTED())
        } else if is_connected {
            (" Enabled ", colors::ACCENT_GREEN())
        } else {
            (" No key ", colors::STATUS_WARN())
        };
        (lbl, pl, pc)
    };
    let header_label_w = cell_w(&header_label);
    let pill_w = cell_w(&pill);
    // 右侧操作：⚡ Test connection（可点击/按 t——keymap 命中与键路由同权威）。
    let mode_text = "⚡ Test";
    let mode_w = cell_w(&mode_text);
    let header = hstack().gap(1)
        .child_sized(Text::new(header_label).fg(title_color).bold(), header_label_w)
        .child_sized(Text::new(pill).fg(pill_color).bold(), pill_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(mode_text).fg(colors::FG_TRACE()), mode_w)
        .child_sized(Text::new("  "), 2);

    // ── Name 字段(editing 两模式均显示:Add=新建命名(兼 id),Edit=rename)──
    let name_field_opt = if is_add {
        let st = edit_state.expect("is_add => edit_state Some");
        let focused_field = st.focus == SettingsEditField::Name;
        Some(field_block_editing(
            "Name (also used as ID)",
            st.name_input.clone(),
            "",
            focused_field,
            cursor_on,
        ))
    } else if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::Name;
        Some(field_block_editing(
            "Name",
            st.name_input.clone(),
            "",
            focused_field,
            cursor_on,
        ))
    } else {
        None
    };

    // ── BaseURL 字段 ──
    let base_field = if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::BaseUrl;
        field_block_editing("Base URL", st.base_url_input.clone(), "", focused_field, cursor_on)
    } else {
        let p = provider_opt.expect("non-editing => provider_opt Some");
        let base_value = p
            .base_url
            .clone()
            .unwrap_or_else(|| "(not set)".to_string());
        let base_color = if p.base_url.is_some() { colors::FG_PRIMARY() } else { colors::FG_TRACE() };
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
        let protocol_color = if p.protocol.is_some() { colors::FG_PRIMARY() } else { colors::FG_TRACE() };
        field_block("Protocol", protocol_value, protocol_color, "")
    };

    // ── APIKey 字段:editing 时 Input.password 显示 `•`,buffer 明文(submit 时取);
    // 非 editing 时 placeholder 永远 `••••••` 永不下发;focused 时 hint "e: Edit"
    // 让用户知道改 key 走 e——Providers 栏按 e 弹 ProviderEditDialog
    // (Details 栏有选中 model 时 e 编辑 model,无选中 model 时同样弹 provider 编辑)。
    let api_key_field = if editing_active {
        let st = edit_state.expect("editing_active");
        let focused_field = st.focus == SettingsEditField::ApiKey;
        field_block_editing("API key", st.api_key_input.clone(), "", focused_field, cursor_on)
    } else {
        let api_key_hint = if focused { "e: Edit" } else { "" };
        field_block(
            "API key",
            "••••••••••••••••••••",
            colors::FG_MUTED(),
            api_key_hint,
        )
    };

    // ── Models 列表 ──
    // Models header 一行平铺:左侧 "Models",右侧(focused 时)keymap hint。
    // 与 Providers footer 同形态(金律·成形语法权威):每个区段的"可用动作"
    // 都贴在该区段的边缘,而非藏在屏幕底栏一并展示。
    let models_label = "  Models";
    let models_label_w = cell_w(&models_label);
    let models_hint = if focused && !editing_active { "m: Add  e: Edit  d: Delete  " } else { "" };
    let models_hint_w = cell_w(&models_hint);
    // 动作区高亮（E_AMBER）：focused 时让可用动作一眼可见（与 API key 行
    // "e: Edit" hint 同信号，金律·成形语法权威）。
    let models_hint_color = if focused { colors::E_AMBER() } else { colors::FG_TRACE() };
    let models_header_row = hstack().gap(0)
        .child_sized(
            Text::new(models_label).fg(colors::FG_SECONDARY()).bold(),
            models_label_w,
        )
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(models_hint).fg(models_hint_color), models_hint_w);
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
                Text::new("   (no models configured)").fg(colors::FG_TRACE()),
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
            Text::new("   (add models after saving)").fg(colors::FG_TRACE()).italic(),
            1,
        );
    } else {
        models_block = models_block.child_sized(
            Text::new("   (models hidden while editing)").fg(colors::FG_TRACE()).italic(),
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
            Text::new(format!("  {}", label)).fg(colors::FG_SECONDARY()),
            1.0,
        )
    } else {
        let lab = format!("  {}", label);
        let lab_w = cell_w(&lab);
        let hint_w = cell_w(&hint);
        hstack().gap(0)
            .child_sized(Text::new(lab).fg(colors::FG_SECONDARY()), lab_w)
            .child_flex(Text::new(""), 1.0)
            // 动作 hint（如 API key 行的 "e: Edit"）用 E_AMBER + bold 显眼化——
            // 仅在 pane focused 时出现（调用方控制），高亮动作区引导发现编辑入口。
            .child_sized(Text::new(hint).fg(colors::E_AMBER()).bold(), hint_w)
            .child_sized(Text::new("  "), 2)
    };
    let bordered = Border::only_bottom()
        .fg(colors::BORDER())
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
///     focused = false(其他字段在编辑但不是当前焦点):
///   - label / 边框 用 BORDER 暗色
///   - Input.focused(false)
///
/// 返回总高 3 行(与 field_block 等高,便于 Add 模式插入 Name 字段不引起跳层)。
fn field_block_editing(
    label: &str,
    input: Input,
    hint: &str,
    focused: bool,
    cursor_on: bool,
) -> revue::widget::Stack {
    let label_color = if focused { colors::E_AMBER() } else { colors::FG_SECONDARY() };
    let border_color = if focused { colors::E_AMBER() } else { colors::BORDER() };
    let label_line = if hint.is_empty() {
        hstack().gap(0).child_flex(
            Text::new(format!("  {}", label)).fg(label_color),
            1.0,
        )
    } else {
        let lab = format!("  {}", label);
        let lab_w = cell_w(&lab);
        let hint_w = cell_w(&hint);
        hstack().gap(0)
            .child_sized(Text::new(lab).fg(label_color), lab_w)
            .child_flex(Text::new(""), 1.0)
            .child_sized(Text::new(hint).fg(colors::FG_TRACE()), hint_w)
            .child_sized(Text::new("  "), 2)
    };
    // Input 是 Clone:此处的 owned 副本只为渲染存在,渲染完即销毁;
    // SettingsEditState 保留原 Input 等下一次 handle_key 修改 buffer/cursor。
    let input_view = input
        .focused(focused)
        .cursor_visible(cursor_on)
        .fg(colors::FG_PRIMARY());
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
    let label_color = if focused { colors::E_AMBER() } else { colors::FG_SECONDARY() };
    let border_color = if focused { colors::E_AMBER() } else { colors::BORDER() };
    let arrow_color = if focused { colors::E_AMBER() } else { colors::FG_TRACE() };
    let value_color = if focused { colors::FG_PRIMARY() } else { colors::FG_SECONDARY() };
    let label_line = hstack().gap(0).child_flex(
        Text::new(format!("  {}", label)).fg(label_color),
        1.0,
    );
    let value_text = format!("  ‹ {} ›", choice_label);
    let arrow_hint = if focused { "  ←/→ to change  " } else { "" };
    let arrow_hint_w = cell_w(&arrow_hint);
    let value_w = cell_w(&value_text);
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
        let color = if pane_focused { colors::E_AMBER() } else { colors::FG_SECONDARY() };
        ("▸ ", color)
    } else {
        ("  ", colors::FG_PRIMARY())
    };
    let ctx_label = match m.context_window {
        Some(n) if n > 0 => format!("{}K context", n / 1000),
        _ => "—".to_string(),
    };
    let name_str = format!("{}{}", marker, name);
    let name_w = cell_w(&name_str);
    let ctx_w = cell_w(&ctx_label);
    hstack().gap(0)
        .child_sized(Text::new(name_str).fg(name_color), name_w)
        .child_flex(Text::new(""), 1.0)
        .child_sized(Text::new(ctx_label).fg(colors::FG_MUTED()), ctx_w)
        .child_sized(Text::new("   "), 3)
        .child_sized(Text::new("🔗").fg(colors::FG_TRACE()), 2)
        .child_sized(Text::new(" "), 1)
        .child_sized(Text::new("✎").fg(colors::FG_TRACE()), 1)
        .child_sized(Text::new(" "), 1)
        .child_sized(Text::new("✕").fg(colors::FG_TRACE()), 1)
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
    let active_color = if focused { colors::FG_SECONDARY() } else { colors::FG_TRACE() };
    let line = if editing_active {
        "  Tab: Next field   ←/→: Protocol   Enter: Save   Esc: Cancel"
    } else if focused && has_selected_model {
        "  ↑/↓: Model   m: Add  e: Edit  d: Delete  t: Test   Tab: Pane   Esc: Back"
    } else if focused {
        "  ↑/↓: Browse models   m: Add model   Tab: Pane   Esc: Back"
    } else {
        "  Tab: Cycle Panes   ↑/↓: Navigate   Enter: Select   Esc: Back"
    };
    let w = cell_w(&line);
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
        let stack = SettingsScreen::build(&store, 40, None, true);
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
                disabled: false,
        }]);
        store.settings_selected_provider.set(Some("p1".to_string()));
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
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

    /// General 分类:body 渲染 6 个 toggle 行 + 当前值,不 panic。
    #[test]
    fn render_general_category_shows_toggles() {
        use crate::store::types::SettingsCategory;
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::General);
        store.show_thinking.set(true);
        store.compact_density.set(false);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("General"), "General title missing:\n{}", merged);
        assert!(merged.contains("Show thinking blocks"), "toggle row missing:\n{}", merged);
        assert!(merged.contains("Theme"), "Theme row missing:\n{}", merged);
    }

    /// Keybindings 分类:body 渲染快捷键参考(数据源 = help::KEYBINDINGS),不 panic。
    #[test]
    fn render_keybindings_category_shows_bindings() {
        use crate::store::types::SettingsCategory;
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::Keybindings);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("Keybindings"), "title missing:\n{}", merged);
        assert!(merged.contains("Send prompt"), "binding missing:\n{}", merged);
    }

    /// MCP Servers 分类:空列表也能渲染,不 panic。
    #[test]
    fn render_mcp_category_empty() {
        use crate::store::types::SettingsCategory;
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::McpServers);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("MCP"), "MCP title missing:\n{}", merged);
    }

    /// Skills 分类:带 catalog + proposal 行时渲染标签。
    #[test]
    fn render_skills_category_with_rows() {
        use crate::store::types::{SettingsCategory, SettingsSkillRow};
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::Skills);
        store.settings_skills.set(vec![
            SettingsSkillRow::Proposal {
                id: "p1".into(),
                title: "Add trigger".into(),
                status: "draft".into(),
                kind: "PatchExistingSkill".into(),
            },
            SettingsSkillRow::Catalog {
                name: "review".into(),
                description: "Code review skill".into(),
                location: "/skills/review".into(),
                category: Some("dev".into()),
                writable: false,
                disabled: false,
            },
        ]);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("Skills"), "title missing:\n{}", merged);
        assert!(merged.contains("Add trigger") || merged.contains("[P]"), "proposal missing:\n{}", merged);
    }

    /// Skills 分类:catalog 行带 [ On ] 开关 pill;disabled 行带 [ Off ]。
    #[test]
    fn render_skills_rows_show_switch_pills() {
        use crate::store::types::{SettingsCategory, SettingsSkillRow};
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::Skills);
        store.settings_skills.set(vec![
            SettingsSkillRow::Catalog {
                name: "review".into(),
                description: String::new(),
                location: "/skills/review".into(),
                category: Some("dev".into()),
                writable: true,
                disabled: false,
            },
            SettingsSkillRow::Catalog {
                name: "lint".into(),
                description: String::new(),
                location: "/skills/lint".into(),
                category: Some("dev".into()),
                writable: true,
                disabled: true,
            },
        ]);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("[ On ]"), "on pill missing:\n{}", merged);
        assert!(merged.contains("[ Off ]"), "off pill missing:\n{}", merged);
        // 类目头聚合：1/2 启用。
        assert!(merged.contains("[1/2]"), "partial pill missing:\n{}", merged);
    }

    /// Tools 分类:family 类目头 + 行开关 pill + protected 行无 pill。
    #[test]
    fn render_tools_category_with_rows() {
        use crate::store::types::{SettingsCategory, SettingsToolRow};
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::Tools);
        store.settings_tools.set(vec![
            SettingsToolRow {
                id: "bash".into(),
                description: "Run shell".into(),
                family: Some("shell".into()),
                protected: false,
                disabled: false,
            },
            SettingsToolRow {
                id: "tool_catalog_call".into(),
                description: "Facade".into(),
                family: Some("tool_catalog".into()),
                protected: true,
                disabled: false,
            },
        ]);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("Tools"), "title missing:\n{}", merged);
        assert!(merged.contains("shell"), "family group missing:\n{}", merged);
        assert!(merged.contains("[ On ]"), "on pill missing:\n{}", merged);
        // protected 行详情：开关禁用说明。
        assert!(merged.contains("protected") || merged.contains("[*]"), "protected marker missing:\n{}", merged);
    }

    /// About 分类:body 渲染版本号,不 panic。
    #[test]
    fn render_about_category_shows_version() {
        use crate::store::types::SettingsCategory;
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::About);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("AgenDao TUI"), "About title missing:\n{}", merged);
    }

    #[test]
    fn providers_pane_viewport_follows_selection_to_bottom() {
        let store = AppStore::new();
        store.navigate_settings();
        // 默认分类已是 General（issue 修复），本测试针对 ModelSettings body，显式切换。
        store.settings_category.set(SettingsCategory::ModelSettings);
        let mut providers = Vec::with_capacity(14);
        for i in 1..=14 {
            providers.push(agendao_client::ProviderInfo {
                id: format!("p{}", i),
                name: format!("P{}", i),
                models: vec![],
                base_url: None,
                protocol: None,
                    disabled: false,
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
        let stack = SettingsScreen::build(&store, 12, None, true);
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

    /// Plugins 分类：managed/discovered 行渲染 tag + 开关 pill + detail 字段。
    #[test]
    fn render_plugins_category_with_rows() {
        use crate::store::types::{SettingsCategory, SettingsPluginRow};
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::Plugins);
        store.settings_plugins.set(vec![
            SettingsPluginRow {
                name: "my-tools".into(),
                plugin_type: "file".into(),
                managed: true,
                version: None,
                path: Some("/abs/plugins/my-tools/index.ts".into()),
                origin: "config (declared)".into(),
                disabled: false,
            },
            SettingsPluginRow {
                name: "auto-found".into(),
                plugin_type: "file".into(),
                managed: false,
                version: None,
                path: Some("/home/u/.agendao/plugins/auto-found.ts".into()),
                origin: "user (~/.agendao/plugins)".into(),
                disabled: true,
            },
        ]);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("Plugins"), "title missing:\n{}", merged);
        assert!(merged.contains("[M]"), "managed tag missing:\n{}", merged);
        assert!(merged.contains("[D]"), "discovered tag missing:\n{}", merged);
        assert!(merged.contains("[ On ]"), "on pill missing:\n{}", merged);
        assert!(merged.contains("[ Off ]"), "off pill missing:\n{}", merged);
        // detail 区：首行选中 my-tools → Origin 字段。
        assert!(merged.contains("Origin"), "origin field missing:\n{}", merged);
    }

    /// MCP 分类：行渲染启停 pill + 连接 dot；detail 展示 transport/command/enabled。
    #[test]
    fn render_mcp_category_with_config_fields() {
        use crate::store::types::{SettingsCategory, SettingsMcpRow};
        let store = AppStore::new();
        store.navigate_settings();
        store.settings_category.set(SettingsCategory::McpServers);
        store.settings_mcp.set(vec![
            SettingsMcpRow {
                name: "fs".into(),
                status: "connected".into(),
                tools: 3,
                resources: 1,
                error: None,
                transport: "local".into(),
                command: Some("npx -y srv /tmp".into()),
                url: None,
                enabled: true,
            },
            SettingsMcpRow {
                name: "remote-svc".into(),
                status: "disabled".into(),
                tools: 0,
                resources: 0,
                error: None,
                transport: "remote".into(),
                command: None,
                url: Some("https://mcp.example.com".into()),
                enabled: false,
            },
        ]);
        let mut buf = Buffer::new(120, 40);
        let area = Rect::new(0, 0, 120, 40);
        let mut ctx = RenderContext::new(&mut buf, area);
        let stack = SettingsScreen::build(&store, 40, None, true);
        stack.render(&mut ctx);
        let merged: String = (0..40)
            .map(|y| collect_row(&buf, y, 120))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(merged.contains("[ On ]"), "on pill missing:\n{}", merged);
        assert!(merged.contains("[ Off ]"), "off pill missing:\n{}", merged);
        // detail（首行 fs 选中）：transport + command + enabled 字段。
        assert!(merged.contains("Transport"), "transport field missing:\n{}", merged);
        assert!(merged.contains("npx -y srv /tmp"), "command field missing:\n{}", merged);
        assert!(merged.contains("Enabled"), "enabled field missing:\n{}", merged);
    }
}
