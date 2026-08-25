//! Canonical file-path authority shared by every file tool.
//!
//! A tool must derive permission scope and the path it actually opens from
//! the same value. Existing entries are operated through their canonical
//! path; create targets retain their uncreated suffix beneath the deepest
//! canonical ancestor. Component-wise containment comes exclusively from
//! `agendao-sandbox`, never from string-prefix checks.

use std::path::{Path, PathBuf};

use agendao_sandbox::{
    assert_within_root, canonicalize_existing, resolve_create_target, CanonicalPath,
};

use crate::ToolError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePathLocation {
    Workspace { relative: String },
    External,
}

/// One canonical, operation-ready path together with its permission class.
/// Callers must use `operation_path` for I/O rather than reopening their raw
/// input path after the permission decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedFilePath {
    operation_path: PathBuf,
    location: FilePathLocation,
}

impl AuthorizedFilePath {
    pub fn operation_path(&self) -> &Path {
        &self.operation_path
    }

    pub fn display_path(&self) -> String {
        self.operation_path.to_string_lossy().into_owned()
    }

    pub fn is_external(&self) -> bool {
        matches!(self.location, FilePathLocation::External)
    }

    pub fn external_parent(&self) -> PathBuf {
        self.operation_path
            .parent()
            .unwrap_or(self.operation_path.as_path())
            .to_path_buf()
    }

    /// Canonical scope derived from the same classification used for I/O.
    pub fn permission_scope_key(&self) -> String {
        match &self.location {
            FilePathLocation::Workspace { relative } if relative.is_empty() => {
                "workspace:/".to_string()
            }
            FilePathLocation::Workspace { relative } => format!("workspace:/{relative}"),
            FilePathLocation::External => {
                format!(
                    "fs:{}",
                    self.operation_path.to_string_lossy().replace('\\', "/")
                )
            }
        }
    }
}

/// Resolve an already-existing target. Symlinks are followed before the
/// workspace/external classification and the returned canonical path is what
/// callers must subsequently read, edit, delete, or overwrite.
pub fn resolve_existing_file_path(
    path: &Path,
    project_root: &Path,
) -> Result<AuthorizedFilePath, ToolError> {
    let root = canonical_root(project_root)?;
    let target = canonicalize_existing(path).map_err(|err| {
        ToolError::ExecutionError(format!("resolve file {}: {err}", path.display()))
    })?;
    classify(target.as_path(), &root)
}

/// Resolve a file target which may not exist yet. Its deepest existing parent
/// is canonicalized and any uncreated suffix remains part of the operation
/// path. This prevents a missing `workspace/new/file` from being classified
/// or opened as `workspace` itself.
pub fn resolve_create_file_path(
    path: &Path,
    project_root: &Path,
) -> Result<AuthorizedFilePath, ToolError> {
    let root = canonical_root(project_root)?;
    let target = resolve_create_target(path).map_err(|err| {
        ToolError::ExecutionError(format!("resolve create target {}: {err}", path.display()))
    })?;
    if let Some(existing) = target.existing {
        return classify(existing.as_path(), &root);
    }

    let operation_path = target.parent.as_path().join(target.suffix);
    classify_create_under_parent(&operation_path, target.parent.as_path(), &root)
}

fn canonical_root(project_root: &Path) -> Result<CanonicalPath, ToolError> {
    canonicalize_existing(project_root).map_err(|err| {
        ToolError::ExecutionError(format!(
            "resolve project root {}: {err}",
            project_root.display()
        ))
    })
}

fn classify(path: &Path, root: &CanonicalPath) -> Result<AuthorizedFilePath, ToolError> {
    let location = match assert_within_root(path, root.as_path()) {
        Ok(relative) => FilePathLocation::Workspace {
            relative: relative.as_str().replace('\\', "/"),
        },
        Err(_) => FilePathLocation::External,
    };
    Ok(AuthorizedFilePath {
        operation_path: path.to_path_buf(),
        location,
    })
}

fn classify_create_under_parent(
    operation_path: &Path,
    canonical_parent: &Path,
    root: &CanonicalPath,
) -> Result<AuthorizedFilePath, ToolError> {
    let location = match assert_within_root(canonical_parent, root.as_path()) {
        Ok(_) => {
            let relative = operation_path.strip_prefix(root.as_path()).map_err(|err| {
                ToolError::ExecutionError(format!("derive workspace path: {err}"))
            })?;
            FilePathLocation::Workspace {
                relative: relative.to_string_lossy().replace('\\', "/"),
            }
        }
        Err(_) => FilePathLocation::External,
    };
    Ok(AuthorizedFilePath {
        operation_path: operation_path.to_path_buf(),
        location,
    })
}
