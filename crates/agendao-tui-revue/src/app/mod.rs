//! Application entry point, event loop, and root view — 火 (execution authority)
//! + 金 (output shaping).
//!
//! The keymap and slash-action dispatchers live in [`keymap`] / [`slash_action`]
//! / [`panel_dispatch`] so this file can stay focused on wiring (App
//! construction, run loop, RootView render). All four files share the same
//! `AppHandler` struct via split `impl` blocks — Rust allows the impl to live
//! in any sibling module as long as the type and its fields are at least
//! `pub(crate)`-visible.

mod app_op;
mod dispatch_outcome;
mod keymap;
mod panel_dispatch;
pub(crate) mod provider_actions;
mod settings_catalog_actions;
pub(crate) mod settings_edit_state;
mod slash_action;

use anyhow::Context;
use revue::prelude::*;
use std::cell::RefCell;
use tokio::sync::watch;

/// Global publish slot for the SessionList dialog's interactive
/// scrollbar. The dialog writes here every frame; the mouse
/// handler reads it on the next event tick. Lives at module scope
/// (not on `AppHandler`) because the dialog's `render(&self, ctx)`
/// is invoked from a borrowed `&self` deep in the layout tree —
/// there's no `AppHandler` handle in scope at that point.
///
/// We use `std::sync::Mutex` instead of `RefCell` because the
/// slot must be `Sync` to live in a `OnceLock` static. The lock is
/// only ever taken on the render or event thread, so contention is
/// not a concern in practice.
///
/// The other list dialogs (ModelSelect, AgentSelect, Help) are
/// less common and haven't been wired to the global publish; their
/// mouse interactions go through other paths.
pub static SESSION_LIST_SCROLLBAR_PUBLISH: std::sync::OnceLock<
    std::sync::Mutex<Option<crate::dialog::backdrop::ListDialogScrollbarArea>>,
> = std::sync::OnceLock::new();

/// Lazy initialiser for the publish slot — same pattern as
/// `std::sync::OnceLock::get_or_init`. We use this so the cell is
/// created on first access; no need for a static initializer that
/// can't run at const time.
pub fn session_list_scrollbar_slot(
) -> &'static std::sync::Mutex<Option<crate::dialog::backdrop::ListDialogScrollbarArea>> {
    SESSION_LIST_SCROLLBAR_PUBLISH.get_or_init(|| std::sync::Mutex::new(None))
}

use crate::bridge::api::ApiBridge;
use crate::config::AppConfig;
use crate::dialog::backdrop::PromptGeom;
use crate::dialog::{
    AgentSelectDialog, ConfirmDialog, HelpDialog, McpEditDialog, McpListDialog, ModeSelectDialog,
    ModelEditDialog, ModelSelectDialog, PermissionDialog, PluginEditDialog, ProviderEditDialog,
    QuestionDialog, RecoveryListDialog, SessionExportDialog, SessionForkDialog, SessionListDialog,
    SessionRenameDialog, SkillListDialog, SkillProposalDialog, StashDialog, StashEntry,
};
use crate::input::{PromptInput, SlashPopup};
use crate::screen::{build_render_units, transcript_total_height};
use crate::store::app_store::{AppStore, Route};
use crate::store::session_store::SessionStore;
use crate::store::types::{RunStatus, ToolPhase};
use crate::telemetry::event_bus::EventBus;
use crate::theme::colors;
use crate::transport;
use crate::widget::bg_stack::BgStack;
use crate::widget::VLine;

/// 区域失效（"哪里脏画哪里"）：按元素 id 把对应 DOM 节点 `state.dirty`
/// 置真，框架 `collect_dirty_regions` 便只重画该区域，替代全屏
/// `request_redraw()`。节点不存在（首帧/结构未建）时返回 false，
/// 调用方应回退全屏重绘。
fn mark_region_dirty(app: &mut App, region_id: &str) -> bool {
    use revue::prelude::Query as _;
    let tree = app.dom_renderer().tree_mut();
    let found = tree
        .get_by_id(region_id)
        .map(|node| (node.id, node.state.clone()));
    if let Some((dom_id, mut state)) = found {
        state.dirty = true;
        tree.set_state(dom_id, state);
        true
    } else {
        false
    }
}

/// U10：header Error 详情截断——状态芯片宽度有限，取首行前 24 字符，
/// 超长补 …；全文在 transcript 的 Failed notice 里可查。
pub(crate) fn short_err(e: &str) -> String {
    const MAX: usize = 24;
    let first = e.lines().next().unwrap_or("").trim();
    let mut chars = first.chars();
    let prefix: String = chars.by_ref().take(MAX).collect();
    if chars.next().is_some() {
        format!("{}…", prefix)
    } else {
        prefix
    }
}

/// Identity that will be used by the next prompt in the active session.
/// An explicit CLI/picker selection overrides the identity persisted by the
/// server for that session.
pub(crate) fn effective_session_identity(
    store: &AppStore,
    session: &SessionStore,
) -> (Option<String>, Option<String>) {
    let model = store
        .selected_model
        .get()
        .or_else(|| session.session_model.get());
    let agent = store
        .selected_agent
        .get()
        .or_else(|| session.session_agent.get());
    (model, agent)
}

#[cfg(test)]
mod session_identity_tests {
    use super::*;

    #[test]
    fn explicit_selection_precedes_restored_session_identity() {
        let store = AppStore::new();
        let session = SessionStore::new();
        session
            .session_model
            .set(Some("deepseek/deepseek-v4-pro".into()));
        session.session_agent.set(Some("build".into()));

        assert_eq!(
            effective_session_identity(&store, &session),
            (
                Some("deepseek/deepseek-v4-pro".into()),
                Some("build".into())
            )
        );

        store.selected_model.set(Some("openai/gpt-5".into()));
        store.selected_agent.set(Some("plan".into()));
        assert_eq!(
            effective_session_identity(&store, &session),
            (Some("openai/gpt-5".into()), Some("plan".into()))
        );
    }
}

pub fn run_app() -> anyhow::Result<()> {
    run_app_with_config(AppConfig::default())
}

pub fn run_app_with_config(config: crate::config::AppConfig) -> anyhow::Result<()> {
    // 主题收口（阴面唯一注册点）：颜色真值的运行时载体是 theme::Palette，
    // ThemeId 是主题身份唯一权威（ds::theme）。初始主题判定顺序：
    // 持久化 config.theme > OSC11 终端背景探测（ds/osc11，保守 stub 恒 None）
    // > 默认 TokyoNight。API bridge 构建完成后才能读 config，故判定延后到
    // 下方 api 就绪处；此处仅注册。
    crate::ds::theme::register_agendao_themes();

    let store = AppStore::new();
    if let Some(ref dir) = config.working_dir {
        store.working_dir.set(dir.display().to_string());
    }
    let rt = tokio::runtime::Runtime::new().map_err(|e| anyhow::anyhow!("tokio runtime: {}", e))?;
    let (sf_tx, sf_rx) = watch::channel::<Option<String>>(None);
    if let Some(ref sid) = config.session_id {
        sf_tx.send_replace(Some(sid.clone()));
        store.navigate(Route::Session {
            session_id: sid.clone(),
        });
    }
    let eb = EventBus::new();
    let active_session = SessionStore::new();
    let tx = eb.sender();
    let wd = config
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Build ApiBridge: local-direct uses in-process server, external uses HTTP
    let api: Option<ApiBridge> = if config.local_direct {
        // Prefer pre-created local_server from outer async context (host.rs).
        // This matches the old TUI pattern: server created in outer runtime,
        // projector tasks run on outer runtime's thread pool.
        let local_state = if let Some(pre) = config.local_server {
            tracing::info!("using pre-created local server state from host");
            Some(pre)
        } else {
            // Fallback: create server state on our own runtime
            tracing::info!("creating local server state internally");
            match rt.block_on(transport::local::new_local_server_for_workspace(wd.clone())) {
                Ok(state) => Some(state),
                Err(e) => {
                    tracing::error!(%e, "FAILED to init local server; data pipeline will be empty");
                    None
                }
            }
        };
        if let Some(ls) = local_state {
            let _ = transport::spawn_local_event_source(tx, ls.clone(), rt.handle(), sf_rx.clone());
            Some(ApiBridge::new_local(ls, rt.handle().clone()))
        } else {
            // Server creation failed — fall back to transport-based mode
            let _ = transport::spawn_event_source(
                tx,
                wd,
                rt.handle(),
                sf_rx,
                config.unix_socket_path.clone(),
                config.base_url.clone(),
                config.server_password.clone(),
            );
            None
        }
    } else {
        let _ = transport::spawn_event_source(
            tx,
            wd,
            rt.handle(),
            sf_rx,
            config.unix_socket_path.clone(),
            config.base_url.clone(),
            config.server_password.clone(),
        );
        if let Some(socket_path) = config.unix_socket_path.clone() {
            Some(ApiBridge::new_unix(socket_path, rt.handle().clone()))
        } else {
            ApiBridge::new_with_password(
                &config
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://127.0.0.1:3000".into()),
                config.server_password.clone(),
                rt.handle().clone(),
            )
            .ok()
        }
    };
    tracing::info!(
        api_present = api.is_some(),
        "ApiBridge construction complete"
    );
    if let Some(ref a) = config.agent_name {
        store.selected_agent.set(Some(a.clone()));
    }
    if let Some(ref m) = config.model {
        store.selected_model.set(Some(m.clone()));
    }

    // ── 主题初始化（土律归一）：持久化 > OSC11 > 默认 ──
    // apply_theme 单点收口：色板切换 + revue 主题信号 + CSS 变量表产出；
    // 变量在 App 构建后立即注入 stylesheet（见下），首帧即以真色渲染。
    let initial_theme = api
        .as_ref()
        .and_then(|a| a.get_config().ok())
        .and_then(|c| c.theme)
        .and_then(|t| crate::ds::theme::ThemeId::from_id(&t))
        .or_else(|| match crate::ds::osc11::detect_bg() {
            Some((r, g, b)) if crate::ds::osc11::is_light_bg(r, g, b) => {
                Some(crate::ds::theme::ThemeId::TokyoNightLight)
            }
            _ => None,
        })
        .unwrap_or(crate::ds::theme::ThemeId::TokyoNight);
    let initial_theme_vars = crate::ds::theme::apply_theme(initial_theme);
    store.theme_id.set(initial_theme);

    // ── Eager message load for --session / AGENDAO_TUI_SESSION ──
    //
    // The SessionStore is created empty and the historical messages are
    // normally pulled in by AppHandler::load_session_messages when the
    // user picks a row from the SessionList dialog. With an env-var
    // session we skip that dialog and navigate straight to Session
    // route, so the transcript stays blank. Calling the same load path
    // here makes both entry points converge on the same content.
    if let Some(ref sid) = config.session_id {
        active_session.set_session_id(sid);
        keymap::eager_load_session_messages(&active_session, api.as_ref(), sid);
    }

    let mut app = App::builder()
        .mouse_capture(true)
        .style("styles/base.css")
        .build();
    // 初始主题 CSS `:root` 变量注入（stylesheet_mut 自动清样式缓存）。
    app.dom_renderer()
        .stylesheet_mut()
        .variables
        .extend(initial_theme_vars);
    let handler = RefCell::new(AppHandler::new(
        store.clone(),
        api.clone(),
        active_session.clone(),
        eb,
        sf_tx,
        dispatch_outcome::DispatchOutcomes::new(),
        app_op::AppOps::new(),
    ));
    // 初始化 sidebar session 导航树(从 session_list + cwd 构建 NavigateSession 节点)。
    handler.borrow_mut().refresh_sidebar_session_tree();
    // 初始 Home 路由聚焦 prompt——一进去就有块光标，可直接打字（Session 路由保持原 focus 行为）。
    if matches!(store.route.get(), Route::Home) {
        handler.borrow_mut().prompt.focus();
    }
    // AGENDAO_TUI_PROMPT / config.initial_prompt:预填输入框(不自动发送,
    // 用户 Enter 确认——发送语义仍由唯一 dispatch 收口,木律·单一输入权威)。
    if let Some(ref p) = config.initial_prompt {
        let mut h = handler.borrow_mut();
        h.prompt.set_text(p);
        h.prompt.focus();
    }
    let view = RootView { store, handler };

    // 渲染速率上限（仅 Tick 源）：触发全是变化驱动（keymap 里内容事件/
    // spinner 帧翻转/blink 翻转），但流式期 chunk 每 50ms 都是"变化"，
    // 不加帽仍会 20fps 全帧重绘。Tick 源合并到 ~10fps；键鼠不限（输入
    // 响应不滞后）。
    let mut last_tick_redraw = std::time::Instant::now();

    app.run(view, move |event, view, app| {
        let is_tick = matches!(event, revue::runtime::event::Event::Tick);
        let mut h = view.handler.borrow_mut();
        let handled = h.handle(event);
        // U4：q 双击//exit 的退出请求（revue 只替我们管 Ctrl+C；
        // 自控退出经此旗标交还 App 单点收口）。
        let quit_requested = std::mem::take(&mut h.quit_requested);
        let layout_dirty = h.layout_dirty;
        h.layout_dirty = false;
        let transcript_dirty = h.transcript_dirty;
        h.transcript_dirty = false;
        let prompt_dirty = h.prompt_dirty;
        h.prompt_dirty = false;
        // 主题切换的 CSS 面交接：slash_action 经 apply_theme 换好色板后把
        // `:root` 变量留在 pending 槽（它拿不到 &mut App），此处收口应用。
        let theme_vars = h.pending_theme_vars.take();
        drop(h);
        if quit_requested {
            app.quit();
        }
        if let Some(vars) = theme_vars {
            app.dom_renderer().stylesheet_mut().variables.extend(vars);
            app.request_redraw();
        }
        let tick_redraw_due = !is_tick || {
            let due = last_tick_redraw.elapsed().as_millis() >= 100;
            if due {
                last_tick_redraw = std::time::Instant::now();
            }
            due
        };
        if handled && tick_redraw_due {
            // 区域失效优先：streaming 时内容/sidebar token/prompt spinner
            // 都会变，标这三个区域（header/status 带与背景不重画）；
            // 其余情况（结构/对话框/主题/首帧）回退全屏 request_redraw。
            let region_marked = if transcript_dirty {
                let t = mark_region_dirty(app, "transcript");
                let s = mark_region_dirty(app, "sidebar");
                let p = mark_region_dirty(app, "prompt");
                t && s && p
            } else if prompt_dirty {
                // blink 翻转只有 prompt 条内块光标显隐变化，单区失效即可；
                // 节点不存在（Home/Settings 无 prompt 条）时回退全屏。
                mark_region_dirty(app, "prompt")
            } else {
                false
            };
            if !region_marked {
                app.request_redraw();
            }
            // The DOM incremental update only refreshes nodes that
            // changed in the structural sense (added/removed/re-typed).
            // When fold state changes, the OUTER tree shape is the same
            // (still a vstack of 637 hstacks) but the LEAF widget
            // heights differ. Without a layout rebuild, the new content
            // is rendered into the cached height slots from before the
            // toggle and the visible result is the same stale frame.
            if layout_dirty {
                app.request_layout_rebuild();
            }
        }
        handled
    })
    .context("agendao TUI runtime exited with error")
}

