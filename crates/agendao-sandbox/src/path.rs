//! Canonical path authority for the sandbox domain.
//!
//! Layered semantics:
//!
//! 1. `resolve_user_path` turns raw tool input into a lexically
//!    normalized absolute path, rejecting `workspace:`-scheme escapes
//!    (`workspace:/../x` is always invalid).
//! 2. `canonicalize_existing` / `resolve_create_target` resolve real
//!    filesystem state (symlinks followed).
//! 3. `assert_within_root` performs containment only on canonical paths,
//!    using component-wise `Path::starts_with` — never string prefixes —
//!    so `/workspace2` is not a subtree of `/workspace` and symlink
//!    targets cannot slip an escape between check and use.

use std::path::{Component, Path, PathBuf};

/// Metadata directories that stay read-only even inside writable roots.
pub const PROTECTED_METADATA_COMPONENTS: [&str; 4] = [".git", ".agendao", ".agents", ".codex"];

#[derive(Debug, thiserror::Error)]
pub enum PathViolation {
    #[error("path escapes its base directory: {raw}")]
    LexicalEscape { raw: String },
    #[error("canonical path {path} escapes root {root}")]
    CanonicalEscape { root: PathBuf, path: PathBuf },
    #[error("symlink resolves outside the allowed root: {path} -> {target}")]
    SymlinkEscape { path: PathBuf, target: PathBuf },
    #[error("invalid path input: {0}")]
    InvalidInput(String),
    #[error("io error resolving {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Lexically normalized absolute path; no filesystem state was consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPath(PathBuf);

impl UserPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Canonical path of an existing filesystem object (symlinks resolved).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalPath(pub(crate) PathBuf);

impl CanonicalPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Path relative to (and verified within) a root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativePath(String);

impl RelativePath {
    pub fn new() -> Self {
        Self(String::new())
    }
}

impl Default for RelativePath {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for RelativePath {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl RelativePath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// First-level scope key inside a workspace (used by permission scoping
/// and fingerprint stability).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeKey(String);

impl ScopeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A write/create target resolved against real filesystem state: the
/// parent directory is canonicalized (it must exist) and the final
/// component is validated; an already-existing symlink target is
/// resolved so containment checks see the real write destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTarget {
    /// Deepest existing ancestor, canonicalized with every symlink resolved.
    pub parent: CanonicalPath,
    /// Lexically validated path components below `parent`. This preserves
    /// uncreated directory suffixes instead of collapsing a target such as
    /// `workspace/new/nested/file` back to `workspace`.
    pub suffix: PathBuf,
    pub component: std::ffi::OsString,
    /// Canonical path of the existing entry, if any (symlinks followed).
    pub existing: Option<CanonicalPath>,
}

/// A protected metadata directory detected on a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPath {
    pub component: &'static str,
}

/// Fold `.` and `..` components lexically. `..` at a filesystem root is
/// absorbed (POSIX semantics). The result is absolute when `path` is.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

/// Resolve raw user input against a base directory.
///
/// Accepted forms: `workspace:<relative>` (must stay inside `base`),
/// plain relative paths (joined onto `base`, must stay inside), and
/// absolute paths (passed through; containment is decided later by
/// `assert_within_root` on canonical paths). Tilde is not expanded —
/// shell-style semantics belong to callers, containment belongs here.
pub fn resolve_user_path(raw: &str, base: &Path) -> Result<UserPath, PathViolation> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(PathViolation::InvalidInput("empty path".to_string()));
    }
    if raw.contains('\0') {
        return Err(PathViolation::InvalidInput("NUL byte in path".to_string()));
    }
    if raw.starts_with('~') {
        return Err(PathViolation::InvalidInput(format!(
            "tilde paths are not resolved by the sandbox path authority: {raw}"
        )));
    }

    let (joined, enforce_base) = if let Some(relative) = raw.strip_prefix("workspace:") {
        (base.join(relative), true)
    } else if Path::new(raw).is_absolute() {
        (PathBuf::from(raw), false)
    } else {
        (base.join(raw), true)
    };

    let normalized = lexical_normalize(&joined);
    if enforce_base && !normalized.starts_with(base) {
        return Err(PathViolation::LexicalEscape {
            raw: raw.to_string(),
        });
    }
    Ok(UserPath(normalized))
}

/// Canonicalize a path that must exist on disk.
pub fn canonicalize_existing(path: &Path) -> Result<CanonicalPath, PathViolation> {
    std::fs::canonicalize(path)
        .map(CanonicalPath)
        .map_err(|source| PathViolation::Io {
            path: path.to_path_buf(),
            source,
        })
}

