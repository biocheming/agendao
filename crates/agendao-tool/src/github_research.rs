use async_trait::async_trait;
use base64::Engine;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Metadata, PermissionRequest, Tool, ToolContext, ToolError, ToolResult};

const API_BASE_URL: &str = "https://api.github.com";
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_GIT_TIMEOUT_SECS: u64 = 120;
const DEFAULT_COMMENTS_LIMIT: usize = 20;
const DEFAULT_CLONE_DEPTH: u64 = 1;
const MAX_FILE_OUTPUT_CHARS: usize = 20_000;

const DESCRIPTION: &str = r#"Structured GitHub research for remote repository evidence gathering.

Implemented operations:
- Phase 1: search_code, search_issues, search_prs, view_issue, view_pr, view_pr_files
- Phase 2: get_head_sha, build_permalink, read_file, clone_repo, list_releases, list_tags, git_log, git_blame

Git-native operations use local git when possible:
- clone_repo
- list_tags
- git_log
- git_blame
- read_file prefers an existing local clone, then falls back to the GitHub contents API

GitHub-native platform operations use the GitHub API:
- search_code
- search_issues
- search_prs
- view_issue
- view_pr
- view_pr_files
- list_releases"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GitHubResearchOperation {
    SearchCode,
    SearchIssues,
    SearchPrs,
    ViewIssue,
    ViewPr,
    ViewPrFiles,
    GetHeadSha,
    BuildPermalink,
    ReadFile,
    CloneRepo,
    ListReleases,
    ListTags,
    GitLog,
    GitBlame,
}

impl GitHubResearchOperation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SearchCode => "search_code",
            Self::SearchIssues => "search_issues",
            Self::SearchPrs => "search_prs",
            Self::ViewIssue => "view_issue",
            Self::ViewPr => "view_pr",
            Self::ViewPrFiles => "view_pr_files",
            Self::GetHeadSha => "get_head_sha",
            Self::BuildPermalink => "build_permalink",
            Self::ReadFile => "read_file",
            Self::CloneRepo => "clone_repo",
            Self::ListReleases => "list_releases",
            Self::ListTags => "list_tags",
            Self::GitLog => "git_log",
            Self::GitBlame => "git_blame",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubResearchInput {
    operation: GitHubResearchOperation,
    repo: String,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    local_alias: Option<String>,
    #[serde(default)]
    line_start: Option<u64>,
    #[serde(default)]
    line_end: Option<u64>,
    #[serde(default)]
    depth: Option<u64>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_include_comments", alias = "include_comments")]
    include_comments: bool,
}

fn default_limit() -> usize {
    DEFAULT_LIMIT
}