/// Which overlay is currently active (only one at a time).
///
/// `pub(crate)` so [`super::keymap`] can `match` on it for per-panel key
/// routing and the render path in this module can pattern-match the
/// overlay layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Panel {
    None,
    Slash,
    ModelSelect,
    ModeSelect,
    AgentSelect,
    SessionList,
    Rename,
    Stash,
    Fork,
    Export,
    Confirm,
    Help,
    SkillList,
    SkillProposal,
    McpList,
    Recovery,
    /// Settings Details 内 m / e → 弹 model 添加/编辑 form dialog。
    ModelEdit,
    /// Settings→MCP 内 a / e → 弹 MCP server 添加/编辑 form dialog。
    McpEdit,
    /// Settings→Plugins 内 a → 弹插件安装（file 类型）form dialog。
    PluginEdit,
    /// Settings→Providers 内 a / e → 弹 provider 添加/编辑 form dialog。
    ProviderEdit,
    /// U7③：通知中心（toast_history 只读回看）。
    Notifications,
}

/// Confirm-dialog outcome discriminator. `Panel::Confirm` only yields a bool;
/// this carries *what* the user is confirming so the dispatcher can route the
/// confirmed action. One `Panel::Confirm` variant then serves every confirm
/// kind, instead of ballooning the Panel enum per confirm species
/// (土律: single state ownership per domain).
#[derive(Clone, Debug)]
pub(crate) enum PendingConfirm {
    DeleteSession(String),
    /// session_id 在 panel_dispatch 处理 Execute 时从 active_session 取。
    ExecuteRecovery {
        session_id: String,
        action: agendao_client::RecoveryActionKind,
    },
    /// 批量删除会话（SessionList dialog 'x' 标记 + 'D' 触发）。
    /// 与 DeleteSession(单个) 共享 Confirm dialog 同栈,删一组 session id。
    DeleteSessionsBatch(Vec<String>),
    /// Settings Providers 栏 'd':确认后调 client.delete_provider(id)
    /// → server config_store.replace_with + AuthManager.remove(土律·第四条单点权威)。
    DeleteProvider(String),
    /// Settings Details 内 model 'd':确认后调 client.delete_provider_model_config(provider, key)。
    DeleteProviderModel {
        provider_id: String,
        model_key: String,
    },
    /// Settings Skills 列表 'x'/'d':确认后调 client.manage_skill(Delete)
    /// → server SkillGovernanceAuthority.delete_skill（土律·第四条单点权威）。
    /// 仅 writable == true（项目 .agendao/skills 内）的 catalog skill 可到此。
    DeleteSkill(String),
    /// Settings→MCP 列表 'x':确认后调 client.delete_mcp_config(name)
    /// （DELETE `/config/mcp/{key}`，土律·第四条单点权威）。
    DeleteMcp(String),
    /// Settings→Plugins 列表 'x'/'d':确认后调 client.delete_plugin_config(name)
    /// （仅 managed 条目可到此；discovered 走 toast 指引目录删除）。
    DeletePlugin(String),
}

/// Application state + event handler.
///
/// Fields are `pub(crate)` because the keymap dispatcher lives in a
/// sibling module (`keymap`) and matches / mutates them directly. The
/// struct itself is `pub(crate)` so the sibling can `impl` it.
///
/// Why not a `&mut self` API on a private struct? Each handler reads
/// 2-5 different fields, and threading a typed accessor for every
/// one would dwarf the actual logic. The fields are protected by
/// `RefCell` in `RootView` at the consumer side, which is the only
/// boundary that matters.
pub(crate) struct AppHandler {
    pub(crate) store: AppStore,
    pub(crate) api: Option<ApiBridge>,
    pub(crate) prompt: PromptInput,
    pub(crate) slash_popup: SlashPopup,
    pub(crate) model_select: ModelSelectDialog,
    pub(crate) mode_select: ModeSelectDialog,
    pub(crate) agent_select: AgentSelectDialog,
    pub(crate) session_list: SessionListDialog,
    pub(crate) sidebar_visible: bool,
    pub(crate) permission_dialog: PermissionDialog,
    pub(crate) question_dialog: QuestionDialog,
    pub(crate) rename_dialog: SessionRenameDialog,
    pub(crate) stash_dialog: StashDialog,
    pub(crate) stash_entries: Vec<StashEntry>,
    pub(crate) fork_dialog: SessionForkDialog,
    pub(crate) export_dialog: SessionExportDialog,
    pub(crate) confirm_dialog: ConfirmDialog,
    /// What a confirmed `Panel::Confirm` should execute. Set when opening the
    /// confirm dialog, consumed and cleared when the user answers (道纪第九条:
    /// 写入即承诺回收 —— pending 不许悬空跨多轮)。
    pub(crate) pending_confirm: Option<PendingConfirm>,
    /// U15①：Confirm 解决后要回到的 panel（默认 None）。仅当来源弹窗在
    /// Confirm 期间保持打开时设置——目前只有 SessionList 批量删（dialog
    /// 不关，删完回列表继续操作）；其余来源弹窗在打开 Confirm 前已自行
    /// close，回 None 即可。解决时 `mem::replace` 取出即复位，不跨轮悬空。
    pub(crate) confirm_return: Panel,
    pub(crate) help: HelpDialog,
    pub(crate) skill_list: SkillListDialog,
    pub(crate) skill_proposal: SkillProposalDialog,
    pub(crate) mcp_list: McpListDialog,
    pub(crate) recovery_list: RecoveryListDialog,
    /// U7③：通知中心（toast_history 只读回看，数据真相在 store signal）。
    pub(crate) notification_dialog: crate::dialog::NotificationDialog,
    /// Provider Model 添加/编辑 dialog(Settings Details 内 m/e 入口)。
    /// 走 client.put_provider_model_config / delete_provider_model_config 唯一写路径。
    pub(crate) model_edit_dialog: ModelEditDialog,
    /// MCP server 添加/编辑 dialog(Settings→MCP 内 a/e 入口)。
    /// 走 client.put_mcp_config 唯一写路径（PUT `/config/mcp/{key}`）。
    pub(crate) mcp_edit_dialog: McpEditDialog,
    /// 插件安装 dialog(Settings→Plugins 内 a 入口，file 类型 name+path)。
    /// 走 client.put_plugin_config 唯一写路径（PUT `/config/plugin/{key}`）。
    pub(crate) plugin_edit_dialog: PluginEditDialog,
    /// Provider 添加/编辑 dialog(Settings→Providers 内 a/e 入口)。
    /// submit 载荷即 `ProviderEditSubmission`，走 `submit_provider_edit` 唯一写路径
    /// （与 in-place `settings_edit` 表单共享同一写入链路，土律·第四条）。
    pub(crate) provider_edit_dialog: ProviderEditDialog,
    /// Settings Details pane 的 *in-place* 编辑态(金律·唯一成形权威 — 同一 Details
    /// 区段既是只读 view 又是编辑 form,无 modal dialog 第二窗口)。
    /// `active=false` 时所有 Settings 渲染走只读旧路径,active=true 时 Providers/Details
    /// pane 切到 editable form。Provider 字段(name/base_url/protocol/api_key)由此收口;
    /// Model 字段仍在 ModelEditDialog(后续可同迁)。
    pub(crate) settings_edit: settings_edit_state::SettingsEditState,
    pub(crate) panel: Panel,
    pub(crate) active_session: SessionStore,
    pub(crate) spinner_tick: u64,
    /// transcript 内容本帧已变（流式 chunk/消息/工具结果落地时置真）：
    /// 事件循环据此只把 transcript 节点 mark dirty（区域重画），
    /// 替代全屏 request_redraw。
    pub(crate) transcript_dirty: bool,
    /// 光标 blink 相位本帧翻转且无面板/对话框打开（keymap Tick 置真）：
    /// 事件循环据此只把 prompt 条（块光标所在）mark dirty，替代 600ms
    /// 一次的全屏重绘。
    pub(crate) prompt_dirty: bool,
    /// 最近事件活动时间：任何 FrontendEvent/发送回执到达都会刷新。
    /// run_status 卡在 Running（如流挂死）时，超过阈值无活动即停止
    /// 20fps 强制重绘（陈旧 Running 刹车），有活动即自愈。
    pub(crate) last_activity: std::time::Instant,
    /// 光标闪烁节拍：每个 Tick 单调递增（与运行态无关），`widget::blink::blink_visible`
    /// 据此判相，驱动所有输入处块光标 600ms 量级闪烁。
    pub(crate) blink_tick: u64,
    pub(crate) interrupt_pending: bool,
    /// U10：运行期排队 prompt 计数——server 回执 Sent{status:"queued"}
    /// 累加（server 口径，不猜），run_status 回 Idle 时 Tick 归零。
    /// prompt hint 据此显示 "Queued (n) — will send when current run finishes"。
    pub(crate) queued_prompts: u32,
    /// U10：最近一次发送失败的原始 prompt 文本（Failed 回执留存，
    /// Ctrl+R 重试 take 消费；发送成功即清除防误重发旧文）。
    pub(crate) last_failed_prompt: Option<String>,
    /// U4：agendao 自控退出通道。revue（第三方库，不可改）只对 Ctrl+C 无条件
    /// quit；q 双击、/exit 的退出收口于此——keymap/slash_action 置位，run 循环
    /// 读旗后调 `app.quit()`（AppHandler 拿不到 &mut App，旗标是唯一出口）。
    pub(crate) quit_requested: bool,
    /// U4：q 退出确认的 arm 时刻（双击窗口 keymap::QUIT_CONFIRM_WINDOW）。
    pub(crate) quit_armed_at: Option<std::time::Instant>,
    /// U4：首次 q 暂扣标记——窗口内改按其他字符键时把 'q' 补回输入框
    /// （"query" 这类 q 开头的正常输入不丢首字母）。
    pub(crate) quit_armed_via_q: bool,
    /// `/compact <focus>` 的参数通道：sync_slash_from_text 解析出 focus 后
    /// 置位，slash_action 的 CompactSession 臂 take() 消费（UiActionId 不
    /// 带参——命令注册表跨越多个前端，参数走 app 本地暂存）。
    pub(crate) pending_compact_focus: Option<String>,
    /// /compact 触发调用在飞（U6）：true 期间重复触发被防抖吞掉；
    /// 回执 drain（CompactionTriggered）清零。
    pub(crate) compact_in_flight: bool,
    /// 发 prompt 后置位；一轮结束（Idle）时由 Tick 分支消费一次——拉取服务端
    /// LLM 生成的 title 同步到 active_session.title，然后清除。闭合新建 session
    /// 首轮 title 无事件回流的缺口（header 不再恒显 "New Session"）。
    pub(crate) title_refresh_pending: bool,
    pub(crate) interrupt_time: std::time::Instant,
    pub(crate) event_bus: EventBus,
    pub(crate) sf_tx: watch::Sender<Option<String>>,
    /// 本地发送回执 channel（与 `event_bus` 严格分离）。dispatch 的后台 task
    /// 经 `sender()` 投递 Sent/Failed，`Event::Tick` 非阻塞 drain 回收。
    pub(crate) dispatch_outcomes: dispatch_outcome::DispatchOutcomes,
    /// 非 prompt 异步操作回执 channel（U6：测连接/compact/settings 写等）。
    /// 与 `dispatch_outcomes` 语义分离（见 app_op.rs 模块注释）。
    pub(crate) app_ops: app_op::AppOps,
    /// Set by event handlers whose state change might alter widget
    /// heights (fold toggle, message push, scroll, etc.). The run loop
    /// reads this after `handle()` and calls `request_layout_rebuild()`
    /// so the layout tree is recomputed before the next draw — without
    /// this, a folded→unfolded block is rendered into its OLD height
    /// slot by the cached layout and the visible frame stays the same.
    pub(crate) layout_dirty: bool,
    /// Height of the transcript viewport in rows. Updated every frame
    /// by `RootView::render` from the layout's actual area, then read
    /// by cursor-moving handlers (Tab, j/k) to call
    /// `ensure_cursor_visible(viewport_h)` so the cursor's block lands
    /// inside the visible window after a navigation jump.
    pub(crate) transcript_viewport_h: u16,
    /// Y-coordinate of the transcript area on screen (after header+divider).
    /// Used by mouse click handler to map click_y to transcript row.
    pub(crate) transcript_area_y: u16,
    /// 内联 permission 块的屏幕命中矩形（render 后发布，与 sidebar_nav_hits
    /// 同构）。None = 非 Session 路由或 dialog 不可见。keymap 左键点击命中
    /// 资源区行范围时 toggle 折叠。
    pub(crate) permission_hit: Option<crate::dialog::permission::PermissionBlockHit>,

