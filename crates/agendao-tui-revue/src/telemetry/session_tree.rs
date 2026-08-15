//! 水 — Session navigation tree builder.
//!
//! 从 `AppStore.session_list` 构建 sidebar「Session Tree」的 `TreeNode` 列表,
//! 语义对齐 web `buildSessionTree`(parent_id 父子 fork 关系 + 按 updated 排序)。
//! 每个节点携带 `TreeIntent::NavigateSession(id)` —— sidebar 点击/open_session
//! 的唯一数据权威(土律·第四条·单点权威)。
//!
//! 与 execution topology(`SessionProjectionReplaced`)分离:topology 是运行中
//! agent/stage 树,不是会话导航树;二者不得混写进 `session_nodes`(之前
//! event_handler 覆写 session_nodes 导致点击无 NavigateSession intent)。

use crate::store::types::{SessionListItem, TreeIntent, TreeNode};

/// API `SessionListItem` → store `SessionListItem` 单点映射(土律·第四条)。
pub fn map_api_session_item(s: &agendao_client::SessionListItem) -> SessionListItem {
    SessionListItem {
        id: s.id.clone(),
        title: s.title.clone(),
        run_status: None,
        parent_id: s.parent_id.clone(),
        directory: s.directory.clone(),
        updated: s.time.updated,
    }
}

/// 从 session 列表 + 当前工作目录构建 sidebar 导航树。
///
/// `workspace_path` = `AppStore.working_dir`(canonical cwd);只包含
/// `directory == workspace_path` 的会话,按 fork 关系(parent_id)组树,
/// 根节点与子节点均按 id 映射后递归展开。每个节点 intent =
/// `NavigateSession(session_id)`。
///
/// `expanded_ids` = 展开态唯一权威(AppHandler 持有):默认**全部折叠**
/// （只显示 root,sub session 不占视野）;命中集合的节点才展开——
/// 此前硬编码 expanded:true 全展开,长 fork 链把 Session Tree 顶满。
pub fn build_session_nav_tree(
    sessions: &[SessionListItem],
    workspace_path: &str,
    expanded_ids: &std::collections::HashSet<String>,
) -> Vec<TreeNode> {
    if workspace_path.is_empty() {
        return Vec::new();
    }
    let workspace: Vec<&SessionListItem> = sessions
        .iter()
        .filter(|s| s.directory.trim() == workspace_path.trim())
        .collect();
    if workspace.is_empty() {
        return Vec::new();
    }

    let id_set: std::collections::HashSet<&str> = workspace.iter().map(|s| s.id.as_str()).collect();

    // parent_id → children
    let mut child_map: std::collections::HashMap<&str, Vec<&SessionListItem>> =
        std::collections::HashMap::new();
    for s in &workspace {
        if let Some(ref pid) = s.parent_id {
            if id_set.contains(pid.as_str()) {
                child_map.entry(pid.as_str()).or_default().push(s);
            }
        }
    }

    // Roots: no parent, or parent not in this workspace set.
    let mut roots: Vec<&SessionListItem> = workspace
        .iter()
        .copied()
        .filter(|s| {
            s.parent_id
                .as_ref()
                .map(|pid| !id_set.contains(pid.as_str()))
                .unwrap_or(true)
        })
        .collect();
    // Most recently updated first (web parity).
    roots.sort_by(|a, b| b.updated.cmp(&a.updated));

    roots
        .into_iter()
        .map(|s| visit(s, &child_map, 0, expanded_ids))
        .collect()
}

fn visit(
    session: &SessionListItem,
    child_map: &std::collections::HashMap<&str, Vec<&SessionListItem>>,
    depth: u8,
    expanded_ids: &std::collections::HashSet<String>,
) -> TreeNode {
    let mut children: Vec<&SessionListItem> = child_map
        .get(session.id.as_str())
        .cloned()
        .unwrap_or_default();
    children.sort_by(|a, b| b.updated.cmp(&a.updated));
    TreeNode {
        label: session.title.clone(),
        depth,
        expanded: expanded_ids.contains(&session.id),
        children: children
            .into_iter()
            .map(|c| visit(c, child_map, depth.saturating_add(1), expanded_ids))
            .collect(),
        intent: Some(TreeIntent::NavigateSession(session.id.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        id: &str,
        title: &str,
        parent: Option<&str>,
        dir: &str,
        updated: i64,
    ) -> SessionListItem {
        SessionListItem {
            id: id.into(),
            title: title.into(),
            run_status: None,
            parent_id: parent.map(str::to_string),
            directory: dir.into(),
            updated,
        }
    }

    #[test]
    fn builds_fork_tree_with_navigate_intent() {
        let dir = "/proj";
        let sessions = vec![
            item("root", "Root session", None, dir, 200),
            item("fork1", "Fork one", Some("root"), dir, 100),
            item("other", "Other root", None, dir, 150),
        ];
        let tree = build_session_nav_tree(&sessions, dir, &Default::default());
        assert_eq!(tree.len(), 2, "two roots: root + other");
        assert_eq!(tree[0].label, "Root session");
        assert!(matches!(
            tree[0].intent,
            Some(TreeIntent::NavigateSession(ref id)) if id == "root"
        ));
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].label, "Fork one");
        assert!(matches!(
            tree[0].children[0].intent,
            Some(TreeIntent::NavigateSession(ref id)) if id == "fork1"
        ));
    }

    #[test]
    fn filters_by_workspace_directory() {
        let sessions = vec![
            item("a", "A", None, "/proj", 100),
            item("b", "B", None, "/other", 100),
        ];
        let tree = build_session_nav_tree(&sessions, "/proj", &Default::default());
        assert_eq!(tree.len(), 1);
        assert!(matches!(
            tree[0].intent,
            Some(TreeIntent::NavigateSession(ref id)) if id == "a"
        ));
    }

    #[test]
    fn empty_when_no_sessions_in_workspace() {
        assert!(build_session_nav_tree(&[], "/proj", &Default::default()).is_empty());
    }

    #[test]
    fn collapsed_by_default_and_expanded_ids_honored() {
        let dir = "/proj";
        let sessions = vec![
            item("root", "Root session", None, dir, 200),
            item("fork1", "Fork one", Some("root"), dir, 100),
        ];
        // 默认（空集合）：全部折叠——root 节点 expanded=false（子节点仍在树内,
        // 由 flatten 按 expanded 决定是否渲染）。
        let collapsed = build_session_nav_tree(&sessions, dir, &Default::default());
        assert_eq!(collapsed.len(), 1);
        assert!(!collapsed[0].expanded, "default must be collapsed");
        assert_eq!(
            collapsed[0].children.len(),
            1,
            "children kept in tree structure"
        );
        // 命中展开集合：仅该节点展开。
        let ids: std::collections::HashSet<String> = ["root".to_string()].into_iter().collect();
        let expanded = build_session_nav_tree(&sessions, dir, &ids);
        assert!(expanded[0].expanded, "expanded_ids member must expand");
        assert!(!expanded[0].children[0].expanded, "child stays collapsed");
    }
}