fn default_include_comments() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
struct SearchCodeItem {
    repo: String,
    path: String,
    url: String,
    sha: Option<String>,
    snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SearchIssueItem {
    repo: String,
    number: u64,
    title: String,
    state: String,
    url: String,
    kind: &'static str,
    snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SimpleComment {
    author: Option<String>,
    body: String,
    url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct IssueView {
    repo: String,
    number: u64,
    title: String,
    state: String,
    author: Option<String>,
    url: String,
    labels: Vec<String>,
    body: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    comments: Vec<SimpleComment>,
}

#[derive(Debug, Clone, Serialize)]
struct PullRequestView {
    repo: String,
    number: u64,
    title: String,
    state: String,
    author: Option<String>,
    url: String,
    labels: Vec<String>,
    body: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    merged: Option<bool>,
    draft: Option<bool>,
    additions: Option<u64>,
    deletions: Option<u64>,
    changed_files: Option<u64>,
    commits: Option<u64>,
    base_ref: Option<String>,
    head_ref: Option<String>,
    head_sha: Option<String>,
    comments: Vec<SimpleComment>,
}

#[derive(Debug, Clone, Serialize)]
struct PullRequestFileItem {
    filename: String,
    status: String,
    additions: u64,
    deletions: u64,
    changes: u64,
    blob_url: Option<String>,
    raw_url: Option<String>,
    patch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HeadShaView {
    repo: String,
    resolved_ref: String,
    sha: String,
    default_branch: String,
}

#[derive(Debug, Clone, Serialize)]
struct PermalinkView {
    repo: String,
    path: String,
    sha: String,
    resolved_ref: String,
    url: String,
    line_start: Option<u64>,
    line_end: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct ReadFileView {
    repo: String,
    path: String,
    resolved_ref: String,
    commit_sha: String,
    file_sha: Option<String>,
    size: u64,
    url: Option<String>,
    download_url: Option<String>,
    source: &'static str,
    content: String,
    line_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct CloneRepoView {
    repo: String,
    remote_url: String,
    local_path: String,
    default_branch: String,
    resolved_ref: String,
    head_sha: String,
    depth: u64,
    reused_cache: bool,
    local_alias: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ReleaseItem {
    tag_name: String,
    name: Option<String>,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    target_commitish: Option<String>,
    url: String,
    body_snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TagItem {
    name: String,
    sha: String,
    url: String,
    zipball_url: String,
    tarball_url: String,
}

#[derive(Debug, Clone, Serialize)]
struct GitLogItem {
    sha: String,
    author: String,
    author_email: String,
    date: String,
    subject: String,
    body: Option<String>,
    url: String,
}

#[derive(Debug, Clone, Serialize)]
struct GitBlameLine {
    line_number: u64,
    commit_sha: String,
    author: Option<String>,
    author_mail: Option<String>,
    author_time: Option<String>,
    summary: Option<String>,
    previous: Option<String>,
    content: String,
    commit_url: String,
}

#[derive(Debug, Clone, Default)]
struct BlameMeta {
    author: Option<String>,
    author_mail: Option<String>,
    author_time: Option<String>,
    summary: Option<String>,
    previous: Option<String>,
}

#[derive(Debug, Clone)]
struct LocalRepoState {
    local_path: PathBuf,
    remote_url: String,
    default_branch: String,
    resolved_ref: String,
    head_sha: String,
    depth: u64,
    reused_cache: bool,
    local_alias: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedGitRef {
    resolved_ref: String,
    commit_sha: String,
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct GitHubSearchCodeResponse {
    total_count: u64,
    items: Vec<GitHubCodeItem>,
}

#[derive(Debug, Deserialize)]
struct GitHubCodeItem {
    path: String,
    html_url: String,
    sha: Option<String>,
    repository: GitHubRepositoryRef,
    #[serde(default)]
    text_matches: Vec<GitHubTextMatch>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepositoryRef {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubTextMatch {
    fragment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubSearchIssuesResponse {
    total_count: u64,
    items: Vec<GitHubIssueSearchItem>,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueSearchItem {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    body: Option<String>,
    repository_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueResponse {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    body: Option<String>,
    user: Option<GitHubUser>,
    #[serde(default)]
    labels: Vec<GitHubLabel>,
    comments_url: String,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestResponse {
    number: u64,
    title: String,
    state: String,
    html_url: String,
    body: Option<String>,
    user: Option<GitHubUser>,
    #[serde(default)]
    labels: Vec<GitHubLabel>,
    created_at: Option<String>,
    updated_at: Option<String>,
    merged: Option<bool>,
    draft: Option<bool>,
    additions: Option<u64>,
    deletions: Option<u64>,
    changed_files: Option<u64>,
    commits: Option<u64>,
    issue_url: String,
    base: GitHubPullRequestBranch,
    head: GitHubPullRequestBranch,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestBranch {
    #[serde(rename = "ref")]
    branch_ref: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubComment {
    body: Option<String>,
    html_url: Option<String>,
    user: Option<GitHubUser>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestFile {
    filename: String,
    status: String,
    additions: u64,
    deletions: u64,
    changes: u64,
    blob_url: Option<String>,
    raw_url: Option<String>,
    patch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoResponse {
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct GitHubCommitResponse {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContentResponse {
    path: String,
    sha: String,
    size: u64,
    html_url: Option<String>,
    download_url: Option<String>,
    encoding: Option<String>,
    content: Option<String>,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseResponse {
    tag_name: String,
    name: Option<String>,
    html_url: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    target_commitish: Option<String>,
    body: Option<String>,
}

pub struct GitHubResearchTool {
    client: Client,
}

impl GitHubResearchTool {
    pub fn new() -> Self {
        Self {
            client: build_github_client(),
        }
    }

    async fn execute_impl(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        match input.operation {
            GitHubResearchOperation::SearchCode => self.search_code(input, ctx).await,
            GitHubResearchOperation::SearchIssues => self.search_issues(input, ctx).await,
            GitHubResearchOperation::SearchPrs => self.search_prs(input, ctx).await,
            GitHubResearchOperation::ViewIssue => self.view_issue(input, ctx).await,
            GitHubResearchOperation::ViewPr => self.view_pr(input, ctx).await,
            GitHubResearchOperation::ViewPrFiles => self.view_pr_files(input, ctx).await,
            GitHubResearchOperation::GetHeadSha => self.get_head_sha(input, ctx).await,
            GitHubResearchOperation::BuildPermalink => self.build_permalink(input, ctx).await,
            GitHubResearchOperation::ReadFile => self.read_file(input, ctx).await,
            GitHubResearchOperation::CloneRepo => self.clone_repo(input, ctx).await,
            GitHubResearchOperation::ListReleases => self.list_releases(input, ctx).await,
            GitHubResearchOperation::ListTags => self.list_tags(input, ctx).await,
            GitHubResearchOperation::GitLog => self.git_log(input, ctx).await,
            GitHubResearchOperation::GitBlame => self.git_blame(input, ctx).await,
        }
    }

    async fn search_code(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let query = required_query(input)?;
        let q = build_code_search_query(query, &input.repo, input.language.as_deref());
        let url = format!(
            "{}/search/code?per_page={}&q={}",
            API_BASE_URL,
            input.limit,
            urlencoding::encode(&q)
        );
        let response: GitHubSearchCodeResponse = self.fetch_json(&url, ctx, true).await?;
        let items: Vec<SearchCodeItem> = response
            .items
            .into_iter()
            .map(|item| SearchCodeItem {
                repo: item.repository.full_name,
                path: item.path,
                url: item.html_url,
                sha: item.sha,
                snippet: item
                    .text_matches
                    .into_iter()
                    .find_map(|m| m.fragment)
                    .map(trim_snippet),
            })
            .collect();

        let output = render_search_code(&input.repo, query, &items, response.total_count);
        let mut metadata = base_metadata(input);
        metadata.insert("count".to_string(), serde_json::json!(items.len()));
        metadata.insert(
            "total_count".to_string(),
            serde_json::json!(response.total_count),
        );
        insert_metadata_serialized(&mut metadata, "items", &items);

        Ok(ToolResult {
            title: format!("GitHub code search: {}", input.repo),
            output,
            metadata,
            truncated: response.total_count as usize > items.len(),
        })
    }

    async fn search_issues(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.search_issue_like(input, ctx, "issue").await
    }

    async fn search_prs(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        self.search_issue_like(input, ctx, "pr").await
    }

    async fn search_issue_like(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
        kind: &'static str,
    ) -> Result<ToolResult, ToolError> {
        let query = required_query(input)?;
        let q = build_issue_search_query(query, &input.repo, kind, input.state.as_deref());
        let url = format!(
            "{}/search/issues?per_page={}&q={}",
            API_BASE_URL,
            input.limit,
            urlencoding::encode(&q)
        );
        let response: GitHubSearchIssuesResponse = self.fetch_json(&url, ctx, false).await?;
        let items: Vec<SearchIssueItem> = response
            .items
            .into_iter()
            .map(|item| SearchIssueItem {
                repo: repo_from_repository_url(&item.repository_url),
                number: item.number,
                title: item.title,
                state: item.state,
                url: item.html_url,
                kind,
                snippet: item.body.as_deref().map(trim_body_snippet),
            })
            .collect();

        let output = render_issue_search(kind, &input.repo, query, &items, response.total_count);
        let mut metadata = base_metadata(input);
        metadata.insert("count".to_string(), serde_json::json!(items.len()));
        metadata.insert(
            "total_count".to_string(),
            serde_json::json!(response.total_count),
        );
        insert_metadata_serialized(&mut metadata, "items", &items);

        Ok(ToolResult {
            title: format!("GitHub {} search: {}", kind, input.repo),
            output,
            metadata,
            truncated: response.total_count as usize > items.len(),
        })
    }

    async fn view_issue(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let number = required_number(input)?;
        let url = format!("{}/repos/{}/issues/{}", API_BASE_URL, input.repo, number);
        let response: GitHubIssueResponse = self.fetch_json(&url, ctx, false).await?;
        let comments = if input.include_comments {
            self.fetch_comments(&response.comments_url, ctx).await?
        } else {
            Vec::new()
        };
        let view = IssueView {
            repo: input.repo.clone(),
            number: response.number,
            title: response.title,
            state: response.state,
            author: response.user.map(|u| u.login),
            url: response.html_url,
            labels: response.labels.into_iter().map(|l| l.name).collect(),
            body: response.body,
            created_at: response.created_at,
            updated_at: response.updated_at,
            comments,
        };

        let output = render_issue_view(&view);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "issue", &view);
        metadata.insert("count".to_string(), serde_json::json!(view.comments.len()));

        Ok(ToolResult {
            title: format!("GitHub issue #{}: {}", view.number, input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn view_pr(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let number = required_number(input)?;
        let url = format!("{}/repos/{}/pulls/{}", API_BASE_URL, input.repo, number);
        let response: GitHubPullRequestResponse = self.fetch_json(&url, ctx, false).await?;
        let issue_comments_url = response.issue_url.replace("/pulls/", "/issues/") + "/comments";
        let comments = if input.include_comments {
            self.fetch_comments(&issue_comments_url, ctx).await?
        } else {
            Vec::new()
        };
        let view = PullRequestView {
            repo: input.repo.clone(),
            number: response.number,
            title: response.title,
            state: response.state,
            author: response.user.map(|u| u.login),
            url: response.html_url,
            labels: response.labels.into_iter().map(|l| l.name).collect(),
            body: response.body,
            created_at: response.created_at,
            updated_at: response.updated_at,
            merged: response.merged,
            draft: response.draft,
            additions: response.additions,
            deletions: response.deletions,
            changed_files: response.changed_files,
            commits: response.commits,
            base_ref: Some(response.base.branch_ref),
            head_ref: Some(response.head.branch_ref),
            head_sha: Some(response.head.sha),
            comments,
        };

        let output = render_pr_view(&view);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "pull_request", &view);
        metadata.insert("count".to_string(), serde_json::json!(view.comments.len()));

        Ok(ToolResult {
            title: format!("GitHub PR #{}: {}", view.number, input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn view_pr_files(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let number = required_number(input)?;
        let url = format!(
            "{}/repos/{}/pulls/{}/files?per_page={}",
            API_BASE_URL, input.repo, number, input.limit
        );
        let response: Vec<GitHubPullRequestFile> = self.fetch_json(&url, ctx, false).await?;
        let files: Vec<PullRequestFileItem> = response
            .into_iter()
            .map(|file| PullRequestFileItem {
                filename: file.filename,
                status: file.status,
                additions: file.additions,
                deletions: file.deletions,
                changes: file.changes,
                blob_url: file.blob_url,
                raw_url: file.raw_url,
                patch: file.patch.map(limit_patch),
            })
            .collect();

        let output = render_pr_files(&input.repo, number, &files);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "files", &files);
        metadata.insert("count".to_string(), serde_json::json!(files.len()));

        Ok(ToolResult {
            title: format!("GitHub PR files #{}: {}", number, input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn get_head_sha(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let resolved = self.resolve_git_ref(input, ctx).await?;
        let view = HeadShaView {
            repo: input.repo.clone(),
            resolved_ref: resolved.resolved_ref,
            sha: resolved.commit_sha,
            default_branch: resolved.default_branch,
        };

        let output = render_head_sha(&view);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "head", &view);
        metadata.insert("count".to_string(), serde_json::json!(1));

        Ok(ToolResult {
            title: format!("GitHub HEAD SHA: {}", input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn build_permalink(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let path = required_path(input)?;
        let resolved = self.resolve_git_ref(input, ctx).await?;
        let view = PermalinkView {
            repo: input.repo.clone(),
            path: path.clone(),
            sha: resolved.commit_sha.clone(),
            resolved_ref: resolved.resolved_ref,
            url: format_blob_permalink(
                &input.repo,
                &path,
                &resolved.commit_sha,
                input.line_start,
                input.line_end,
            ),
            line_start: input.line_start,
            line_end: input.line_end,
        };

        let output = render_permalink(&view);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "permalink", &view);
        metadata.insert("count".to_string(), serde_json::json!(1));

        Ok(ToolResult {
            title: format!("GitHub permalink: {}:{}", input.repo, path),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn read_file(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let path = required_path(input)?;

        if let Some(local) = self.open_existing_local_repo(input, ctx).await? {
            return self.read_file_from_local(input, &path, local, ctx).await;
        }

        self.read_file_remote(input, &path, ctx).await
    }

    async fn clone_repo(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let local = self.ensure_local_repo(input, ctx).await?;
        let view = CloneRepoView {
            repo: input.repo.clone(),
            remote_url: local.remote_url,
            local_path: local.local_path.to_string_lossy().to_string(),
            default_branch: local.default_branch,
            resolved_ref: local.resolved_ref,
            head_sha: local.head_sha,
            depth: local.depth,
            reused_cache: local.reused_cache,
            local_alias: local.local_alias,
        };

        let output = render_clone_repo(&view);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "clone", &view);
        metadata.insert("count".to_string(), serde_json::json!(1));

        Ok(ToolResult {
            title: format!("GitHub clone: {}", input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn list_releases(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let url = format!(
            "{}/repos/{}/releases?per_page={}",
            API_BASE_URL, input.repo, input.limit
        );
        let response: Vec<GitHubReleaseResponse> = self.fetch_json(&url, ctx, false).await?;
        let items: Vec<ReleaseItem> = response
            .into_iter()
            .map(|release| ReleaseItem {
                tag_name: release.tag_name,
                name: release.name,
                draft: release.draft,
                prerelease: release.prerelease,
                published_at: release.published_at,
                target_commitish: release.target_commitish,
                url: release.html_url,
                body_snippet: release.body.as_deref().map(trim_body_snippet),
            })
            .collect();

        let output = render_releases(&input.repo, &items);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "releases", &items);
        metadata.insert("count".to_string(), serde_json::json!(items.len()));

        Ok(ToolResult {
            title: format!("GitHub releases: {}", input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn list_tags(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let local = self.ensure_local_repo(input, ctx).await?;
        self.run_git(
            vec![
                "fetch".to_string(),
                "--tags".to_string(),
                "--force".to_string(),
                "origin".to_string(),
            ],
            Some(&local.local_path),
            ctx,
        )
        .await?;
        let items = self
            .list_tags_in_repo(&input.repo, &local.local_path, input.limit, ctx)
            .await?;

        let output = render_tags(&input.repo, &items);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "tags", &items);
        metadata.insert(
            "local_path".to_string(),
            serde_json::json!(local.local_path.to_string_lossy().to_string()),
        );
        metadata.insert("count".to_string(), serde_json::json!(items.len()));

        Ok(ToolResult {
            title: format!("GitHub tags: {}", input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn git_log(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let local = self.ensure_local_repo(input, ctx).await?;
        let items = self
            .git_log_in_repo(
                &input.repo,
                &local.local_path,
                input.path.as_deref(),
                input.limit,
                ctx,
            )
            .await?;
        let output = render_git_log(&input.repo, &local.resolved_ref, &items);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "log", &items);
        metadata.insert(
            "local_path".to_string(),
            serde_json::json!(local.local_path.to_string_lossy().to_string()),
        );
        metadata.insert(
            "resolved_ref".to_string(),
            serde_json::json!(local.resolved_ref),
        );
        metadata.insert("head_sha".to_string(), serde_json::json!(local.head_sha));
        metadata.insert("count".to_string(), serde_json::json!(items.len()));

        Ok(ToolResult {
            title: format!("GitHub git log: {}", input.repo),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn git_blame(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let local = self.ensure_local_repo(input, ctx).await?;
        let path = required_path(input)?;
        let (line_start, line_end) = blame_line_window(input);
        let lines = self
            .git_blame_in_repo(
                &input.repo,
                &local.local_path,
                &path,
                line_start,
                line_end,
                ctx,
            )
            .await?;
        let output = render_git_blame(&input.repo, &path, line_start, line_end, &lines);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "blame", &lines);
        metadata.insert(
            "local_path".to_string(),
            serde_json::json!(local.local_path.to_string_lossy().to_string()),
        );
        metadata.insert(
            "resolved_ref".to_string(),
            serde_json::json!(local.resolved_ref),
        );
        metadata.insert("head_sha".to_string(), serde_json::json!(local.head_sha));
        metadata.insert("count".to_string(), serde_json::json!(lines.len()));
        metadata.insert("line_start".to_string(), serde_json::json!(line_start));
        metadata.insert("line_end".to_string(), serde_json::json!(line_end));

        Ok(ToolResult {
            title: format!("GitHub git blame: {}:{}", input.repo, path),
            output,
            metadata,
            truncated: false,
        })
    }

    async fn list_tags_in_repo(
        &self,
        repo: &str,
        repo_path: &Path,
        limit: usize,
        ctx: &ToolContext,
    ) -> Result<Vec<TagItem>, ToolError> {
        let names_raw = self
            .run_git(
                vec!["tag".to_string(), "--sort=-version:refname".to_string()],
                Some(repo_path),
                ctx,
            )
            .await?;
        let mut items = Vec::new();
        for name in names_raw
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(limit)
        {
            let sha = self
                .run_git(
                    vec![
                        "rev-list".to_string(),
                        "-n".to_string(),
                        "1".to_string(),
                        name.to_string(),
                    ],
                    Some(repo_path),
                    ctx,
                )
                .await?;
            items.push(TagItem {
                name: name.to_string(),
                sha: sha.trim().to_string(),
                url: format!("https://github.com/{}/tree/{}", repo, name),
                zipball_url: format!("https://github.com/{}/archive/refs/tags/{}.zip", repo, name),
                tarball_url: format!(
                    "https://github.com/{}/archive/refs/tags/{}.tar.gz",
                    repo, name
                ),
            });
        }
        Ok(items)
    }

    async fn git_log_in_repo(
        &self,
        repo: &str,
        repo_path: &Path,
        path: Option<&str>,
        limit: usize,
        ctx: &ToolContext,
    ) -> Result<Vec<GitLogItem>, ToolError> {
        let mut args = vec![
            "log".to_string(),
            format!("-n{}", limit),
            "--date=iso-strict".to_string(),
            "--format=%H%x1f%an%x1f%ae%x1f%ad%x1f%s%x1f%b%x1e".to_string(),
        ];
        if let Some(path) = normalized_optional_path(path) {
            args.push("--".to_string());
            args.push(path);
        }
        let raw = self.run_git(args, Some(repo_path), ctx).await?;
        Ok(parse_git_log_output(repo, &raw))
    }

    async fn git_blame_in_repo(
        &self,
        repo: &str,
        repo_path: &Path,
        path: &str,
        line_start: u64,
        line_end: u64,
        ctx: &ToolContext,
    ) -> Result<Vec<GitBlameLine>, ToolError> {
        let raw = self
            .run_git(
                vec![
                    "blame".to_string(),
                    "--line-porcelain".to_string(),
                    "-L".to_string(),
                    format!("{},{}", line_start, line_end),
                    "--".to_string(),
                    path.to_string(),
                ],
                Some(repo_path),
                ctx,
            )
            .await?;
        Ok(parse_git_blame_porcelain(repo, &raw))
    }

    async fn read_file_from_local(
        &self,
        input: &GitHubResearchInput,
        path: &str,
        local: LocalRepoState,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let full_path = local.local_path.join(path);
        let content = fs::read_to_string(&full_path).map_err(|e| {
            ToolError::ExecutionError(format!(
                "Failed to read local file {} from {}: {}",
                path,
                local.local_path.display(),
                e
            ))
        })?;
        let size = fs::metadata(&full_path)
            .map(|m| m.len())
            .unwrap_or(content.len() as u64);
        let file_sha = self
            .run_git(
                vec!["rev-parse".to_string(), format!("HEAD:{}", path)],
                Some(&local.local_path),
                ctx,
            )
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let truncated = content.chars().count() > MAX_FILE_OUTPUT_CHARS;
        let rendered_content = if truncated {
            truncate_chars(&content, MAX_FILE_OUTPUT_CHARS)
        } else {
            content.clone()
        };
        let view = ReadFileView {
            repo: input.repo.clone(),
            path: path.to_string(),
            resolved_ref: local.resolved_ref.clone(),
            commit_sha: local.head_sha.clone(),
            file_sha,
            size,
            url: Some(format_blob_permalink(
                &input.repo,
                path,
                &local.head_sha,
                input.line_start,
                input.line_end,
            )),
            download_url: None,
            source: "local_git",
            content: rendered_content,
            line_count: content.lines().count(),
        };

        let output = render_read_file(&view, truncated);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "file", &view);
        metadata.insert(
            "local_path".to_string(),
            serde_json::json!(local.local_path.to_string_lossy().to_string()),
        );
        metadata.insert("count".to_string(), serde_json::json!(view.line_count));

        Ok(ToolResult {
            title: format!("GitHub file: {}:{}", input.repo, path),
            output,
            metadata,
            truncated,
        })
    }

    async fn read_file_remote(
        &self,
        input: &GitHubResearchInput,
        path: &str,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let resolved = self.resolve_git_ref(input, ctx).await?;
        let url = format!(
            "{}/repos/{}/contents/{}?ref={}",
            API_BASE_URL,
            input.repo,
            encode_repo_path(path),
            urlencoding::encode(&resolved.commit_sha)
        );
        let response: GitHubContentResponse = self.fetch_json(&url, ctx, false).await?;
        if response.entry_type != "file" {
            return Err(ToolError::ExecutionError(format!(
                "GitHub contents API returned {} for {}, expected file",
                response.entry_type, path
            )));
        }

        let content = decode_github_content(&response)?;
        let truncated = content.chars().count() > MAX_FILE_OUTPUT_CHARS;
        let rendered_content = if truncated {
            truncate_chars(&content, MAX_FILE_OUTPUT_CHARS)
        } else {
            content.clone()
        };
        let view = ReadFileView {
            repo: input.repo.clone(),
            path: response.path,
            resolved_ref: resolved.resolved_ref,
            commit_sha: resolved.commit_sha.clone(),
            file_sha: Some(response.sha),
            size: response.size,
            url: response.html_url,
            download_url: response.download_url,
            source: "github_api",
            content: rendered_content,
            line_count: content.lines().count(),
        };

        let output = render_read_file(&view, truncated);
        let mut metadata = base_metadata(input);
        insert_metadata_serialized(&mut metadata, "file", &view);
        metadata.insert("count".to_string(), serde_json::json!(view.line_count));

        Ok(ToolResult {
            title: format!("GitHub file: {}:{}", input.repo, path),
            output,
            metadata,
            truncated,
        })
    }

    async fn fetch_comments(
        &self,
        url: &str,
        ctx: &ToolContext,
    ) -> Result<Vec<SimpleComment>, ToolError> {
        let url = if url.contains('?') {
            format!("{}&per_page={}", url, DEFAULT_COMMENTS_LIMIT)
        } else {
            format!("{}?per_page={}", url, DEFAULT_COMMENTS_LIMIT)
        };
        let response: Vec<GitHubComment> = self.fetch_json(&url, ctx, false).await?;
        Ok(response
            .into_iter()
            .map(|comment| SimpleComment {
                author: comment.user.map(|u| u.login),
                body: comment.body.unwrap_or_default(),
                url: comment.html_url,
                created_at: comment.created_at,
                updated_at: comment.updated_at,
            })
            .collect())
    }

    async fn fetch_repo_metadata(
        &self,
        repo: &str,
        ctx: &ToolContext,
    ) -> Result<GitHubRepoResponse, ToolError> {
        if let Some(default_branch) =
            configured_repo_override(ctx, "github_research_default_branch_overrides", repo)
        {
            return Ok(GitHubRepoResponse { default_branch });
        }
        let url = format!("{}/repos/{}", API_BASE_URL, repo);
        self.fetch_json(&url, ctx, false).await
    }

    async fn fetch_commit(
        &self,
        repo: &str,
        reference: &str,
        ctx: &ToolContext,
    ) -> Result<GitHubCommitResponse, ToolError> {
        if let Some(sha) =
            configured_repo_ref_override(ctx, "github_research_commit_overrides", repo, reference)
        {
            return Ok(GitHubCommitResponse { sha });
        }
        let url = format!(
            "{}/repos/{}/commits/{}",
            API_BASE_URL,
            repo,
            urlencoding::encode(reference)
        );
        self.fetch_json(&url, ctx, false).await
    }

    async fn resolve_git_ref(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<ResolvedGitRef, ToolError> {
        let repo = self.fetch_repo_metadata(&input.repo, ctx).await?;
        if let Some(sha) = normalized_optional_sha(input.sha.as_deref()) {
            return Ok(ResolvedGitRef {
                resolved_ref: sha.clone(),
                commit_sha: sha,
                default_branch: repo.default_branch,
            });
        }

        let reference = input
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&repo.default_branch)
            .to_string();
        let commit = self.fetch_commit(&input.repo, &reference, ctx).await?;
        Ok(ResolvedGitRef {
            resolved_ref: reference,
            commit_sha: commit.sha,
            default_branch: repo.default_branch,
        })
    }

    async fn open_existing_local_repo(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<Option<LocalRepoState>, ToolError> {
        ensure_git_available()?;
        let repo_root = local_repo_root(ctx);
        let repo_dir = repo_root.join(repo_cache_key(&input.repo, input.local_alias.as_deref()));
        if !is_git_repo_dir(&repo_dir) {
            return Ok(None);
        }

        let default_branch = self
            .fetch_repo_metadata(&input.repo, ctx)
            .await?
            .default_branch;
        let remote_url = remote_url_for_repo(&input.repo, ctx);
        self.sync_local_repo(&repo_dir, &remote_url, input, &default_branch, true, ctx)
            .await
            .map(Some)
    }

    async fn ensure_local_repo(
        &self,
        input: &GitHubResearchInput,
        ctx: &ToolContext,
    ) -> Result<LocalRepoState, ToolError> {
        ensure_git_available()?;
        let repo_root = local_repo_root(ctx);
        fs::create_dir_all(&repo_root).map_err(|e| {
            ToolError::ExecutionError(format!(
                "Failed to create github_research cache root {}: {}",
                repo_root.display(),
                e
            ))
        })?;

        let repo_dir = repo_root.join(repo_cache_key(&input.repo, input.local_alias.as_deref()));
        let remote_url = remote_url_for_repo(&input.repo, ctx);
        let default_branch = self
            .fetch_repo_metadata(&input.repo, ctx)
            .await?
            .default_branch;
        let existed = is_git_repo_dir(&repo_dir);

        if !existed {
            if repo_dir.exists() {
                fs::remove_dir_all(&repo_dir).map_err(|e| {
                    ToolError::ExecutionError(format!(
                        "Failed to reset non-git cache directory {}: {}",
                        repo_dir.display(),
                        e
                    ))
                })?;
            }
            self.clone_repo_into(&repo_dir, &remote_url, input, &default_branch, ctx)
                .await?;
        }

        self.sync_local_repo(&repo_dir, &remote_url, input, &default_branch, existed, ctx)
            .await
    }

    async fn clone_repo_into(
        &self,
        repo_dir: &Path,
        remote_url: &str,
        input: &GitHubResearchInput,
        default_branch: &str,
        ctx: &ToolContext,
    ) -> Result<(), ToolError> {
        if let Some(parent) = repo_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                ToolError::ExecutionError(format!(
                    "Failed to create cache parent {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        let reference = input
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default_branch)
            .to_string();
        let depth = clone_depth(input);

        let mut args = vec![
            "clone".to_string(),
            "--depth".to_string(),
            depth.to_string(),
        ];
        if normalized_optional_sha(input.sha.as_deref()).is_none() {
            args.push("--branch".to_string());
            args.push(reference);
        }
        args.push(remote_url.to_string());
        args.push(repo_dir.to_string_lossy().to_string());
        self.run_git(args, None, ctx).await?;
        Ok(())
    }

    async fn sync_local_repo(
        &self,
        repo_dir: &Path,
        remote_url: &str,
        input: &GitHubResearchInput,
        default_branch: &str,
        reused_cache: bool,
        ctx: &ToolContext,
    ) -> Result<LocalRepoState, ToolError> {
        self.run_git(
            vec![
                "remote".to_string(),
                "set-url".to_string(),
                "origin".to_string(),
                remote_url.to_string(),
            ],
            Some(repo_dir),
            ctx,
        )
        .await?;

        let depth = clone_depth(input);
        let resolved_ref = input
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default_branch)
            .to_string();

        if let Some(sha) = normalized_optional_sha(input.sha.as_deref()) {
            self.run_git(
                vec![
                    "fetch".to_string(),
                    "--depth".to_string(),
                    depth.to_string(),
                    "origin".to_string(),
                    sha.clone(),
                ],
                Some(repo_dir),
                ctx,
            )
            .await?;
            self.run_git(
                vec![
                    "checkout".to_string(),
                    "--detach".to_string(),
                    "FETCH_HEAD".to_string(),
                ],
                Some(repo_dir),
                ctx,
            )
            .await?;
        } else {
            self.run_git(
                vec![
                    "fetch".to_string(),
                    "--depth".to_string(),
                    depth.to_string(),
                    "origin".to_string(),
                    resolved_ref.clone(),
                ],
                Some(repo_dir),
                ctx,
            )
            .await?;
            self.run_git(
                vec![
                    "checkout".to_string(),
                    "--detach".to_string(),
                    "FETCH_HEAD".to_string(),
                ],
                Some(repo_dir),
                ctx,
            )
            .await?;
        }

        let head_sha = self
            .run_git(
                vec!["rev-parse".to_string(), "HEAD".to_string()],
                Some(repo_dir),
                ctx,
            )
            .await?
            .trim()
            .to_string();

        Ok(LocalRepoState {
            local_path: repo_dir.to_path_buf(),
            remote_url: remote_url.to_string(),
            default_branch: default_branch.to_string(),
            resolved_ref: normalized_optional_sha(input.sha.as_deref()).unwrap_or(resolved_ref),
            head_sha,
            depth,
            reused_cache,
            local_alias: normalized_local_alias(input.local_alias.as_deref()),
        })
    }

    async fn run_git(
        &self,
        args: Vec<String>,
        cwd: Option<&Path>,
        ctx: &ToolContext,
    ) -> Result<String, ToolError> {
        crate::git_runtime::run_git_command(&args, cwd, ctx, DEFAULT_GIT_TIMEOUT_SECS).await
    }

    async fn fetch_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        ctx: &ToolContext,
        text_matches_preview: bool,
    ) -> Result<T, ToolError> {
        let mut request = self
            .client
            .get(url)
            .header(header::USER_AGENT, "agendao-github-research")
            .header(header::ACCEPT, accept_header(text_matches_preview))
            .header("X-GitHub-Api-Version", "2022-11-28");

        if let Some(token) = github_token() {
            request = request.bearer_auth(token);
        }

        let request_future = async move {
            request
                .send()
                .await
                .map_err(|e| ToolError::ExecutionError(format!("GitHub request failed: {}", e)))
        };

        let response = tokio::select! {
            result = request_future => result?,
            _ = ctx.abort.cancelled() => return Err(ToolError::Cancelled),
            _ = tokio::time::sleep(Duration::from_secs(DEFAULT_TIMEOUT_SECS)) => {
                return Err(ToolError::Timeout(format!("GitHub request timed out after {} seconds", DEFAULT_TIMEOUT_SECS)));
            }
        };

        let status = response.status();
        if !status.is_success() {
            return Err(map_github_error(response, status).await);
        }

        response.json::<T>().await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to decode GitHub response: {}", e))
        })
    }
}

impl Default for GitHubResearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubResearchTool {
    fn id(&self) -> &str {
        "github_research"
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
                    "enum": [
                        "search_code",
                        "search_issues",
                        "search_prs",
                        "view_issue",
                        "view_pr",
                        "view_pr_files",
                        "get_head_sha",
                        "build_permalink",
                        "read_file",
                        "clone_repo",
                        "list_releases",
                        "list_tags",
                        "git_log",
                        "git_blame"
                    ],
                    "description": "GitHub research operation to execute"
                },
                "repo": {
                    "type": "string",
                    "description": "GitHub repository in owner/name format"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for search_code, search_issues, or search_prs"
                },
                "number": {
                    "type": "integer",
                    "description": "Issue or pull request number for view_issue, view_pr, or view_pr_files"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "default": 10,
                    "description": "Maximum number of results or entries to return"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "description": "Optional state filter for search_issues and search_prs"
                },
                "language": {
                    "type": "string",
                    "description": "Optional language filter for search_code"
                },
                "includeComments": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether to include comments for view_issue and view_pr"
                },
                "include_comments": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether to include comments for view_issue and view_pr (snake_case alias)"
                },
                "path": {
                    "type": "string",
                    "description": "Repository-relative file path for build_permalink, read_file, git_log filtering, or git_blame"
                },
                "sha": {
                    "type": "string",
                    "description": "Optional commit SHA used to pin get_head_sha, build_permalink, read_file, clone_repo, git_log, or git_blame"
                },
                "branch": {
                    "type": "string",
                    "description": "Optional branch name used when sha is not provided"
                },
                "localAlias": {
                    "type": "string",
                    "description": "Optional local cache alias for clone_repo and local git-backed operations"
                },
                "lineStart": {
                    "type": "integer",
                    "description": "Optional starting line for build_permalink or git_blame"
                },
                "lineEnd": {
                    "type": "integer",
                    "description": "Optional ending line for build_permalink or git_blame"
                },
                "depth": {
                    "type": "integer",
                    "description": "Optional clone/fetch depth for local git-backed operations"
                }
            },
            "required": ["operation", "repo"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: GitHubResearchInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        validate_input(&input)?;

        let mut permission = PermissionRequest::new("github_research")
            .with_pattern(&input.repo)
            .with_metadata("operation", serde_json::json!(input.operation.as_str()))
            .with_metadata("repo", serde_json::json!(&input.repo))
            .with_metadata("limit", serde_json::json!(input.limit))
            .always_allow();
        if let Some(query) = input.query.as_ref() {
            permission = permission.with_metadata("query", serde_json::json!(query));
        }
        if let Some(number) = input.number {
            permission = permission.with_metadata("number", serde_json::json!(number));
        }
        if let Some(state) = input.state.as_ref() {
            permission = permission.with_metadata("state", serde_json::json!(state));
        }
        if let Some(language) = input.language.as_ref() {
            permission = permission.with_metadata("language", serde_json::json!(language));
        }
        if let Some(path) = input.path.as_ref() {
            permission = permission.with_metadata("path", serde_json::json!(path));
        }
        if let Some(sha) = input.sha.as_ref() {
            permission = permission.with_metadata("sha", serde_json::json!(sha));
        }
        if let Some(branch) = input.branch.as_ref() {
            permission = permission.with_metadata("branch", serde_json::json!(branch));
        }
        if let Some(depth) = input.depth {
            permission = permission.with_metadata("depth", serde_json::json!(depth));
        }
        if let Some(local_alias) = input.local_alias.as_ref() {
            permission = permission.with_metadata("local_alias", serde_json::json!(local_alias));
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

fn validate_input(input: &GitHubResearchInput) -> Result<(), ToolError> {
    if !is_valid_repo(&input.repo) {
        return Err(ToolError::InvalidArguments(
            "repo must be in owner/name format".to_string(),
        ));
    }
    if input.limit == 0 || input.limit > MAX_LIMIT {
        return Err(ToolError::InvalidArguments(format!(
            "limit must be between 1 and {}",
            MAX_LIMIT
        )));
    }
    if let Some(state) = input.state.as_deref() {
        if !matches!(state, "open" | "closed" | "all") {
            return Err(ToolError::InvalidArguments(
                "state must be one of: open, closed, all".to_string(),
            ));
        }
    }
    if let Some(depth) = input.depth {
        if depth == 0 {
            return Err(ToolError::InvalidArguments(
                "depth must be greater than 0".to_string(),
            ));
        }
    }

    match input.operation {
        GitHubResearchOperation::SearchCode
        | GitHubResearchOperation::SearchIssues
        | GitHubResearchOperation::SearchPrs => {
            let query = input.query.as_deref().unwrap_or_default().trim();
            if query.is_empty() {
                return Err(ToolError::InvalidArguments(
                    "query is required for search operations".to_string(),
                ));
            }
        }
        GitHubResearchOperation::ViewIssue
        | GitHubResearchOperation::ViewPr
        | GitHubResearchOperation::ViewPrFiles => {
            if input.number.unwrap_or(0) == 0 {
                return Err(ToolError::InvalidArguments(
                    "number is required for view operations".to_string(),
                ));
            }
        }
        GitHubResearchOperation::BuildPermalink => {
            validate_required_path(input)?;
            validate_line_range(input)?;
        }
        GitHubResearchOperation::ReadFile => {
            validate_required_path(input)?;
        }
        GitHubResearchOperation::GitBlame => {
            validate_required_path(input)?;
            validate_line_range(input)?;
        }
        GitHubResearchOperation::GitLog
        | GitHubResearchOperation::GetHeadSha
        | GitHubResearchOperation::CloneRepo
        | GitHubResearchOperation::ListReleases
        | GitHubResearchOperation::ListTags => {}
    }

    Ok(())
}

fn is_valid_repo(repo: &str) -> bool {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    })
}

fn build_code_search_query(query: &str, repo: &str, language: Option<&str>) -> String {
    let mut out = format!("{} repo:{}", query.trim(), repo);
    if let Some(language) = language.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!(" language:{}", language));
    }
    out
}

fn build_issue_search_query(query: &str, repo: &str, kind: &str, state: Option<&str>) -> String {
    let mut out = format!("{} repo:{} is:{}", query.trim(), repo, kind);
    if let Some(state) = state.filter(|s| *s != "all") {
        out.push_str(&format!(" state:{}", state));
    }
    out
}

fn validate_required_path(input: &GitHubResearchInput) -> Result<(), ToolError> {
    let path = normalize_repo_path(input.path.as_deref().unwrap_or_default());
    if path.is_empty() {
        return Err(ToolError::InvalidArguments(
            "path is required for this operation".to_string(),
        ));
    }
    Ok(())
}

fn validate_line_range(input: &GitHubResearchInput) -> Result<(), ToolError> {
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
        if line_end == 0 {
            return Err(ToolError::InvalidArguments(
                "line_end must be greater than 0".to_string(),
            ));
        }
        if line_end < line_start {
            return Err(ToolError::InvalidArguments(
                "line_end must be greater than or equal to line_start".to_string(),
            ));
        }
    }
    Ok(())
}

fn github_token() -> Option<String> {
    env::var("GITHUB_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| env::var("GH_TOKEN").ok().filter(|v| !v.trim().is_empty()))
}

fn build_github_client() -> Client {
    match Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::error!(
                %error,
                "failed to build configured github_research client; falling back to default client"
            );
            Client::new()
        }
    }
}

fn required_query(input: &GitHubResearchInput) -> Result<&str, ToolError> {
    input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ToolError::InvalidArguments("query is required for search operations".to_string())
        })
}

fn required_number(input: &GitHubResearchInput) -> Result<u64, ToolError> {
    input.number.filter(|number| *number > 0).ok_or_else(|| {
        ToolError::InvalidArguments("number is required for view operations".to_string())
    })
}

fn required_path(input: &GitHubResearchInput) -> Result<String, ToolError> {
    let path = normalize_repo_path(input.path.as_deref().unwrap_or_default());
    if path.is_empty() {
        return Err(ToolError::InvalidArguments(
            "path is required for this operation".to_string(),
        ));
    }
    Ok(path)
}

fn accept_header(text_matches_preview: bool) -> &'static str {
    if text_matches_preview {
        "application/vnd.github.text-match+json, application/vnd.github+json"
    } else {
        "application/vnd.github+json"
    }
}

async fn map_github_error(response: reqwest::Response, status: StatusCode) -> ToolError {
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(%error, "failed to read GitHub error response body");
            format!("<failed to read GitHub error response body: {}>", error)
        }
    };
    let message =
        if status == StatusCode::FORBIDDEN && body.to_ascii_lowercase().contains("rate limit") {
            format!("GitHub API rate limit exceeded: {}", body)
        } else if status == StatusCode::UNAUTHORIZED {
            format!("GitHub authentication failed: {}", body)
        } else if status == StatusCode::NOT_FOUND {
            format!("GitHub resource not found: {}", body)
        } else {
            format!("GitHub API error ({}): {}", status, body)
        };
    ToolError::ExecutionError(message)
}

fn repo_from_repository_url(url: &str) -> String {
    url.strip_prefix("https://api.github.com/repos/")
        .unwrap_or(url)
        .to_string()
}

fn trim_snippet(snippet: String) -> String {
    snippet.trim().replace('\n', " ")
}

fn trim_body_snippet(body: &str) -> String {
    let mut snippet = body.trim().replace('\n', " ");
    if snippet.len() > 180 {
        snippet.truncate(180);
        snippet.push_str("...");
    }
    snippet
}

fn limit_patch(patch: String) -> String {
    if patch.len() <= 800 {
        patch
    } else {
        format!("{}...", &patch[..800])
    }
}

fn base_metadata(input: &GitHubResearchInput) -> Metadata {
    let mut metadata = Metadata::new();
    metadata.insert(
        "operation".to_string(),
        serde_json::json!(input.operation.as_str()),
    );
    metadata.insert("repo".to_string(), serde_json::json!(&input.repo));
    metadata.insert("limit".to_string(), serde_json::json!(input.limit));
    metadata.insert(
        "include_comments".to_string(),
        serde_json::json!(input.include_comments),
    );
    metadata.insert("implemented".to_string(), serde_json::json!(true));
    metadata.insert(
        "phase".to_string(),
        serde_json::json!(implemented_phase(&input.operation)),
    );
    metadata
}

fn insert_metadata_serialized<T: Serialize>(metadata: &mut Metadata, key: &str, value: &T) {
    metadata.insert(key.to_string(), metadata_value(key, value));
}

fn metadata_value<T: Serialize>(key: &str, value: &T) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(metadata_key = key, %error, "failed to serialize github_research metadata");
            serde_json::json!({
                "serialization_error": error.to_string(),
            })
        }
    }
}

fn render_search_code(
    repo: &str,
    query: &str,
    items: &[SearchCodeItem],
    total_count: u64,
) -> String {
    if items.is_empty() {
        return format!(
            "No GitHub code matches found in {} for query: {}",
            repo, query
        );
    }

    let mut lines = vec![format!(
        "Found {} code matches in {} for '{}' (GitHub reported total: {})",
        items.len(),
        repo,
        query,
        total_count
    )];
    for (idx, item) in items.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, item.path));
        lines.push(format!("   repo: {}", item.repo));
        lines.push(format!("   url: {}", item.url));
        if let Some(snippet) = item.snippet.as_ref() {
            lines.push(format!("   snippet: {}", snippet));
        }
    }
    lines.join("\n")
}

fn render_issue_search(
    kind: &str,
    repo: &str,
    query: &str,
    items: &[SearchIssueItem],
    total_count: u64,
) -> String {
    if items.is_empty() {
        return format!(
            "No GitHub {} matches found in {} for query: {}",
            kind, repo, query
        );
    }

    let mut lines = vec![format!(
        "Found {} GitHub {} matches in {} for '{}' (GitHub reported total: {})",
        items.len(),
        kind,
        repo,
        query,
        total_count
    )];
    for (idx, item) in items.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. #{} {}", idx + 1, item.number, item.title));
        lines.push(format!("   state: {}", item.state));
        lines.push(format!("   url: {}", item.url));
        if let Some(snippet) = item.snippet.as_ref() {
            lines.push(format!("   snippet: {}", snippet));
        }
    }
    lines.join("\n")
}

fn render_issue_view(view: &IssueView) -> String {
    let mut lines = vec![format!("Issue #{}: {}", view.number, view.title)];
    lines.push(format!("repo: {}", view.repo));
    lines.push(format!("state: {}", view.state));
    lines.push(format!("url: {}", view.url));
    if let Some(author) = view.author.as_ref() {
        lines.push(format!("author: {}", author));
    }
    if !view.labels.is_empty() {
        lines.push(format!("labels: {}", view.labels.join(", ")));
    }
    if let Some(body) = view.body.as_ref().filter(|b| !b.trim().is_empty()) {
        lines.push(String::new());
        lines.push("body:".to_string());
        lines.push(body.trim().to_string());
    }
    if !view.comments.is_empty() {
        lines.push(String::new());
        lines.push(format!("comments (showing up to {}):", view.comments.len()));
        for (idx, comment) in view.comments.iter().enumerate() {
            lines.push(format!(
                "{}. {}",
                idx + 1,
                comment.author.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!("   {}", trim_body_snippet(&comment.body)));
        }
    }
    lines.join("\n")
}

fn render_pr_view(view: &PullRequestView) -> String {
    let mut lines = vec![format!("Pull request #{}: {}", view.number, view.title)];
    lines.push(format!("repo: {}", view.repo));
    lines.push(format!("state: {}", view.state));
    lines.push(format!("url: {}", view.url));
    if let Some(author) = view.author.as_ref() {
        lines.push(format!("author: {}", author));
    }
    if let Some(merged) = view.merged {
        lines.push(format!("merged: {}", merged));
    }
    if let Some(draft) = view.draft {
        lines.push(format!("draft: {}", draft));
    }
    if let Some(base_ref) = view.base_ref.as_ref() {
        lines.push(format!("base: {}", base_ref));
    }
    if let Some(head_ref) = view.head_ref.as_ref() {
        lines.push(format!("head: {}", head_ref));
    }
    if let Some(head_sha) = view.head_sha.as_ref() {
        lines.push(format!("head_sha: {}", head_sha));
    }
    if let (Some(additions), Some(deletions), Some(changed_files)) =
        (view.additions, view.deletions, view.changed_files)
    {
        lines.push(format!(
            "changes: +{} / -{} across {} files",
            additions, deletions, changed_files
        ));
    }
    if let Some(commits) = view.commits {
        lines.push(format!("commits: {}", commits));
    }
    if let Some(body) = view.body.as_ref().filter(|b| !b.trim().is_empty()) {
        lines.push(String::new());
        lines.push("body:".to_string());
        lines.push(body.trim().to_string());
    }
    if !view.comments.is_empty() {
        lines.push(String::new());
        lines.push(format!("comments (showing up to {}):", view.comments.len()));
        for (idx, comment) in view.comments.iter().enumerate() {
            lines.push(format!(
                "{}. {}",
                idx + 1,
                comment.author.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!("   {}", trim_body_snippet(&comment.body)));
        }
    }
    lines.join("\n")
}

fn render_pr_files(repo: &str, number: u64, files: &[PullRequestFileItem]) -> String {
    if files.is_empty() {
        return format!("No files found for PR #{} in {}", number, repo);
    }

    let mut lines = vec![format!(
        "PR #{} in {} changes {} files",
        number,
        repo,
        files.len()
    )];
    for (idx, file) in files.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, file.filename));
        lines.push(format!("   status: {}", file.status));
        lines.push(format!(
            "   changes: +{} / -{} ({} total)",
            file.additions, file.deletions, file.changes
        ));
        if let Some(blob_url) = file.blob_url.as_ref() {
            lines.push(format!("   blob_url: {}", blob_url));
        }
        if let Some(patch) = file.patch.as_ref() {
            lines.push("   patch:".to_string());
            for line in patch.lines().take(12) {
                lines.push(format!("   {}", line));
            }
        }
    }
    lines.join("\n")
}

fn implemented_phase(operation: &GitHubResearchOperation) -> u8 {
    match operation {
        GitHubResearchOperation::SearchCode
        | GitHubResearchOperation::SearchIssues
        | GitHubResearchOperation::SearchPrs
        | GitHubResearchOperation::ViewIssue
        | GitHubResearchOperation::ViewPr
        | GitHubResearchOperation::ViewPrFiles => 1,
        _ => 2,
    }
}

fn normalize_repo_path(path: &str) -> String {
    path.trim().trim_start_matches('/').to_string()
}

fn normalized_optional_path(path: Option<&str>) -> Option<String> {
    path.map(normalize_repo_path)
        .filter(|value| !value.is_empty())
}

fn normalized_optional_sha(sha: Option<&str>) -> Option<String> {
    sha.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalized_local_alias(alias: Option<&str>) -> Option<String> {
    alias
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn encode_repo_path(path: &str) -> String {
    normalize_repo_path(path)
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn format_blob_permalink(
    repo: &str,
    path: &str,
    sha: &str,
    line_start: Option<u64>,
    line_end: Option<u64>,
) -> String {
    let mut url = format!(
        "https://github.com/{}/blob/{}/{}",
        repo,
        sha,
        encode_repo_path(path)
    );
    if let Some(line_start) = line_start {
        if let Some(line_end) = line_end {
            url.push_str(&format!("#L{}-L{}", line_start, line_end));
        } else {
            url.push_str(&format!("#L{}", line_start));
        }
    }
    url
}

fn decode_github_content(response: &GitHubContentResponse) -> Result<String, ToolError> {
    match response.encoding.as_deref() {
        Some("base64") => {
            let content = response.content.as_deref().ok_or_else(|| {
                ToolError::ExecutionError(format!(
                    "GitHub file content missing for {}",
                    response.path
                ))
            })?;
            let normalized = content.replace('\n', "");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(normalized.as_bytes())
                .map_err(|e| {
                    ToolError::ExecutionError(format!(
                        "Failed to decode GitHub file content for {}: {}",
                        response.path, e
                    ))
                })?;
            String::from_utf8(bytes).map_err(|e| {
                ToolError::ExecutionError(format!(
                    "GitHub file {} is not valid UTF-8: {}",
                    response.path, e
                ))
            })
        }
        Some(other) => Err(ToolError::ExecutionError(format!(
            "Unsupported GitHub content encoding for {}: {}",
            response.path, other
        ))),
        None => Err(ToolError::ExecutionError(format!(
            "GitHub content encoding missing for {}",
            response.path
        ))),
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn render_head_sha(view: &HeadShaView) -> String {
    format!(
        "Resolved HEAD for {}\nresolved_ref: {}\ndefault_branch: {}\nsha: {}",
        view.repo, view.resolved_ref, view.default_branch, view.sha
    )
}

fn render_permalink(view: &PermalinkView) -> String {
    let mut lines = vec![format!("Permalink for {}:{}", view.repo, view.path)];
    lines.push(format!("resolved_ref: {}", view.resolved_ref));
    lines.push(format!("sha: {}", view.sha));
    lines.push(format!("url: {}", view.url));
    lines.join("\n")
}

fn render_read_file(view: &ReadFileView, truncated: bool) -> String {
    let mut lines = vec![format!("File {}:{}", view.repo, view.path)];
    lines.push(format!("source: {}", view.source));
    lines.push(format!("resolved_ref: {}", view.resolved_ref));
    lines.push(format!("commit_sha: {}", view.commit_sha));
    if let Some(file_sha) = view.file_sha.as_ref() {
        lines.push(format!("file_sha: {}", file_sha));
    }
    lines.push(format!("size: {} bytes", view.size));
    lines.push(format!("line_count: {}", view.line_count));
    if let Some(url) = view.url.as_ref() {
        lines.push(format!("url: {}", url));
    }
    if let Some(download_url) = view.download_url.as_ref() {
        lines.push(format!("download_url: {}", download_url));
    }
    if truncated {
        lines.push(format!(
            "content: showing first {} characters",
            MAX_FILE_OUTPUT_CHARS
        ));
    } else {
        lines.push("content:".to_string());
    }
    lines.push(view.content.clone());
    lines.join("\n")
}

fn render_clone_repo(view: &CloneRepoView) -> String {
    let mut lines = vec![format!("Cloned {}", view.repo)];
    lines.push(format!("remote_url: {}", view.remote_url));
    lines.push(format!("local_path: {}", view.local_path));
    lines.push(format!("default_branch: {}", view.default_branch));
    lines.push(format!("resolved_ref: {}", view.resolved_ref));
    lines.push(format!("head_sha: {}", view.head_sha));
    lines.push(format!("depth: {}", view.depth));
    lines.push(format!("reused_cache: {}", view.reused_cache));
    if let Some(alias) = view.local_alias.as_ref() {
        lines.push(format!("local_alias: {}", alias));
    }
    lines.join("\n")
}

fn render_releases(repo: &str, items: &[ReleaseItem]) -> String {
    if items.is_empty() {
        return format!("No releases found for {}", repo);
    }

    let mut lines = vec![format!("Found {} releases for {}", items.len(), repo)];
    for (idx, item) in items.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, item.tag_name));
        if let Some(name) = item.name.as_ref() {
            lines.push(format!("   name: {}", name));
        }
        lines.push(format!("   draft: {}", item.draft));
        lines.push(format!("   prerelease: {}", item.prerelease));
        if let Some(published_at) = item.published_at.as_ref() {
            lines.push(format!("   published_at: {}", published_at));
        }
        if let Some(target_commitish) = item.target_commitish.as_ref() {
            lines.push(format!("   target_commitish: {}", target_commitish));
        }
        lines.push(format!("   url: {}", item.url));
        if let Some(snippet) = item.body_snippet.as_ref() {
            lines.push(format!("   body: {}", snippet));
        }
    }
    lines.join("\n")
}

fn render_tags(repo: &str, items: &[TagItem]) -> String {
    if items.is_empty() {
        return format!("No tags found for {}", repo);
    }

    let mut lines = vec![format!("Found {} tags for {}", items.len(), repo)];
    for (idx, item) in items.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, item.name));
        lines.push(format!("   sha: {}", item.sha));
        lines.push(format!("   url: {}", item.url));
    }
    lines.join("\n")
}

fn render_git_log(repo: &str, resolved_ref: &str, items: &[GitLogItem]) -> String {
    if items.is_empty() {
        return format!("No git log entries found for {} at {}", repo, resolved_ref);
    }

    let mut lines = vec![format!(
        "Found {} git log entries for {} at {}",
        items.len(),
        repo,
        resolved_ref
    )];
    for (idx, item) in items.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!("{}. {}", idx + 1, item.subject));
        lines.push(format!("   sha: {}", item.sha));
        lines.push(format!(
            "   author: {} <{}>",
            item.author, item.author_email
        ));
        lines.push(format!("   date: {}", item.date));
        lines.push(format!("   url: {}", item.url));
        if let Some(body) = item.body.as_ref() {
            lines.push(format!("   body: {}", body));
        }
    }
    lines.join("\n")
}

fn render_git_blame(
    repo: &str,
    path: &str,
    line_start: u64,
    line_end: u64,
    lines_data: &[GitBlameLine],
) -> String {
    if lines_data.is_empty() {
        return format!(
            "No git blame data found for {}:{} lines {}-{}",
            repo, path, line_start, line_end
        );
    }

    let mut lines = vec![format!(
        "Git blame for {}:{} lines {}-{}",
        repo, path, line_start, line_end
    )];
    for item in lines_data {
        lines.push(String::new());
        lines.push(format!("L{} {}", item.line_number, item.content));
        lines.push(format!("   sha: {}", item.commit_sha));
        if let Some(author) = item.author.as_ref() {
            lines.push(format!("   author: {}", author));
        }
        if let Some(summary) = item.summary.as_ref() {
            lines.push(format!("   summary: {}", summary));
        }
        lines.push(format!("   url: {}", item.commit_url));
    }
    lines.join("\n")
}

fn ensure_git_available() -> Result<(), ToolError> {
    crate::git_runtime::ensure_git_available()
}

fn configured_repo_override(ctx: &ToolContext, key: &str, repo: &str) -> Option<String> {
    ctx.extra
        .get(key)
        .and_then(|value| value.as_object())
        .and_then(|map| map.get(repo))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn configured_repo_ref_override(
    ctx: &ToolContext,
    key: &str,
    repo: &str,
    reference: &str,
) -> Option<String> {
    ctx.extra
        .get(key)
        .and_then(|value| value.as_object())
        .and_then(|repos| repos.get(repo))
        .and_then(|value| value.as_object())
        .and_then(|refs| refs.get(reference))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn remote_url_for_repo(repo: &str, ctx: &ToolContext) -> String {
    configured_repo_override(ctx, "github_research_remote_url_overrides", repo)
        .unwrap_or_else(|| format!("https://github.com/{}.git", repo))
}

fn local_repo_root(ctx: &ToolContext) -> PathBuf {
    ctx.extra
        .get("github_research_cache_root")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("agendao")
                .join("github_research")
        })
}

fn repo_cache_key(repo: &str, alias: Option<&str>) -> String {
    let base = alias
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(repo);
    sanitize_cache_component(base)
}

fn sanitize_cache_component(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "repo".to_string()
    } else {
        out
    }
}

fn clone_depth(input: &GitHubResearchInput) -> u64 {
    input.depth.unwrap_or(DEFAULT_CLONE_DEPTH).max(1)
}

fn is_git_repo_dir(path: &Path) -> bool {
    path.join(".git").exists()
}

fn parse_git_log_output(repo: &str, raw: &str) -> Vec<GitLogItem> {
    raw.split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim();
            if record.is_empty() {
                return None;
            }
            let mut parts = record.split('\u{1f}');
            let sha = parts.next()?.trim().to_string();
            let author = parts.next()?.trim().to_string();
            let author_email = parts.next()?.trim().to_string();
            let date = parts.next()?.trim().to_string();
            let subject = parts.next()?.trim().to_string();
            let body = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            Some(GitLogItem {
                sha: sha.clone(),
                author,
                author_email,
                date,
                subject,
                body,
                url: format!("https://github.com/{}/commit/{}", repo, sha),
            })
        })
        .collect()
}

fn parse_git_blame_porcelain(repo: &str, raw: &str) -> Vec<GitBlameLine> {
    let mut lines = Vec::new();
    let mut current_meta: HashMap<String, BlameMeta> = HashMap::new();
    let mut pending_sha: Option<String> = None;
    let mut pending_final_line: Option<u64> = None;

    for raw_line in raw.lines() {
        if raw_line.starts_with('\t') {
            let commit_sha = pending_sha.take().unwrap_or_default();
            let final_line = pending_final_line.take().unwrap_or(0);
            let meta = current_meta.get(&commit_sha).cloned().unwrap_or_default();
            lines.push(GitBlameLine {
                line_number: final_line,
                commit_sha: commit_sha.clone(),
                author: meta.author,
                author_mail: meta.author_mail,
                author_time: meta.author_time,
                summary: meta.summary,
                previous: meta.previous,
                content: raw_line.trim_start_matches('\t').to_string(),
                commit_url: format!("https://github.com/{}/commit/{}", repo, commit_sha),
            });
            continue;
        }

        let mut fields = raw_line.split_whitespace();
        if let (Some(first), Some(_orig), Some(final_line)) =
            (fields.next(), fields.next(), fields.next())
        {
            if first.len() == 40 && first.chars().all(|ch| ch.is_ascii_hexdigit()) {
                pending_sha = Some(first.to_string());
                pending_final_line = final_line.parse::<u64>().ok();
                current_meta.entry(first.to_string()).or_default();
                continue;
            }
        }

        if let Some(sha) = pending_sha.as_ref() {
            let meta = current_meta.entry(sha.clone()).or_default();
            if let Some(value) = raw_line.strip_prefix("author ") {
                meta.author = Some(value.to_string());
            } else if let Some(value) = raw_line.strip_prefix("author-mail ") {
                meta.author_mail = Some(value.to_string());
            } else if let Some(value) = raw_line.strip_prefix("author-time ") {
                meta.author_time = Some(value.to_string());
            } else if let Some(value) = raw_line.strip_prefix("summary ") {
                meta.summary = Some(value.to_string());
            } else if let Some(value) = raw_line.strip_prefix("previous ") {
                meta.previous = Some(value.to_string());
            }
        }
    }

    lines
}

fn blame_line_window(input: &GitHubResearchInput) -> (u64, u64) {
    let line_start = input.line_start.unwrap_or(1);
    let line_end = input.line_end.unwrap_or(
        line_start
            .saturating_add(input.limit as u64)
            .saturating_sub(1),
    );
    (line_start, line_end.max(line_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input(operation: GitHubResearchOperation) -> GitHubResearchInput {
        GitHubResearchInput {
            operation,
            repo: "owner/repo".to_string(),
            query: None,
            number: None,
            path: None,
            sha: None,
            language: None,
            branch: None,
            local_alias: None,
            line_start: None,
            line_end: None,
            depth: None,
            state: None,
            limit: 10,
            include_comments: true,
        }
    }

    fn test_tool_context(directory: &Path) -> ToolContext {
        ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            directory.to_string_lossy().to_string(),
        )
        .with_ask(|_| async { Ok(()) })
    }

    fn test_tool_context_with_repo_overrides(
        directory: &Path,
        repo: &str,
        remote_url: &Path,
        cache_root: &Path,
        default_branch: &str,
    ) -> ToolContext {
        let mut ctx = test_tool_context(directory);
        ctx.extra.insert(
            "github_research_remote_url_overrides".to_string(),
            serde_json::json!({ repo: remote_url.to_string_lossy().to_string() }),
        );
        ctx.extra.insert(
            "github_research_default_branch_overrides".to_string(),
            serde_json::json!({ repo: default_branch }),
        );
        ctx.extra.insert(
            "github_research_cache_root".to_string(),
            serde_json::json!(cache_root.to_string_lossy().to_string()),
        );
        ctx
    }

    fn git_fixture_output(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git should execute in test fixture");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn git_fixture_run(cwd: &Path, args: &[&str]) {
        let _ = git_fixture_output(cwd, args);
    }

    fn build_local_repo_state(repo_dir: &Path) -> LocalRepoState {
        LocalRepoState {
            local_path: repo_dir.to_path_buf(),
            remote_url: "https://github.com/owner/repo.git".to_string(),
            default_branch: "main".to_string(),
            resolved_ref: "HEAD".to_string(),
            head_sha: git_fixture_output(repo_dir, &["rev-parse", "HEAD"]),
            depth: 1,
            reused_cache: true,
            local_alias: Some("fixture".to_string()),
        }
    }

    fn create_git_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_dir = dir.path();
        git_fixture_run(repo_dir, &["init", "-b", "main"]);
        git_fixture_run(repo_dir, &["config", "user.name", "Fixture User"]);
        git_fixture_run(repo_dir, &["config", "user.email", "fixture@example.com"]);
        std::fs::create_dir_all(repo_dir.join("src")).expect("create src dir");
        std::fs::write(
            repo_dir.join("src/lib.rs"),
            "pub fn alpha() {}
",
        )
        .expect("write first version");
        git_fixture_run(repo_dir, &["add", "."]);
        git_fixture_run(repo_dir, &["commit", "-m", "Initial commit"]);
        git_fixture_run(repo_dir, &["tag", "v0.1.0"]);

        std::fs::write(
            repo_dir.join("src/lib.rs"),
            "pub fn alpha() {}
pub fn beta() {}
",
        )
        .expect("write second version");
        git_fixture_run(repo_dir, &["add", "."]);
        git_fixture_run(repo_dir, &["commit", "-m", "Second commit"]);
        git_fixture_run(repo_dir, &["tag", "v0.2.0"]);
        dir
    }

    fn create_bare_remote_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let source = root.path().join("source");
        let bare = root.path().join("remote.git");
        std::fs::create_dir_all(&source).expect("create source dir");
        git_fixture_run(&source, &["init", "-b", "main"]);
        git_fixture_run(&source, &["config", "user.name", "Fixture User"]);
        git_fixture_run(&source, &["config", "user.email", "fixture@example.com"]);
        std::fs::create_dir_all(source.join("src")).expect("create src dir");
        std::fs::write(
            source.join("src/lib.rs"),
            "pub fn alpha() {}
",
        )
        .expect("write source file");
        git_fixture_run(&source, &["add", "."]);
        git_fixture_run(&source, &["commit", "-m", "Initial commit"]);
        git_fixture_run(&source, &["tag", "v0.1.0"]);
        git_fixture_run(
            root.path(),
            &["init", "--bare", bare.to_string_lossy().as_ref()],
        );
        git_fixture_run(
            &source,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );
        git_fixture_run(&source, &["push", "origin", "main", "--tags"]);
        (root, source, bare)
    }

    fn push_fixture_change(source: &Path, body: &str, message: &str, tag: Option<&str>) {
        std::fs::write(source.join("src/lib.rs"), body).expect("update source file");
        git_fixture_run(source, &["add", "."]);
        git_fixture_run(source, &["commit", "-m", message]);
        if let Some(tag) = tag {
            git_fixture_run(source, &["tag", tag]);
        }
        git_fixture_run(source, &["push", "origin", "main", "--tags"]);
    }

    #[test]
    fn schema_exposes_all_operations() {
        let tool = GitHubResearchTool::new();
        let schema = tool.parameters();
        let ops = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("enum should exist");
        for value in [
            "search_code",
            "search_issues",
            "search_prs",
            "view_issue",
            "view_pr",
            "view_pr_files",
            "get_head_sha",
            "build_permalink",
            "read_file",
            "clone_repo",
            "list_releases",
            "list_tags",
            "git_log",
            "git_blame",
        ] {
            assert!(ops.iter().any(|v| v == value), "missing {value}");
        }
    }

    #[test]
    fn validates_repo_format() {
        let mut input = base_input(GitHubResearchOperation::SearchCode);
        input.repo = "badrepo".to_string();
        input.query = Some("foo".to_string());
        assert!(validate_input(&input).is_err());
        input.repo = "owner/repo".to_string();
        assert!(validate_input(&input).is_ok());
    }

    #[test]
    fn search_query_builders_match_expected_shape() {
        assert_eq!(
            build_code_search_query("useQuery", "tanstack/query", Some("TypeScript")),
            "useQuery repo:tanstack/query language:TypeScript"
        );
        assert_eq!(
            build_issue_search_query("hydration bug", "vercel/next.js", "issue", Some("open")),
            "hydration bug repo:vercel/next.js is:issue state:open"
        );
        assert_eq!(
            build_issue_search_query("streaming", "vercel/next.js", "pr", Some("all")),
            "streaming repo:vercel/next.js is:pr"
        );
    }

    #[test]
    fn view_operations_require_number() {
        let input = base_input(GitHubResearchOperation::ViewPr);
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("number is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn permalink_and_read_file_require_path() {
        let mut permalink_input = base_input(GitHubResearchOperation::BuildPermalink);
        permalink_input.line_start = Some(10);
        permalink_input.line_end = Some(20);
        match validate_input(&permalink_input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("path is required")),
            other => panic!("unexpected result: {:?}", other),
        }

        let mut read_input = base_input(GitHubResearchOperation::ReadFile);
        read_input.path = Some(" ".to_string());
        match validate_input(&read_input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("path is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn git_blame_requires_path() {
        let input = base_input(GitHubResearchOperation::GitBlame);
        match validate_input(&input) {
            Err(ToolError::InvalidArguments(msg)) => assert!(msg.contains("path is required")),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn permalink_validation_rejects_invalid_line_ranges() {
        let mut missing_start = base_input(GitHubResearchOperation::BuildPermalink);
        missing_start.path = Some("src/lib.rs".to_string());
        missing_start.line_end = Some(12);
        match validate_input(&missing_start) {
            Err(ToolError::InvalidArguments(msg)) => {
                assert!(msg.contains("line_start is required"))
            }
            other => panic!("unexpected result: {:?}", other),
        }

        let mut reversed = base_input(GitHubResearchOperation::BuildPermalink);
        reversed.path = Some("src/lib.rs".to_string());
        reversed.line_start = Some(20);
        reversed.line_end = Some(10);
        match validate_input(&reversed) {
            Err(ToolError::InvalidArguments(msg)) => {
                assert!(msg.contains("line_end must be greater than or equal to line_start"))
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn build_permalink_formats_expected_url() {
        assert_eq!(
            format_blob_permalink("owner/repo", "src/lib.rs", "abc123", Some(5), Some(9),),
            "https://github.com/owner/repo/blob/abc123/src/lib.rs#L5-L9"
        );
        assert_eq!(
            format_blob_permalink("owner/repo", "src/lib.rs", "abc123", Some(7), None),
            "https://github.com/owner/repo/blob/abc123/src/lib.rs#L7"
        );
    }

    #[test]
    fn decode_github_content_handles_base64_payloads() {
        let response = GitHubContentResponse {
            path: "docs/readme.md".to_string(),
            sha: "blobsha".to_string(),
            size: 5,
            html_url: None,
            download_url: None,
            encoding: Some("base64".to_string()),
            content: Some("aGVsbG8=\n".to_string()),
            entry_type: "file".to_string(),
        };
        assert_eq!(decode_github_content(&response).unwrap(), "hello");
    }

    #[test]
    fn parse_git_log_output_extracts_records() {
        let raw = "abc123\u{1f}Alice\u{1f}alice@example.com\u{1f}2026-03-06T00:00:00Z\u{1f}Subject\u{1f}Body\u{1e}";
        let items = parse_git_log_output("owner/repo", raw);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].sha, "abc123");
        assert_eq!(items[0].subject, "Subject");
        assert_eq!(items[0].body.as_deref(), Some("Body"));
    }

    #[test]
    fn parse_git_blame_porcelain_extracts_lines() {
        let raw = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 1\nauthor Alice\nauthor-mail <alice@example.com>\nauthor-time 1700000000\nsummary Initial\n\tlet x = 1;\n";
        let lines = parse_git_blame_porcelain("owner/repo", raw);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_number, 1);
        assert_eq!(lines[0].author.as_deref(), Some("Alice"));
        assert_eq!(lines[0].summary.as_deref(), Some("Initial"));
        assert_eq!(lines[0].content, "let x = 1;");
    }

    #[test]
    fn blame_window_defaults_follow_limit() {
        let input = base_input(GitHubResearchOperation::GitBlame);
        assert_eq!(blame_line_window(&input), (1, 10));
    }

    #[tokio::test]
    async fn list_tags_in_repo_reads_local_fixture() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(dir.path());
        let tags = tool
            .list_tags_in_repo("owner/repo", dir.path(), 10, &ctx)
            .await
            .expect("tags should load from local fixture");
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "v0.2.0");
        assert_eq!(tags[1].name, "v0.1.0");
    }

    #[tokio::test]
    async fn git_log_in_repo_reads_local_fixture() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(dir.path());
        let items = tool
            .git_log_in_repo("owner/repo", dir.path(), Some("src/lib.rs"), 10, &ctx)
            .await
            .expect("git log should load from local fixture");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].subject, "Second commit");
        assert_eq!(items[1].subject, "Initial commit");
    }

    #[tokio::test]
    async fn git_blame_in_repo_reads_local_fixture() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(dir.path());
        let lines = tool
            .git_blame_in_repo("owner/repo", dir.path(), "src/lib.rs", 1, 2, &ctx)
            .await
            .expect("git blame should load from local fixture");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line_number, 1);
        assert_eq!(lines[0].author.as_deref(), Some("Fixture User"));
        assert_eq!(lines[1].line_number, 2);
        assert_eq!(lines[1].content, "pub fn beta() {}");
    }

    #[tokio::test]
    async fn read_file_from_local_prefers_fixture_repo() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(dir.path());
        let local = build_local_repo_state(dir.path());
        let mut input = base_input(GitHubResearchOperation::ReadFile);
        input.path = Some("src/lib.rs".to_string());
        let result = tool
            .read_file_from_local(&input, "src/lib.rs", local, &ctx)
            .await
            .expect("local read should succeed");
        assert!(result.output.contains("source: local_git"));
        assert!(result.output.contains("pub fn beta() {}"));
        assert_eq!(
            result.metadata["file"]["source"].as_str(),
            Some("local_git")
        );
    }

    #[tokio::test]
    async fn get_head_sha_can_use_commit_overrides_without_network() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let mut ctx = test_tool_context(dir.path());
        ctx.extra.insert(
            "github_research_default_branch_overrides".to_string(),
            serde_json::json!({ "owner/repo": "main" }),
        );
        ctx.extra.insert(
            "github_research_commit_overrides".to_string(),
            serde_json::json!({ "owner/repo": { "main": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef" } }),
        );
        let input = base_input(GitHubResearchOperation::GetHeadSha);
        let result = tool
            .get_head_sha(&input, &ctx)
            .await
            .expect("get_head_sha should resolve via override");
        assert_eq!(
            result.metadata["head"]["sha"].as_str(),
            Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        );
        assert_eq!(
            result.metadata["head"]["resolved_ref"].as_str(),
            Some("main")
        );
    }

    #[tokio::test]
    async fn build_permalink_can_use_default_branch_and_commit_overrides() {
        let dir = create_git_fixture();
        let tool = GitHubResearchTool::new();
        let mut ctx = test_tool_context(dir.path());
        ctx.extra.insert(
            "github_research_default_branch_overrides".to_string(),
            serde_json::json!({ "owner/repo": "main" }),
        );
        ctx.extra.insert(
            "github_research_commit_overrides".to_string(),
            serde_json::json!({ "owner/repo": { "main": "feedfacefeedfacefeedfacefeedfacefeedface" } }),
        );
        let mut input = base_input(GitHubResearchOperation::BuildPermalink);
        input.path = Some("src/lib.rs".to_string());
        input.line_start = Some(2);
        input.line_end = Some(4);
        let result = tool
            .build_permalink(&input, &ctx)
            .await
            .expect("build_permalink should resolve via override");
        assert_eq!(
            result.metadata["permalink"]["sha"].as_str(),
            Some("feedfacefeedfacefeedfacefeedfacefeedface")
        );
        assert!(result.output.contains("#L2-L4"));
    }

    #[tokio::test]
    async fn clone_repo_can_clone_and_fetch_from_local_bare_fixture() {
        let (root, source, bare) = create_bare_remote_fixture();
        let cache_root = root.path().join("cache");
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context_with_repo_overrides(
            root.path(),
            "owner/repo",
            &bare,
            &cache_root,
            "main",
        );
        let mut input = base_input(GitHubResearchOperation::CloneRepo);
        input.local_alias = Some("fixture-clone".to_string());
        input.branch = Some("main".to_string());

        let first = tool
            .ensure_local_repo(&input, &ctx)
            .await
            .expect("first clone should succeed");
        assert!(!first.reused_cache);
        assert!(first.local_path.join("src/lib.rs").exists());
        let first_head = first.head_sha.clone();

        push_fixture_change(
            &source,
            "pub fn alpha() {}
pub fn gamma() {}
",
            "Third commit",
            Some("v0.3.0"),
        );

        let second = tool
            .ensure_local_repo(&input, &ctx)
            .await
            .expect("second fetch should succeed");
        assert!(second.reused_cache);
        assert_ne!(second.head_sha, first_head);
        assert_eq!(
            second.head_sha,
            git_fixture_output(&source, &["rev-parse", "HEAD"])
        );
    }

    #[tokio::test]
    #[ignore = "requires GitHub network access"]
    async fn integration_list_releases_hits_github_api() {
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(Path::new("."));
        let mut input = base_input(GitHubResearchOperation::ListReleases);
        input.repo = "cli/cli".to_string();
        input.limit = 3;
        let result = tool
            .list_releases(&input, &ctx)
            .await
            .expect("GitHub releases API should succeed");
        assert!(result.metadata["count"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test]
    #[ignore = "requires GitHub network access"]
    async fn integration_search_issues_hits_github_api() {
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(Path::new("."));
        let mut input = base_input(GitHubResearchOperation::SearchIssues);
        input.repo = "cli/cli".to_string();
        input.query = Some("windows".to_string());
        input.state = Some("all".to_string());
        input.limit = 3;
        let result = tool
            .search_issues(&input, &ctx)
            .await
            .expect("GitHub issue search should succeed");
        assert!(result.metadata["count"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test]
    #[ignore = "requires GitHub network access"]
    async fn integration_view_pr_files_hits_github_api() {
        let tool = GitHubResearchTool::new();
        let ctx = test_tool_context(Path::new("."));
        let mut input = base_input(GitHubResearchOperation::ViewPrFiles);
        input.repo = "cli/cli".to_string();
        input.number = Some(1);
        input.limit = 20;
        let result = tool
            .view_pr_files(&input, &ctx)
            .await
            .expect("GitHub PR files API should succeed");
        assert!(result.metadata["count"].as_u64().unwrap_or(0) >= 1);
    }

    #[test]
    fn cache_key_uses_alias_when_present() {
        assert_eq!(
            repo_cache_key("owner/repo", Some("docs-research")),
            "docs-research"
        );
        assert_eq!(repo_cache_key("owner/repo", None), "owner_repo");
    }
}
