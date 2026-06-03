use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootPathFallbackPolicy {
    /// Do not correct suspicious root-level paths.
    Disabled,
    /// Correct only when the fallback exists in session directory.
    ExistingFallbackOnly,
    /// Correct whenever root-level target is missing.
    PreferSessionDirWhenMissing,
}

#[derive(Debug, Clone)]
pub struct ResolvedPath {
    pub resolved: PathBuf,
    pub corrected_from: Option<PathBuf>,
}

pub fn resolve_user_path(
    raw_path: &str,
    base_dir: &Path,
    fallback_policy: RootPathFallbackPolicy,
) -> ResolvedPath {
    let candidate = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        base_dir.join(raw_path)
    };

    if fallback_policy == RootPathFallbackPolicy::Disabled {
        return ResolvedPath {
            resolved: candidate,
            corrected_from: None,
        };
    }

    let Some(basename) = root_level_basename(Path::new(raw_path)) else {
        return ResolvedPath {
            resolved: candidate,
            corrected_from: None,
        };
    };

    let fallback = base_dir.join(basename);
    if fallback == candidate {
        return ResolvedPath {
            resolved: candidate,
            corrected_from: None,
        };
    }

    let should_correct = match fallback_policy {
        RootPathFallbackPolicy::Disabled => false,
        RootPathFallbackPolicy::ExistingFallbackOnly => !candidate.exists() && fallback.exists(),
        RootPathFallbackPolicy::PreferSessionDirWhenMissing => !candidate.exists(),
    };

    if should_correct {
        ResolvedPath {
            resolved: fallback,
            corrected_from: Some(candidate),
        }
    } else {
        ResolvedPath {
            resolved: candidate,
            corrected_from: None,
        }
    }
}

fn root_level_basename(path: &Path) -> Option<&std::ffi::OsStr> {
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::RootDir), Some(Component::Normal(name)), None) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fallback_for_root_single_segment_when_present_in_session_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();
        let fallback = base.join("t2.html");
        fs::write(&fallback, "<html/>").expect("write");

        let resolved = resolve_user_path(
            "/t2.html",
            base,
            RootPathFallbackPolicy::ExistingFallbackOnly,
        );

        assert_eq!(resolved.resolved, fallback);
        assert_eq!(resolved.corrected_from, Some(PathBuf::from("/t2.html")));
    }

    #[test]
    fn does_not_fallback_for_non_root_single_segment() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();

        let resolved = resolve_user_path(
            "/tmp/t2.html",
            base,
            RootPathFallbackPolicy::ExistingFallbackOnly,
        );

        assert_eq!(resolved.resolved, PathBuf::from("/tmp/t2.html"));
        assert!(resolved.corrected_from.is_none());
    }

    #[test]
    fn write_policy_prefers_session_dir_for_missing_root_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();

        let resolved = resolve_user_path(
            "/new-file.html",
            base,
            RootPathFallbackPolicy::PreferSessionDirWhenMissing,
        );

        assert_eq!(resolved.resolved, base.join("new-file.html"));
        assert_eq!(
            resolved.corrected_from,
            Some(PathBuf::from("/new-file.html"))
        );
    }
}
