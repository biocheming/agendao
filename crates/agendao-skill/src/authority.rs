use crate::catalog::{
    load_snapshot_from_disk, persist_snapshot_to_disk, SkillCatalogCache, SkillCatalogSnapshot,
};
use crate::detail::read_skill_detail;
use crate::discovery::{
    collect_skill_roots, compute_root_signature, config_for_skill_discovery,
    is_valid_relative_skill_path, read_skill_body, root_signature_is_current, scan_skill_roots,
};
use crate::write::{
    apply_frontmatter_patch, atomic_write_string, build_create_frontmatter, build_skill_document,
    delete_file, delete_skill_directory, ensure_workspace_skill_markdown, load_skill_document,
    parse_skill_document, parse_skill_frontmatter, prune_empty_skill_parent_dirs,
    render_skill_document, render_skill_frontmatter_lines, resolve_create_skill_markdown_path,
    supporting_file_path, validate_skill_body, validate_skill_description,
    validate_skill_markdown_size, validate_skill_name, validate_supporting_file_size,
    workspace_skill_root, CreateSkillRequest, DeleteSkillRequest, EditSkillRequest,
    PatchSkillRequest, RemoveSkillFileRequest, SkillWriteAction, SkillWriteResult,
    WriteSkillFileRequest,
};
use crate::{
    LoadedSkill, LoadedSkillFile, SkillCategoryView, SkillDetailView, SkillError, SkillMeta,
    SkillMetaView, SkillSummary,
};
use agendao_config::{Config, ConfigStore};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone, Default)]
pub struct SkillFilter<'a> {
    pub available_tools: Option<&'a HashSet<String>>,
    pub available_toolsets: Option<&'a HashSet<String>>,
    pub current_stage: Option<&'a str>,
    pub category: Option<&'a str>,
}

#[derive(Clone)]
pub struct SkillAuthority {
    base_dir: PathBuf,
    config_store: Option<Arc<ConfigStore>>,
    config_snapshot: Option<Arc<Config>>,
    cache: Arc<RwLock<SkillCatalogCache>>,
}

impl SkillAuthority {
    pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self {
        Self {
            base_dir: base_dir.into(),
            config_snapshot: config_store.as_ref().map(|store| store.config()),
            config_store,
            cache: Arc::new(RwLock::new(SkillCatalogCache::default())),
        }
    }

    pub fn with_config(base_dir: impl Into<PathBuf>, config_snapshot: Option<Arc<Config>>) -> Self {
        Self {
            base_dir: base_dir.into(),
            config_store: None,
            config_snapshot,
            cache: Arc::new(RwLock::new(SkillCatalogCache::default())),
        }
    }

