use crate::iterative_workflow::{
    CommitPolicyDefinition, IterativeWorkflowConfig, ObjectiveDefinition,
};
use crate::workflow_artifacts::WorkflowCommitRecord;
use crate::{ExecutionContext, OrchestratorError};
use glob::Pattern;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_COMMIT_MESSAGE: &str = "autoresearch iteration {iteration}: {decision}";
const DEFAULT_SQUASH_MESSAGE: &str = "autoresearch final state after {count} kept iterations";

#[derive(Debug, Clone)]
pub(crate) struct WorkflowWorkspaceUpdate {
    pub commit: Option<WorkflowCommitRecord>,
    pub kept_commits: Vec<WorkflowCommitRecord>,
    pub squashed_commit: Option<WorkflowCommitRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkflowWorkspaceService {
    enabled: bool,
    repo_root: Option<PathBuf>,
    workdir_prefix: Option<PathBuf>,
    include: Vec<Pattern>,
    exclude: Vec<Pattern>,
    protected: Vec<Pattern>,
    commit_policy: Option<CommitPolicyDefinition>,
    kept_commits: Vec<WorkflowCommitRecord>,
    pre_run_head: Option<String>,
}

impl WorkflowWorkspaceService {
    pub(crate) fn new(
        config: &IterativeWorkflowConfig,
        objective: &ObjectiveDefinition,
        exec_ctx: &ExecutionContext,
    ) -> Result<Self, OrchestratorError> {
        let workdir = PathBuf::from(&exec_ctx.workdir);
        let include = compile_patterns(&objective.scope.include, "include")?;
        let exclude = compile_patterns(&objective.scope.exclude, "exclude")?;
        let protected = config
            .workspace_policy
            .as_ref()
            .map(|policy| compile_patterns(&policy.protected_paths, "protected"))
            .transpose()?
            .unwrap_or_default();
        let commit_policy = config
            .workspace_policy
            .as_ref()
            .and_then(|policy| policy.commit_policy.clone());
        let enabled = commit_policy
            .as_ref()
            .map(|policy| {
                policy.commit_kept_iterations.unwrap_or(false)
                    || policy.squash_on_completion.unwrap_or(false)
            })
            .unwrap_or(false);

        if !enabled {
            return Ok(Self {
                enabled,
                repo_root: None,
                workdir_prefix: None,
                include,
                exclude,
                protected,
                commit_policy,
                kept_commits: Vec::new(),
                pre_run_head: None,
            });
        }

        let repo_root = PathBuf::from(run_git(&workdir, ["rev-parse", "--show-toplevel"])?);
        let workdir_prefix = workdir.strip_prefix(&repo_root).ok().map(PathBuf::from);
        let pre_run_head = Some(run_git(&workdir, ["rev-parse", "HEAD"])?);
        let service = Self {
            enabled,
            repo_root: Some(repo_root),
            workdir_prefix,
            include,
            exclude,
            protected,
            commit_policy,
            kept_commits: Vec::new(),
            pre_run_head,
        };
        service.ensure_clean_index()?;
        service.ensure_clean_scoped_workspace()?;
        Ok(service)
    }

    pub(crate) fn record_kept_iteration(
        &mut self,
        iteration: u32,
        decision: &str,
        summary: &str,
    ) -> Result<WorkflowWorkspaceUpdate, OrchestratorError> {
        if !self.commit_kept_iterations_enabled() {
            return Ok(self.snapshot_update(None, None));
        }
        let paths = self.collect_scoped_changed_paths()?;
        if paths.is_empty() {
            return Ok(self.snapshot_update(None, None));
        }
        self.stage_paths(&paths)?;
        if !self.has_staged_changes()? {
            return Ok(self.snapshot_update(None, None));
        }
        let message = self.render_commit_message(iteration, decision, summary);
        run_git(
            self.repo_root(),
            ["commit", "-m", message.as_str(), "--no-verify"],
        )?;
        let commit_sha = run_git(self.repo_root(), ["rev-parse", "HEAD"])?;
        let record = WorkflowCommitRecord {
            iteration,
            commit_sha,
            message,
            decision: decision.to_string(),
            summary: summary.trim().to_string(),
        };
        self.kept_commits.push(record.clone());
        Ok(self.snapshot_update(Some(record), None))
    }

