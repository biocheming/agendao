use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub path: String,
    pub line_number: usize,
    pub lines: String,
    pub absolute_offset: usize,
    pub submatches: Vec<SubMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubMatch {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub elapsed: String,
    pub searches: usize,
    pub bytes_searched: usize,
    pub matched_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchOptions {
    pub glob: Vec<String>,
    pub hidden: bool,
    pub follow: bool,
    pub max_depth: Option<usize>,
}

impl Default for FileSearchOptions {
    fn default() -> Self {
        Self {
            glob: vec![],
            hidden: true,
            follow: false,
            max_depth: None,
        }
    }
}

pub struct Ripgrep;

impl Ripgrep {
    pub fn search<P: AsRef<Path>>(
        path: P,
        pattern: &str,
    ) -> Result<Vec<MatchResult>, Box<dyn std::error::Error>> {
        Self::search_with_limit(path, pattern, usize::MAX)
    }

    pub fn search_with_limit<P: AsRef<Path>>(
        path: P,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<MatchResult>, Box<dyn std::error::Error>> {
        let regex = regex::Regex::new(pattern)?;
        let path = path.as_ref();
        let mut matches = Vec::new();

        if path.is_file() {
            search_file(path, &regex, &mut matches, limit)?;
        } else if path.is_dir() {
            let walker = WalkDir::new(path)
                .into_iter()
                .filter_entry(|entry| !is_git_entry(entry));
            for entry in walker.filter_map(Result::ok) {
                if matches.len() >= limit {
                    break;
                }
                if entry.file_type().is_file() {
                    let _ = search_file(entry.path(), &regex, &mut matches, limit);
                }
            }
        }

        Ok(matches)
    }

    pub fn files<P: AsRef<Path>>(
        path: P,
        options: FileSearchOptions,
    ) -> Result<Vec<PathBuf>, io::Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No such directory: '{}'", path.display()),
            ));
        }

        let matcher = GlobMatcher::new(&options.glob)?;
        let mut result = Vec::new();
        let mut walk = WalkDir::new(path).follow_links(options.follow);

        if let Some(depth) = options.max_depth {
            walk = walk.max_depth(depth);
        }

        for entry in walk
            .into_iter()
            .filter_entry(|entry| {
                !is_git_entry(entry)
                    && (options.hidden || entry.depth() == 0 || !is_hidden_entry(entry))
            })
            .filter_map(Result::ok)
        {
            let entry_path = entry.path();

            if !entry.file_type().is_file() {
                continue;
            }

            if !matcher.matches(entry_path) {
                continue;
            }

            result.push(entry_path.to_path_buf());
        }

        Ok(result)
    }

    pub fn tree<P: AsRef<Path>>(path: P, limit: Option<usize>) -> Result<String, io::Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("No such directory: '{}'", path.display()),
            ));
        }

        // Traverse breadth-first and stop as soon as the display budget is
        // exceeded. This intentionally reports only that the tree is
        // truncated: calculating the exact remaining count would require the
        // full walk and defeat the limit.
        let mut directories = VecDeque::from([path.to_path_buf()]);
        let mut lines = Vec::new();
        let mut truncated = false;

        'walk: while let Some(directory) = directories.pop_front() {
            let mut entries = match fs::read_dir(&directory) {
                Ok(entries) => entries.filter_map(Result::ok).collect::<Vec<_>>(),
                Err(_) => continue,
            };
            entries.sort_by_key(|entry| entry.file_name());

            for entry in entries {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() || is_ignored_tree_directory(&entry) {
                    continue;
                }
                if limit.is_some_and(|max| lines.len() >= max) {
                    truncated = true;
                    break 'walk;
                }

                let entry_path = entry.path();
                let rel_path = entry_path.strip_prefix(path).unwrap_or(&entry_path);
                lines.push(
                    rel_path
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/"),
                );
                directories.push_back(entry_path);
            }
        }

        if truncated {
            lines.push("[truncated]".to_string());
        }
        Ok(lines.join("\n"))
    }
}