    pub fn list_skill_meta(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillMetaView>, SkillError> {
        let skills = self.filtered_skills(filter)?;
        Ok(skills.iter().map(SkillMetaView::from).collect())
    }

    pub fn list_skill_catalog(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillMeta>, SkillError> {
        self.filtered_skills(filter)
    }

    pub fn list_skill_categories(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillCategoryView>, SkillError> {
        let snapshot = self.current_snapshot()?;
        let mut categories = BTreeMap::<String, usize>::new();
        for skill in snapshot
            .skills
            .iter()
            .filter(|skill| skill_matches_filter(skill, filter))
        {
            let Some(category) = skill.category.as_deref().filter(|value| !value.is_empty()) else {
                continue;
            };
            *categories.entry(category.to_string()).or_default() += 1;
        }

        Ok(categories
            .into_iter()
            .map(|(name, skill_count)| SkillCategoryView {
                description: snapshot
                    .roots
                    .iter()
                    .find_map(|root| load_category_description(&root.path, &name)),
                name,
                skill_count,
            })
            .collect())
    }

    pub fn list_skills(&self) -> Vec<SkillSummary> {
        self.current_snapshot()
            .map(|snapshot| snapshot.skills.iter().map(SkillSummary::from).collect())
            .unwrap_or_default()
    }

    pub fn discover_skills(&self) -> Vec<SkillMeta> {
        self.current_snapshot()
            .map(|snapshot| snapshot.skills)
            .unwrap_or_default()
    }

    /// Inspection-only skill meta resolution.
    ///
    /// This is the raw catalog/read path. Runtime-facing callers must resolve
    /// through `SkillRuntimeResolver`, which layers vitality and governance
    /// rules on top of this authority.
    pub fn resolve_skill_for_inspection(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<SkillMeta, SkillError> {
        let skills = self.filtered_skills(filter)?;
        skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| unknown_skill_error(name, &self.discover_skills()))
    }

    /// Inspection-only raw skill load.
    pub fn load_skill_for_inspection(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<LoadedSkill, SkillError> {
        let meta = self.resolve_skill_for_inspection(name, filter)?;
        self.load_resolved_skill_for_inspection(meta)
    }

    pub(crate) fn load_resolved_skill_for_inspection(
        &self,
        meta: SkillMeta,
    ) -> Result<LoadedSkill, SkillError> {
        {
            let mut guard = self.write_cache()?;
            if let Some(loaded) = guard.cached_loaded_skill(&meta.name) {
                if loaded.meta.location == meta.location {
                    return Ok(loaded);
                }
            }
        }

        let content = read_skill_body(&meta.location).map_err(|error| SkillError::ReadFailed {
            path: meta.location.clone(),
            message: error.to_string(),
        })?;
        let loaded = LoadedSkill { meta, content };
        let mut guard = self.write_cache()?;
        guard.remember_loaded_skill(loaded.clone());
        Ok(loaded)
    }

    /// Inspection-only raw markdown source load.
    pub fn load_skill_source_for_inspection(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<String, SkillError> {
        let meta = self.resolve_skill_for_inspection(name, filter)?;
        self.load_resolved_skill_source_for_inspection(&meta)
    }

    pub(crate) fn load_resolved_skill_source_for_inspection(
        &self,
        meta: &SkillMeta,
    ) -> Result<String, SkillError> {
        std::fs::read_to_string(&meta.location).map_err(|error| SkillError::ReadFailed {
            path: meta.location.clone(),
            message: error.to_string(),
        })
    }

    /// Inspection-only structured detail load.
    pub fn load_skill_detail_for_inspection(
        &self,
        name: &str,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<SkillDetailView, SkillError> {
        let meta = self.resolve_skill_for_inspection(name, filter)?;
        self.load_skill_detail_for_meta_for_inspection(&meta)
    }

    pub fn load_skill_detail_for_meta_for_inspection(
        &self,
        meta: &SkillMeta,
    ) -> Result<SkillDetailView, SkillError> {
        read_skill_detail(&meta.location).map_err(|error| SkillError::ReadFailed {
            path: meta.location.clone(),
            message: error.to_string(),
        })
    }

    pub fn workspace_skill_root(&self) -> PathBuf {
        workspace_skill_root(&self.base_dir)
    }

    pub fn base_dir(&self) -> &std::path::Path {
        &self.base_dir
    }

    pub fn is_skill_meta_writable(&self, meta: &SkillMeta) -> bool {
        let root = self.workspace_skill_root();
        meta.location.starts_with(&root)
            && meta
                .location
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("SKILL.md"))
                .unwrap_or(false)
    }

    /// Inspection-only supporting file load.
    pub fn load_skill_file_for_inspection(
        &self,
        name: &str,
        file_path: &str,
    ) -> Result<LoadedSkillFile, SkillError> {
        let meta = self.resolve_skill_for_inspection(name, None)?;
        self.load_resolved_skill_file_for_inspection(&meta, file_path)
    }

    pub(crate) fn load_resolved_skill_file_for_inspection(
        &self,
        meta: &SkillMeta,
        file_path: &str,
    ) -> Result<LoadedSkillFile, SkillError> {
        if !is_valid_relative_skill_path(file_path) {
            return Err(SkillError::InvalidSkillFilePath {
                skill: meta.name.clone(),
                file_path: file_path.to_string(),
            });
        }

        let file_ref = meta
            .supporting_files
            .iter()
            .find(|file| file.relative_path == file_path)
            .ok_or_else(|| SkillError::SkillFileNotFound {
                skill: meta.name.clone(),
                file_path: file_path.to_string(),
            })?;

        let content = std::fs::read_to_string(&file_ref.location).map_err(|error| {
            SkillError::ReadFailed {
                path: file_ref.location.clone(),
                message: error.to_string(),
            }
        })?;

        Ok(LoadedSkillFile {
            skill_name: meta.name.clone(),
            file_path: file_ref.relative_path.clone(),
            location: file_ref.location.clone(),
            content,
        })
    }

    /// Inspection-only raw skill rendering.
    ///
    /// This preserves full SKILL.md bodies for catalog / audit / debugging style
    /// reads. Runtime-facing consumers must go through `SkillRuntimeResolver`
    /// and consume the structured runtime prompt packet instead of bypassing
    /// governance with raw markdown injection.
    pub fn render_loaded_skills_context_for_inspection(
        &self,
        requested_names: &[String],
    ) -> Result<(String, Vec<String>), SkillError> {
        let requested = normalize_requested_skill_names(requested_names);
        if requested.is_empty() {
            return Ok((String::new(), Vec::new()));
        }

        let mut context = String::new();
        let mut loaded = Vec::new();
        context.push_str("<loaded_skills>\n");
        for requested_name in &requested {
            let skill = self.load_skill_for_inspection(requested_name, None)?;
            context.push_str(&format!("<skill name=\"{}\">\n\n", skill.meta.name));
            context.push_str(&format!("# Skill: {}\n\n", skill.meta.name));
            context.push_str(&skill.content);
            context.push_str("\n\n");
            context.push_str(&format!(
                "Base directory: {}\n",
                skill
                    .meta
                    .location
                    .parent()
                    .unwrap_or(&self.base_dir)
                    .to_string_lossy()
            ));
            context.push_str("</skill>\n");
            loaded.push(skill.meta.name);
        }
        context.push_str("</loaded_skills>");

        Ok((context, loaded))
    }

    pub fn refresh(&self) -> Result<SkillCatalogSnapshot, SkillError> {
        let snapshot = self.build_snapshot(None);
        self.persist_snapshot(&snapshot);
        let config_revision = self.current_config_revision();
        let mut guard = self.write_cache()?;
        guard.set_snapshot(snapshot.clone(), config_revision);
        Ok(snapshot)
    }

    pub fn invalidate(&self) -> Result<(), SkillError> {
        let mut guard = self.write_cache()?;
        guard.clear();
        Ok(())
    }

    /// Future skill-manage write paths should call this after a successful
    /// mutation so the authority becomes the single post-write refresh point.
    pub fn refresh_after_mutation(&self) -> Result<SkillCatalogSnapshot, SkillError> {
        self.invalidate()?;
        self.refresh()
    }

    pub fn create_skill(&self, req: CreateSkillRequest) -> Result<SkillWriteResult, SkillError> {
        let name = validate_skill_name(&req.name)?;
        let description = validate_skill_description(&name, &req.description)?;
        let body = validate_skill_body(&req.body)?;

        if self
            .discover_skills()
            .iter()
            .any(|skill| skill.name.eq_ignore_ascii_case(&name))
        {
            return Err(SkillError::SkillAlreadyExists { name });
        }

        let target = resolve_create_skill_markdown_path(
            &self.base_dir,
            &CreateSkillRequest {
                name: name.clone(),
                description: description.clone(),
                body: body.clone(),
                frontmatter: req.frontmatter.clone(),
                category: req.category.clone(),
                directory_name: req.directory_name.clone(),
            },
        )?;
        if target.exists() {
            return Err(SkillError::InvalidWriteTarget { path: target });
        }

        let frontmatter = build_create_frontmatter(&name, &description, req.frontmatter.as_ref())?;
        let content = build_skill_document(&frontmatter, &body)?;
        validate_skill_markdown_size(&content, &target.to_string_lossy())?;
        atomic_write_string(&target, &content)?;

        let snapshot = self.refresh_after_mutation()?;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| SkillError::UnknownSkill {
                requested: name.clone(),
                available: String::new(),
            })?;
        Ok(SkillWriteResult::with_skill(
            SkillWriteAction::Created,
            skill,
        ))
    }

    pub fn patch_skill(&self, req: PatchSkillRequest) -> Result<SkillWriteResult, SkillError> {
        let meta = self.resolve_skill_for_inspection(&req.name, None)?;
        ensure_workspace_skill_markdown(&self.base_dir, &meta.name, &meta.location)?;

        if req.new_name.is_none()
            && req.description.is_none()
            && req.body.is_none()
            && req.frontmatter.is_none()
        {
            return Err(SkillError::InvalidSkillContent {
                message: "patch requires at least one field".to_string(),
            });
        }

        let mut document = load_skill_document(&meta.location)?;
        let mut frontmatter = parse_skill_frontmatter(&document)?;
        let next_name = match req.new_name.as_deref() {
            Some(value) => validate_skill_name(value)?,
            None => meta.name.clone(),
        };
        if !next_name.eq_ignore_ascii_case(&meta.name)
            && self
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(&next_name))
        {
            return Err(SkillError::SkillAlreadyExists { name: next_name });
        }

        let next_description = match req.description.as_deref() {
            Some(value) => validate_skill_description(&next_name, value)?,
            None => meta.description.clone(),
        };
        let next_body = match req.body.as_deref() {
            Some(value) => validate_skill_body(value)?,
            None => document.body.clone(),
        };

        frontmatter.name = next_name.clone();
        frontmatter.description = next_description;
        if let Some(patch) = req.frontmatter.as_ref() {
            apply_frontmatter_patch(&mut frontmatter, patch);
        }
        document.frontmatter_lines = render_skill_frontmatter_lines(&frontmatter)?;
        document.body = next_body;

        let content = render_skill_document(&document);
        validate_skill_markdown_size(&content, &meta.location.to_string_lossy())?;
        atomic_write_string(&meta.location, &content)?;

        let snapshot = self.refresh_after_mutation()?;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(&next_name))
            .ok_or_else(|| SkillError::UnknownSkill {
                requested: next_name.clone(),
                available: String::new(),
            })?;
        Ok(SkillWriteResult::with_skill(
            SkillWriteAction::Patched,
            skill,
        ))
    }

    pub fn edit_skill(&self, req: EditSkillRequest) -> Result<SkillWriteResult, SkillError> {
        let meta = self.resolve_skill_for_inspection(&req.name, None)?;
        ensure_workspace_skill_markdown(&self.base_dir, &meta.name, &meta.location)?;
        validate_skill_markdown_size(&req.content, &meta.location.to_string_lossy())?;

        let mut document = parse_skill_document(&req.content)?;
        let frontmatter = parse_skill_frontmatter(&document)?;
        let next_name = validate_skill_name(&frontmatter.name)?;
        let next_description = validate_skill_description(&next_name, &frontmatter.description)?;
        let next_body = validate_skill_body(&document.body)?;
        if !next_name.eq_ignore_ascii_case(&meta.name)
            && self
                .discover_skills()
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(&next_name))
        {
            return Err(SkillError::SkillAlreadyExists { name: next_name });
        }

        let mut frontmatter = frontmatter;
        frontmatter.name = next_name.clone();
        frontmatter.description = next_description;
        document.frontmatter_lines = render_skill_frontmatter_lines(&frontmatter)?;
        document.body = next_body;

        let content = render_skill_document(&document);
        atomic_write_string(&meta.location, &content)?;

        let snapshot = self.refresh_after_mutation()?;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(&next_name))
            .ok_or_else(|| SkillError::UnknownSkill {
                requested: next_name.clone(),
                available: String::new(),
            })?;
        Ok(SkillWriteResult::with_skill(
            SkillWriteAction::Edited,
            skill,
        ))
    }