    /// Absolute screen rect of the transcript scrollbar column,
    /// captured every frame by `RootView::render` and consumed by the
    /// mouse handler to hit-test arrow clicks and thumb drags. The
    /// Rect is the scrollbar's *full* span (▲ + track + ▼), one column
    /// wide. `None` when not on the session route or content fits in
    /// the viewport.
    pub(crate) transcript_scrollbar_area: Option<Rect>,
    /// Metrics paired with `transcript_scrollbar_area`: the
    /// total content rows and viewport rows the scrollbar was drawn
    /// against. Together they form the `ScrollbarOverlay` view-model
    /// for hit-testing without re-walking the transcript.
    pub(crate) transcript_scrollbar_metrics: Option<(u16, u16)>,
    /// Active drag on the transcript scrollbar, if any. Set on
    /// `BeginDrag`, mutated on every `Drag` event, cleared on `Up`.
    pub(crate) transcript_scrollbar_drag: Option<crate::widget::ScrollbarDrag>,
    /// Per-frame slot the `ScrollableTranscript` writes into during
    /// `render`. `RootView::render` drains it into
    /// `transcript_scrollbar_area` / `transcript_scrollbar_metrics`
    /// after `layout.render(ctx)` returns and the immutable borrow
    /// is released. Lives on `AppHandler` so the borrow for the
    /// handler's other fields can coexist with the publish clone.
    pub(crate) transcript_scrollbar_publish:
        std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
    /// Sidebar 当前选中 tab 索引（0 Token / 1 Cache / 2 Context / 3 Sessions / 4 Tools / 5 MCP / 6 Pricing）。
    /// 默认 0（Token）。点击符号行切换。
    pub(crate) sidebar_active_tab: usize,
    /// Sidebar tab 符号行的绝对屏幕 y（render 后发布），供鼠标点击命中切 tab。
    pub(crate) sidebar_tab_y: u16,
    /// Sidebar session tree 各 NavigateSession 行的 (绝对 y, session_id)（render 后发布），
    /// 供鼠标左键点击命中打开会话。空 = 无 sidebar / 无会话（点击不命中）。
    pub(crate) sidebar_nav_hits: Vec<crate::telemetry::sidebar::SidebarNavHit>,
    /// Session tree 展开态唯一权威（土律）：命中的 session_id 才展开其子节点。
    /// 默认空 = 全折叠（root 独占视野）；点击箭头 toggle、点击行打开时自动展开
    /// 被打开会话的祖先链。`refresh_sidebar_session_tree` 重建时按此重放。
    pub(crate) session_tree_expanded: std::collections::HashSet<String>,
    /// 终端总高（render 后发布）。sidebar 底部用户栏在 y = terminal_h - 1（sidebar
    /// 是全高左列、user_bar 是其最后一个 `child_sized(...,1)`），用此值定位 ⚙ 命中行。
    /// 同步发布与 sidebar_tab_y 同构（土律：可观测性单点）。
    pub(crate) terminal_h: u16,
    /// 终端总宽（render 后发布，与 terminal_h 同源）。Settings Details 栏 models 行尾
    /// ✎/✕ 图标的右缘命中、弹窗几何都需要宽度口径。
    pub(crate) terminal_w: u16,
    // Session header dir 点击命中区（金：dir 全路径 tooltip 的阳面命中口径）。
    // header_y=dir 所在行（顶端空行后=1）；header_dir_x/w=dir 文本绝对列范围。
    // render 算好后 publish，keymap click handler 只读命中（土律：编排单点真相）。
    pub(crate) header_y: u16,
    pub(crate) header_dir_x: u16,
    pub(crate) header_dir_w: u16,
    /// Diff 汇总角标命中区（绝对 x, y, w；render 后发布，与 header dir 同模式）。
    /// None = 非 Session 路由或无未决 diff（点击不命中）。
    pub(crate) diff_badge_hit: Option<(u16, u16, u16)>,
    /// Active drag on the session-list dialog's scrollbar. The
    /// dialog uses its own `selected: usize` as the cursor; the
    /// drag state is just a remembered y origin so Drag events
    /// can map cursor-y → new selected index.
    pub(crate) session_list_scrollbar_drag: Option<crate::widget::ScrollbarDrag>,
    /// 主题切换待应用的 CSS `:root` 变量（`ds::theme::apply_theme` 产出）。
    /// slash_action 拿不到 `&mut App`，写此槽位；app 事件闭包在 handle 后
    /// 取走并 merge 进 `dom_renderer().stylesheet_mut()`（土律：单点交接）。
    pub(crate) pending_theme_vars: Option<Vec<(String, String)>>,
    /// ModelEditDialog 渲染后发布的外框 Rect（绝对坐标，render 返回）。
    /// keymap 鼠标据此做字段聚焦命中；panel != ModelEdit 时每帧回落 None。
    pub(crate) model_edit_rect: Option<revue::prelude::Rect>,
    /// McpEditDialog 渲染后发布的外框 Rect（同 model_edit_rect 语义）。
    pub(crate) mcp_edit_rect: Option<revue::prelude::Rect>,
    /// PluginEditDialog 渲染后发布的外框 Rect（同 model_edit_rect 语义）。
    pub(crate) plugin_edit_rect: Option<revue::prelude::Rect>,
    /// ProviderEditDialog 渲染后发布的外框 Rect（同 model_edit_rect 语义）。
    pub(crate) provider_edit_rect: Option<revue::prelude::Rect>,
    /// ConfirmDialog 渲染后发布的外框 Rect（同构），供 y/n 按钮命中。
    pub(crate) confirm_rect: Option<revue::prelude::Rect>,
    /// U7②：可见 toast 的 (id, Rect) 列表（render 每帧重发），供点击
    /// dismiss 命中测试。栈序 = 渲染序（最新一条在最后）。
    pub(crate) toast_rects: Vec<(u64, revue::prelude::Rect)>,
    /// U8：status bar ⏸ 待决策角标的 Rect（render 每帧重发；无待决策时
    /// None），点击 → 重新打开首个 pending permission/question。
    pub(crate) pending_rect: Option<revue::prelude::Rect>,
}

pub(crate) const HOME_PROMPT_PLACEHOLDERS: &[&str] = &[
    "Fix a TODO in the codebase",
    "What is the tech stack of this project?",
    "Fix broken tests",
];
pub(crate) const HOME_SHELL_PLACEHOLDERS: &[&str] = &["ls -la", "git status", "pwd"];

/// Sidebar 列宽（土律：唯一权威）。渲染布局 hstack child_sized、transcript 可用宽
/// 扣除、鼠标命中 x 边界三处共用——避免「32」散落漂移致 sidebar 显隐时宽/命中错位。
pub(crate) const SIDEBAR_WIDTH: u16 = 32;

/// U24：窄终端降级阈值——内容列至少保有的列数。窗口宽 < SIDEBAR_WIDTH +
/// MIN_CONTENT_W 时渲染层自动隐藏 sidebar（不动 sidebar_visible 用户态，
/// 拉宽即恢复），否则内容列被挤成个位数宽、深井/PAD 裁切后全是碎片。
pub(crate) const MIN_CONTENT_W: u16 = 30;

/// U24：sidebar 有效可见性（渲染与命中**同一口径**，金律：几何不得漂移）。
/// 所有布局/命中站点一律经此换算，不得直读 sidebar_visible。
/// width==0 = 几何未发布（首帧 render 前，或无 render 的测试环境）——
/// 不降级，回用户态（真实终端宽度永不为 0）。
pub(crate) fn effective_sidebar_visible(sidebar_visible: bool, width: u16) -> bool {
    sidebar_visible && (width == 0 || width >= SIDEBAR_WIDTH + 1 + MIN_CONTENT_W)
}

/// U24：可排布下限——低于此尺寸不硬排（任何布局都是碎片），整屏画诚实
/// 警告（RootView::render 顶部短路）。
pub(crate) const MIN_TERMINAL_W: u16 = 24;
pub(crate) const MIN_TERMINAL_H: u16 = 8;

/// 全局左右气口宽（深川·流白「流白」物理载体）。transcript 内 messageblock 左右各留 PAD；
/// page_inner 内的非 transcript 元素（header/ctx/attachment/prompt/status）左侧留 PAD，
/// 对齐 messageblock 内容列起点（= SIDEBAR_WIDTH + PAD）。气口宽度单点（金律：唯一成形口径）。
pub(crate) const PAD: u16 = 4;

/// Home 居中输入框宽度（不占满主区；左右 flex 楔子居中）。窄屏（主区 < HOME_INPUT_W+2）
/// 会顶满——fallback 后续优化。
pub(crate) const HOME_INPUT_W: u16 = 64;

/// 左气口包装：page_inner 内 footer/header 元素左留 PAD spacer，与 transcript 内
/// messageblock 内容列对齐。transcript 自身已有 messageblock 级 PAD，不经此包装。
fn gutter(content: impl View + 'static) -> revue::widget::Stack {
    hstack()
        .gap(0)
        .child_sized(Text::new(" ".repeat(PAD as usize)), PAD)
        .child_flex(content, 1.0)
}

/// Session 底部信息条：`↑1.2k ↓456  cache 89r/11m  ctx ▓▓▓▓▓░░░░░ 42% (85k)  $0.003`。
/// token/成本为 session 累计（投影权威）;context 为最新 turn 占用百分比（非累计）。
/// 全部数据来自 SessionProjectionReplaced,无第二真相（水律·回流可观测）。
///
/// `diffs` 非空时尾部追加 diff 汇总角标（`📝 N files +X -Y`，`DiffReplaced`
/// 会话级汇总）。返回角标在 strip 内容区内的 (x, w)——渲染层据此发布鼠标
/// 命中区（点击展开逐文件明细），None = 无角标不命中。
fn build_session_info_strip(
    tokens: &crate::store::types::TokenUsage,
    ctx_pct: u8,
    diffs: &[crate::store::types::DiffStat],
) -> (revue::widget::Stack, Option<(u16, u16)>) {
    // 段 = (文本, 颜色)；宽度在构造处按字符数算（Bar 段 ▓/░ 均单宽,口径一致）。
    let mut spans: Vec<(String, revue::prelude::Color)> = Vec::new();
    if tokens.total > 0 {
        spans.push((
            format!(
                " ↑{} ↓{}",
                crate::theme::fmt_tokens(tokens.input),
                crate::theme::fmt_tokens(tokens.output),
            ),
            colors::FG_MUTED(),
        ));
        if tokens.cache_read > 0 || tokens.cache_miss > 0 {
            spans.push((
                format!(
                    "  cache {}r/{}m",
                    crate::theme::fmt_tokens(tokens.cache_read),
                    crate::theme::fmt_tokens(tokens.cache_miss),
                ),
                colors::FG_TRACE(),
            ));
        }
        if tokens.total_cost > 0.0 {
            spans.push((
                format!("  {}", crate::theme::fmt_cost(tokens.total_cost)),
                colors::FG_TRACE(),
            ));
        }
    }
    if tokens.context_tokens > 0 || ctx_pct > 0 {
        // 进度条：10 格,按百分比着色(绿<50/黄50-80/红>80,与 sidebar meter 同口径)。
        // context_tokens > 0 即显示——大上下文模型下 pct 可能四舍五入为 0%,
        // 那也是真实状态,不该把 meter 藏起来。
        const BAR_W: usize = 10;
        let filled = ((ctx_pct as usize * BAR_W) + 50) / 100;
        let bar_color = if ctx_pct > 80 {
            colors::ACCENT_RED()
        } else if ctx_pct > 50 {
            colors::ACCENT_YELLOW()
        } else {
            colors::ACCENT_GREEN()
        };
        spans.push((
            format!(
                "  ctx {}{} {}%",
                "▓".repeat(filled.min(BAR_W)),
                "░".repeat(BAR_W - filled.min(BAR_W)),
                ctx_pct,
            ),
            bar_color,
        ));
        if tokens.context_tokens > 0 {
            spans.push((
                format!(" ({})", crate::theme::fmt_tokens(tokens.context_tokens)),
                colors::FG_TRACE(),
            ));
        }
    }
    // Diff 汇总角标（DiffReplaced，会话级、replace 语义）。跨文件合计：
    // 文件数 muted / 增行绿 / 删行红。记录角标起点 x 供点击命中。
    let mut badge_geom: Option<(u16, u16)> = None;
    if !diffs.is_empty() {
        let adds: u64 = diffs.iter().map(|d| d.additions).sum();
        let dels: u64 = diffs.iter().map(|d| d.deletions).sum();
        let badge_x: u16 = spans.iter().map(|(t, _)| t.chars().count() as u16).sum();
        // 📝 是双宽 emoji：chars 计数比渲染格数少 1，emoji 后多留一个空格
        // 补齐（宽口径 = chars 计数，命中/截断才不偏 1 列）。
        let head = format!("  📝  {} files", diffs.len());
        let add = format!(" +{}", adds);
        let del = format!(" -{}", dels);
        let badge_w = (head.chars().count() + add.chars().count() + del.chars().count()) as u16;
        spans.push((head, colors::FG_MUTED()));
        spans.push((add, colors::ACCENT_GREEN()));
        spans.push((del, colors::ACCENT_RED()));
        badge_geom = Some((badge_x, badge_w));
    }
    let mut row = hstack().gap(0);
    for (text, color) in spans {
        let w = text.chars().count() as u16;
        row = row.child_sized(Text::new(text).fg(color), w);
    }
    (row.child_flex(Text::new(""), 1.0), badge_geom)
}

/// 当前路由输入框的屏幕几何（绝对坐标），返回 [`crate::dialog::backdrop::PromptGeom`]。
/// 所有 `/` 弹框（SlashPopup 补全框 + Bottom 锚点对话框）的宽/x/垂直位置都从此
/// 派生——唯一真相（土律），避免两处各自算居中/宽度而漂移。Home 的 y_top 与
/// `home_center` 的 flex 3:2 分配逐行同源（revue `stack.rs` 用 `.round()`）。
fn prompt_geometry(
    route: &Route,
    area: Rect,
    sidebar_visible: bool,
    prompt_input_rows: u16,
) -> PromptGeom {
    let sidebar = if sidebar_visible {
        SIDEBAR_WIDTH + 1
    } else {
        0
    }; // +1 VLine
    let main_x = area.x + sidebar;
    let main_w = area.width.saturating_sub(sidebar);
    match route {
        Route::Home => {
            // 底部 status_bar 占 1 行 → content_stack 高 = height-1。
            let content_h = area.height.saturating_sub(1);
            let w = HOME_INPUT_W.min(main_w);
            let x = main_x + main_w.saturating_sub(w) / 2;
            // 下移：上 spacer flex 3、下 spacer flex 2；上 = round((content_h-input_h)*3/5)，
            // 与 home_center 的 flex 分配同源（revue stack.rs 非 last flex 用 round）。
            let input_h = prompt_input_rows + 1; // 内容行 + 下边框(1)
            let upper = ((content_h.saturating_sub(input_h)) as f32 * 3.0 / 5.0).round() as u16;
            PromptGeom {
                x,
                y_top: area.y + upper,
                w,
            }
        }
        Route::Session { .. } => {
            // prompt_bar 底部：status(1) + info_strip(1) + prompt_bar(hint1+内容行+底线1)，
            // 输入区上沿 = height - (prompt_bar_h + 2)（覆盖 hint 行,浮层锚定不遮输入区）。
            let prompt_bar_h = prompt_input_rows + 2;
            PromptGeom {
                x: main_x + PAD,
                y_top: area.y + area.height.saturating_sub(prompt_bar_h + 2),
                w: main_w.saturating_sub(PAD),
            }
        }
        Route::Settings => {
            // Settings 路由不画 prompt_bar(对话框输入不在 Settings 出现);
            // 但 prompt_geometry 仍需返回合法 Rect 给 PromptOverlay 的 noop 渲染。
            PromptGeom {
                x: main_x,
                y_top: area.y + area.height,
                w: 0,
            }
        }
    }
}

