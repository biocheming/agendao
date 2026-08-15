use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("Not a git repository")]
    NotGitRepo,
    #[error("Git command failed: {0}")]
    GitError(String),
    #[error("Worktree not found: {0}")]
    NotFound(String),
    #[error("Invalid branch name: {0}")]
    InvalidBranch(String),
    #[error("Worktree target path is outside allowed locations: {0}")]
    PathNotAllowed(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub head: String,
}

fn run_git(args: &[&str], cwd: &Path) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| WorktreeError::GitError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::GitError(stderr.to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists() || run_git(&["rev-parse", "--git-dir"], path).is_ok()
}

pub fn list_worktrees(repo_path: &Path) -> Result<Vec<WorktreeInfo>, WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let output = run_git(&["worktree", "list", "--porcelain"], repo_path)?;

    let mut worktrees = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;
    let mut current_head: Option<String> = None;

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let (Some(path), Some(branch), Some(head)) =
                (&current_path, &current_branch, &current_head)
            {
                worktrees.push(WorktreeInfo {
                    path: path.clone(),
                    branch: branch.clone(),
                    head: head.clone(),
                });
            }
            current_path = Some(path.to_string());
            current_branch = None;
            current_head = None;
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            current_head = Some(head.to_string());
        } else if let Some(branch_full) = line.strip_prefix("branch ") {
            let branch = branch_full
                .strip_prefix("refs/heads/")
                .unwrap_or(branch_full);
            current_branch = Some(branch.to_string());
        }
    }

    if let (Some(path), Some(branch), Some(head)) = (&current_path, &current_branch, &current_head)
    {
        worktrees.push(WorktreeInfo {
            path: path.clone(),
            branch: branch.clone(),
            head: head.clone(),
        });
    } else if let (Some(path), Some(head)) = (&current_path, &current_head) {
        worktrees.push(WorktreeInfo {
            path: path.clone(),
            branch: "HEAD".to_string(),
            head: head.clone(),
        });
    }

    Ok(worktrees)
}

/// 仿 `git check-ref-format` 核心规则的保守白名单校验：
/// 只允许 `[A-Za-z0-9._/-]`，不以 `-`/`/` 开头结尾，不含 `..`、`@{`，
/// 不以 `.lock` 或 `.` 结尾，单段不能为空（`//`）且不能以 `.` 开头。
fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
    {
        return false;
    }
    if name.starts_with('-') || name.starts_with('/') {
        return false;
    }
    if name.ends_with('-') || name.ends_with('/') || name.ends_with('.') {
        return false;
    }
    if name.contains("..") || name.contains("@{") || name.ends_with(".lock") {
        return false;
    }
    if name
        .split('/')
        .any(|segment| segment.is_empty() || segment.starts_with('.'))
    {
        return false;
    }
    true
}

/// 词法归一化（不触盘）：解析 `.` 与 `..`，`..` 越过根时截断在根。
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 宽松版 canonicalize：不存在的尾部组件逐层回退到最近的现存祖先，
/// canonicalize 该祖先（解析符号链接）后再把尾部组件拼回。
fn resolve_loosely(path: &Path) -> PathBuf {
    let normalized = normalize_lexically(path);
    let mut missing: Vec<OsString> = Vec::new();
    let mut cursor = normalized.as_path();
    loop {
        if let Ok(canonical) = cursor.canonicalize() {
            let mut resolved = canonical;
            for comp in missing.iter().rev() {
                resolved.push(comp);
            }
            return resolved;
        }
        match cursor.file_name() {
            Some(name) => {
                missing.push(name.to_os_string());
                cursor = cursor.parent().unwrap_or(cursor);
            }
            None => return normalized,
        }
    }
}

/// worktree 目标路径策略：`resolved` 必须位于以下之一——
/// (a) 项目根目录之内；(b) agendao home 的 `worktrees` 目录之内；
/// (c) 项目根目录的直接父目录之下（兄弟目录，worktree 惯例位置）。
fn is_allowed_worktree_path(resolved: &Path, project_root: &Path, home_worktrees: &Path) -> bool {
    let root = resolve_loosely(project_root);
    if resolved.starts_with(&root) {
        return true;
    }
    if resolved.starts_with(resolve_loosely(home_worktrees)) {
        return true;
    }
    match root.parent() {
        Some(parent) => resolved.starts_with(parent),
        None => false,
    }
}

