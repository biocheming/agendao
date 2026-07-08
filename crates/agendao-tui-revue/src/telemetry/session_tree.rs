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
pub fn build_session_nav_tree(
    sessions: &[SessionListItem],
    workspace_path: &str,
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

    let id_set: std::collections::HashSet<&str> =
        workspace.iter().map(|s| s.id.as_str()).collect();

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
        .map(|s| visit(s, &child_map, 0))
        .collect()
}

fn visit(
    session: &SessionListItem,
    child_map: &std::collections::HashMap<&str, Vec<&SessionListItem>>,
    depth: u8,
) -> TreeNode {
    let mut children: Vec<&SessionListItem> =
        child_map.get(session.id.as_str()).cloned().unwrap_or_default();
    children.sort_by(|a, b| b.updated.cmp(&a.updated));
    TreeNode {
        label: session.title.clone(),
        depth,
        expanded: true,
        children: children
            .into_iter()
            .map(|c| visit(c, child_map, depth.saturating_add(1)))
            .collect(),
        intent: Some(TreeIntent::NavigateSession(session.id.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str, parent: Option<&str>, dir: &str, updated: i64) -> SessionListItem {
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
        let tree = build_session_nav_tree(&sessions, dir);
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
        let tree = build_session_nav_tree(&sessions, "/proj");
        assert_eq!(tree.len(), 1);
        assert!(matches!(
            tree[0].intent,
            Some(TreeIntent::NavigateSession(ref id)) if id == "a"
        ));
    }

    #[test]
    fn empty_when_no_sessions_in_workspace() {
        assert!(build_session_nav_tree(&[], "/proj").is_empty());
    }
}