impl AppHandler {
    fn new(
        s: AppStore,
        a: Option<ApiBridge>,
        ss: SessionStore,
        eb: EventBus,
        sf: watch::Sender<Option<String>>,
        outcomes: dispatch_outcome::DispatchOutcomes,
        ops: app_op::AppOps,
    ) -> Self {
        let prompt = PromptInput::new()
            .with_persistence()
            .with_placeholders(HOME_PROMPT_PLACEHOLDERS, HOME_SHELL_PLACEHOLDERS);
        let mut model_select = ModelSelectDialog::new();
        let mut agent_select = AgentSelectDialog::new();
        let mut prompt_commands = Vec::new();

        // ── 完整启动初始化 ──
        if let Some(ref api) = a {
            tracing::info!("starting initialization: API bridge present");
            let mut phase_start = std::time::Instant::now();

            // 1. 工作区配置
            match api.get_workspace_context() {
                Ok(ctx) => {
                    tracing::info!(workspace = %ctx.identity.workspace_key, "init: workspace_context loaded");
                    s.working_dir.set(ctx.identity.workspace_key);
                    if let Some(commands) = ctx.config.command.as_ref() {
                        prompt_commands = commands
                            .iter()
                            .map(|(id, command)| {
                                (
                                    id.clone(),
                                    command.name.clone().unwrap_or_else(|| id.clone()),
                                    command
                                        .description
                                        .clone()
                                        .unwrap_or_else(|| "Run configured command".to_string()),
                                )
                            })
                            .collect();
                        prompt_commands.sort_by(|left, right| left.0.cmp(&right.0));
                    }
                    if !ctx.recent_models.is_empty() {
                        let _ = api.put_recent_models(ctx.recent_models);
                    }
                }
                Err(e) => tracing::error!(%e, "init: workspace_context FAILED"),
            }
            tracing::info!(target: "agendao::startup", phase = "tui_workspace_context", elapsed_ms = phase_start.elapsed().as_millis() as u64, "startup phase done");
            phase_start = std::time::Instant::now();

            // 2. 模型列表
            match api.get_all_providers() {
                Ok(resp) => {
                    let connected: std::collections::HashSet<String> =
                        resp.connected.iter().cloned().collect();
                    let n_connected = connected.len();
                    let total = resp.all.len();
                    // ProviderInfo carries both `id` (registry key, e.g. "aihubmix",
                    // "deepseek") and `name` (display label, e.g. "AIHubMix"). The
                    // server's parse_model_string resolves "<provider_id>/<model_id>",
                    // so storing display name as provider here makes send_prompt fail
                    // with "Provider not found: AIHubMix". Group label still uses
                    // `name` for human-friendly display, and `connected` is keyed by
                    // id, matching how the server tracks connection state.
                    let entries: Vec<crate::dialog::ModelEntry> = resp
                        .all
                        .into_iter()
                        .flat_map(|p| {
                            let provider_available = connected.contains(&p.id);
                            let display_name = p.name.clone();
                            let provider_id = p.id.clone();
                            p.models
                                .into_iter()
                                .map(move |m| crate::dialog::ModelEntry {
                                    provider: provider_id.clone(),
                                    provider_display: display_name.clone(),
                                    model_id: m.id.clone(),
                                    display: format!("{} ({})", m.name, display_name),
                                    variants: vec![],
                                    available: m.available.unwrap_or(provider_available),
                                })
                        })
                        .collect();
                    // Surface the connected providers so the user knows
                    // which models will actually work — useful when the
                    // dialog shows 5,140 entries but only 8 providers are
                    // wired in.
                    tracing::info!(
                        connected_provider_ids = ?resp.connected,
                        "init: connected providers"
                    );
                    tracing::info!(
                        providers_total = total,
                        providers_connected = n_connected,
                        model_entries = entries.len(),
                        "init: providers loaded"
                    );
                    ss.set_mcp_lsp(n_connected, total, vec![]);
                    model_select.set_models(entries);
                }
                Err(e) => tracing::error!(%e, "init: get_all_providers FAILED"),
            }
            tracing::info!(target: "agendao::startup", phase = "tui_providers", elapsed_ms = phase_start.elapsed().as_millis() as u64, "startup phase done");
            phase_start = std::time::Instant::now();

            // 3. Agent 列表
            match api.list_agents() {
                Ok(agents) => {
                    tracing::info!(count = agents.len(), "init: agents loaded");
                    agent_select.set_agents(
                        agents
                            .into_iter()
                            .map(|a| crate::dialog::AgentEntry {
                                name: a.name.clone(),
                                display: a.name,
                                description: a.description.unwrap_or_default(),
                            })
                            .collect(),
                    );
                }
                Err(e) => tracing::error!(%e, "init: list_agents FAILED"),
            }
            tracing::info!(target: "agendao::startup", phase = "tui_agents", elapsed_ms = phase_start.elapsed().as_millis() as u64, "startup phase done");
            phase_start = std::time::Instant::now();

            // 4. 执行模式
            match api.list_execution_modes() {
                Ok(modes) => {
                    tracing::info!(count = modes.len(), "init: execution modes loaded");
                    // store 契约：`"kind:id"` 复合（对齐 web `App.tsx:836`）；
                    // dispatch 处再 split 分流。取首个非 hidden 项作为默认。
                    if let Some(first) = modes.iter().find(|m| !m.hidden.unwrap_or(false)) {
                        s.selected_mode
                            .set(Some(format!("{}:{}", first.kind, first.id)));
                    }
                }
                Err(e) => tracing::error!(%e, "init: list_execution_modes FAILED"),
            }
            tracing::info!(target: "agendao::startup", phase = "tui_modes", elapsed_ms = phase_start.elapsed().as_millis() as u64, "startup phase done");
            phase_start = std::time::Instant::now();

            // 5. 会话列表（按 cwd 过滤，与 /sessions 对话框语义一致）
            let cwd = s.working_dir.get();
            let cwd_filter = if cwd.is_empty() { None } else { Some(cwd) };
            match api.list_sessions_in_directory(cwd_filter) {
                Ok(sessions) => {
                    tracing::info!(count = sessions.len(), "init: sessions loaded");
                    s.session_list.set(
                        sessions
                            .iter()
                            .map(crate::telemetry::session_tree::map_api_session_item)
                            .collect(),
                    );
                }
                Err(e) => tracing::error!(%e, "init: list_sessions FAILED"),
            }
            tracing::info!(target: "agendao::startup", phase = "tui_sessions", elapsed_ms = phase_start.elapsed().as_millis() as u64, "startup phase done");
        } else {
            tracing::error!(
                "init: NO API BRIDGE — all data will be empty. Check local server creation."
            );
        }
        Self {
            store: s,
            api: a,
            prompt,
            slash_popup: SlashPopup::with_prompt_commands(prompt_commands),
            model_select,
            agent_select,
            mode_select: ModeSelectDialog::new(),
            session_list: SessionListDialog::new(),
            sidebar_visible: true,
            permission_dialog: PermissionDialog::new(),
            question_dialog: QuestionDialog::new(),
            rename_dialog: SessionRenameDialog::new(),
            stash_dialog: StashDialog::new(),
            stash_entries: crate::dialog::prompt_stash::load_stash(),
            fork_dialog: SessionForkDialog::new(),
            export_dialog: SessionExportDialog::new(),
            confirm_dialog: ConfirmDialog::new(),
            pending_confirm: None,
            confirm_return: Panel::None,
            help: HelpDialog::new(),
            skill_list: SkillListDialog::new(),
            skill_proposal: SkillProposalDialog::new(),
            mcp_list: McpListDialog::new(),
            recovery_list: RecoveryListDialog::new(),
            notification_dialog: crate::dialog::NotificationDialog::new(),
            model_edit_dialog: ModelEditDialog::new(),
            mcp_edit_dialog: McpEditDialog::new(),
            plugin_edit_dialog: PluginEditDialog::new(),
            provider_edit_dialog: ProviderEditDialog::new(),
            settings_edit: settings_edit_state::SettingsEditState::new(),
            panel: Panel::None,
            spinner_tick: 0,
            transcript_dirty: false,
            prompt_dirty: false,
            last_activity: std::time::Instant::now(),
            blink_tick: 0,
            interrupt_pending: false,
            queued_prompts: 0,
            last_failed_prompt: None,
            quit_requested: false,
            quit_armed_at: None,
            quit_armed_via_q: false,
            pending_compact_focus: None,
            compact_in_flight: false,
            title_refresh_pending: false,
            interrupt_time: std::time::Instant::now(),
            active_session: ss,
            event_bus: eb,
            sf_tx: sf,
            dispatch_outcomes: outcomes,
            app_ops: ops,
            layout_dirty: false,
            transcript_viewport_h: 30, // overwritten on first render
            transcript_area_y: 3,      // after empty + header + divider
            permission_hit: None,
            transcript_scrollbar_area: None,
            transcript_scrollbar_metrics: None,
            transcript_scrollbar_drag: None,
            transcript_scrollbar_publish: std::rc::Rc::new(RefCell::new(None)),
            sidebar_active_tab: 0,
            sidebar_tab_y: 0,
            sidebar_nav_hits: Vec::new(),
            session_tree_expanded: std::collections::HashSet::new(),
            terminal_h: 0,
            terminal_w: 0,
            header_y: 1, // 顶端空行后（Session 路由固定；Home 不渲染 header）
            header_dir_x: 0,
            header_dir_w: 0,
            diff_badge_hit: None,
            session_list_scrollbar_drag: None,
            pending_theme_vars: None,
            model_edit_rect: None,
            mcp_edit_rect: None,
            plugin_edit_rect: None,
            provider_edit_rect: None,
            confirm_rect: None,
            toast_rects: Vec::new(),
            pending_rect: None,
        }
    }
}

struct RootView {
    store: AppStore,
    handler: RefCell<AppHandler>,
}

/// Wrapper that renders a Stack inside a ScrollView, slicing the
/// rendered content to the viewport via a private content buffer.
///
/// The wrapping flow is:
///   1. Build the full transcript Stack with content_h rows of natural height.
///   2. Allocate a content buffer of size (area.width, content_h).
///   3. Render the Stack into that buffer at (0, 0).
///   4. Hand the buffer to ScrollView::render_content, which copies
///      the visible window (rows scroll_top..scroll_top+area.height)
///      into the actual draw context, plus an inline scrollbar.
///
/// Without this wrapper a Stack with content > area.height clips
/// silently from the bottom — it does NOT scroll, and the user sees
/// no indication that there's more above. The ScrollView call here
/// is the same one revue's example_widgets.rs uses for log views.
struct ScrollableTranscript {
    /// Refined ScrollView from the agendao widget base. Drops in
    /// cleanly for what was a raw `revue::ScrollView`; the only added
    /// responsibility for the caller is the `publish` callback below,
    /// which the mouse handler reads to hit-test scrollbar clicks.
    sv: crate::widget::ScrollView,
    content: Stack,
    content_h: u16,
    /// Captured for the interactive scrollbar overlay (▲ ▼ thumb).
    /// Same value as `sv.scroll_offset`; kept as a field so overlay
    /// construction does not re-derive it from the store mid-render.
    scroll_top: u16,
    /// Sink the widget writes its absolute screen rect + metrics into
    /// during `render`. `RootView::render` drains it into
    /// `AppHandler.transcript_scrollbar_*` after the immutable borrow
    /// is released. `Rc<RefCell<…>>` because `View::render` only gets
    /// `&self` and we have no other writable channel back to the
    /// handler.
    publish: std::rc::Rc<std::cell::RefCell<Option<TranscriptScrollbarPublish>>>,
    /// 木→金：UI 偏好 toggle。false 时不画交互式 scrollbar（▲▼thumb），
    /// 也不 publish scroll 几何——彻底关闭，鼠标 hit-test 也落空（因为
    /// area 为 None）。这是「金克木」之例：规则压住输出本体。
    show_scrollbar: bool,
}

/// Per-frame publish from [`ScrollableTranscript`] back to the handler:
/// the scrollbar's absolute screen geometry (1 column wide, full
/// transcript height) and the metrics needed to build a `Scrollbar`
/// view-model on the event side.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TranscriptScrollbarPublish {
    /// Absolute screen rect of the scrollbar column.
    area: Rect,
    /// Total content rows.
    content_h: u16,
    /// Visible window rows.
    viewport_h: u16,
}

impl View for ScrollableTranscript {
    /// 元素 id（区域失效定位用）：内容变化时经 `mark_region_dirty` 只把
    /// 这个节点弄脏，框架的增量渲染就只重画 transcript 区而非全屏。
    fn id(&self) -> Option<&str> {
        Some("transcript")
    }

    fn render(&self, ctx: &mut RenderContext) {
        use revue::layout::Rect;
        let area = ctx.area;
        if area.width < 2 || area.height == 0 {
            return;
        }

        // Build the offscreen content buffer at full content height,
        // render the entire stack into it, then let ScrollView copy
        // the visible window into the real ctx.
        let content_width = area.width.saturating_sub(1); // reserve scrollbar col
        let mut content_buf = self.sv.create_content_buffer(content_width);
        let content_area = Rect::new(0, 0, content_width, self.content_h);
        let mut content_ctx = RenderContext::new(&mut content_buf, content_area);
        self.content.render(&mut content_ctx);

        // ScrollView takes the visible window starting at scroll_top
        // and paints it into ctx (alongside its scrollbar).
        self.sv.render_content(ctx, &content_buf);

        // Now overlay agendao's interactive scrollbar (▲ ▼ thumb) on
        // top of the simple `│/█` that `revue::ScrollView` just
        // painted. `area` == `ctx.area` and is ABSOLUTE screen coords —
        // revue accumulates child offsets via `ctx.sub_area`, so each
        // nested view's `ctx.area` is its real on-screen rect. The
        // scrollbar sits on the last column of `area`, top row to
        // bottom row: ▲ at `area.y` (transcript top), ▼ at
        // `area.y + height - 1` (transcript bottom).
        //
        // ScrollbarOverlay expects `content_area` RELATIVE to
        // `ctx_root_xy`; passing absolute `area` there double-counts
        // `area.y` (= header + divider = 2 rows) and shifts the whole
        // bar down — ▲ landed 2 rows below the top, ▼ fell off the
        // bottom into the prompt border. Anchor `content_area` at the
        // origin to cancel the double-add (x is unaffected only
        // because transcript's `area.x == 0`).
        let scrollbar_area_abs = Rect::new(
            area.x.saturating_add(area.width).saturating_sub(1),
            area.y,
            1,
            area.height,
        );
        if self.show_scrollbar {
            let overlay = crate::widget::ScrollbarOverlay::new(
                (ctx.area.x, ctx.area.y),
                Rect::new(0, 0, area.width, area.height),
                self.content_h,
                area.height,
                self.scroll_top,
            );
            overlay.render(ctx);
        }

        // Publish for the next event tick. 仅在 scrollbar 可见时发布几何——
        // 关闭时落空，鼠标 hit-test 也判 None（金克木：规则压住输出本体）。
        if self.show_scrollbar {
            if let Ok(mut slot) = self.publish.try_borrow_mut() {
                *slot = Some(TranscriptScrollbarPublish {
                    area: scrollbar_area_abs,
                    content_h: self.content_h,
                    viewport_h: area.height,
                });
            }
        }
    }
}

