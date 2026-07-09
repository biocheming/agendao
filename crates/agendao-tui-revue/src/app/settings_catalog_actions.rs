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
    /// 拉 MCP 状态回灌 `store.settings_mcp`。OpenSettings / connect/disconnect 后调用。
    pub(crate) fn refresh_mcp_into_store(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        match api.get_mcp_status() {
            Ok(servers) => {
                let rows: Vec<SettingsMcpRow> = servers
                    .into_iter()
                    .map(|s| SettingsMcpRow {
                        name: s.name,
                        status: s.status,
                        tools: s.tools,
                        resources: s.resources,
                        error: s.error,
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

        match api.list_skills(None) {
            Ok(skills) => {
                for s in skills {
                    rows.push(SettingsSkillRow::Catalog {
                        name: s.name,
                        description: s.description,
                        location: s.location,
                        category: s.category,
                        writable: s.writable,
                    });
                }
            }
            Err(e) => self.store.push_toast(
                &format!("Failed to load skills: {}", e),
                ToastMsgVariant::Error,
            ),
        }

        let prev = self.store.settings_skills_selected.get();
        let n = rows.len();
        self.store.settings_skills.set(rows);
        if n == 0 {
            self.store.settings_skills_selected.set(0);
        } else if prev >= n {
            self.store.settings_skills_selected.set(n - 1);
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
    /// catalog 行上按 a/r → toast 提示(诚实标注,非 proposal)。
    pub(crate) fn decide_settings_skill_proposal(&mut self, accept: bool) {
        let rows = self.store.settings_skills.get();
        let idx = self.store.settings_skills_selected.get();
        let Some(row) = rows.get(idx) else { return };
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
