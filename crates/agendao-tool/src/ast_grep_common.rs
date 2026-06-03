use ast_grep_core::Pattern as AstPattern;
use ast_grep_language::SupportLang;
use std::path::{Component, Path, PathBuf};
use walkdir::DirEntry;

use crate::{ToolContext, ToolError};

pub(crate) const SUPPORTED_LANGUAGES: &[&str] = &["rust"];
pub(crate) const DEFAULT_GLOB: &str = "**/*.rs";
pub(crate) const DEFAULT_MAX_RESULTS: usize = 100;
pub(crate) const MAX_RESULTS_LIMIT: usize = 500;
pub(crate) const MAX_CONTEXT_LINES: usize = 20;
pub(crate) const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "vendor",
    "node_modules",
    "dist",
    "build",
    "coverage",
    ".venv",
    "venv",
    "env",
    "__pycache__",
    ".idea",
    ".vscode",
    ".cache",
    "cache",
    "tmp",
    "temp",
    "logs",
];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AstGrepLanguage {
    Rust,
}

impl AstGrepLanguage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
        }
    }
}

pub(crate) fn compile_pattern(
    pattern: &str,
    language: &AstGrepLanguage,
) -> Result<AstPattern, ToolError> {
    match language {
        AstGrepLanguage::Rust => compile_rust_pattern(pattern),
    }
}

fn compile_rust_pattern(pattern: &str) -> Result<AstPattern, ToolError> {
    AstPattern::try_new(pattern.trim(), SupportLang::Rust)
        .map_err(|e| ToolError::InvalidArguments(format!("Invalid Rust ast-grep pattern: {}", e)))
}

pub(crate) fn count_placeholders(pattern: &str) -> usize {
    let bytes = pattern.as_bytes();
    let mut count = 0;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            let next = bytes[i + 1] as char;
            if next == '_' || next.is_ascii_alphabetic() {
                count += 1;
                i += 2;
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    if ch == '_' || ch.is_ascii_alphanumeric() {
                        i += 1;
                    } else {
                        break;
                    }
                }
                continue;
            }
        }
        i += 1;
    }

    count
}

pub(crate) fn resolve_directory_root(
    ctx: &ToolContext,
    requested: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let path = resolve_any_path(ctx, requested)?;
    if !path.is_dir() {
        return Err(ToolError::InvalidArguments(format!(
            "Search path must be a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

pub(crate) fn resolve_any_path(
    ctx: &ToolContext,
    requested: Option<&str>,
) -> Result<PathBuf, ToolError> {
    let requested = requested.unwrap_or(".");
    if requested.trim() == "/" {
        return Err(ToolError::InvalidArguments(
            "Refusing to operate on filesystem root '/'. Use '.' or a project-relative path instead."
                .to_string(),
        ));
    }

    let mut path = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        PathBuf::from(&ctx.directory).join(requested)
    };

    if let Ok(canonical) = path.canonicalize() {
        path = canonical;
    }

    if !path.exists() {
        return Err(ToolError::FileNotFound(path.display().to_string()));
    }

    Ok(path)
}

pub(crate) fn should_visit(entry: &DirEntry, base_dir: &Path) -> bool {
    if entry.depth() == 0 {
        return true;
    }

    let rel = match entry.path().strip_prefix(base_dir) {
        Ok(rel) => rel,
        Err(_) => return true,
    };

    !rel.components().any(is_ignored_component)
}

fn is_ignored_component(component: Component<'_>) -> bool {
    let Component::Normal(name) = component else {
        return false;
    };
    let Some(name) = name.to_str() else {
        return false;
    };
    DEFAULT_IGNORED_DIRS.contains(&name)
}

pub(crate) fn display_path(base_dir: &Path, path: &Path) -> String {
    path.strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}