impl View for RootView {
    fn render(&self, ctx: &mut RenderContext) {
        let route = self.store.route.get();
        let h = self.handler.borrow();
        // U24：最小尺寸警告——低于可排布下限不硬排（任何布局都是碎片），
        // 整屏画一句诚实提示；用户拉大即自动恢复正常渲染。
        if ctx.area.width < MIN_TERMINAL_W || ctx.area.height < MIN_TERMINAL_H {
            let msg = format!(
                "Terminal too small: {}x{} — need at least {}x{}",
                ctx.area.width, ctx.area.height, MIN_TERMINAL_W, MIN_TERMINAL_H
            );
            Text::new(&msg).fg(colors::ACCENT_YELLOW()).render(ctx);
            return;
        }
        // U24：窄终端降级——宽度不足时 sidebar 自动隐藏（不动 sidebar_visible
        // 用户态，拉宽即恢复）。本帧全部布局站点统一用 sidebar_on，与 keymap
        // 命中同口径（effective_sidebar_visible，金律：几何不得漂移）。
        let sidebar_on = effective_sidebar_visible(h.sidebar_visible, ctx.area.width);
        let is_running = matches!(
            h.active_session.run_status.get(),
            RunStatus::Sending | RunStatus::Running | RunStatus::Compacting
        );
        let is_slash = h.panel == Panel::Slash;
        // Transcript viewport height, hoisted out of the inner
        // session-route branch so we can publish it to the handler
        // (for `ensure_cursor_visible` in the next event) after the
        // borrow is released at the bottom of `render`. Defaults to
        // the Home route's full height.
        // 动态可视高 = 屏高 - 非transcript固定行（顶端空行1+header1+divider1+info1+status1=5）
        // - prompt_bar(prompt_bar_h) - attachment_h（与下方 transcript_viewport_h 同口径）。
        // attachments 提前至此：attachment_h 与布局层、attachment_strip 共用此 borrow（避免重复 get + 重复计算）。
        // 注：保持 get() 克隆——read() guard 会借用 h 直至 render 末尾，与下方
        // drop(h) 冲突；attachments 通常为空/极小，克隆代价可忽略。
        let attachments = h.active_session.attachments.get();
        let attachment_h: u16 = if attachments.is_empty() {
            0
        } else {
            attachments.len().min(3) as u16
        };
        // 光标闪烁相（土律·单点：blink_tick 由 Tick 推进，blink_visible 判相）。
        let cursor_blink_on = crate::widget::blink::blink_visible(h.blink_tick);
        // 多行 prompt：soft-wrap 折行后内容自适应行数（封顶 MAX_VISIBLE_LINES，
        // 超出滚动条）。宽度走 prompt_geometry 单点权威（Home 居中宽 /
        // Session 主区-PAD；rows 不影响宽，先传 0 取宽再算行数）。
        // prompt_bar 高 = hint(1) + 内容行 + 底边框(1)。
        let prompt_w = prompt_geometry(&route, ctx.area, sidebar_on, 0).w;
        let prompt_input_rows = h.prompt.visible_height_for(prompt_w);
        let prompt_bar_h = prompt_input_rows + 2;
        // 动态可视高 = 屏高 - 非transcript固定行（顶端空行1+header1+divider1+info1+status1=5）
        // - prompt_bar(prompt_bar_h) - attachment_h。
        let transcript_viewport_h: u16 = ctx
            .area
            .height
            .saturating_sub(5 + prompt_bar_h + attachment_h);

        // ── Content area ──
        let mut content_stack = vstack();
        // Sidebar（全高左列，Ctrl+B toggle）。Home/Session 都基于 sidebar_visible 构建：Home 时
        // active_session 是默认空 SessionStore（detail 全 0/默认、Session Tree "(no sessions)"），
        // 视觉保留 sidebar。内容树存 sidebar_opt，page 层 match 外包成全高左列。
        let mut sidebar_opt: Option<(revue::widget::Stack, u16)> = None;
        // Session tree 可点击导航命中快照(阳面命中口径);build 内算好绝对 y,
        // publish 段发布到 handler 供 keymap click hit-test(与 sidebar_tab_y 同构)。
        let mut sidebar_nav_hits: Vec<crate::telemetry::sidebar::SidebarNavHit> = Vec::new();
        if sidebar_on {
            let token = h.active_session.token_usage.get();
            let cache = h.active_session.cache_stats.get();
            let price = h.active_session.pricing.get();
            let ctx_pct = h.active_session.context_pct.get();
            let trees = h.active_session.sidebar_trees.read(); // 零拷贝读 guard
            let mcp = h.active_session.mcp_lsp.get();
            let tools = h.active_session.active_tools.get();
            let active_sid = h.active_session.get_session_id();
            let (content, tab_y, nav_hits) = crate::telemetry::SessionSidebar::build(
                &token,
                &cache,
                &price,
                ctx_pct,
                &trees,
                &mcp,
                &tools,
                h.sidebar_active_tab,
                active_sid.as_deref(),
                ctx.area.height,
            );
            sidebar_opt = Some((content, tab_y));
            sidebar_nav_hits = nav_hits;
        }
        // Session header dir 点击命中区快照（None=非 Session 路由）。Session 分支算好后填，
        // publish 段（match 外）发布到 handler 供 keymap click 命中（与 sidebar_tab_y_snapshot 同构）。
        let mut dir_hit: Option<(u16, u16)> = None;
        // 内联 permission 块命中矩形快照（None=非 Session 路由/不可见）。块位置随
        // transcript 滚动而变，只能在渲染时（scroll_top 已知后）算绝对 y——与
        // dir_hit 同模式，publish 段发布。
        let mut perm_hit: Option<crate::dialog::permission::PermissionBlockHit> = None;
        match &route {
            Route::Home => {
                // 极简首页：主区中间只放一个居中输入框（❯ 引导符由 PromptView 首行
                // 自绘 + 块光标 + 下边框），不画倒角全框——边框权威归 Border（金律），
                // Border::only_bottom() 只画底边。多行自适应与 Session 同一权威。
                let input_border = Border::only_bottom()
                    .fg(colors::BORDER())
                    .max_width(HOME_INPUT_W)
                    .child(h.prompt.view(cursor_blink_on));
                // 水平居中（左右 flex 楔子）+ 垂直居中（上下 flex 楔子），不占满主区。
                let centered = hstack()
                    .gap(0)
                    .child_flex(Text::new(""), 1.0)
                    .child_sized(input_border, HOME_INPUT_W)
                    .child_flex(Text::new(""), 1.0);
                let home_center = vstack()
                    .gap(0)
                    .child_flex(Text::new(""), 3.0) // 上 spacer（3/5，给 SlashPopup 补全框让位）
                    .child_sized(centered, prompt_input_rows + 1) // 内容行 + 下边框(1)
                    .child_flex(Text::new(""), 2.0); // 下 spacer（2/5）
                content_stack = content_stack.child(home_center);
            }
            Route::Session { .. } => {
                let title = h.active_session.title.get();
                let dir = self.store.working_dir.get();
                let dir_short = dir.rsplit('/').next().unwrap_or(&dir);

                // ── Header (single row): title · dir · badges · status ──
                //
                // Use a fixed-width left segment for the dir/title pair so
                // the badges hang at a predictable spot regardless of the
                // session title length. The previous loose `hstack().gap(2)`
                // pushed each child to its Auto slot — on a 160-col terminal
                // that meant the title floated near column 80 with 60 cols
                // of dead air around it.
                //
                // Layout: [title]·[dir]·[· model][· agent]   …  [status]
                let title_w = title.chars().count() as u16 + 1;
                let dir_w = dir_short.chars().count() as u16;
                let mut header = hstack().gap(2);
                header = header
                    .child_sized(Text::new(&title).bold().fg(colors::FG_PRIMARY()), title_w)
                    .child_sized(Text::new(dir_short).fg(colors::FG_MUTED()), dir_w);

                // dir 点击命中区：page_x = sidebar 显示 ? SIDEBAR_WIDTH+1(vline) : 0；
                // header 经 gutter 左留 PAD=4；title 在前占 title_w（含尾随气口），gap(2)，dir 紧接其后。
                let page_x: u16 = if sidebar_on { SIDEBAR_WIDTH + 1 } else { 0 };
                dir_hit = Some((page_x + PAD + title_w + 2, dir_w));

                let (effective_model, effective_agent) =
                    effective_session_identity(&self.store, &h.active_session);
                if let Some(ref m) = effective_model {
                    let label = format!("· Model: {}", m);
                    let w = label.chars().count() as u16 + 1;
                    header = header.child_sized(Text::new(label).fg(colors::FG_MUTED()), w);
                }
                if let Some(ref a) = effective_agent {
                    let label = format!("· Agent: {}", a);
                    let w = label.chars().count() as u16 + 1;
                    header = header.child_sized(Text::new(label).fg(colors::FG_MUTED()), w);
                }
                // Task-governance line: the ledger's single Next, when a
                // ledger exists. Typed fields only; absent for ungoverned
                // sessions so the header stays unchanged by default.
                if let Some(ledger) = h.active_session.task_ledger.get() {
                    if ledger.revision > 0 {
                        if let Some(next) = ledger.next.as_ref() {
                            let mut label = format!("· Next: {}", next.statement);
                            // 窄终端防溢出：截到 34 chars（含前缀）。
                            if label.chars().count() > 34 {
                                label = label.chars().take(33).collect::<String>() + "…";
                            }
                            let w = label.chars().count() as u16 + 1;
                            header = header.child_sized(Text::new(label).fg(colors::FG_MUTED()), w);
                        }
                    }
                }
                // Run status indicator pinned to the right via a flex spacer.
                let (status_text, status_color) = match &h.active_session.run_status.get() {
                    RunStatus::Running => (Some(" ● Running".to_string()), colors::ACCENT_GREEN()),
                    // U9：压缩相位独立可辨（◍ 琥珀，区别于 Running 的 ● 绿）。
                    RunStatus::Compacting => (Some(" ◍ Compacting".to_string()), colors::E_AMBER()),
                    RunStatus::Sending => (Some(" ○ Sending".to_string()), colors::ACCENT_YELLOW()),
                    RunStatus::WaitingUser => {
                        (Some(" ⏸ Waiting".to_string()), colors::ACCENT_YELLOW())
                    }
                    // U10：Error 带截断详情——GUI 惯例状态栏错误可读出原因，
                    // 全文仍在 transcript 的 Failed notice 里。
                    RunStatus::Error(e) => {
                        (Some(format!(" ✕ {}", short_err(e))), colors::ACCENT_RED())
                    }
                    RunStatus::Idle => (None, colors::FG_MUTED()),
                };
                // Spacer flex grows to push the status to the right edge.
                header = header.child_flex(Text::new(""), 1.0);
                if let Some(s) = status_text {
                    let w = s.chars().count() as u16 + 1;
                    header = header.child_sized(Text::new(&s).fg(status_color), w);
                }

                // 顶部留白：header 上方 1 行空行（page_inner 首 child 前的呼吸感；header 不再顶窗口边）。
                content_stack = content_stack.child_sized(Text::new(""), 1);
                let show_header = self.store.show_header.get();
                if show_header {
                    content_stack = content_stack.child_sized(gutter(header), 1);
                    // Divider: thin line, single row, FG_MUTED so it recedes
                    // visually rather than competing with the message content.
                    content_stack = content_stack.child_sized(
                        Text::new("─".repeat(ctx.area.width as usize)).fg(colors::BORDER()),
                        1,
                    );
                } else {
                    // header 隐藏：dir 点击不命中（金克木——规则压住输出本体，
                    // header 几何一并收）。
                    dir_hit = None;
                }

                // 零拷贝读（read guard，Deref 到 Vec<TranscriptBlock>）：
                // get() 是 guard.clone()，每帧深克隆整条 transcript。
                // 渲染只读、guard 生命周期限于本分支，无写路径冲突。
                let msgs = h.active_session.messages.read();

                // Build transcript + optional sidebar.
                //
                // CRITICAL: every block must be `child_sized` to its
                // estimated natural height. Without this, vstack
                // distributes the transcript area equally across all
                // children — a single user prompt fills the whole pane
                // while every assistant message gets only 1-2 rows and
                // looks empty (the bug we hit on first send).
                //
                // We also need bottom-anchored truncation so the latest
                // tool result and assistant text stay visible: if total
                // height exceeds the available transcript area, drop
                // blocks from the FRONT (oldest) until the remainder
                // fits. Without this, a long capability search result
                // pushes the final assistant answer off the bottom of
                // the screen.
                // main_area：transcript 容器（单 child_flex）。sidebar 已拆离——它在 page 层
                // （match 外）作为全高左列，贯穿顶到底，不受 footer/header 高度影响。
                let mut main_area = hstack().gap(0);

                // sidebar 已在 match 外基于 sidebar_visible 构建（Home/Session 共用），此处不再构造。

                let mut transcript = vstack().gap(0);

                if msgs.is_empty() {
                    transcript = transcript.child(
                        Text::new("   Type below to start a conversation.").fg(colors::FG_MUTED()),
                    );
                    main_area = main_area.child_flex(transcript, 1.0);
                } else {
                    // True scrollable timeline.
                    //
                    // We compute total content height = Σ block heights,
                    // then apply the user's scroll_offset (rows-from-bottom)
                    // to slide a viewport over it. PageUp/PageDown adjust
                    // scroll_offset; new messages auto-pin to the bottom
                    // ONLY when offset is 0, so reading old history
                    // doesn't get yanked back to the latest mid-read.
                    let available = ctx.area.height.saturating_sub(5 + prompt_bar_h);
                    // total_h 与渲染/鼠标命中/cursor 滚动同口径（聚合），单点
                    // transcript_total_height——避免逐块高度与聚合渲染错位致命中/滚动失准。
                    let cursor_idx = h.active_session.transcript_cursor.get();
                    // transcript 可用宽（几何规则基准）：扣除 sidebar（32）。
                    // scrollbar（1-2 列）作为微小偏差容忍。
                    let sidebar_w: u16 = if sidebar_on { SIDEBAR_WIDTH } else { 0 };
                    let transcript_w = ctx.area.width.saturating_sub(sidebar_w);
                    // 全局左右气口（Gemini 第二轮指令#1）：禁止全宽通铺。每块左右
                    // 各留 PAD 字符，让终端 BG_PRIMARY 主背景像流水在两侧贯通——
                    // 这是「流白」呼吸感的物理载体。块内几何（气泡右靠 / 深井下沉 /
                    // ❯ 引导）在 inner_w 内成形。
                    const PAD: u16 = 4;
                    let inner_w = transcript_w.saturating_sub(PAD.saturating_mul(2));
                    let total_h = transcript_total_height(
                        &msgs,
                        self.store.show_thinking.get(),
                        self.store.compact_density.get(),
                        inner_w,
                    );
                    // turn 级思考延续标记：UserPrompt 起一个新 turn，其后首个
                    // Thinking 用 ✻，同 turn 内被 text/tool 夹断的后续 Thinking
                    // 用 ┆ 续接符（避免 reasoning 流被拆成一串重复 ✻ 独立块）。
                    // 视觉单元序列：聚合决策单点（build_render_units）。连续 ToolResult /
                    // 连续 Thinking 各自成井，其余逐块。渲染只消费 unit（height/content/
                    // 包装属性），不认块类型——新增聚合种类不触此处（金律：渲染触点 1）。
                    //
                    // viewport 估算：内联 permission/question/Sending 块（extra_h）此刻
                    // 尚未追加，但 SAFETY_PAD（16 行）已涵盖这点偏差——保守换简洁。
                    // pinned 时 user_offset 强 0、scroll_top=max_offset（钉底），与下方
                    // 真实 scroll_top 同口径；否则 user_offset 由 scroll_offset.get() 决定。
                    let est_max_offset = total_h.saturating_sub(available);
                    let est_pinned = h.permission_dialog.visible || h.question_dialog.visible;
                    let est_user_offset = if est_pinned {
                        0
                    } else {
                        h.active_session.scroll_offset.get().min(est_max_offset)
                    };
                    let est_scroll_top = est_max_offset.saturating_sub(est_user_offset);
                    let viewport_range = crate::screen::ViewportRange {
                        scroll_top: est_scroll_top,
                        viewport_h: available,
                    };
                    let units = build_render_units(
                        &msgs,
                        cursor_idx,
                        h.spinner_tick,
                        self.store.show_thinking.get(),
                        Some(viewport_range),
                        inner_w,
                        self.store.compact_density.get(),
                    );
                    // 逐行记账 transcript 内容行数（unit 高 + 块间空行），
                    // 供内联 permission 块算绝对屏幕 y（命中矩形发布）。
                    let mut content_rows: u16 = 0;
                    for unit in units {
                        let is_cursor_unit = cursor_idx
                            .map(|c| c >= unit.base_index && c < unit.base_index + unit.block_span)
                            .unwrap_or(false);
                        // 字段提前解构：content move 进 child_sized，glyph/accent/bg 拷出
                        // 供闭包与分支复用（避免部分 move 复杂性）。
                        let glyph = unit.glyph;
                        let glyph_w = unit.glyph_w;
                        let accent = unit.accent;
                        let bg = unit.bg;
                        let content = unit.content;
                        // 引导符（符号归一）：对话块 ❯，工具/思考 ┊。cursor 指示由
                        // 引导符加粗承担（替代 ▌ 竖线）。
                        let mk_glyph = || {
                            if is_cursor_unit {
                                Text::new(glyph).fg(accent).bold()
                            } else {
                                Text::new(glyph).fg(accent)
                            }
                        };
                        // 块内成形（严格「禁止全宽通铺」）：井几何（is_well = 聚合井或
                        // 单个 ToolResult）走左缩进2 + 右断15% + bg=>BgStack；其余 glyph +
                        // 内容 + bg。气口层（PAD）在外层 padded 提供。
                        let inner: Box<dyn View> = if unit.is_well {
                            let avail = inner_w.saturating_sub(glyph_w); // 扣引导符
                            let well_inner = avail.saturating_sub(2); // 左缩进 2
                            let well_w = (well_inner as u32 * 85 / 100) as u16; // 右断 15%
                            let well = hstack()
                                .gap(0)
                                .child_sized(Text::new(" ".repeat(2)), 2)
                                .child_sized(content, well_w);
                            let well_wrapped: Box<dyn View> = match bg {
                                Some(c) => Box::new(BgStack::new(well, c)),
                                None => Box::new(well),
                            };
                            Box::new(
                                hstack()
                                    .gap(0)
                                    .child_sized(mk_glyph(), glyph_w)
                                    .child_sized(well_wrapped, well_w.saturating_add(2)) // 井总宽 = well_w + 左缩进2
                                    .child_flex(Text::new(""), 1.0),
                            ) // 右断留白
                        } else {
                            let with_glyph = hstack()
                                .gap(0)
                                .child_sized(mk_glyph(), glyph_w)
                                .child_flex(content, 1.0);
                            match bg {
                                Some(c) => Box::new(BgStack::new(with_glyph, c)),
                                None => Box::new(with_glyph),
                            }
                        };
                        // 气口层：左右各 PAD 字符留白，BG_PRIMARY 流白在两侧贯通。
                        let padded = hstack()
                            .gap(0)
                            .child_sized(Text::new(" ".repeat(PAD as usize)), PAD)
                            .child_sized(inner, inner_w)
                            .child_sized(Text::new(" ".repeat(PAD as usize)), PAD);
                        // U13②：选中态整行背景条（surface_selected 主题色）——
                        // 原仅引导符加粗太弱（流式滚动中一眼找不到 cursor）。
                        // GUI 选中行惯例：整行低对比高亮，含两侧气口。
                        let row: Box<dyn View> = if is_cursor_unit {
                            Box::new(BgStack::new(padded, colors::BG_HIGHLIGHT()))
                        } else {
                            Box::new(padded)
                        };
                        transcript = transcript.child_sized(row, unit.height);
                        content_rows = content_rows.saturating_add(unit.height);
                        // 块间留白（1 行 BG_PRIMARY 空行）：井/气泡之间透气。不包
                        // BgStack——空行保持主背景。total_h 已同步 compact_density
                        // （transcript_total_height 同口径 gap）——紧凑模式跳过此空行。
                        if !self.store.compact_density.get() {
                            transcript = transcript.child_sized(Text::new(""), 1);
                            content_rows = content_rows.saturating_add(1);
                        }
                    }
                    let status = h.active_session.run_status.get();
                    let mut extra_h: u16 = 0;
                    // 内联 permission 块起始内容行（units 之后第一个追加块）。
                    let perm_content_y = content_rows;
                    let mut perm_blk: Option<(u16, Option<(u16, u16)>)> = None;
                    // 内联 permission/question（终端内联 CLI 风格）：
                    // transcript 流末尾顶格块，不浮不黑。
                    if h.permission_dialog.visible {
                        if let Some(blk) = h.permission_dialog.render_inline(transcript_w) {
                            perm_blk = Some((
                                blk.height,
                                h.permission_dialog.resource_row_range(transcript_w),
                            ));
                            transcript = transcript.child_sized(blk.view, blk.height);
                            extra_h = extra_h.saturating_add(blk.height);
                        }
                    }
                    if h.question_dialog.visible {
                        if let Some(blk) = h.question_dialog.render_inline() {
                            transcript = transcript.child_sized(blk.view, blk.height);
                            extra_h = extra_h.saturating_add(blk.height);
                        }
                    }
                    if matches!(status, RunStatus::Sending) {
                        transcript = transcript.child_sized(
                            Text::new(" ⏳ Sending...").fg(colors::ACCENT_YELLOW()),
                            1,
                        );
                        extra_h = extra_h.saturating_add(1);
                    }
                    // U6③：open_session 后台拉取进行中的处理中指示（与
                    // Sending 块同位同构——transcript 流末尾顶格行）。
                    if h.store.session_loading.get() {
                        transcript = transcript.child_sized(
                            Text::new(" ⏳ Loading session...").fg(colors::ACCENT_YELLOW()),
                            1,
                        );
                        extra_h = extra_h.saturating_add(1);
                    }
                    // 把内联块计入 scroll 视口高度，并在 permission/question
                    // pending 时强制钉底（阴：收束注意力到待决策项）。
                    let total_h = total_h.saturating_add(extra_h);
                    let max_offset = total_h.saturating_sub(available);
                    let pinned = h.permission_dialog.visible || h.question_dialog.visible;
                    // U11①④：内容锚定 + 未读记账（单点回写，语义见
                    // SessionStore::sync_scroll_frame）。须在 user_offset
                    // 读取之前——锚定补的 Δ 本帧即生效。
                    h.active_session
                        .sync_scroll_frame(total_h, msgs.len(), pinned);
                    let user_offset = if pinned {
                        0
                    } else {
                        h.active_session.scroll_offset.get().min(max_offset)
                    };
                    // ScrollView counts offset from the TOP, but our store
                    // treats it as "rows back from bottom" so the latest
                    // message stays visible by default. Convert.
                    let scroll_top = max_offset.saturating_sub(user_offset);

                    // 内联 permission 块命中矩形：绝对 y = transcript 区起始 y
                    // （空行+header+divider=3，header 隐藏=1）+ 块起始内容行 -
                    // scroll_top。permission 可见即 pinned 钉底，块总在视口内。
                    if let Some((blk_h, res_rows)) = perm_blk {
                        let transcript_y_abs: u16 = if show_header { 3 } else { 1 };
                        let y = transcript_y_abs as i32 + perm_content_y as i32 - scroll_top as i32;
                        perm_hit = Some(crate::dialog::permission::PermissionBlockHit {
                            y_start: y.max(0) as u16,
                            height: blk_h,
                            resource_rows: res_rows,
                        });
                    }

                    let sv = crate::widget::scroll_view()
                        .with_content_height(total_h)
                        .scroll_offset(scroll_top)
                        .show_scrollbar(self.store.show_scrollbar.get());

                    main_area = main_area.child_flex(
                        ScrollableTranscript {
                            sv,
                            content: transcript,
                            content_h: total_h,
                            scroll_top,
                            publish: h.transcript_scrollbar_publish.clone(),
                            show_scrollbar: self.store.show_scrollbar.get(),
                        },
                        1.0,
                    );
                }

                // main_area takes all remaining vertical space below the
                // header + divider; without `child_flex` the outer vstack
                // splits its area equally and the transcript ends up with
                // less than half the screen even when content_stack has
                // only three children.
                content_stack = content_stack.child_flex(main_area, 1.0);
            }
            Route::Settings => {
                // 全屏 Settings(三栏:分类 22 | Providers 28 | Details flex);
                // 单一权威:`AppStore.providers`/`settings_*` signals,SettingsScreen 只读消费。
                // `ctx.area.height` 透传给 Providers 栏:滑窗算法据此推导可见行数,
                // selected 超视野时跟随移动(金律·成形语法唯一,`list_viewport_window`)。
                let settings = crate::screen::SettingsScreen::build(
                    &self.store,
                    ctx.area.height,
                    Some(&h.settings_edit),
                    cursor_blink_on,
                );
                content_stack = content_stack.child_flex(settings, 1.0);
            }
        }

        // ── Attachment strip ──（attachments / attachment_h 已在 viewport 区提前借用）
        let attachment_strip = if attachments.is_empty() {
            vstack()
        } else {
            let mut strip = vstack();
            for att in &attachments {
                let label = match &att.kind {
                    crate::store::types::AttachmentKind::File { path, .. } => {
                        format!(" ▸ {} ({})", att.name, path)
                    }
                    crate::store::types::AttachmentKind::Image { mime, .. } => {
                        format!(" ▸ {} [{}]", att.name, mime)
                    }
                };
                strip = strip.child(Text::new(&label).fg(colors::FG_MUTED()));
            }
            strip
        };

        // ── Prompt bar ──
        // 运行态指示：五行相生帧（木→火→土→金→水,各相位语义色）——
        // 不是转圈,是一次完整的相生流转：运行中的一回合即一个阴阳闭环。
        let hint = if h.interrupt_pending {
            " ⚠ Press Esc again to interrupt".to_string()
        } else if is_slash {
            h.slash_popup.hint_line().to_string()
        } else if matches!(h.active_session.run_status.get(), RunStatus::Compacting) {
            // U9：压缩相位独立文案（spinner 同转，语义可辨）。
            " ◍ Compacting… Esc: stop".to_string()
        } else {
            h.prompt.status_hint(is_running)
        };
        // 金克木：tips 隐藏时 hint 行内容置空（几何不变以保 PromptGeom y_top 与
        // Session 底部行高同口径——金律：渲染与命中几何不得漂移）。
        let show_tips = self.store.show_tips.get();
        let hint_text = if show_tips {
            let mut line = hstack().gap(0);
            if is_running {
                // 墨晕帧（·∘○◉●◉○∘·）：墨滴入水晕开再收,传"运行中"。
                // 帧号按 SPINNER_FRAME_DIV 降速（与 keymap 的帧翻转检测同源）。
                let frame = crate::widget::spinner::ink_frame(
                    h.spinner_tick / crate::app::keymap::SPINNER_FRAME_DIV,
                );
                // U9：stall 分级——3s 无事件转琥珀、10s 转红 + hint 补
                // "still waiting…"（与 keymap last_activity 同源：任何
                // 服务端事件/发送回执都会刷新它，活动恢复即自愈回原色）。
                let stall_secs = h.last_activity.elapsed().as_secs();
                let frame_color = crate::widget::spinner::stall_color(
                    crate::widget::spinner::ink_color(),
                    stall_secs,
                );
                let hint = if stall_secs >= 10 && !h.interrupt_pending {
                    format!("{} · still waiting…", hint)
                } else {
                    hint.clone()
                };
                // U10：server 口径的排队计数（Sent{status:"queued"} 累加，
                // Tick 于 run_status 回 Idle 时归零），入队可见即 GUI 的
                // "已收到，稍后处理"反馈。
                let hint = if h.queued_prompts > 0 {
                    format!(
                        "{} · Queued ({}) — will send when current run finishes",
                        hint, h.queued_prompts
                    )
                } else {
                    hint
                };
                line = line
                    .child_sized(Text::new(format!(" {}", frame)).fg(frame_color), 2)
                    .child_flex(Text::new(&hint).fg(colors::FG_MUTED()), 1.0);
            } else if h.interrupt_pending || is_slash {
                // 瞬态有用提示（Esc 再按打断 / slash 导航）保留。
                line = line.child_flex(Text::new(format!(" {}", hint)).fg(colors::FG_MUTED()), 1.0);
            } else {
                // 静止态：只留一点墨（◉）。去掉 "Type to start..." 静态提示——
                // 输入框自带 placeholder,这里是冗余信息（用户反馈：用处不大）。
                line = line.child_sized(
                    Text::new(format!(" {}", crate::widget::spinner::INK_REST))
                        .fg(crate::widget::spinner::ink_color()),
                    2,
                );
            }
            line
        } else {
            hstack().gap(0).child_flex(Text::new(""), 1.0)
        };
        let input_border = if h.prompt.is_focused() {
            // 与 Home 同一成形语法：只画底线（金律·单一边界权威）。Session 此前
            // 用 rounded 全框,与 Home 的克制语言不一致,且视觉重量过大。
            Border::only_bottom().fg(colors::E_TEAL())
        } else {
            Border::only_bottom().fg(colors::BORDER())
        };
        let input_widget = input_border.child(h.prompt.view(cursor_blink_on));
        // hint: 1 row, only_bottom 输入框: 内容行(自适应,封顶10) + 底线 1。
        let prompt_bar = vstack()
            .element_id("prompt") // 区域失效定位（输入/spinner 变只重画此条）
            .child_sized(hint_text, 1)
            .child_sized(input_widget, prompt_input_rows + 1);

        // ── 会话信息条（prompt 下方,status 上方）──
        // 上行/下行 token（session 累计）、cache、成本、context 使用进度条。
        // 数据全部来自 SessionProjectionReplaced 投影（水律·回流可观测）。
        // 尾部附 diff 汇总角标（DiffReplaced）；badge_geom = 角标绝对 (x, y, w)，
        // publish 到 handler 供 keymap 点击命中（与 header dir 命中同模式）。
        let tokens = h.active_session.token_usage.get();
        let ctx_pct = h.active_session.context_pct.get();
        let diff_summary = h.active_session.diff_summary.get();
        let diff_detail_open = h.active_session.diff_detail_open.get();
        let (info_strip, badge_offset) = build_session_info_strip(&tokens, ctx_pct, &diff_summary);
        let diff_badge_geom: Option<(u16, u16, u16)> = badge_offset.map(|(bx, bw)| {
            let page_x: u16 = if sidebar_on { SIDEBAR_WIDTH + 1 } else { 0 };
            let badge_y = ctx.area.height.saturating_sub(2); // info_strip 行 = status_bar 上一行
            (page_x + PAD + bx, badge_y, bw)
        });

        // ── Status bar ──
        let panel_label = match h.panel {
            Panel::Slash => "slash",
            Panel::ModelSelect => "model",
            Panel::ModeSelect => "mode",
            Panel::AgentSelect => "agent",
            Panel::SessionList => "sessions",
            Panel::Stash => "stash",
            Panel::Rename => "rename",
            Panel::Fork => "fork",
            Panel::Export => "export",
            Panel::Confirm => "confirm",
            Panel::Help => "help",
            Panel::SkillList => "skills",
            Panel::SkillProposal => "proposals",
            Panel::McpList => "mcps",
            Panel::Recovery => "recovery",
            Panel::Notifications => "notifications",
            Panel::ModelEdit => "modelEdit",
            Panel::McpEdit => "mcpEdit",
            Panel::PluginEdit => "pluginEdit",
            Panel::ProviderEdit => "providerEdit",
            Panel::None => route.as_str(),
        };
        let dir = self.store.working_dir.get();
        let dir_short = dir.rsplit('/').next().unwrap_or(&dir);
        // token stats 归 info_strip 独一份（build_session_info_strip 消费同一
        // 份 token_usage——状态栏不再重复展示，金律·输出口径单点）。
        // Active tasks count
        let active_tools = h.active_session.active_tools.get();
        let running = active_tools
            .iter()
            .filter(|t| t.phase == ToolPhase::Running)
            .count();
        let tasks_hint = if running > 0 {
            format!(" tasks:{}", running)
        } else {
            String::new()
        };
        // Show cursor + key hints for transcript navigation. Without
        // these, users have no clue Tab/Space exist — and Space/Tab
        // don't visibly do anything until they happen to hover their
        // eyes on the right spot. Status-bar hint advertises the
        // shortcut and confirms the cursor moved.
        let cursor_hint = match h.active_session.transcript_cursor.get() {
            Some(idx) => format!(" cursor:{}", idx + 1),
            None => String::new(),
        };
        // U20：宣传与实现双向对齐（第十条）。status bar 只留基座短口径；
        // 全量键位说明归 `?` help 弹窗（dialog::help::KEYBINDINGS 文档
        // 权威——j/k/Space/g/G/Home/End/C/e/c/Tab/S-Tab 均在表内）。
        let nav_hint = if matches!(route, Route::Session { .. }) {
            " Tab:nav PgUp/Dn:scroll ?:help"
        } else {
            ""
        };
        // U11④：翻上去阅读期间新到底的块数——"↓ N new"，回底消失。
        // U20：附回底键提示（G/End 空 prompt 才生效——非空时按键归编辑器）。
        let unread_count = h.active_session.unread_count();
        let unread_hint = if unread_count > 0 {
            if h.prompt.text().is_empty() {
                format!(" ↓ {} new (G:jump)", unread_count)
            } else {
                format!(" ↓ {} new", unread_count)
            }
        } else {
            String::new()
        };
        // U6⑤：后台弹窗拉取在途 → 状态栏 ◌ 段（与 settings 左栏 ◌ 同一
        // 指示语言；单闸保证至多一个在途，文案即点火处 label）。
        let fetch_hint = match h.store.dialog_fetch_pending.get() {
            Some(label) => format!(" ◌ {}…", label),
            None => String::new(),
        };
        // U9：运行态但 30s 无任何活动（与 keymap running_stale 同闸同
        // 阈值）→ 状态栏明示"可能挂死"。spinner 此时已冻帧，这里给文案
        // 层确认；活动恢复（任一服务端事件）即自愈消失。
        let stale_hint = if is_running
            && h.last_activity.elapsed().as_secs() >= crate::app::keymap::RUNNING_STALE_SECS
        {
            " ⚠ no activity 30s+ — connection may be stalled"
        } else {
            ""
        };
        // U8：⏸ 待决策角标（permission+question 队列合计）——Esc 仅收起
        // 后的重发现入口；点击 / Ctrl+O 回到首个 pending 请求。计数直读
        // 队列（pending_decision_count），与队列天然一致。
        let pending_count = h.pending_decision_count();
        let pending_seg = if pending_count > 0 {
            format!(" ⏸{}", pending_count.min(99))
        } else {
            String::new()
        };
        let status_prefix = format!(
            " {} │ [{}]{}{}{}{}{} │{}",
            dir_short,
            panel_label,
            tasks_hint,
            fetch_hint,
            stale_hint,
            unread_hint,
            cursor_hint,
            nav_hint,
        );
        let pending_rect = {
            use unicode_width::UnicodeWidthStr;
            let sy = ctx.area.height.saturating_sub(1);
            let px = PAD + UnicodeWidthStr::width(status_prefix.as_str()) as u16;
            if pending_seg.is_empty() {
                None
            } else {
                let pw = UnicodeWidthStr::width(pending_seg.as_str()) as u16;
                Some(revue::prelude::Rect::new(px, sy, pw, 1))
            }
        };
        let status_text = format!("{}{} q:quit ^P:cmd ?:help ", status_prefix, pending_seg);
        let status_bar = Text::new(&status_text)
            .fg(colors::FG_MUTED())
            .bg(colors::BG_SECONDARY());

        // ── Full layout ──
        // page_inner：右列（content_stack + footer）。footer 元素经 gutter() 左留 PAD，
        // 对齐 transcript 内 messageblock 内容列起点（SIDEBAR_WIDTH + PAD）。
        // Sidebar 全高左列（纯黑合一：不再 BG_DEEP，与主窗口共享终端纯黑背景）提到 page
        // 最外层 hstack 左 child——贯穿顶到底，不受 footer/header 高度影响；右侧以 VLine
        // （#2e3440 极暗淡 `│`）划界。sidebar 不显示（Home / Ctrl+B 关）→ 直接 page_inner。
        let sidebar_tab_y_snapshot: u16 =
            sidebar_opt.as_ref().map(|(_, tab_y)| *tab_y).unwrap_or(0);
        let page_inner = match &route {
            // Home：主区居中输入框 + 底部 status_bar（去掉 context/attachment/prompt_bar）。
            Route::Home => vstack()
                .child_flex(content_stack, 1.0)
                .child_sized(gutter(status_bar), 1),
            Route::Session { .. } => vstack()
                .child_flex(content_stack, 1.0)
                .child_sized(gutter(attachment_strip), attachment_h)
                .child_sized(gutter(prompt_bar), prompt_bar_h) // hint(1) + 多行输入(自适应≤10) + 底线(1)
                .child_sized(gutter(info_strip), 1) // token + context 进度条
                .child_sized(gutter(status_bar), 1),
            // Settings:仅 content_stack(SettingsScreen 全屏)+ 底部 status_bar;
            // 不画 attachment/prompt_bar(Settings 不接受 prompt 输入)。
            Route::Settings => vstack()
                .child_flex(content_stack, 1.0)
                .child_sized(gutter(status_bar), 1),
        };
        let layout = if let Some((sidebar_view, _)) = sidebar_opt {
            // 纯黑合一：sidebar 不再包 BgStack(BG_DEEP)——保持终端纯黑背景，与主窗口完全一致；
            // 两区之间仅一根极暗淡 VLine（SIDEBAR_DIVIDER #2e3440）划界。
            hstack()
                .gap(0)
                .child_sized(sidebar_view, SIDEBAR_WIDTH)
                .child_sized(VLine::new(colors::SIDEBAR_DIVIDER()), 1)
                .child_flex(page_inner, 1.0)
        } else {
            page_inner
        };

        layout.render(ctx);

        // ── Render overlays (positioned above prompt bar) ──
        drop(h); // Release borrow before re-borrowing
                 // Publish the transcript viewport height so the NEXT event
                 // handler (Tab / j / k / fold) knows how much room is left for
                 // the cursor to land in. Without this, `ensure_cursor_visible`
                 // would have to guess, and on a 30-row terminal the cursor
                 // would sometimes scroll past the visible window when
                 // navigating to a far block.
        self.handler.borrow_mut().transcript_viewport_h = transcript_viewport_h;
        // Transcript area y：show_header 时 y=3（空行+header+divider），隐藏时 y=1（仅空行）。
        // 鼠标点击 click_y → transcript row 依赖此值，必须与 render 几何同口径。
        let show_header_now = self.store.show_header.get();
        self.handler.borrow_mut().transcript_area_y = if show_header_now { 3 } else { 1 };
        // Sidebar tab 符号行绝对 y（sidebar 在 page 顶 y=0，tab_y 已是相对内容顶 = 绝对）。
        // 无 sidebar（Home / Ctrl+B 关）→ 0，点击命中不触发（y=0 落在 logo 区不切 tab）。
        self.handler.borrow_mut().sidebar_tab_y = sidebar_tab_y_snapshot;
        // Session tree 可点击导航命中发布（阴面记账 → keymap click 读）。无 sidebar
        // 或无会话时为空 Vec，点击不命中（水生木：会话树能回到"打开会话"输入动作）。
        self.handler.borrow_mut().sidebar_nav_hits = sidebar_nav_hits;
        // 内联 permission 块命中矩形发布（None=非 Session 路由/不可见 → 点击不命中）。
        self.handler.borrow_mut().permission_hit = perm_hit;
        // 终端总高：sidebar 底部用户栏 ⚙ 命中在 y = terminal_h - 1（sidebar 全高左列，
        // user_bar 是其最后一个 child_sized(...,1)）。同 sidebar_tab_y 模式发布。
        self.handler.borrow_mut().terminal_h = ctx.area.height;
        // 终端总宽：与 terminal_h 同源发布，供 Settings models 行尾 ✎/✕ 右缘命中。
        self.handler.borrow_mut().terminal_w = ctx.area.width;
        // Session header dir 点击命中区 publish（None=非 Session 路由→dir_w=0 不命中）。
        let (dir_x_snap, dir_w_snap) = dir_hit.unwrap_or((0, 0));
        // header 隐藏时 header_y=0（dir 不在屏幕上，点击不命中）。
        self.handler.borrow_mut().header_y = if show_header_now { 1 } else { 0 };
        self.handler.borrow_mut().header_dir_x = dir_x_snap;
        self.handler.borrow_mut().header_dir_w = dir_w_snap;
        // Diff 角标命中区 publish（仅 Session 路由；无 diff 时 geom 为 None 不命中）。
        self.handler.borrow_mut().diff_badge_hit = match &route {
            Route::Session { .. } => diff_badge_geom,
            _ => None,
        };
        // Drain the transcript scrollbar's per-frame publish into the
        // handler so the next mouse event can hit-test arrow clicks
        // and thumb drags. The publish slot is None when the session
        // route's content fits in the viewport, in which case the
        // scrollbar area stays None and the mouse handler skips it.
        // Snapshot the publish slot via Copy. Doing it in a single
        // expression means the temporary `Ref<AppHandler>` and inner
        // `Ref<…publish…>` both drop at the `;`. The previous `if let`
        // form extended the outer `Ref`'s lifetime across the arm body
        // (Edition 2021 temp-lifetime rules for `if let` initializers),
        // colliding with the in-arm `borrow_mut()` and panicking
        // with "RefCell already borrowed" on first render in direct mode.
        let publish_snapshot: Option<TranscriptScrollbarPublish> = self
            .handler
            .borrow()
            .transcript_scrollbar_publish
            .try_borrow()
            .ok()
            .and_then(|opt| opt.as_ref().copied());
        match publish_snapshot {
            Some(p) => {
                self.handler.borrow_mut().transcript_scrollbar_area = Some(p.area);
                self.handler.borrow_mut().transcript_scrollbar_metrics =
                    Some((p.content_h, p.viewport_h));
            }
            None => {
                self.handler.borrow_mut().transcript_scrollbar_area = None;
                self.handler.borrow_mut().transcript_scrollbar_metrics = None;
            }
        }
        let h = self.handler.borrow();
        // 所有 `/` 弹框（SlashPopup 补全框 + Bottom 锚点对话框）共用同一输入框几何
        // ——prompt_geometry 唯一权威（土律）：宽=输入框宽、x 对齐输入框、贴输入框正上方。
        // 在 match 外算一次,补全框与 7 个对话框复用同一 geom,杜绝各算各的而漂移。
        let geom = prompt_geometry(&route, ctx.area, sidebar_on, prompt_input_rows);
        // Toast 底部锚 = 输入框上方第一条可用行：与弹窗同源取 prompt bar 顶边
        // （geom.y_top）再上一行。旧常量 5 是单行输入时代口径——多行输入
        // （rows≤10）时 toast 会压 hint 行/输入框（金律：几何不得漂移）。
        let prompt_y = geom.y_top.saturating_sub(1);
        let mut model_edit_rect: Option<revue::prelude::Rect> = None;
        let mut mcp_edit_rect: Option<revue::prelude::Rect> = None;
        let mut plugin_edit_rect: Option<revue::prelude::Rect> = None;
        let mut provider_edit_rect: Option<revue::prelude::Rect> = None;
        let mut confirm_rect: Option<revue::prelude::Rect> = None;
        match h.panel {
            Panel::Slash => {
                // 补全框几何同上(外层 geom);fill 用绝对坐标填输入框宽(非全屏宽),挡下层 transcript。
                let popup = h.slash_popup.render_popup(geom.w);
                // 上方空间上限 = 输入框上沿到屏顶，留 1 行顶 margin，避免压顶。
                let ph = h
                    .slash_popup
                    .display_height()
                    .min(geom.y_top.saturating_sub(ctx.area.y).saturating_sub(1));
                let py_abs = geom.y_top.saturating_sub(ph).max(1);
                let py_rel = py_abs.saturating_sub(ctx.area.y);
                let px_rel = geom.x.saturating_sub(ctx.area.x);
                h.slash_popup
                    .fill_background(ctx.buffer, geom.x, py_abs, geom.w, ph);
                revue::widget::positioned(popup)
                    .x(px_rel as i16)
                    .y(py_rel as i16)
                    .width(geom.w)
                    .height(ph)
                    .render(ctx);
            }
            Panel::ModelSelect => h.model_select.render(ctx, geom),
            Panel::ModeSelect => h.mode_select.render(ctx, geom),
            Panel::AgentSelect => h.agent_select.render(ctx, geom),
            Panel::SessionList => h.session_list.render(ctx, geom),
            Panel::Stash => h.stash_dialog.render(ctx, geom),
            Panel::Rename => h.rename_dialog.render(ctx, geom),
            Panel::Fork => h.fork_dialog.render(ctx, geom),
            Panel::Export => h.export_dialog.render(ctx, geom),
            Panel::Confirm => {
                confirm_rect = h.confirm_dialog.render(ctx, geom);
            }
            Panel::Help => h.help.render(ctx, geom),
            Panel::SkillList => h.skill_list.render(ctx, geom),
            Panel::SkillProposal => h.skill_proposal.render(ctx, geom),
            Panel::McpList => h.mcp_list.render(ctx, geom),
            Panel::Recovery => h.recovery_list.render(ctx, geom),
            Panel::Notifications => {
                // 只读回看：数据真相在 store signal（土律·单一权威）。
                let history = h.store.toast_history.get();
                h.notification_dialog.render(ctx, geom, &history);
            }
            Panel::ModelEdit => {
                model_edit_rect = h.model_edit_dialog.render(ctx, cursor_blink_on);
            }
            Panel::McpEdit => {
                mcp_edit_rect = h.mcp_edit_dialog.render(ctx, cursor_blink_on);
            }
            Panel::PluginEdit => {
                plugin_edit_rect = h.plugin_edit_dialog.render(ctx, cursor_blink_on);
            }
            Panel::ProviderEdit => {
                provider_edit_rect = h.provider_edit_dialog.render(ctx, cursor_blink_on);
            }
            _ => {}
        }
        // 弹窗几何发布（render 后、借用于此处归还）——keymap 鼠标命中的唯一真相。
        drop(h);
        self.handler.borrow_mut().model_edit_rect = model_edit_rect;
        self.handler.borrow_mut().mcp_edit_rect = mcp_edit_rect;
        self.handler.borrow_mut().plugin_edit_rect = plugin_edit_rect;
        self.handler.borrow_mut().provider_edit_rect = provider_edit_rect;
        self.handler.borrow_mut().confirm_rect = confirm_rect;

        // ── Toast overlay（U7①：队列堆叠，最多 3 条）─────────────────
        // Pending toasts hover above the prompt bar so the user sees
        // why an action was rejected (e.g. "Provider not connected").
        // 最新一条贴锚点（底部锚=prompt 上方 / 顶部锚=header 下沿，弹窗
        // 打开时避开 Bottom 弹窗 footer），旧条依次向远离锚点方向堆。
        // Error 优先：窗口外仍有未过期 Error 时，挤掉窗口内最老的非
        // Error——Error 不被 Success/Info 顶掉（金·失败语义不漂移）。
        let toasts = self.store.toasts.get();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // expires_at == 0 is treated as "no deadline" for backwards
        // compatibility — present code always sets it, but this leaves
        // room for legacy callers. 窗口选择口径收在 store 单点
        // （select_visible_toasts，土律·单一权威）。
        let visible = crate::store::app_store::select_visible_toasts(&toasts, now_ms);
        let mut toast_rects: Vec<(u64, revue::prelude::Rect)> = Vec::new();
        let panel_open = self.handler.borrow().panel != Panel::None;
        for (i, t) in visible.iter().enumerate() {
            use crate::store::types::ToastMsgVariant;
            let (icon, color) = match t.variant {
                ToastMsgVariant::Success => ("✓", colors::ACCENT_GREEN()),
                ToastMsgVariant::Error => ("✕", colors::ACCENT_RED()),
                ToastMsgVariant::Warning => ("⚠", colors::ACCENT_YELLOW()),
                ToastMsgVariant::Info => ("•", colors::ACCENT_CYAN()),
            };
            let max_w = ctx.area.width.saturating_sub(4).min(80);
            let raw = format!("{} {}", icon, t.text);
            // Truncate to fit so emojis at the edge don't half-render.
            let display: String = if raw.chars().count() as u16 > max_w {
                let mut s: String = raw.chars().take(max_w as usize).collect();
                s.push('…');
                s
            } else {
                raw
            };
            let w = (display.chars().count() as u16).min(max_w).max(10);
            let x = (ctx.area.width.saturating_sub(w + 2)) / 2;
            // i = 距锚点的层数（0=最新）。底部锚向上堆；顶部锚（弹窗
            // 打开）从 header 下沿(y=2)向下堆。
            let y = if panel_open {
                2 + (i as u16) * 3
            } else {
                prompt_y.saturating_sub(2 + (i as u16) * 3).max(1)
            };
            // Bordered toast keeps the message visually distinct from
            // the transcript text underneath.
            let toast_widget = Border::rounded()
                .fg(color)
                .child(Text::new(display).fg(color));
            revue::widget::positioned(toast_widget)
                .x(x as i16)
                .y(y as i16)
                .width(w + 2)
                .height(3)
                .render(ctx);
            toast_rects.push((t.id, revue::prelude::Rect::new(x, y, w + 2, 3)));
        }
        self.handler.borrow_mut().toast_rects = toast_rects;
        self.handler.borrow_mut().pending_rect = pending_rect;

        // ── Session header dir 全路径 tooltip（click-to-reveal）─────────────────────
        // 点击 header dir 区 → store.dir_tooltip = Some(DirTooltip{ path, x, y })（keymap toggle）；
        // 此处读出并经 positioned(Border::rounded + Text) 画在 (x, y)——复用上方 toast 范式。
        // 无 motion tracking：显示靠 click，消失靠再点/点外。
        if let Some(dt) = self.store.dir_tooltip.get() {
            let max_w = ctx.area.width.saturating_sub(4).min(80);
            let display: String = if dt.path.chars().count() as u16 > max_w {
                let mut s: String = dt.path.chars().take(max_w as usize).collect();
                s.push('…');
                s
            } else {
                dt.path.clone()
            };
            let w = (display.chars().count() as u16).min(max_w).max(10);
            // 贴近 dir 起点；超右边界则左移 clamp（不溢出屏幕）。
            let x = dt.x.min(ctx.area.width.saturating_sub(w + 2));
            let tooltip_widget = Border::rounded()
                .fg(colors::FG_MUTED())
                .child(Text::new(display).fg(colors::FG_SECONDARY()));
            revue::widget::positioned(tooltip_widget)
                .x(x as i16)
                .y(dt.y as i16)
                .width(w + 2)
                .height(3)
                .render(ctx);
        }

        // ── Diff 角标逐文件明细（click-to-reveal，与 dir tooltip 同范式）─────────────
        // 点击 info_strip 的 📝 角标 → keymap toggle session.diff_detail_open；
        // 此处按 diff_summary 展开每文件 `path +a -d`（path 正文色 / +a 绿 / -d 红），
        // 画在角标上方（footer 区只向上有空间）。超 10 文件折一行 "+M more"。
        if diff_detail_open && !diff_summary.is_empty() {
            if let Some((badge_x, badge_y, _)) = diff_badge_geom {
                const MAX_FILES: usize = 10;
                let shown = diff_summary.len().min(MAX_FILES);
                let mut lines = vstack().gap(0);
                let mut rows: u16 = 0;
                let mut max_w: u16 = 10;
                for d in diff_summary.iter().take(shown) {
                    let path_disp = if d.path.chars().count() > 40 {
                        format!(
                            "…{}",
                            d.path
                                .chars()
                                .skip(d.path.chars().count() - 39)
                                .collect::<String>()
                        )
                    } else {
                        d.path.clone()
                    };
                    let add = format!(" +{}", d.additions);
                    let del = format!(" -{}", d.deletions);
                    let path_w = path_disp.chars().count() as u16;
                    let add_w = add.chars().count() as u16;
                    max_w = max_w.max(path_w + add_w + del.chars().count() as u16);
                    lines = lines.child_sized(
                        hstack()
                            .gap(0)
                            .child_sized(Text::new(path_disp).fg(colors::FG_SECONDARY()), path_w)
                            .child_sized(Text::new(add).fg(colors::ACCENT_GREEN()), add_w)
                            .child_flex(Text::new(del).fg(colors::ACCENT_RED()), 1.0),
                        1,
                    );
                    rows += 1;
                }
                if diff_summary.len() > shown {
                    lines = lines.child_sized(
                        Text::new(format!("  … +{} more files", diff_summary.len() - shown))
                            .fg(colors::FG_MUTED())
                            .italic(),
                        1,
                    );
                    rows += 1;
                }
                let w = max_w.min(ctx.area.width.saturating_sub(4));
                let x = badge_x.min(ctx.area.width.saturating_sub(w + 2));
                let y = badge_y.saturating_sub(rows + 2);
                let detail_widget = Border::rounded().fg(colors::FG_MUTED()).child(lines);
                revue::widget::positioned(detail_widget)
                    .x(x as i16)
                    .y(y as i16)
                    .width(w + 2)
                    .height(rows + 2)
                    .render(ctx);
            }
        }
    }
}