    pub(crate) fn finalize(
        &mut self,
        final_iteration: Option<u32>,
        final_decision: Option<&str>,
        final_summary: Option<&str>,
    ) -> Result<WorkflowWorkspaceUpdate, OrchestratorError> {
        if !self.squash_on_completion_enabled() || self.kept_commits.len() < 2 {
            return Ok(self.snapshot_update(None, None));
        }
        let base = self.pre_run_head.as_deref().ok_or_else(|| {
            OrchestratorError::Other(
                "workflow workspace service is missing the pre-run commit anchor".to_string(),
            )
        })?;
        run_git(self.repo_root(), ["reset", "--soft", base])?;
        let message = self.render_squash_message(final_iteration, final_decision, final_summary);
        run_git(
            self.repo_root(),
            ["commit", "-m", message.as_str(), "--no-verify"],
        )?;
        let squashed_commit = WorkflowCommitRecord {
            iteration: final_iteration.unwrap_or(self.kept_commits.len() as u32),
            commit_sha: run_git(self.repo_root(), ["rev-parse", "HEAD"])?,
            message,
            decision: final_decision.unwrap_or("squash").to_string(),
            summary: final_summary
                .map(str::trim)
                .unwrap_or("squashed kept workflow iterations")
                .to_string(),
        };
        Ok(self.snapshot_update(None, Some(squashed_commit)))
    }

    pub(crate) fn snapshot(&self) -> WorkflowWorkspaceUpdate {
        self.snapshot_update(None, None)
    }

    fn ensure_clean_scoped_workspace(&self) -> Result<(), OrchestratorError> {
        let paths = self.collect_scoped_changed_paths()?;
        if paths.is_empty() {
            return Ok(());
        }
        Err(OrchestratorError::Other(format!(
            "workflow commitPolicy requires a clean scoped workspace before the run starts; found changes in: {}",
            paths.join(", ")
        )))
    }

    fn ensure_clean_index(&self) -> Result<(), OrchestratorError> {
        if self.has_staged_changes()? {
            return Err(OrchestratorError::Other(
                "workflow commitPolicy requires a clean git index before the run starts"
                    .to_string(),
            ));
        }
        Ok(())
    }

    fn collect_scoped_changed_paths(&self) -> Result<Vec<String>, OrchestratorError> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let output = run_git_raw(
            self.repo_root(),
            [
                "status",
                "--porcelain",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ],
        )?;
        let mut changed = Vec::new();
        for entry in output.split('\0').filter(|entry| !entry.is_empty()) {
            if entry.len() < 4 {
                continue;
            }
            let repo_relative = entry[3..].trim();
            if repo_relative.is_empty() {
                continue;
            }
            let Some(workdir_relative) = self.to_workdir_relative(repo_relative) else {
                continue;
            };
            if !self.matches_scope(&workdir_relative) || self.matches_protected(&workdir_relative) {
                continue;
            }
            changed.push(repo_relative.to_string());
        }
        changed.sort();
        changed.dedup();
        Ok(changed)
    }

    fn repo_root(&self) -> &Path {
        self.repo_root
            .as_deref()
            .expect("repo_root should exist when workspace service is enabled")
    }

    fn commit_kept_iterations_enabled(&self) -> bool {
        self.commit_policy
            .as_ref()
            .and_then(|policy| policy.commit_kept_iterations)
            .unwrap_or(false)
    }

    fn squash_on_completion_enabled(&self) -> bool {
        self.commit_policy
            .as_ref()
            .and_then(|policy| policy.squash_on_completion)
            .unwrap_or(false)
    }

    fn matches_scope(&self, relative_path: &str) -> bool {
        let included = if self.include.is_empty() {
            true
        } else {
            self.include
                .iter()
                .any(|pattern| pattern.matches(relative_path))
        };
        included
            && !self
                .exclude
                .iter()
                .any(|pattern| pattern.matches(relative_path))
    }

    fn matches_protected(&self, relative_path: &str) -> bool {
        self.protected
            .iter()
            .any(|pattern| pattern.matches(relative_path))
    }