/// Resolve a create/write target: canonicalize its deepest existing
/// ancestor and retain every not-yet-created suffix component. If the target
/// already exists and is a symlink, its destination is canonicalized into
/// `existing` so callers can reject out-of-root writes before any file is
/// touched.
pub fn resolve_create_target(path: &Path) -> Result<CreateTarget, PathViolation> {
    let component = path.file_name().ok_or_else(|| {
        PathViolation::InvalidInput(format!("path has no final component: {path:?}"))
    })?;
    let component_text = component.to_string_lossy();
    if component_text == "." || component_text == ".." {
        return Err(PathViolation::InvalidInput(format!(
            "final path component must be a real name: {path:?}"
        )));
    }

    let mut current = path.to_path_buf();
    let mut suffix_components = Vec::new();
    while !current.exists() {
        let name = current.file_name().ok_or_else(|| {
            PathViolation::InvalidInput(format!("path has no existing ancestor: {path:?}"))
        })?;
        let text = name.to_string_lossy();
        if text == "." || text == ".." {
            return Err(PathViolation::InvalidInput(format!(
                "path component must be a real name: {path:?}"
            )));
        }
        suffix_components.push(name.to_os_string());
        current = current
            .parent()
            .ok_or_else(|| {
                PathViolation::InvalidInput(format!("path has no existing ancestor: {path:?}"))
            })?
            .to_path_buf();
    }

    // Existing entries are operated through their canonical destination;
    // otherwise the deepest existing directory is the secure parent and
    // every remaining suffix component is retained verbatim.
    let (parent, suffix) = if suffix_components.is_empty() {
        let parent = path
            .parent()
            .ok_or_else(|| PathViolation::InvalidInput(format!("path has no parent: {path:?}")))?;
        (parent.to_path_buf(), PathBuf::from(component))
    } else {
        suffix_components.reverse();
        let mut suffix = PathBuf::new();
        for component in suffix_components {
            suffix.push(component);
        }
        (current, suffix)
    };
    let parent_canon = canonicalize_existing(&parent)?;
    let absolute = parent_canon.as_path().join(&suffix);
    let existing = if absolute.symlink_metadata().is_ok() {
        Some(canonicalize_existing(&absolute)?)
    } else {
        None
    };
    Ok(CreateTarget {
        parent: parent_canon,
        suffix,
        component: component.to_os_string(),
        existing,
    })
}

/// Component-wise containment check on canonical paths. This is the only
/// containment authority; it must never be replaced by string prefixes.
pub fn assert_within_root(path: &Path, root: &Path) -> Result<RelativePath, PathViolation> {
    if path.starts_with(root) && path != root {
        let relative = path
            .strip_prefix(root)
            .expect("starts_with succeeded; strip_prefix cannot fail");
        Ok(RelativePath(relative.to_string_lossy().into_owned()))
    } else if path == root {
        Ok(RelativePath(String::new()))
    } else {
        Err(PathViolation::CanonicalEscape {
            root: root.to_path_buf(),
            path: path.to_path_buf(),
        })
    }
}

/// Check that a possibly-symlinked path stays inside `root` after
/// canonicalization. Symlinks pointing outside the root are a violation
/// even when the link itself lives inside the workspace.
pub fn assert_no_symlink_escape(path: &Path, root: &Path) -> Result<(), PathViolation> {
    let canonical = canonicalize_existing(path)?;
    if canonical.as_path().starts_with(root) {
        Ok(())
    } else {
        Err(PathViolation::SymlinkEscape {
            path: path.to_path_buf(),
            target: canonical.as_path().to_path_buf(),
        })
    }
}

/// Derive the workspace scope key from a verified-relative path.
pub fn workspace_scope(relative: &RelativePath) -> ScopeKey {
    let first = relative
        .as_str()
        .split(std::path::MAIN_SEPARATOR)
        .find(|segment| !segment.is_empty())
        .unwrap_or(".");
    ScopeKey(first.to_string())
}

/// Detect protected metadata directories (`.git`, `.agendao`, ...)
/// anywhere on a path.
pub fn protected_metadata(path: &Path) -> Option<ProtectedPath> {
    for component in path.components() {
        if let Component::Normal(name) = component {
            if let Some(text) = name.to_str() {
                if let Some(found) = PROTECTED_METADATA_COMPONENTS
                    .iter()
                    .find(|candidate| **candidate == text)
                {
                    return Some(ProtectedPath { component: found });
                }
            }
        }
    }
    None
}