pub fn create_worktree(
    repo_path: &Path,
    branch: Option<&str>,
    target_path: Option<&str>,
) -> Result<WorktreeInfo, WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let default_branch_name = format!("worktree-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let branch_name = branch.unwrap_or(&default_branch_name);

    if !is_valid_branch_name(branch_name) {
        return Err(WorktreeError::InvalidBranch(branch_name.to_string()));
    }

    let repo_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let default_worktree_path = format!("{}-{}", repo_name, branch_name);

    let worktree_path = target_path
        .map(|s| s.to_string())
        .unwrap_or(default_worktree_path);

    let full_path = if Path::new(&worktree_path).is_absolute() {
        worktree_path.clone()
    } else {
        repo_path
            .parent()
            .map(|p| p.join(&worktree_path))
            .unwrap_or_else(|| repo_path.join(&worktree_path))
            .to_string_lossy()
            .to_string()
    };

    let resolved_path = resolve_loosely(Path::new(&full_path));
    let home_worktrees = agendao_util::agendao_home().join("worktrees");
    if !is_allowed_worktree_path(&resolved_path, repo_path, &home_worktrees) {
        return Err(WorktreeError::PathNotAllowed(full_path));
    }

    let branch_exists = run_git(&["branch", "--list", branch_name], repo_path)
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    if branch_exists {
        run_git(
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                &full_path,
                branch_name,
            ],
            repo_path,
        )?;
    } else {
        let default_branch = run_git(&["symbolic-ref", "--short", "HEAD"], repo_path)
            .unwrap_or_else(|_| "main".to_string());

        run_git(
            &[
                "worktree",
                "add",
                "-b",
                branch_name,
                &full_path,
                &default_branch,
            ],
            repo_path,
        )?;
    }

    let head = run_git(&["rev-parse", "HEAD"], Path::new(&full_path))?;

    Ok(WorktreeInfo {
        path: full_path,
        branch: branch_name.to_string(),
        head,
    })
}

pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &str,
    force: bool,
) -> Result<(), WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    let mut args = vec!["worktree", "remove", worktree_path];
    if force {
        args.push("--force");
    }

    run_git(&args, repo_path)?;
    Ok(())
}

pub fn prune_worktrees(repo_path: &Path) -> Result<(), WorktreeError> {
    if !is_git_repo(repo_path) {
        return Err(WorktreeError::NotGitRepo);
    }

    run_git(&["worktree", "prune"], repo_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_branch_names() {
        for name in [
            "main",
            "dev-2.x",
            "feature/login",
            "feature/nested/deep",
            "fix-123",
            "release_v1.2.3",
            "user.name/topic_x",
            "worktree-20260719-074936",
        ] {
            assert!(is_valid_branch_name(name), "should accept: {name}");
        }
    }

    #[test]
    fn rejects_invalid_branch_names() {
        for name in [
            "",
            "-bad",
            "bad-",
            "/bad",
            "bad/",
            "trailing.",
            "a..b",
            "a@{b}",
            "foo.lock",
            ".hidden",
            "a/.b",
            "a//b",
            "with space",
            "with~tilde",
            "with^caret",
            "with:colon",
            "with?question",
            "with*star",
            "with[bracket",
            "with\\backslash",
            "a;b",
            "$(id)",
            "--upload-pack=evil",
        ] {
            assert!(!is_valid_branch_name(name), "should reject: {name}");
        }
    }

    /// (tmp, project_root, home_worktrees)：root 为 workspace/repo，home 不预先创建。
    fn setup_dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace").join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let home = tmp.path().join("agendao-home").join("worktrees");
        (tmp, root, home)
    }

    #[test]
    fn allows_sibling_directory_of_project_root() {
        let (_tmp, root, home) = setup_dirs();
        let target = root.parent().unwrap().join("repo-feature-x");
        let resolved = resolve_loosely(&target);
        assert!(is_allowed_worktree_path(&resolved, &root, &home));
    }

    #[test]
    fn allows_path_inside_project_root() {
        let (_tmp, root, home) = setup_dirs();
        let resolved = resolve_loosely(&root.join("nested").join("wt"));
        assert!(is_allowed_worktree_path(&resolved, &root, &home));
    }

    #[test]
    fn allows_agendao_home_worktrees_dir() {
        let (_tmp, root, home) = setup_dirs();
        let resolved = resolve_loosely(&home.join("some").join("wt"));
        assert!(is_allowed_worktree_path(&resolved, &root, &home));
    }

    #[test]
    fn rejects_unrelated_absolute_path() {
        let (_tmp, root, home) = setup_dirs();
        let other = tempfile::tempdir().unwrap();
        let resolved = resolve_loosely(&other.path().join("wt"));
        assert!(!is_allowed_worktree_path(&resolved, &root, &home));
    }

    #[test]
    fn rejects_parent_escape_via_dotdot() {
        let (tmp, root, home) = setup_dirs();
        // root = workspace/repo，`../..` 逃出 workspace（兄弟目录的上一层）。
        let resolved = resolve_loosely(&root.join("..").join("..").join("escape-wt"));
        assert_eq!(
            resolved,
            tmp.path().canonicalize().unwrap().join("escape-wt")
        );
        assert!(!is_allowed_worktree_path(&resolved, &root, &home));
    }

    #[test]
    fn resolve_loosely_resolves_existing_prefix_and_keeps_missing_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let resolved = resolve_loosely(&tmp.path().join("a").join("b").join("c"));
        assert_eq!(
            resolved,
            tmp.path()
                .canonicalize()
                .unwrap()
                .join("a")
                .join("b")
                .join("c")
        );
    }

    #[test]
    fn resolve_loosely_normalizes_dot_and_dotdot() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("x");
        std::fs::create_dir(&dir).unwrap();
        let resolved = resolve_loosely(&dir.join(".").join("..").join("y"));
        assert_eq!(resolved, tmp.path().canonicalize().unwrap().join("y"));
    }
}
