use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
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
        let files = Self::files(path, FileSearchOptions::default())?;

        let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();

        for file in &files {
            let rel_path = file.strip_prefix(path).unwrap_or(file);
            let rel_str = rel_path.to_string_lossy();

            if rel_str.contains(".opencode") {
                continue;
            }

            let parts: Vec<&str> = rel_str.split(std::path::MAIN_SEPARATOR).collect();
            if parts.len() < 2 {
                continue;
            }

            let mut current = &mut root;
            for part in parts.iter().take(parts.len() - 1) {
                let node = current.entry(part.to_string()).or_insert(TreeNode {
                    name: part.to_string(),
                    children: BTreeMap::new(),
                });
                current = &mut node.children;
            }
        }

        let total = count_nodes(&root);
        let limit = limit.unwrap_or(total);
        let mut lines: Vec<String> = Vec::new();
        let mut queue: VecDeque<(String, &TreeNode)> = VecDeque::new();

        for node in root.values() {
            queue.push_back((node.name.clone(), node));
        }

        let mut used = 0;
        while let Some((path_str, node)) = queue.pop_front() {
            if used >= limit {
                break;
            }
            lines.push(path_str.clone());
            used += 1;

            queue.extend(
                node.children
                    .values()
                    .map(|child| (format!("{}/{}", path_str, child.name), child)),
            );
        }

        if total > used {
            lines.push(format!("[{} truncated]", total - used));
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

#[derive(Debug, Clone)]
struct TreeNode {
    name: String,
    children: BTreeMap<String, TreeNode>,
}

fn count_nodes(node: &BTreeMap<String, TreeNode>) -> usize {
    let mut total = 0;
    for child in node.values() {
        total += 1 + count_nodes(&child.children);
    }
    total
}

fn is_git_entry(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0 && entry.file_type().is_dir() && entry.file_name() == ".git"
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
    fn glob_matcher_applies_includes_and_excludes_once() {
        let matcher = GlobMatcher::new(&["*.rs".into(), "!generated.rs".into()]).unwrap();
        assert!(matcher.matches(Path::new("src/main.rs")));
        assert!(!matcher.matches(Path::new("src/generated.rs")));
        assert!(!matcher.matches(Path::new("src/main.ts")));
    }
}