    pub fn write_supporting_file(
        &self,
        req: WriteSkillFileRequest,
    ) -> Result<SkillWriteResult, SkillError> {
        let meta = self.resolve_skill_for_inspection(&req.name, None)?;
        ensure_workspace_skill_markdown(&self.base_dir, &meta.name, &meta.location)?;
        let path =
            supporting_file_path(&meta.location, &req.file_path).map_err(|error| match error {
                SkillError::InvalidSkillFilePath { file_path, .. } => {
                    SkillError::InvalidSkillFilePath {
                        skill: meta.name.clone(),
                        file_path,
                    }
                }
                other => other,
            })?;
        validate_supporting_file_size(&req.file_path, &req.content)?;
        atomic_write_string(&path, &req.content)?;

        let snapshot = self.refresh_after_mutation()?;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(&meta.name))
            .ok_or_else(|| SkillError::UnknownSkill {
                requested: meta.name.clone(),
                available: String::new(),
            })?;
        Ok(
            SkillWriteResult::with_skill(SkillWriteAction::SupportingFileWritten, skill)
                .with_supporting_file(req.file_path),
        )
    }

    pub fn remove_supporting_file(
        &self,
        req: RemoveSkillFileRequest,
    ) -> Result<SkillWriteResult, SkillError> {
        let meta = self.resolve_skill_for_inspection(&req.name, None)?;
        ensure_workspace_skill_markdown(&self.base_dir, &meta.name, &meta.location)?;
        let path =
            supporting_file_path(&meta.location, &req.file_path).map_err(|error| match error {
                SkillError::InvalidSkillFilePath { file_path, .. } => {
                    SkillError::InvalidSkillFilePath {
                        skill: meta.name.clone(),
                        file_path,
                    }
                }
                other => other,
            })?;
        delete_file(&path, &meta.name, &req.file_path)?;
        let skill_root = workspace_skill_root(&self.base_dir);
        let stop_at = meta.location.parent().unwrap_or(skill_root.as_path());
        prune_empty_skill_parent_dirs(&path, stop_at);

        let snapshot = self.refresh_after_mutation()?;
        let skill = snapshot
            .skills
            .into_iter()
            .find(|skill| skill.name.eq_ignore_ascii_case(&meta.name))
            .ok_or_else(|| SkillError::UnknownSkill {
                requested: meta.name.clone(),
                available: String::new(),
            })?;
        Ok(
            SkillWriteResult::with_skill(SkillWriteAction::SupportingFileRemoved, skill)
                .with_supporting_file(req.file_path),
        )
    }

    pub fn delete_skill(&self, req: DeleteSkillRequest) -> Result<SkillWriteResult, SkillError> {
        let meta = self.resolve_skill_for_inspection(&req.name, None)?;
        ensure_workspace_skill_markdown(&self.base_dir, &meta.name, &meta.location)?;
        let skill_dir = meta
            .location
            .parent()
            .ok_or_else(|| SkillError::InvalidWriteTarget {
                path: meta.location.clone(),
            })?
            .to_path_buf();
        if skill_dir == workspace_skill_root(&self.base_dir) {
            return Err(SkillError::InvalidWriteTarget { path: skill_dir });
        }

        delete_skill_directory(&skill_dir)?;
        let result = SkillWriteResult::deleted(meta.name.clone(), meta.location.clone());
        let _ = self.refresh_after_mutation()?;
        Ok(result)
    }

    fn filtered_skills(
        &self,
        filter: Option<&SkillFilter<'_>>,
    ) -> Result<Vec<SkillMeta>, SkillError> {
        let snapshot = self.current_snapshot()?;
        Ok(snapshot
            .skills
            .into_iter()
            .filter(|skill| skill_matches_filter(skill, filter))
            .collect())
    }

    fn current_snapshot(&self) -> Result<SkillCatalogSnapshot, SkillError> {
        let config = self.current_config();
        let roots = collect_skill_roots(&self.base_dir, config.as_deref());
        let config_revision = self.current_config_revision();
        {
            let guard = self.read_cache()?;
            if let Some(snapshot) = &guard.snapshot {
                if guard.config_revision == config_revision
                    && snapshot.roots == roots
                    && self.snapshot_signatures_match(snapshot, &roots)
                {
                    return Ok(snapshot.clone());
                }
            }
        }

        if let Some(snapshot) = load_snapshot_from_disk(&self.base_dir) {
            if snapshot.roots == roots && self.snapshot_signatures_match(&snapshot, &roots) {
                let mut guard = self.write_cache()?;
                guard.set_snapshot(snapshot.clone(), config_revision);
                return Ok(snapshot);
            }
        }

        let next_snapshot = self.build_snapshot(Some((config.clone(), roots.clone())));
        self.persist_snapshot(&next_snapshot);
        let mut guard = self.write_cache()?;
        guard.set_snapshot(next_snapshot.clone(), config_revision);
        Ok(next_snapshot)
    }

    fn build_snapshot(
        &self,
        prepared: Option<(Option<Arc<Config>>, Vec<crate::catalog::SkillRoot>)>,
    ) -> SkillCatalogSnapshot {
        let (_, roots) = prepared.unwrap_or_else(|| {
            let config = self.current_config();
            let roots = collect_skill_roots(&self.base_dir, config.as_deref());
            (config, roots)
        });
        let signatures = roots.iter().map(compute_root_signature).collect();
        let skills = scan_skill_roots(&roots);
        SkillCatalogSnapshot {
            roots,
            signatures,
            skills,
        }
    }

    fn current_config(&self) -> Option<Arc<Config>> {
        self.config_store
            .as_deref()
            .map(ConfigStore::config)
            .or_else(|| self.config_snapshot.clone())
            .or_else(|| config_for_skill_discovery(&self.base_dir, None))
    }

    fn current_config_revision(&self) -> u64 {
        self.config_store
            .as_deref()
            .map(ConfigStore::revision)
            .unwrap_or(0)
    }

    fn snapshot_signatures_match(
        &self,
        snapshot: &SkillCatalogSnapshot,
        roots: &[crate::catalog::SkillRoot],
    ) -> bool {
        snapshot.signatures.len() == roots.len()
            && snapshot
                .signatures
                .iter()
                .zip(roots.iter())
                .all(|(expected, root)| {
                    expected.root == root.path && root_signature_is_current(expected)
                })
    }

    fn persist_snapshot(&self, snapshot: &SkillCatalogSnapshot) {
        if let Err(error) = persist_snapshot_to_disk(&self.base_dir, snapshot) {
            tracing::debug!(
                path = %self.base_dir.display(),
                %error,
                "failed to persist skill catalog snapshot"
            );
        }
    }

    fn read_cache(&self) -> Result<RwLockReadGuard<'_, SkillCatalogCache>, SkillError> {
        self.cache.read().map_err(|_| SkillError::CachePoisoned {
            resource: "skill catalog cache",
        })
    }

    fn write_cache(&self) -> Result<RwLockWriteGuard<'_, SkillCatalogCache>, SkillError> {
        self.cache.write().map_err(|_| SkillError::CachePoisoned {
            resource: "skill catalog cache",
        })
    }
}

