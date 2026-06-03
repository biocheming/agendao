use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::git_runtime::{ensure_git_available, run_git_command, DEFAULT_GIT_TIMEOUT_SECS};
use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const MAX_OUTPUT_CHARS: usize = 25_000;

const DESCRIPTION: &str = r#"Structured local git history for the current working repository.

Implemented operations:
- status
- head
- log
- show_commit
- diff_uncommitted
- blame

This tool is a thin, read-only wrapper around the local `git` executable.
It exists to give models stable, structured git semantics instead of free-form shell output."#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RepoHistoryOperation {
    Status,
    Head,
    Log,
    ShowCommit,
    DiffUncommitted,
    Blame,
}

impl RepoHistoryOperation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Head => "head",
            Self::Log => "log",
            Self::ShowCommit => "show_commit",
            Self::DiffUncommitted => "diff_uncommitted",
            Self::Blame => "blame",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoHistoryInput {
    operation: RepoHistoryOperation,
    #[serde(default)]
    path: Option<String>,
    #[serde(default, alias = "sha")]
    commit: Option<String>,
    #[serde(default)]
    line_start: Option<u64>,
    #[serde(default)]
    line_end: Option<u64>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

#[derive(Debug, Clone, Serialize)]
struct RepoHeadView {
    repo_root: String,
    branch: Option<String>,
    head_sha: String,
    detached: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RepoStatusEntry {
    staged: String,
    unstaged: String,
    path: String,
    original_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RepoStatusView {
    repo_root: String,
    branch_summary: String,
    entries: Vec<RepoStatusEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct RepoLogItem {
    sha: String,
    author: String,
    email: String,
    date: String,
    subject: String,
    body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RepoCommitView {
    repo_root: String,
    sha: String,
    author: String,
    email: String,
    date: String,
    subject: String,
    body: Option<String>,
    stats: String,
}

#[derive(Debug, Clone, Serialize)]
struct RepoBlameLine {
    line_number: u64,
    commit: String,
    author: Option<String>,
    email: Option<String>,
    summary: Option<String>,
    content: String,
}

pub struct RepoHistoryTool;

impl RepoHistoryTool {
    pub fn new() -> Self {
        Self
    }

    async fn execute_impl(
        &self,
        input: &RepoHistoryInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        ensure_git_available()?;
        let repo_root = resolve_repo_root(ctx).await?;
        match input.operation {
            RepoHistoryOperation::Status => self.status(input, &repo_root, ctx).await,
            RepoHistoryOperation::Head => self.head(&repo_root, ctx).await,
            RepoHistoryOperation::Log => self.log(input, &repo_root, ctx).await,
            RepoHistoryOperation::ShowCommit => self.show_commit(input, &repo_root, ctx).await,
            RepoHistoryOperation::DiffUncommitted => {
                self.diff_uncommitted(input, &repo_root, ctx).await
            }
            RepoHistoryOperation::Blame => self.blame(input, &repo_root, ctx).await,
        }
    }

    async fn status(
        &self,
        input: &RepoHistoryInput,
        repo_root: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let mut args = vec![
            "status".to_string(),
            "--short".to_string(),
            "--branch".to_string(),
            "--untracked-files=all".to_string(),
        ];
        if let Some(path) = normalize_repo_path(input.path.as_deref(), repo_root)? {
            args.push("--".to_string());
            args.push(path);
        }
        let raw = run_git_command(&args, Some(repo_root), ctx, DEFAULT_GIT_TIMEOUT_SECS).await?;
        let view = parse_status_output(repo_root, &raw);
        let output = render_status(&view);
        let mut metadata = base_metadata(input, repo_root);
        metadata.insert("status".to_string(), serde_json::to_value(&view).unwrap());
        metadata.insert("count".to_string(), serde_json::json!(view.entries.len()));
        Ok(ToolResult {
            title: "Repository Status".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn head(&self, repo_root: &Path, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let view = load_head_view(repo_root, ctx).await?;
        let output = render_head(&view);
        let mut metadata = Metadata::new();
        metadata.insert("operation".to_string(), serde_json::json!("head"));
        metadata.insert("head".to_string(), serde_json::to_value(&view).unwrap());
        Ok(ToolResult {
            title: "Repository Head".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn log(
        &self,
        input: &RepoHistoryInput,
        repo_root: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let mut args = vec![
            "log".to_string(),
            format!("-n{}", input.limit),
            "--date=iso-strict".to_string(),
            "--format=%H%x1f%an%x1f%ae%x1f%ad%x1f%s%x1f%b%x1e".to_string(),
        ];
        if let Some(path) = normalize_repo_path(input.path.as_deref(), repo_root)? {
            args.push("--".to_string());
            args.push(path);
        }
        let raw = run_git_command(&args, Some(repo_root), ctx, DEFAULT_GIT_TIMEOUT_SECS).await?;
        let items = parse_log_output(&raw);
        let output = render_log(repo_root, &items);
        let mut metadata = base_metadata(input, repo_root);
        metadata.insert("commits".to_string(), serde_json::to_value(&items).unwrap());
        metadata.insert("count".to_string(), serde_json::json!(items.len()));
        Ok(ToolResult {
            title: "Repository Log".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn show_commit(
        &self,
        input: &RepoHistoryInput,
        repo_root: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let commit = input.commit.as_deref().unwrap_or_default().trim();
        let summary_raw = run_git_command(
            &[
                "show".to_string(),
                "-s".to_string(),
                "--date=iso-strict".to_string(),
                "--format=%H%x1f%an%x1f%ae%x1f%ad%x1f%s%x1f%b%x1e".to_string(),
                commit.to_string(),
            ],
            Some(repo_root),
            ctx,
            DEFAULT_GIT_TIMEOUT_SECS,
        )
        .await?;
        let item = parse_log_output(&summary_raw)
            .into_iter()
            .next()
            .ok_or_else(|| {
                ToolError::ExecutionError(format!("commit `{}` was not found", commit))
            })?;
        let stats = run_git_command(
            &[
                "show".to_string(),
                "--stat".to_string(),
                "--summary".to_string(),
                "--format=".to_string(),
                commit.to_string(),
            ],
            Some(repo_root),
            ctx,
            DEFAULT_GIT_TIMEOUT_SECS,
        )
        .await?;
        let view = RepoCommitView {
            repo_root: repo_root.display().to_string(),
            sha: item.sha.clone(),
            author: item.author.clone(),
            email: item.email.clone(),
            date: item.date.clone(),
            subject: item.subject.clone(),
            body: item.body.clone(),
            stats: stats.trim().to_string(),
        };
        let output = render_show_commit(&view);
        let mut metadata = base_metadata(input, repo_root);
        metadata.insert("commit".to_string(), serde_json::to_value(&view).unwrap());
        Ok(ToolResult {
            title: format!("Commit {}", short_sha(&view.sha)),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn diff_uncommitted(
        &self,
        input: &RepoHistoryInput,
        repo_root: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let path = normalize_repo_path(input.path.as_deref(), repo_root)?;
        let unstaged = git_diff_output(repo_root, path.as_deref(), false, ctx).await?;
        let staged = git_diff_output(repo_root, path.as_deref(), true, ctx).await?;
        let mut sections = Vec::new();
        if !unstaged.trim().is_empty() {
            sections.push(format!("[unstaged]\n{}", unstaged.trim()));
        }
        if !staged.trim().is_empty() {
            sections.push(format!("[staged]\n{}", staged.trim()));
        }
        let combined = if sections.is_empty() {
            "No uncommitted changes.".to_string()
        } else {
            sections.join("\n\n")
        };
        let truncated = combined.chars().count() > MAX_OUTPUT_CHARS;
        let output = if truncated {
            format!(
                "{}\n\n[output truncated at {} characters]",
                truncate_chars(&combined, MAX_OUTPUT_CHARS),
                MAX_OUTPUT_CHARS
            )
        } else {
            combined.clone()
        };
        let mut metadata = base_metadata(input, repo_root);
        metadata.insert(
            "has_unstaged".to_string(),
            serde_json::json!(!unstaged.trim().is_empty()),
        );
        metadata.insert(
            "has_staged".to_string(),
            serde_json::json!(!staged.trim().is_empty()),
        );
        if let Some(path) = path {
            metadata.insert("path".to_string(), serde_json::json!(path));
        }
        Ok(ToolResult {
            title: "Repository Diff".to_string(),
            output,
            metadata,
            truncated,
        })
    }

    async fn blame(
        &self,
        input: &RepoHistoryInput,
        repo_root: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let path = normalize_repo_path(input.path.as_deref(), repo_root)?
            .ok_or_else(|| ToolError::InvalidArguments("path is required for blame".to_string()))?;
        let (line_start, line_end) = blame_line_window(input)?;
        let raw = run_git_command(
            &[
                "blame".to_string(),
                "--line-porcelain".to_string(),
                "-L".to_string(),
                format!("{},{}", line_start, line_end),
                "--".to_string(),
                path.clone(),
            ],
            Some(repo_root),
            ctx,
            DEFAULT_GIT_TIMEOUT_SECS,
        )
        .await?;
        let lines = parse_blame_porcelain(&raw);
        let output = render_blame(repo_root, &path, line_start, line_end, &lines);
        let mut metadata = base_metadata(input, repo_root);
        metadata.insert("blame".to_string(), serde_json::to_value(&lines).unwrap());
        metadata.insert("path".to_string(), serde_json::json!(path));
        metadata.insert("count".to_string(), serde_json::json!(lines.len()));
        Ok(ToolResult {
            title: "Repository Blame".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }
}

impl Default for RepoHistoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for RepoHistoryTool {
    fn id(&self) -> &str {
        "repo_history"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "head", "log", "show_commit", "diff_uncommitted", "blame"],
                    "description": "Local repository history operation to execute"
                },
                "path": {
                    "type": "string",
                    "description": "Optional repository-relative path for status/log/diff_uncommitted, required for blame"
                },
                "commit": {
                    "type": "string",
                    "description": "Commit SHA or ref for show_commit"
                },
                "sha": {
                    "type": "string",
                    "description": "Alias for commit"
                },
                "lineStart": {
                    "type": "integer",
                    "description": "Optional starting line for blame"
                },
                "lineEnd": {
                    "type": "integer",
                    "description": "Optional ending line for blame"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "default": 20,
                    "description": "Maximum number of commits, or fallback line count for blame"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: RepoHistoryInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        validate_input(&input)?;

        let mut permission = PermissionRequest::new("repo_history")
            .with_metadata("operation", serde_json::json!(input.operation.as_str()))
            .with_metadata("limit", serde_json::json!(input.limit))
            .always_allow();
        if let Some(path) = input.path.as_ref() {
            permission = permission.with_metadata("path", serde_json::json!(path));
        }
        if let Some(commit) = input.commit.as_ref() {
            permission = permission.with_metadata("commit", serde_json::json!(commit));
        }
        if let Some(line_start) = input.line_start {
            permission = permission.with_metadata("line_start", serde_json::json!(line_start));
        }
        if let Some(line_end) = input.line_end {
            permission = permission.with_metadata("line_end", serde_json::json!(line_end));
        }
        ctx.ask_permission(permission).await?;

        self.execute_impl(&input, &ctx).await
    }
}

fn validate_input(input: &RepoHistoryInput) -> Result<(), ToolError> {
    if input.limit == 0 || input.limit > MAX_LIMIT {
        return Err(ToolError::InvalidArguments(format!(
            "limit must be between 1 and {}",
            MAX_LIMIT
        )));
    }
    if let Some(line_start) = input.line_start {
        if line_start == 0 {
            return Err(ToolError::InvalidArguments(
                "line_start must be greater than 0".to_string(),
            ));
        }
    }
    if let Some(line_end) = input.line_end {
        let line_start = input.line_start.ok_or_else(|| {
            ToolError::InvalidArguments(
                "line_start is required when line_end is provided".to_string(),
            )
        })?;
        if line_end == 0 || line_end < line_start {
            return Err(ToolError::InvalidArguments(
                "line_end must be greater than or equal to line_start".to_string(),
            ));
        }
    }
    match input.operation {
        RepoHistoryOperation::ShowCommit => {
            if input
                .commit
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return Err(ToolError::InvalidArguments(
                    "commit is required for show_commit".to_string(),
                ));
            }
        }
        RepoHistoryOperation::Blame => {
            if input.path.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(ToolError::InvalidArguments(
                    "path is required for blame".to_string(),
                ));
            }
        }
        RepoHistoryOperation::Status
        | RepoHistoryOperation::Head
        | RepoHistoryOperation::Log
        | RepoHistoryOperation::DiffUncommitted => {}
    }
    Ok(())
}

async fn resolve_repo_root(ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let cwd = Path::new(&ctx.directory);
    let root = run_git_command(
        &["rev-parse".to_string(), "--show-toplevel".to_string()],
        Some(cwd),
        ctx,
        DEFAULT_GIT_TIMEOUT_SECS,
    )
    .await?;
    Ok(PathBuf::from(root.trim()))
}

async fn load_head_view(repo_root: &Path, ctx: &ToolContext) -> Result<RepoHeadView, ToolError> {
    let head_sha = run_git_command(
        &["rev-parse".to_string(), "HEAD".to_string()],
        Some(repo_root),
        ctx,
        DEFAULT_GIT_TIMEOUT_SECS,
    )
    .await?
    .trim()
    .to_string();
    let branch_raw = run_git_command(
        &["branch".to_string(), "--show-current".to_string()],
        Some(repo_root),
        ctx,
        DEFAULT_GIT_TIMEOUT_SECS,
    )
    .await?;
    let branch = branch_raw.trim().to_string();
    Ok(RepoHeadView {
        repo_root: repo_root.display().to_string(),
        branch: if branch.is_empty() {
            None
        } else {
            Some(branch)
        },
        head_sha,
        detached: branch_raw.trim().is_empty(),
    })
}

async fn git_diff_output(
    repo_root: &Path,
    path: Option<&str>,
    staged: bool,
    ctx: &ToolContext,
) -> Result<String, ToolError> {
    let mut args = vec![
        "diff".to_string(),
        "--stat".to_string(),
        "--patch".to_string(),
    ];
    if staged {
        args.push("--cached".to_string());
    }
    if let Some(path) = path {
        args.push("--".to_string());
        args.push(path.to_string());
    }
    run_git_command(&args, Some(repo_root), ctx, DEFAULT_GIT_TIMEOUT_SECS).await
}

fn base_metadata(input: &RepoHistoryInput, repo_root: &Path) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert(
        "operation".to_string(),
        serde_json::json!(input.operation.as_str()),
    );
    metadata.insert(
        "repo_root".to_string(),
        serde_json::json!(repo_root.display().to_string()),
    );
    metadata
}

fn normalize_repo_path(value: Option<&str>, repo_root: &Path) -> Result<Option<String>, ToolError> {
    let raw = match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw,
        None => return Ok(None),
    };
    let path = Path::new(raw);
    if path.is_absolute() {
        let relative = path.strip_prefix(repo_root).map_err(|_| {
            ToolError::InvalidArguments(format!(
                "path `{}` must stay within repository root `{}`",
                raw,
                repo_root.display()
            ))
        })?;
        return Ok(Some(relative.to_string_lossy().replace('\\', "/")));
    }
    Ok(Some(raw.trim_start_matches("./").replace('\\', "/")))
}

fn blame_line_window(input: &RepoHistoryInput) -> Result<(u64, u64), ToolError> {
    let line_start = input.line_start.unwrap_or(1);
    if line_start == 0 {
        return Err(ToolError::InvalidArguments(
            "line_start must be greater than 0".to_string(),
        ));
    }
    let line_end = input.line_end.unwrap_or(
        line_start
            .saturating_add(input.limit as u64)
            .saturating_sub(1),
    );
    if line_end < line_start {
        return Err(ToolError::InvalidArguments(
            "line_end must be greater than or equal to line_start".to_string(),
        ));
    }
    Ok((line_start, line_end))
}

fn parse_status_output(repo_root: &Path, raw: &str) -> RepoStatusView {
    let mut lines = raw.lines();
    let branch_summary = lines
        .next()
        .unwrap_or("## HEAD")
        .trim_start_matches("## ")
        .trim()
        .to_string();
    let entries = lines
        .filter_map(|line| {
            if line.len() < 3 {
                return None;
            }
            let staged = line[0..1].to_string();
            let unstaged = line[1..2].to_string();
            let remainder = line[3..].trim();
            let (original_path, path) = if let Some((from, to)) = remainder.split_once(" -> ") {
                (Some(from.trim().to_string()), to.trim().to_string())
            } else {
                (None, remainder.to_string())
            };
            Some(RepoStatusEntry {
                staged,
                unstaged,
                path,
                original_path,
            })
        })
        .collect();
    RepoStatusView {
        repo_root: repo_root.display().to_string(),
        branch_summary,
        entries,
    }
}

fn parse_log_output(raw: &str) -> Vec<RepoLogItem> {
    raw.split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim();
            if record.is_empty() {
                return None;
            }
            let mut parts = record.split('\u{1f}');
            let sha = parts.next()?.trim().to_string();
            let author = parts.next().unwrap_or_default().trim().to_string();
            let email = parts.next().unwrap_or_default().trim().to_string();
            let date = parts.next().unwrap_or_default().trim().to_string();
            let subject = parts.next().unwrap_or_default().trim().to_string();
            let body = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            Some(RepoLogItem {
                sha,
                author,
                email,
                date,
                subject,
                body,
            })
        })
        .collect()
}

fn parse_blame_porcelain(raw: &str) -> Vec<RepoBlameLine> {
    let mut out = Vec::new();
    let mut current_commit = String::new();
    let mut current_line = 0_u64;
    let mut author = None;
    let mut email = None;
    let mut summary = None;

    for line in raw.lines() {
        if line.starts_with('\t') {
            out.push(RepoBlameLine {
                line_number: current_line,
                commit: current_commit.clone(),
                author: author.clone(),
                email: email.clone(),
                summary: summary.clone(),
                content: line.trim_start_matches('\t').to_string(),
            });
            author = None;
            email = None;
            summary = None;
            continue;
        }

        if let Some((commit, rest)) = line.split_once(' ') {
            if commit.len() == 40 && commit.chars().all(|ch| ch.is_ascii_hexdigit()) {
                current_commit = commit.to_string();
                let mut rest_parts = rest.split_whitespace();
                current_line = rest_parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                continue;
            }
        }

        if let Some(value) = line.strip_prefix("author ") {
            author = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("author-mail <") {
            email = Some(value.trim_end_matches('>').to_string());
        } else if let Some(value) = line.strip_prefix("summary ") {
            summary = Some(value.to_string());
        }
    }

    out
}

fn render_status(view: &RepoStatusView) -> String {
    let mut lines = vec![
        format!("repo: {}", view.repo_root),
        format!("branch: {}", view.branch_summary),
    ];
    if view.entries.is_empty() {
        lines.push("working tree clean".to_string());
    } else {
        lines.push("changes:".to_string());
        for entry in &view.entries {
            if let Some(original) = entry.original_path.as_ref() {
                lines.push(format!(
                    "- {}{} {} <- {}",
                    entry.staged, entry.unstaged, entry.path, original
                ));
            } else {
                lines.push(format!(
                    "- {}{} {}",
                    entry.staged, entry.unstaged, entry.path
                ));
            }
        }
    }
    lines.join("\n")
}

fn render_head(view: &RepoHeadView) -> String {
    let branch = view.branch.as_deref().unwrap_or("(detached HEAD)");
    format!(
        "repo: {}\nbranch: {}\nhead: {}\ndetached: {}",
        view.repo_root, branch, view.head_sha, view.detached
    )
}

fn render_log(repo_root: &Path, items: &[RepoLogItem]) -> String {
    let mut lines = vec![format!("repo: {}", repo_root.display())];
    if items.is_empty() {
        lines.push("no commits found".to_string());
        return lines.join("\n");
    }
    for item in items {
        lines.push(format!(
            "- {} {} ({}, {})",
            short_sha(&item.sha),
            item.subject,
            item.author,
            item.date
        ));
        if let Some(body) = item.body.as_ref() {
            lines.push(format!("  {}", body.replace('\n', " ")));
        }
    }
    lines.join("\n")
}

fn render_show_commit(view: &RepoCommitView) -> String {
    let mut lines = vec![
        format!("repo: {}", view.repo_root),
        format!("commit: {}", view.sha),
        format!("author: {} <{}>", view.author, view.email),
        format!("date: {}", view.date),
        format!("subject: {}", view.subject),
    ];
    if let Some(body) = view.body.as_ref() {
        lines.push(String::new());
        lines.push(body.clone());
    }
    if !view.stats.is_empty() {
        lines.push(String::new());
        lines.push(view.stats.clone());
    }
    lines.join("\n")
}

fn render_blame(
    repo_root: &Path,
    path: &str,
    line_start: u64,
    line_end: u64,
    lines: &[RepoBlameLine],
) -> String {
    let mut out = vec![format!(
        "repo: {}\npath: {}\nlines: {}-{}",
        repo_root.display(),
        path,
        line_start,
        line_end
    )];
    for line in lines {
        out.push(format!(
            "- {:>4} {} {} | {}",
            line.line_number,
            short_sha(&line.commit),
            line.author.as_deref().unwrap_or("unknown"),
            line.content
        ));
    }
    out.join("\n")
}

fn short_sha(sha: &str) -> &str {
    if sha.len() > 12 {
        &sha[..12]
    } else {
        sha
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn test_tool_context(directory: &Path) -> ToolContext {
        ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            directory.to_string_lossy().to_string(),
        )
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should run");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_git_fixture() -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir should create");
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "Fixture User"]);
        git(dir.path(), &["config", "user.email", "fixture@example.com"]);
        fs::create_dir_all(dir.path().join("src")).expect("src should create");
        fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").expect("file should write");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "Initial commit"]);
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() {}\n",
        )
        .expect("file should update");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "Second commit"]);
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta_changed() {}\n",
        )
        .expect("worktree change should write");
        dir
    }

    #[test]
    fn schema_exposes_expected_operations() {
        let tool = RepoHistoryTool::new();
        let schema = tool.parameters();
        let ops = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("enum should exist");
        assert!(ops.iter().any(|v| v == "status"));
        assert!(ops.iter().any(|v| v == "head"));
        assert!(ops.iter().any(|v| v == "log"));
        assert!(ops.iter().any(|v| v == "show_commit"));
        assert!(ops.iter().any(|v| v == "diff_uncommitted"));
        assert!(ops.iter().any(|v| v == "blame"));
    }

    #[tokio::test]
    async fn head_reads_fixture_repo() {
        let dir = create_git_fixture();
        let tool = RepoHistoryTool::new();
        let result = tool
            .execute(
                serde_json::json!({"operation": "head"}),
                test_tool_context(dir.path()),
            )
            .await
            .expect("head should succeed");
        assert!(result.output.contains("branch: main"));
        assert_eq!(result.metadata["head"]["branch"], serde_json::json!("main"));
    }

    #[tokio::test]
    async fn log_reads_fixture_repo() {
        let dir = create_git_fixture();
        let tool = RepoHistoryTool::new();
        let result = tool
            .execute(
                serde_json::json!({"operation": "log", "path": "src/lib.rs", "limit": 10}),
                test_tool_context(dir.path()),
            )
            .await
            .expect("log should succeed");
        assert!(result.output.contains("Second commit"));
        assert_eq!(result.metadata["count"], serde_json::json!(2));
    }

    #[tokio::test]
    async fn blame_reads_fixture_repo() {
        let dir = create_git_fixture();
        let tool = RepoHistoryTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "operation": "blame",
                    "path": "src/lib.rs",
                    "lineStart": 1,
                    "lineEnd": 2
                }),
                test_tool_context(dir.path()),
            )
            .await
            .expect("blame should succeed");
        assert!(result.output.contains("Fixture User"));
        assert_eq!(result.metadata["count"], serde_json::json!(2));
    }

    #[tokio::test]
    async fn diff_uncommitted_reads_fixture_repo() {
        let dir = create_git_fixture();
        let tool = RepoHistoryTool::new();
        let result = tool
            .execute(
                serde_json::json!({"operation": "diff_uncommitted", "path": "src/lib.rs"}),
                test_tool_context(dir.path()),
            )
            .await
            .expect("diff should succeed");
        assert!(result.output.contains("beta_changed"));
        assert_eq!(result.metadata["has_unstaged"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn show_commit_reads_fixture_repo() {
        let dir = create_git_fixture();
        let sha = git_output(dir.path(), &["rev-parse", "HEAD~1"]);
        let tool = RepoHistoryTool::new();
        let result = tool
            .execute(
                serde_json::json!({"operation": "show_commit", "commit": sha}),
                test_tool_context(dir.path()),
            )
            .await
            .expect("show_commit should succeed");
        assert!(result.output.contains("Initial commit"));
    }
}