    fn to_workdir_relative<'a>(&self, repo_relative: &'a str) -> Option<String> {
        let path = Path::new(repo_relative);
        let prefix = self
            .workdir_prefix
            .as_deref()
            .unwrap_or_else(|| Path::new(""));
        let relative = if prefix.as_os_str().is_empty() {
            path
        } else {
            path.strip_prefix(prefix).ok()?
        };
        Some(normalize_relative_path(relative))
    }

    fn stage_paths(&self, repo_relative_paths: &[String]) -> Result<(), OrchestratorError> {
        if repo_relative_paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
        args.extend(repo_relative_paths.iter().cloned());
        run_git(self.repo_root(), args)?;
        Ok(())
    }

    fn has_staged_changes(&self) -> Result<bool, OrchestratorError> {
        let status = Command::new("git")
            .args(["diff", "--cached", "--quiet", "--exit-code"])
            .current_dir(self.repo_root())
            .status()
            .map_err(|err| {
                OrchestratorError::Other(format!(
                    "failed to inspect staged workflow changes in '{}': {err}",
                    self.repo_root().display()
                ))
            })?;
        Ok(!status.success())
    }

    fn render_commit_message(&self, iteration: u32, decision: &str, summary: &str) -> String {
        let template = self
            .commit_policy
            .as_ref()
            .and_then(|policy| policy.message_template.as_deref())
            .unwrap_or(DEFAULT_COMMIT_MESSAGE);
        render_message_template(
            template,
            iteration,
            decision,
            summary,
            self.kept_commits.len() + 1,
        )
    }

    fn render_squash_message(
        &self,
        final_iteration: Option<u32>,
        final_decision: Option<&str>,
        final_summary: Option<&str>,
    ) -> String {
        let template = self
            .commit_policy
            .as_ref()
            .and_then(|policy| policy.message_template.as_deref())
            .unwrap_or(DEFAULT_SQUASH_MESSAGE);
        render_message_template(
            template,
            final_iteration.unwrap_or(self.kept_commits.len() as u32),
            final_decision.unwrap_or("squash"),
            final_summary.unwrap_or("squashed kept workflow iterations"),
            self.kept_commits.len(),
        )
    }

    fn snapshot_update(
        &self,
        commit: Option<WorkflowCommitRecord>,
        squashed_commit: Option<WorkflowCommitRecord>,
    ) -> WorkflowWorkspaceUpdate {
        WorkflowWorkspaceUpdate {
            commit,
            kept_commits: self.kept_commits.clone(),
            squashed_commit,
        }
    }
}

fn render_message_template(
    template: &str,
    iteration: u32,
    decision: &str,
    summary: &str,
    count: usize,
) -> String {
    template
        .replace("{iteration}", &iteration.to_string())
        .replace("{decision}", decision)
        .replace("{summary}", first_line(summary))
        .replace("{count}", &count.to_string())
}

fn first_line(value: &str) -> &str {
    value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("workflow update")
}

fn run_git<I, S>(cwd: &Path, args: I) -> Result<String, OrchestratorError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args_vec: Vec<_> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .map_err(|err| {
            OrchestratorError::Other(format!("failed to run git in '{}': {err}", cwd.display()))
        })?;

    if !output.status.success() {
        let rendered = args_vec
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(OrchestratorError::Other(format!(
            "git {} failed: {}",
            rendered,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_raw<I, S>(cwd: &Path, args: I) -> Result<String, OrchestratorError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args_vec: Vec<_> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .map_err(|err| {
            OrchestratorError::Other(format!("failed to run git in '{}': {err}", cwd.display()))
        })?;

    if !output.status.success() {
        let rendered = args_vec
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(OrchestratorError::Other(format!(
            "git {} failed: {}",
            rendered,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn compile_patterns(patterns: &[String], label: &str) -> Result<Vec<Pattern>, OrchestratorError> {
    patterns
        .iter()
        .map(|pattern| {
            Pattern::new(pattern).map_err(|err| {
                OrchestratorError::Other(format!(
                    "invalid workflow {label} glob '{pattern}': {err}"
                ))
            })
        })
        .collect()
}

fn normalize_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