#[cfg(test)]
mod drain_publish_regression {
    //! Regression test for the `RefCell already borrowed` panic that
    //! struck the agendao TUI on first frame in Direct mode
    //! (introduced in commit 98108a3, fixed in the follow-up).
    //!
    //! Root cause (Phase 1 of systematic-debugging):
    //! The borrow shape
    //!
    //!     if let Ok(publish) = outer.borrow().field.try_borrow() {
    //!         match publish.as_ref() {
    //!             Some(p) => outer.borrow_mut().x = Some(p.x),  // PANIC
    //!         }
    //!     }
    //!
    //! In Edition 2021, the temporary `Ref<Outer>` (from `outer.borrow()`)
    //! is live for the whole `if let` block because it participates in
    //! the pattern binding — so the in-block `outer.borrow_mut()` collides
    //! with the still-live `Ref<Outer>` and panics with
    //! "RefCell already borrowed".
    //!
    //! The fix snapshots the inner `Option<Copy>` into a local in a single
    //! expression, so all temporaries drop at the `;` and the subsequent
    //! `borrow_mut()` is clean. These tests pin both halves of that story.

    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Payload {
        area: u32,
        content_h: u32,
        viewport_h: u32,
    }

    /// Pin the OLD broken pattern: a `Ref` is alive at the same time as a
    /// `borrow_mut()` on the same `RefCell` — `RefCell` is special-cased by
    /// the borrow checker (multiple `borrow()` calls are statically
    /// allowed; runtime tracks the count), so this **compiles** but
    /// **panics at runtime**. This is the exact shape from the original
    /// `if let Ok(publish) = self.handler.borrow()...` block: the
    /// `if let`-bound `Ref<AppHandler>` stayed alive across the arm body's
    /// `self.handler.borrow_mut()` calls, producing the same runtime panic.
    /// If this test ever stops panicking, the assumption in the module
    /// doc-comment is wrong and the rest of this file is suspect.
    #[test]
    #[should_panic(expected = "already borrowed")]
    fn old_pattern_panics() {
        let cell: RefCell<u32> = RefCell::new(0);
        let _r = cell.borrow(); // Ref alive until end of scope
        let _ = cell.borrow_mut(); // collides with _r at runtime → panic
    }

