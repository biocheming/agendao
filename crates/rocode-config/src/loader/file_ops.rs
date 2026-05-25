use crate::Config;
use anyhow::{Context, Result};
use jsonc_parser::{parse_to_serde_value, ParseOptions};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn get_global_config_paths() -> Vec<PathBuf> {
    let config_dir = if cfg!(target_os = "macos") {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"))
    } else if cfg!(target_os = "windows") {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("%APPDATA%"))
    } else {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"))
    };

    let rocode_dir = config_dir.join("rocode");
    vec![
        rocode_dir.join("rocode.jsonc"),
        rocode_dir.join("rocode.json"),
    ]
}

/// Migrate legacy global TOML config (`~/.config/rocode/config`) into
/// `rocode.json` and merge it into the currently loaded config.
pub(super) fn migrate_legacy_toml_config(
    config_dir: &Path,
    config: &mut Config,
) -> Option<PathBuf> {
    let legacy_path = config_dir.join("config");
    if !legacy_path.exists() {
        return None;
    }

    let content = match fs::read_to_string(&legacy_path) {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to read legacy TOML config"
            );
            return None;
        }
    };

    let legacy_toml: toml::Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to parse legacy TOML config"
            );
            return None;
        }
    };
    let mut legacy_json = match serde_json::to_value(legacy_toml) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to convert legacy TOML config to JSON value"
            );
            return None;
        }
    };

    let mut migrated = Config::default();
    if let Some(table) = legacy_json.as_object_mut() {
        let provider = table
            .remove("provider")
            .and_then(|value| value.as_str().map(str::to_owned));
        let model = table
            .remove("model")
            .and_then(|value| value.as_str().map(str::to_owned));
        if let (Some(provider), Some(model)) = (provider, model) {
            migrated.model = Some(format!("{provider}/{model}"));
        }
    }

    match serde_json::from_value::<Config>(legacy_json) {
        Ok(rest) => migrated.merge(rest),
        Err(error) => {
            tracing::warn!(
                path = %legacy_path.display(),
                %error,
                "failed to deserialize legacy TOML config into schema"
            );
        }
    }

    if migrated.schema.is_none() {
        migrated.schema = Some("https://opencode.ai/config.json".to_string()); // there is no rocode.ai domain name
    }
    config.merge(migrated);

    let json_path = config_dir.join("rocode.json");
    if let Some(parent) = json_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            tracing::warn!(
                path = %parent.display(),
                %error,
                "failed to create config directory during TOML migration"
            );
            return None;
        }
    }

    let serialized = match serde_json::to_string_pretty(config) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!(
                path = %json_path.display(),
                %error,
                "failed to serialize migrated config"
            );
            return None;
        }
    };

    if let Err(error) = fs::write(&json_path, serialized) {
        tracing::warn!(
            path = %json_path.display(),
            %error,
            "failed to write migrated JSON config"
        );
        return None;
    }

    if let Err(error) = fs::remove_file(&legacy_path) {
        tracing::warn!(
            path = %legacy_path.display(),
            %error,
            "failed to remove legacy TOML config after migration"
        );
    }

    tracing::info!(
        legacy = %legacy_path.display(),
        migrated = %json_path.display(),
        "migrated legacy TOML config"
    );

    Some(json_path)
}

/// Substitute `{env:VAR}` patterns with environment variable values.
/// Works on the raw JSONC text before parsing.
pub(super) fn substitute_env_vars(text: &str) -> String {
    let re = regex::Regex::new(r"\{env:([^}]+)\}").unwrap();
    re.replace_all(text, |caps: &regex::Captures| {
        let var_name = &caps[1];
        std::env::var(var_name).unwrap_or_default()
    })
    .to_string()
}

/// Resolve `{file:path}` patterns by reading file contents.
/// Skips patterns on commented lines. Resolves relative paths from `base_dir`.
pub(super) fn resolve_file_references(text: &str, base_dir: &Path) -> Result<String> {
    let re = regex::Regex::new(r"\{file:([^}]+)\}").unwrap();

    let mut result = text.to_string();

    // Collect all matches first to avoid borrow issues
    let matches: Vec<(String, String)> = re
        .captures_iter(text)
        .map(|caps| {
            let full_match = caps.get(0).unwrap().as_str().to_string();
            let file_path_str = caps[1].to_string();
            (full_match, file_path_str)
        })
        .collect();

    for (full_match, file_path_str) in matches {
        // Check if the match is on a commented line
        let match_start = match text.find(&full_match) {
            Some(pos) => pos,
            None => continue,
        };
        let line_start = text[..match_start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line_end = text[line_start..]
            .find('\n')
            .map(|p| line_start + p)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        if line.trim().starts_with("//") {
            continue;
        }

        // Resolve the file path
        let resolved = if let Some(stripped) = file_path_str.strip_prefix("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(stripped)
        } else if Path::new(&file_path_str).is_absolute() {
            PathBuf::from(&file_path_str)
        } else {
            base_dir.join(&file_path_str)
        };

        // Read the file
        let content = fs::read_to_string(&resolved).with_context(|| {
            format!(
                "bad file reference: \"{}\" - {} does not exist",
                full_match,
                resolved.display()
            )
        })?;
        let content = content.trim();

        // Escape for JSON string context (newlines, quotes)
        let escaped = content
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");

        result = result.replace(&full_match, &escaped);
    }

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
