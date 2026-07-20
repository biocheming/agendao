//! 火/土 — Settings→MCP / Settings→Skills 写入与回流。
//!
//! 道纪闭环:
//!   1. 木:Settings 分类 body 选中行 + c/d/a/r 键
//!   2. 火:`toggle_settings_mcp` / `decide_settings_skill_proposal` 调 API
//!   3. 土:写后 `refresh_*_into_store` 回灌 AppStore(单点权威)
//!   4. 金:SettingsScreen 只读 store 渲染
//!   5. 水:toast + 列表刷新滋养下一轮浏览
//!
//! slash `/mcp` `/skills` dialog 仍可独立打开;Settings 路径不经 dialog,
//! 但写操作复用同一 API 权威(木克土:输入变体复用同一权威)。

use crate::app::AppHandler;
use crate::store::types::{SettingsMcpRow, SettingsSkillRow, ToastMsgVariant};

impl AppHandler {
    /// 拉 MCP 状态回灌 `store.settings_mcp`。OpenSettings / connect/disconnect /
    /// 增改删/启停写后调用。`/mcp` 状态 map 无配置字段——transport/command/url/
    /// enabled 从 config.mcp 合并（两源合一，config 缺条目时诚实标 `unknown`）。
    pub(crate) fn refresh_mcp_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        // config 读失败不阻塞状态列表：config 字段退化为 unknown/true。
        let mcp_config = api
            .get_config()
            .ok()
            .and_then(|c| c.mcp.clone())
            .unwrap_or_default();
        match api.get_mcp_status() {
            Ok(servers) => {
                let rows: Vec<SettingsMcpRow> = servers
                    .into_iter()
                    .map(|s| {
                        let (transport, command, url, enabled) =
                            mcp_config_fields(mcp_config.get(&s.name));
                        SettingsMcpRow {
                            name: s.name,
                            status: s.status,
                            tools: s.tools,
                            resources: s.resources,
                            error: s.error,
                            transport,
                            command,
                            url,
                            enabled,
                        }
                    })
                    .collect();
                let prev = self.store.settings_mcp_selected.get();
                let n = rows.len();
                self.store.settings_mcp.set(rows);
                if n == 0 {
                    self.store.settings_mcp_selected.set(0);
                } else if prev >= n {
                    self.store.settings_mcp_selected.set(n - 1);
                }
                // 同步 dialog 列表(若打开),保持 slash 路径与 Settings 同源。
                self.sync_mcp_dialog_from_store();
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load MCP status: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// 拉 skills catalog + pending proposals 合并回灌 `store.settings_skills`。
    /// proposals 排在前(待处理优先),catalog 在后。
    /// `include_disabled=true`：disabled skills 仍列出并打标（否则 Settings
    /// 页看不到被禁项、无法 re-enable）。
    pub(crate) fn refresh_skills_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        let mut rows: Vec<SettingsSkillRow> = Vec::new();

        match api.list_skill_proposals("pending") {
            Ok(proposals) => {
                for p in proposals {
                    rows.push(SettingsSkillRow::Proposal {
                        id: p.id,
                        title: p.title,
                        status: format!("{:?}", p.status).to_lowercase(),
                        kind: format!("{:?}", p.proposal_kind),
                    });
                }
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load skill proposals: {}", e),
                ToastMsgVariant::Warning,
            ),
        }

        let query = agendao_client::SkillCatalogQuery {
            include_disabled: true,
            ..Default::default()
        };
        match api.list_skills(Some(&query)) {
            Ok(skills) => {
                for s in skills {
                    rows.push(SettingsSkillRow::Catalog {
                        name: s.name,
                        description: s.description,
                        location: s.location,
                        category: s.category,
                        writable: s.writable,
                        disabled: s.disabled,
                    });
                }
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load skills: {}", e),
                ToastMsgVariant::Error,
            ),
        }

        let prev = self.store.settings_skills_selected.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        // 选中下标是「展开后可见行」下标（含类目头），clamp 用同一展开口径。
        let n = crate::store::types::flatten_settings_skill_rows(&rows, &collapsed).len();
        self.store.settings_skills.set(rows);
        if n == 0 {
            self.store.settings_skills_selected.set(0);
        } else if prev >= n {
            self.store.settings_skills_selected.set(n - 1);
        }
    }

    /// 拉全量 tool 列表（含 disabled/protected 打标）回灌 `store.settings_tools`。
    pub(crate) fn refresh_tools_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        match api.list_tools() {
            Ok(tools) => {
                let rows: Vec<crate::store::types::SettingsToolRow> = tools
                    .into_iter()
                    .map(|t| crate::store::types::SettingsToolRow {
                        id: t.id,
                        description: t.description,
                        family: t.family,
                        protected: t.protected,
                        disabled: t.disabled,
                    })
                    .collect();
                let prev = self.store.settings_tools_selected.get();
                let collapsed = self.store.settings_tools_collapsed.get();
                let n =
                    crate::store::types::flatten_settings_tool_rows(&rows, &collapsed).len();
                self.store.settings_tools.set(rows);
                if n == 0 {
                    self.store.settings_tools_selected.set(0);
                } else if prev >= n {
                    self.store.settings_tools_selected.set(n - 1);
                }
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load tools: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// Settings MCP:c=connect / d=disconnect。前置校验后调 API,成功则 refresh。
    pub(crate) fn toggle_settings_mcp(&mut self, connect: bool) {
        let rows = self.store.settings_mcp.get();
        let idx = self.store.settings_mcp_selected.get();
        let Some(row) = rows.get(idx) else { return };
        if connect && row.is_connected() {
            self.store.push_toast("Already connected", ToastMsgVariant::Warning);
            return;
        }
        if !connect && !row.is_connected() {
            self.store.push_toast("Not connected", ToastMsgVariant::Warning);
            return;
        }
        let name = row.name.clone();
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let result = if connect {
            api.connect_mcp(&name)
        } else {
            api.disconnect_mcp(&name)
        };
        match result {
            Ok(_) => {
                self.refresh_mcp_into_store();
                self.store.push_toast(
                    &format!(
                        "MCP {}: {}",
                        if connect { "connected" } else { "disconnected" },
                        name
                    ),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!(
                    "{} failed: {}",
                    if connect { "Connect" } else { "Disconnect" },
                    e
                ),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// Settings Skills:a=approve / r=reject 当前选中的 proposal 行。
    /// catalog/类目行上按 a/r → toast 提示(诚实标注,非 proposal)。
    pub(crate) fn decide_settings_skill_proposal(&mut self, accept: bool) {
        use crate::store::types::{flatten_settings_skill_rows, SettingsSkillLine};
        let rows = self.store.settings_skills.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        let lines = flatten_settings_skill_rows(&rows, &collapsed);
        let idx = self.store.settings_skills_selected.get();
        let Some(SettingsSkillLine::Row(src)) = lines.get(idx) else {
            self.store.push_toast(
                "Select a pending proposal to approve/reject (category headers group skills)",
                ToastMsgVariant::Info,
            );
            return;
        };
        let Some(row) = rows.get(*src) else { return };
        let SettingsSkillRow::Proposal { id, title, .. } = row else {
            self.store.push_toast(
                "Select a pending proposal to approve/reject (catalog rows are read-only)",
                ToastMsgVariant::Info,
            );
            return;
        };
        let id = id.clone();
        let title = title.clone();
        let status = if accept { "accepted" } else { "rejected" };
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        match api.update_skill_proposal_status(&id, status) {
            Ok(_) => {
                self.refresh_skills_into_store();
                self.store.push_toast(
                    &format!("Proposal {}: {}", status, title),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("{} failed: {}", status, e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// Confirm 后的 skill 删除执行（土律·第四条单点权威）：
    /// POST `/skill/manage`(Delete)，local-direct 经 `local_manage_skill` 短路；
    /// 成功则 `refresh_skills_into_store` 回灌（水律·回流同源）。
    pub(crate) fn delete_skill_action(&mut self, name: &str) {
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        // server 端 session_id 仅用于权限记账与 memory 回流；Settings 页删除
        // 可能没有活跃会话，用诚实占位标识来源面。
        let session_id = self
            .active_session
            .get_session_id()
            .unwrap_or_else(|| "tui-settings".to_string());
        let req = agendao_client::SkillManageRequest {
            session_id,
            action: agendao_client::SkillManageAction::Delete,
            name: Some(name.to_string()),
            new_name: None,
            description: None,
            body: None,
            methodology: None,
            content: None,
            category: None,
            directory_name: None,
            file_path: None,
        };
        match api.manage_skill(&req) {
            Ok(_) => {
                self.refresh_skills_into_store();
                self.store.push_toast(
                    &format!("Skill deleted: {}", name),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Delete skill failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// `t`（Skills 列表/详情聚焦，或列表行尾开关点击）：启停切换。
    /// 数据行 = `skills.disabled` 加/删精确名；类目头 = 加/删 `类目/*` 通配。
    /// 写后 PUT `/config/disabled` → 重列 catalog 回灌（水律·回流同源）。
    pub(crate) fn toggle_settings_skill(&mut self) {
        use crate::store::types::{
            flatten_settings_skill_rows, SettingsSkillLine, SettingsSkillRow,
            SKILLS_PROPOSALS_GROUP, SKILLS_UNCATEGORIZED_GROUP,
        };
        let rows = self.store.settings_skills.get();
        let collapsed = self.store.settings_skills_collapsed.get();
        let lines = flatten_settings_skill_rows(&rows, &collapsed);
        let sel = self
            .store
            .settings_skills_selected
            .get()
            .min(lines.len().saturating_sub(1));
        let Some(line) = lines.get(sel) else { return };

        enum Edit {
            Exact {
                name: String,
                category: Option<String>,
                disable: bool,
            },
            Group {
                name: String,
                disable: bool,
            },
        }
        let edit = match line {
            SettingsSkillLine::Row(src) => match &rows[*src] {
                SettingsSkillRow::Catalog {
                    name,
                    category,
                    disabled,
                    ..
                } => Edit::Exact {
                    name: name.clone(),
                    category: category.clone(),
                    disable: !disabled,
                },
                SettingsSkillRow::Proposal { .. } => {
                    self.store.push_toast(
                        "Proposals are not toggleable — approve/reject with a/r",
                        ToastMsgVariant::Info,
                    );
                    return;
                }
            },
            SettingsSkillLine::Category { name, .. } => {
                if name == SKILLS_PROPOSALS_GROUP {
                    self.store.push_toast(
                        "Proposal group has no enable switch — decide each proposal with a/r",
                        ToastMsgVariant::Info,
                    );
                    return;
                }
                if name == SKILLS_UNCATEGORIZED_GROUP {
                    self.store.push_toast(
                        "Uncategorized skills have no category wildcard — toggle each row",
                        ToastMsgVariant::Info,
                    );
                    return;
                }
                let members: Vec<&SettingsSkillRow> = rows
                    .iter()
                    .filter(|r| r.group_name() == name.as_str())
                    .collect();
                let all_disabled =
                    !members.is_empty() && members.iter().all(|r| r.is_disabled());
                Edit::Group {
                    name: name.clone(),
                    disable: !all_disabled,
                }
            }
        };

        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let Ok(config) = api.get_config() else {
            self.store.push_toast("Failed to read config", ToastMsgVariant::Error);
            return;
        };
        let mut patterns = config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();

        let toast_label = match &edit {
            Edit::Exact {
                name,
                category,
                disable,
            } => {
                let others: Vec<String> = rows
                    .iter()
                    .filter(|r| {
                        r.group_name()
                            == category
                                .as_deref()
                                .unwrap_or(crate::store::types::SKILLS_UNCATEGORIZED_GROUP)
                            && r.is_disabled()
                            && r.label() != name.as_str()
                    })
                    .map(|r| r.label().to_string())
                    .collect();
                toggle_exact_pattern(&mut patterns, name, category.as_deref(), *disable, &others);
                format!(
                    "Skill {}: {}",
                    if *disable { "disabled" } else { "enabled" },
                    name
                )
            }
            Edit::Group { name, disable } => {
                let members: Vec<String> = rows
                    .iter()
                    .filter(|r| r.group_name() == name.as_str())
                    .map(|r| r.label().to_string())
                    .collect();
                toggle_group_pattern(&mut patterns, name, &members, *disable);
                format!(
                    "Skill category {}: {} ({}/*)",
                    if *disable { "disabled" } else { "enabled" },
                    name,
                    name
                )
            }
        };

        let update = agendao_client::DisabledConfigUpdate {
            tools: None,
            skills: Some(patterns),
            plugins: None,
        };
        match api.put_disabled_config(&update) {
            Ok(_) => {
                self.refresh_skills_into_store();
                self.store.push_toast(&toast_label, ToastMsgVariant::Success);
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Toggle skill failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// `t`（Tools 列表/详情聚焦，或列表行尾开关点击）：启停切换。
    /// 数据行 = `disabled_tools` 加/删精确 id；类目头 = 加/删 `family/*` 通配。
    /// `protected`（facade/bridge）行开关锁定。写后 server 重建 tool registry
    /// （即时生效），再重列回灌。
    pub(crate) fn toggle_settings_tool(&mut self) {
        use crate::store::types::{
            flatten_settings_tool_rows, SettingsToolLine, TOOLS_UNCATEGORIZED_GROUP,
        };
        let rows = self.store.settings_tools.get();
        let collapsed = self.store.settings_tools_collapsed.get();
        let lines = flatten_settings_tool_rows(&rows, &collapsed);
        let sel = self
            .store
            .settings_tools_selected
            .get()
            .min(lines.len().saturating_sub(1));
        let Some(line) = lines.get(sel) else { return };

        enum Edit {
            Exact {
                id: String,
                family: Option<String>,
                disable: bool,
            },
            Group {
                name: String,
                disable: bool,
            },
        }
        let edit = match line {
            SettingsToolLine::Row(src) => {
                let r = &rows[*src];
                if r.protected {
                    self.store.push_toast(
                        &format!(
                            "\"{}\" is a facade/bridge tool — required for model tool access, cannot be disabled",
                            r.id
                        ),
                        ToastMsgVariant::Warning,
                    );
                    return;
                }
                Edit::Exact {
                    id: r.id.clone(),
                    family: r.family.clone(),
                    disable: !r.disabled,
                }
            }
            SettingsToolLine::Category { name, .. } => {
                if name == TOOLS_UNCATEGORIZED_GROUP {
                    self.store.push_toast(
                        "Tools without a family have no category wildcard — toggle each row",
                        ToastMsgVariant::Info,
                    );
                    return;
                }
                let members: Vec<_> = rows
                    .iter()
                    .filter(|r| r.group_name() == name.as_str())
                    .collect();
                let all_disabled = !members.is_empty() && members.iter().all(|r| r.disabled);
                Edit::Group {
                    name: name.clone(),
                    disable: !all_disabled,
                }
            }
        };

        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let Ok(config) = api.get_config() else {
            self.store.push_toast("Failed to read config", ToastMsgVariant::Error);
            return;
        };
        let mut patterns = config.disabled_tools.clone();

        let toast_label = match &edit {
            Edit::Exact {
                id,
                family,
                disable,
            } => {
                let others: Vec<String> = rows
                    .iter()
                    .filter(|r| {
                        r.group_name()
                            == family
                                .as_deref()
                                .unwrap_or(crate::store::types::TOOLS_UNCATEGORIZED_GROUP)
                            && r.disabled
                            && r.id != *id
                    })
                    .map(|r| r.id.clone())
                    .collect();
                toggle_exact_pattern(&mut patterns, id, family.as_deref(), *disable, &others);
                format!(
                    "Tool {}: {} (registry rebuilt)",
                    if *disable { "disabled" } else { "enabled" },
                    id
                )
            }
            Edit::Group { name, disable } => {
                let members: Vec<String> = rows
                    .iter()
                    .filter(|r| r.group_name() == name.as_str())
                    .map(|r| r.id.clone())
                    .collect();
                toggle_group_pattern(&mut patterns, name, &members, *disable);
                format!(
                    "Tool family {}: {} ({}/*, registry rebuilt)",
                    if *disable { "disabled" } else { "enabled" },
                    name,
                    name
                )
            }
        };

        let update = agendao_client::DisabledConfigUpdate {
            tools: Some(patterns),
            skills: None,
            plugins: None,
        };
        match api.put_disabled_config(&update) {
            Ok(_) => {
                self.refresh_tools_into_store();
                self.store.push_toast(&toast_label, ToastMsgVariant::Success);
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Toggle tool failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// `t`（MCP 列表/详情聚焦，或列表行尾开关点击）：启停切换 = 改 config.mcp
    /// 对应条目的 enabled 字段（PUT `/config/mcp/{key}` 整体覆写）。
    /// `Enabled{enabled}` 变体直接翻 bool；`Full` 变体写 `enabled` 字段；
    /// config 缺条目（runtime-only server）诚实 toast，不假装可切。
    pub(crate) fn toggle_settings_mcp_enabled(&mut self) {
        let rows = self.store.settings_mcp.get();
        let idx = self.store.settings_mcp_selected.get();
        let Some(row) = rows.get(idx) else { return };
        let name = row.name.clone();
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let Ok(config) = api.get_config() else {
            self.store.push_toast("Failed to read config", ToastMsgVariant::Error);
            return;
        };
        let Some(entry) = config.mcp.as_ref().and_then(|m| m.get(&name)) else {
            self.store.push_toast(
                &format!("\"{}\" has no config.mcp entry — toggle needs a config entry", name),
                ToastMsgVariant::Warning,
            );
            return;
        };
        let next = match entry {
            agendao_config::McpServerConfig::Enabled { enabled } => {
                agendao_config::McpServerConfig::Enabled { enabled: !enabled }
            }
            agendao_config::McpServerConfig::Full(server) => {
                let mut server = server.clone();
                let cur = server.enabled.unwrap_or(true);
                server.enabled = Some(!cur);
                agendao_config::McpServerConfig::Full(server)
            }
        };
        let disabling = row.enabled;
        match api.put_mcp_config(&name, &next) {
            Ok(_) => {
                self.refresh_mcp_into_store();
                self.store.push_toast(
                    &format!(
                        "MCP {}: {}",
                        if disabling { "disabled" } else { "enabled" },
                        name
                    ),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Toggle MCP failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// McpEditDialog Submit（a=Add / e=Edit）：组装 `McpServerConfig::Full`
    /// 走 PUT `/config/mcp/{key}`。enabled 透传（Add=true / Edit=原值）——
    /// 启停不在表单里，Edit 保存不会意外重置开关（同 model_edit prefill 语义）。
    pub(crate) fn submit_mcp_edit(&mut self, s: crate::dialog::McpEditSubmission) {
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let mut server = agendao_config::McpServer {
            server_type: Some(s.transport.label().to_string()),
            enabled: Some(s.enabled),
            ..Default::default()
        };
        match s.transport {
            crate::dialog::McpTransport::Local => {
                server.command = s.command.split_whitespace().map(str::to_string).collect();
            }
            crate::dialog::McpTransport::Remote => {
                server.url = Some(s.url.clone());
            }
        }
        let cfg = agendao_config::McpServerConfig::Full(Box::new(server));
        let is_add = matches!(s.mode, crate::dialog::McpEditMode::Add);
        match api.put_mcp_config(&s.name, &cfg) {
            Ok(_) => {
                self.refresh_mcp_into_store();
                self.store.push_toast(
                    &format!(
                        "MCP server {}: {}",
                        if is_add { "added" } else { "updated" },
                        s.name
                    ),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Save MCP server failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// Confirm 后的 MCP 删除执行（DELETE `/config/mcp/{key}`，土律·第四条单点权威）。
    pub(crate) fn delete_mcp_action(&mut self, name: &str) {
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        match api.delete_mcp_config(name) {
            Ok(_) => {
                self.refresh_mcp_into_store();
                self.store.push_toast(
                    &format!("MCP server deleted: {}", name),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Delete MCP server failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    // ── Settings→Plugins ──

    /// 拉已安装插件列表回灌 `store.settings_plugins`。OpenSettings 与
    /// 启停/删除/安装写后调用（水律·回流同源）。
    pub(crate) fn refresh_plugins_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        match api.list_plugins() {
            Ok(plugins) => {
                let rows: Vec<crate::store::types::SettingsPluginRow> = plugins
                    .into_iter()
                    .map(|p| crate::store::types::SettingsPluginRow {
                        name: p.name,
                        plugin_type: p.plugin_type,
                        managed: p.source == "managed",
                        version: p.version,
                        path: p.path,
                        origin: p.origin,
                        disabled: p.disabled,
                    })
                    .collect();
                let prev = self.store.settings_plugins_selected.get();
                let n = rows.len();
                self.store.settings_plugins.set(rows);
                if n == 0 {
                    self.store.settings_plugins_selected.set(0);
                } else if prev >= n {
                    self.store.settings_plugins_selected.set(n - 1);
                }
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load plugins: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// `t`（Plugins 列表/详情聚焦，或列表行尾开关点击）：启停切换 = 写顶层
    /// `disabled_plugins`（PUT `/config/disabled` plugins 字段）。插件无分组，
    /// 只按精确名启停；被 `前缀/*` 通配覆盖的行启用时展开通配保住其余成员
    /// （复用 toggle_exact_pattern 单点权威）。
    pub(crate) fn toggle_settings_plugin(&mut self) {
        let rows = self.store.settings_plugins.get();
        let sel = self
            .store
            .settings_plugins_selected
            .get()
            .min(rows.len().saturating_sub(1));
        let Some(row) = rows.get(sel) else { return };
        let name = row.name.clone();
        let disable = !row.disabled;

        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let Ok(config) = api.get_config() else {
            self.store.push_toast("Failed to read config", ToastMsgVariant::Error);
            return;
        };
        let mut patterns = config.disabled_plugins.clone();
        let others: Vec<String> = rows
            .iter()
            .filter(|r| r.disabled && r.name != name)
            .map(|r| r.name.clone())
            .collect();
        toggle_exact_pattern(&mut patterns, &name, None, disable, &others);

        let update = agendao_client::DisabledConfigUpdate {
            tools: None,
            skills: None,
            plugins: Some(patterns),
        };
        match api.put_disabled_config(&update) {
            Ok(_) => {
                self.refresh_plugins_into_store();
                self.store.push_toast(
                    &format!(
                        "Plugin {}: {}",
                        if disable { "disabled" } else { "enabled" },
                        name
                    ),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Toggle plugin failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// Confirm 后的插件删除执行（DELETE `/config/plugin/{key}`，managed 条目；
    /// discovered 条目在 keymap 前置拦截，不会到此）。
    pub(crate) fn delete_plugin_action(&mut self, name: &str) {
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        match api.delete_plugin_config(name) {
            Ok(_) => {
                self.refresh_plugins_into_store();
                self.store.push_toast(
                    &format!("Plugin deleted: {}", name),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Delete plugin failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    /// PluginEditDialog Submit（a=安装）：向 config.plugin 写一条 file 类型条目
    /// （PUT `/config/plugin/{key}`，土律·第四条单点权威）。
    pub(crate) fn install_plugin_action(&mut self, s: crate::dialog::PluginEditSubmission) {
        let Some(api) = self.api.as_ref() else {
            self.store.push_toast("No API bridge", ToastMsgVariant::Error);
            return;
        };
        let cfg = agendao_config::PluginConfig {
            plugin_type: "file".to_string(),
            path: Some(s.path.clone()),
            ..Default::default()
        };
        match api.put_plugin_config(&s.name, &cfg) {
            Ok(_) => {
                self.refresh_plugins_into_store();
                self.store.push_toast(
                    &format!("Plugin installed: {} ({})", s.name, s.path),
                    ToastMsgVariant::Success,
                );
                self.layout_dirty = true;
            }
            Err(e) => self.store.push_toast(
                &format!("Install plugin failed: {}", e),
                ToastMsgVariant::Error,
            ),
        }
    }

    fn sync_mcp_dialog_from_store(&mut self) {
        if !self.mcp_list.is_open() {
            return;
        }
        let entries: Vec<crate::dialog::McpEntry> = self
            .store
            .settings_mcp
            .get()
            .into_iter()
            .map(|r| crate::dialog::McpEntry {
                name: r.name,
                status: r.status,
                tools: r.tools,
                resources: r.resources,
            })
            .collect();
        self.mcp_list.set_entries(entries);
    }
}

/// 从 config.mcp 条目派生展示字段：(transport, command, url, enabled)。
/// - `Enabled{enabled}` 变体：无 transport 信息 → `unknown`；
/// - `Full`：有 url → `remote`，否则有 command → `local`，再否则 server_type
///   原样，缺省 `unknown`；enabled 缺省 true（与 server 端 `unwrap_or(true)` 同源）。
fn mcp_config_fields(
    cfg: Option<&agendao_config::McpServerConfig>,
) -> (String, Option<String>, Option<String>, bool) {
    match cfg {
        Some(agendao_config::McpServerConfig::Enabled { enabled }) => {
            ("unknown".to_string(), None, None, *enabled)
        }
        Some(agendao_config::McpServerConfig::Full(server)) => {
            let transport = if server.url.is_some() {
                "remote"
            } else if !server.command.is_empty() {
                "local"
            } else {
                server.server_type.as_deref().unwrap_or("unknown")
            };
            let command = if server.command.is_empty() {
                None
            } else {
                Some(server.command.join(" "))
            };
            (
                transport.to_string(),
                command,
                server.url.clone(),
                server.enabled.unwrap_or(true),
            )
        }
        None => ("unknown".to_string(), None, None, true),
    }
}

/// 单项启停的 pattern 列表改写（skills.disabled / disabled_tools 共用）。
///
/// - disable=true：加入精确名（去重）。
/// - disable=false：移除精确名；若该行仍被 `category/*` 通配（或同名精确类目
///   pattern）覆盖，则展开通配——移除覆盖它的 pattern，并把同组其余 disabled
///   成员改写为精确名（保住它们的禁用态，只放行当前行）。
fn toggle_exact_pattern(
    patterns: &mut Vec<String>,
    name: &str,
    category: Option<&str>,
    disable: bool,
    other_disabled_members: &[String],
) {
    if disable {
        if !patterns.iter().any(|p| p == name) {
            patterns.push(name.to_string());
        }
        return;
    }
    patterns.retain(|p| p != name);
    let covered = agendao_config::matching::matching_disabled_pattern(patterns, name).is_some()
        || category.is_some_and(|c| {
            agendao_config::matching::matching_disabled_pattern(patterns, c).is_some()
        });
    if !covered {
        return;
    }
    patterns.retain(|p| {
        let single = [p.clone()];
        agendao_config::matching::matching_disabled_pattern(&single, name).is_none()
            && category.is_none_or(|c| {
                agendao_config::matching::matching_disabled_pattern(&single, c).is_none()
            })
    });
    for member in other_disabled_members {
        if !patterns.iter().any(|p| p == member) {
            patterns.push(member.clone());
        }
    }
}

/// 类目头启停的 pattern 列表改写。
///
/// - disable=true：移除类目通配/类目精确/组内成员精确 pattern 后加入 `name/*`
///   （单一通配承载整组，无残留精确项）。
/// - disable=false：移除任何覆盖该类目的 pattern（含父级前缀通配）及组内
///   成员精确名——整组回到启用态。
fn toggle_group_pattern(
    patterns: &mut Vec<String>,
    name: &str,
    members: &[String],
    disable: bool,
) {
    patterns.retain(|p| {
        let single = [p.clone()];
        p.trim() != name
            && agendao_config::matching::matching_disabled_pattern(&single, name).is_none()
            && !members.iter().any(|m| m == p)
    });
    if disable {
        patterns.push(format!("{}/*", name));
    }
}

#[cfg(test)]
mod tests {
    use super::{toggle_exact_pattern, toggle_group_pattern};

    fn list(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn exact_disable_appends_once() {
        let mut p = list(&[]);
        toggle_exact_pattern(&mut p, "bash", Some("shell"), true, &[]);
        toggle_exact_pattern(&mut p, "bash", Some("shell"), true, &[]);
        assert_eq!(p, list(&["bash"]));
    }

    #[test]
    fn exact_enable_removes_entry() {
        let mut p = list(&["bash", "read"]);
        toggle_exact_pattern(&mut p, "bash", Some("shell"), false, &[]);
        assert_eq!(p, list(&["read"]));
    }

    #[test]
    fn exact_enable_under_wildcard_expands_siblings() {
        // "shell" 类目被 shell/* 整禁；单启 bash → 通配展开为其余 disabled 成员精确名。
        let mut p = list(&["shell/*"]);
        toggle_exact_pattern(
            &mut p,
            "bash",
            Some("shell"),
            false,
            &list(&["exec", "shell_session"]),
        );
        assert_eq!(p, list(&["exec", "shell_session"]));
    }

    #[test]
    fn exact_enable_under_wildcard_keeps_other_exact_entries() {
        let mut p = list(&["shell/*", "web_search"]);
        toggle_exact_pattern(&mut p, "bash", Some("shell"), false, &list(&["exec"]));
        assert_eq!(p, list(&["web_search", "exec"]));
    }

    #[test]
    fn group_disable_collapses_to_single_wildcard() {
        let mut p = list(&["bash", "exec", "unrelated"]);
        toggle_group_pattern(&mut p, "shell", &list(&["bash", "exec"]), true);
        assert_eq!(p, list(&["unrelated", "shell/*"]));
    }

    #[test]
    fn group_enable_removes_wildcard_and_member_entries() {
        let mut p = list(&["shell/*", "bash", "unrelated"]);
        toggle_group_pattern(&mut p, "shell", &list(&["bash", "exec"]), false);
        assert_eq!(p, list(&["unrelated"]));
    }

    #[test]
    fn group_enable_also_removes_exact_category_name_pattern() {
        // 精确类目名 pattern（"shell" 无 /*）同样整禁该组——类目头 enable 必须
        // 把它一并移除，否则行仍然 Off。
        let mut p = list(&["shell"]);
        toggle_group_pattern(&mut p, "shell", &list(&["a", "b"]), false);
        assert!(p.is_empty());
    }

    #[test]
    fn mcp_fields_full_local_infers_transport() {
        let cfg = agendao_config::McpServerConfig::Full(Box::new(
            agendao_config::McpServer {
                command: vec!["npx".into(), "srv".into()],
                enabled: Some(false),
                ..Default::default()
            },
        ));
        let (transport, command, url, enabled) =
            super::mcp_config_fields(Some(&cfg));
        assert_eq!(transport, "local");
        assert_eq!(command.as_deref(), Some("npx srv"));
        assert!(url.is_none());
        assert!(!enabled);
    }

    #[test]
    fn mcp_fields_full_remote_wins_by_url() {
        let cfg = agendao_config::McpServerConfig::Full(Box::new(
            agendao_config::McpServer {
                url: Some("https://mcp.example.com".into()),
                ..Default::default()
            },
        ));
        let (transport, command, url, enabled) =
            super::mcp_config_fields(Some(&cfg));
        assert_eq!(transport, "remote");
        assert!(command.is_none());
        assert_eq!(url.as_deref(), Some("https://mcp.example.com"));
        assert!(enabled, "enabled 缺省 true（同 server unwrap_or(true)）");
    }

    #[test]
    fn mcp_fields_enabled_variant_is_unknown_transport() {
        let cfg = agendao_config::McpServerConfig::Enabled { enabled: false };
        let (transport, command, url, enabled) =
            super::mcp_config_fields(Some(&cfg));
        assert_eq!(transport, "unknown");
        assert!(command.is_none() && url.is_none());
        assert!(!enabled);
    }

    #[test]
    fn mcp_fields_missing_entry_defaults_enabled_unknown() {
        let (transport, _, _, enabled) = super::mcp_config_fields(None);
        assert_eq!(transport, "unknown");
        assert!(enabled);
    }
}