    /// Pin the FIX pattern: snapshot via Copy in a single expression so all
    /// temporaries drop at `;`, then a fresh `borrow_mut()` is clean. This
    /// is the exact shape used in `RootView::render` for the transcript
    /// and sidebar scrollbar publish drain.
    #[test]
    fn fix_copy_snapshot_then_mut_borrow_succeeds() {
        struct Outer {
            area: Option<Payload>,
            metrics: Option<(u32, u32)>,
        }
        let outer: RefCell<Outer> = RefCell::new(Outer {
            area: None,
            metrics: None,
        });
        let inner: Rc<RefCell<Option<Payload>>> = Rc::new(RefCell::new(Some(Payload {
            area: 7,
            content_h: 100,
            viewport_h: 30,
        })));

        // Fix: snapshot in one expression; all temporaries drop at `;`.
        let snapshot: Option<Payload> = {
            let _outer_guard = outer.borrow();
            inner
                .try_borrow()
                .ok()
                .and_then(|opt| opt.as_ref().copied())
        };
        // After this `;`, no `Ref`s are alive — safe to `borrow_mut`.

        match snapshot {
            Some(p) => {
                outer.borrow_mut().area = Some(p);
                outer.borrow_mut().metrics = Some((p.content_h, p.viewport_h));
            }
            None => {
                outer.borrow_mut().area = None;
                outer.borrow_mut().metrics = None;
            }
        }
        assert_eq!(
            outer.borrow().area,
            Some(Payload {
                area: 7,
                content_h: 100,
                viewport_h: 30
            })
        );
        assert_eq!(outer.borrow().metrics, Some((100, 30)));
    }
}