fn search_file(
    path: &Path,
    regex: &regex::Regex,
    matches: &mut Vec<MatchResult>,
    limit: usize,
) -> Result<(), io::Error> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let path_str = path.to_string_lossy().to_string();
    let mut offset = 0;

    for (line_num, line_result) in reader.lines().enumerate() {
        if matches.len() >= limit {
            break;
        }

        let line = line_result?;
        let line_len = line.len() + 1;

        if regex.is_match(&line) {
            let mut submatches = Vec::new();
            for cap in regex.find_iter(&line) {
                submatches.push(SubMatch {
                    text: cap.as_str().to_string(),
                    start: cap.start(),
                    end: cap.end(),
                });
            }

            matches.push(MatchResult {
                path: path_str.clone(),
                line_number: line_num + 1,
                lines: line,
                absolute_offset: offset,
                submatches,
            });
        }

        offset += line_len;
    }

    Ok(())
}

fn is_git_entry(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0 && entry.file_type().is_dir() && entry.file_name() == ".git"
}

fn is_ignored_tree_directory(entry: &fs::DirEntry) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    name == ".git" || name.contains(".opencode")
}

fn is_hidden_entry(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_string_lossy().starts_with('.')
}

struct GlobMatcher {
    includes: Vec<regex::Regex>,
    excludes: Vec<regex::Regex>,
}

impl GlobMatcher {
    fn new(patterns: &[String]) -> Result<Self, io::Error> {
        let mut includes = Vec::new();
        let mut excludes = Vec::new();
        for raw in patterns {
            let (target, pattern) = if let Some(pattern) = raw.strip_prefix('!') {
                (&mut excludes, pattern)
            } else {
                (&mut includes, raw.as_str())
            };
            target.push(
                glob_to_regex(pattern).map_err(|error| {
                    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
                })?,
            );
        }
        Ok(Self { includes, excludes })
    }

    fn matches(&self, path: &Path) -> bool {
        let path = path.to_string_lossy();
        !self.excludes.iter().any(|pattern| pattern.is_match(&path))
            && (self.includes.is_empty()
                || self.includes.iter().any(|pattern| pattern.is_match(&path)))
    }
}

fn glob_to_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let contains_wildcard = pattern.contains('*') || pattern.contains('?');
    let mut regex = String::from("^.*");
    for character in pattern.trim_start_matches("./").chars() {
        match character {
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            character => regex.push_str(&regex::escape(&character.to_string())),
        }
    }
    if !contains_wildcard {
        regex.push_str(".*");
    }
    regex.push('$');
    regex::Regex::new(&regex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search() {
        let result = Ripgrep::search(".", "fn main").unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_search_with_limit() {
        let result = Ripgrep::search_with_limit(".", "fn", 5).unwrap();
        assert!(result.len() <= 5);
    }

    #[test]
    fn test_files() {
        let result = Ripgrep::files(".", FileSearchOptions::default()).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_tree() {
        let result = Ripgrep::tree(".", Some(10)).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn tree_stops_at_the_display_limit_without_counting_the_remainder() {
        let root = std::env::temp_dir().join(format!(
            "agendao-grep-tree-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::create_dir_all(root.join("alpha/child")).unwrap();
        fs::create_dir_all(root.join("beta")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();

        let result = Ripgrep::tree(&root, Some(2)).unwrap();
        assert_eq!(
            result.lines().collect::<Vec<_>>(),
            ["alpha", "beta", "[truncated]"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn glob_matcher_applies_includes_and_excludes_once() {
        let matcher = GlobMatcher::new(&["*.rs".into(), "!generated.rs".into()]).unwrap();
        assert!(matcher.matches(Path::new("src/main.rs")));
        assert!(!matcher.matches(Path::new("src/generated.rs")));
        assert!(!matcher.matches(Path::new("src/main.ts")));
    }
}