fn normalize_requested_skill_names(raw_names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in raw_names {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(trimmed))
        {
            out.push(trimmed.to_string());
        }
    }
    out
}

pub fn infer_toolsets_from_tools<'a, I>(tool_ids: I) -> HashSet<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut toolsets = HashSet::new();
    for tool_id in tool_ids {
        let normalized = tool_id.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }

        if normalized == "browser_session" || normalized.starts_with("browser_") {
            toolsets.insert("browser".to_string());
        }

        if normalized == "webfetch"
            || normalized == "websearch"
            || normalized == "github_research"
            || normalized.starts_with("web_")
        {
            toolsets.insert("web".to_string());
        }

        if matches!(
            normalized.as_str(),
            "read"
                | "glob"
                | "grep"
                | "ls"
                | "ast_grep_search"
                | "codesearch"
                | "repo_history"
                | "context_docs"
        ) {
            toolsets.insert("search".to_string());
        }

        if matches!(
            normalized.as_str(),
            "write" | "edit" | "multiedit" | "apply_patch" | "ast_grep_replace"
        ) {
            toolsets.insert("edit".to_string());
        }

        if matches!(normalized.as_str(), "bash" | "shell_session") {
            toolsets.insert("shell".to_string());
        }

        if normalized == "lsp" || normalized.starts_with("lsp_") {
            toolsets.insert("lsp".to_string());
        }

        if normalized == "context_docs" {
            toolsets.insert("docs".to_string());
        }
    }

    toolsets
}