#[cfg(test)]
mod effective_sidebar_tests {
    //! U24：窄终端 sidebar 自动隐藏——纯函数口径钉死（渲染与命中共用此 fn，
    //! 金律：几何不得漂移，所以钉函数即钉全部站点）。

    use super::{effective_sidebar_visible, MIN_CONTENT_W, SIDEBAR_WIDTH};

    /// 阈值 = SIDEBAR_WIDTH + 1(分隔列) + MIN_CONTENT_W：低于则隐藏，
    /// 达到则可见；width==0（几何未发布）不降级；用户关始终关。
    #[test]
    fn degrades_only_below_threshold_and_never_on_unpublished_geometry() {
        let threshold = SIDEBAR_WIDTH + 1 + MIN_CONTENT_W;
        // 宽足够：随用户态。
        assert!(effective_sidebar_visible(true, threshold));
        assert!(effective_sidebar_visible(true, 200));
        assert!(!effective_sidebar_visible(false, 200));
        // 窄一列即隐藏。
        assert!(!effective_sidebar_visible(true, threshold - 1));
        // 几何未发布（0）不降级，回用户态。
        assert!(effective_sidebar_visible(true, 0));
        // 用户关与宽度无关。
        assert!(!effective_sidebar_visible(false, 0));
        assert!(!effective_sidebar_visible(false, threshold - 1));
    }
}
