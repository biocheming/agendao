use crate::{Config, ExternalToolCatalogFile};
use anyhow::{Context, Result};
use jsonc_parser::{parse_to_serde_value, ParseOptions};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static ENV_REFERENCE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{env:([^}]+)\}").unwrap());
static FILE_REFERENCE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{file:([^}]+)\}").unwrap());

pub(super) fn get_global_config_paths() -> Vec<PathBuf> {
    // 全局配置文件统一收在 agendao_home（~/.agendao,土律·单点权威）。
    let agendao_dir = agendao_util::agendao_home();
    vec![
        agendao_dir.join("agendao.jsonc"),
        agendao_dir.join("agendao.json"),
    ]
}

/// Substitute `{env:VAR}` patterns with environment variable values.
/// Works on the raw JSONC text before parsing.
pub(super) fn substitute_env_vars(text: &str) -> String {
    ENV_REFERENCE_RE
        .replace_all(text, |caps: &regex::Captures| {
            let var_name = &caps[1];
            std::env::var(var_name).unwrap_or_default()
        })
        .to_string()
}

/// Resolve `{file:path}` patterns by reading file contents.
/// Skips patterns on commented lines. Resolves relative paths from `base_dir`.
pub(super) fn resolve_file_references(text: &str, base_dir: &Path) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;
    let mut resolved_contents = HashMap::<PathBuf, String>::new();

    for captures in FILE_REFERENCE_RE.captures_iter(text) {
        let full_match = captures.get(0).expect("full regex match must exist");
        let file_path_str = &captures[1];
        result.push_str(&text[last_end..full_match.start()]);
        last_end = full_match.end();

        // Check if the match is on a commented line
        let line_start = text[..full_match.start()]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let line_end = text[line_start..]
            .find('\n')
            .map(|p| line_start + p)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        if line.trim().starts_with("//") {
            result.push_str(full_match.as_str());
            continue;
        }

        // Resolve the file path
        let resolved = if let Some(stripped) = file_path_str.strip_prefix("~/") {
            // 展开配置文本里用户手写的 `~/` 文件引用，要的是真实用户主目录，不经 agendao_home。
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(stripped)
        } else if Path::new(file_path_str).is_absolute() {
            PathBuf::from(file_path_str)
        } else {
            base_dir.join(file_path_str)
        };

        let escaped = match resolved_contents.entry(resolved.clone()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let content = fs::read_to_string(&resolved).with_context(|| {
                    format!(
                        "bad file reference: \"{}\" - {} does not exist",
                        full_match.as_str(),
                        resolved.display()
                    )
                })?;
                entry.insert(
                    content
                        .trim()
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r")
                        .replace('\t', "\\t"),
                )
            }
        };
        result.push_str(escaped);
    }

    result.push_str(&text[last_end..]);

    Ok(result)
}

pub(super) fn parse_jsonc(content: &str) -> Result<Config> {
    let parse_options = ParseOptions {
        allow_trailing_commas: true,
        ..Default::default()
    };
    let parsed = parse_to_serde_value(content, &parse_options)
        .with_context(|| "Failed to parse JSONC")?
        .context("Config content is empty")?;
    serde_json::from_value(parsed).with_context(|| "Failed to parse config JSON")
}

pub(super) fn parse_external_tool_catalog_jsonc(content: &str) -> Result<ExternalToolCatalogFile> {
    let value = parse_to_serde_value(content, &ParseOptions::default())
        .map_err(|error| anyhow::anyhow!("JSONC parse error: {error:?}"))?;
    let value = value.unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
    serde_json::from_value(value).with_context(|| "Failed to deserialize external tool catalog")
}