fn skill_matches_filter(skill: &SkillMeta, filter: Option<&SkillFilter<'_>>) -> bool {
    let Some(filter) = filter else {
        return true;
    };

    if let Some(category) = filter.category {
        if skill.category.as_deref() != Some(category) {
            return false;
        }
    }

    if !skill.conditions.requires_tools.is_empty() {
        let Some(available_tools) = filter.available_tools else {
            return false;
        };
        if !skill
            .conditions
            .requires_tools
            .iter()
            .all(|tool| available_tools.contains(tool))
        {
            return false;
        }
    }

    if !skill.conditions.fallback_for_tools.is_empty() {
        if let Some(available_tools) = filter.available_tools {
            if skill
                .conditions
                .fallback_for_tools
                .iter()
                .any(|tool| available_tools.contains(tool))
            {
                return false;
            }
        }
    }

    if !skill.conditions.requires_toolsets.is_empty() {
        let Some(available_toolsets) = filter.available_toolsets else {
            return false;
        };
        if !skill
            .conditions
            .requires_toolsets
            .iter()
            .all(|toolset| available_toolsets.contains(toolset))
        {
            return false;
        }
    }

    if !skill.conditions.fallback_for_toolsets.is_empty() {
        if let Some(available_toolsets) = filter.available_toolsets {
            if skill
                .conditions
                .fallback_for_toolsets
                .iter()
                .any(|toolset| available_toolsets.contains(toolset))
            {
                return false;
            }
        }
    }

    if !skill.conditions.stage_filter.is_empty() {
        let Some(stage) = filter.current_stage else {
            return false;
        };
        if !skill
            .conditions
            .stage_filter
            .iter()
            .any(|allowed| allowed == stage)
        {
            return false;
        }
    }

    true
}

fn load_category_description(root: &Path, category: &str) -> Option<String> {
    let description_path = root.join(category).join("DESCRIPTION.md");
    let content = fs::read_to_string(description_path).ok()?;
    extract_category_description(&content)
}

fn extract_category_description(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(frontmatter_description) = extract_frontmatter_description(trimmed) {
        return Some(crate::types::truncate_catalog_description(
            &frontmatter_description,
        ));
    }

    let mut in_frontmatter = false;
    for line in trimmed.lines() {
        let candidate = line.trim();
        if candidate == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter || candidate.is_empty() || candidate.starts_with('#') {
            continue;
        }
        return Some(crate::types::truncate_catalog_description(candidate));
    }

    None
}

fn extract_frontmatter_description(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim() != "description" {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'').trim();
        if value.is_empty() {
            return None;
        }
        return Some(value.to_string());
    }

    None
}

fn unknown_skill_error(requested: &str, skills: &[SkillMeta]) -> SkillError {
    let available = skills
        .iter()
        .map(|skill| skill.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    SkillError::UnknownSkill {
        requested: requested.to_string(),
        available,
    }
}
