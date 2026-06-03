use crate::schema::{AgentConfig, CommandConfig};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

static YAML_TOP_LEVEL_KV_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\s*:\s*(.*)$").unwrap());

/// Parse a markdown file as a command definition.
/// Extracts YAML frontmatter and body content.
pub(super) fn parse_markdown_command(
    path: &Path,
    base_dir: &Path,
) -> Option<(String, CommandConfig)> {
    let content = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&content);

    // Derive name from relative path
    let name = derive_name_from_path(path, base_dir, &["command", "commands"]);

    let mut config = if let Some(fm) = frontmatter {
        serde_json::from_value::<CommandConfig>(serde_yaml_frontmatter_to_json(&fm))
            .unwrap_or_default()
    } else {
        CommandConfig::default()
    };

    config.template = Some(body.trim().to_string());

    Some((name, config))
}

/// Parse a markdown file as an agent definition.
pub(super) fn parse_markdown_agent(path: &Path, base_dir: &Path) -> Option<(String, AgentConfig)> {
    let content = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&content);

    let name = derive_name_from_path(path, base_dir, &["agent", "agents", "mode", "modes"]);

    let mut config = if let Some(fm) = frontmatter {
        serde_json::from_value::<AgentConfig>(serde_yaml_frontmatter_to_json(&fm))
            .unwrap_or_default()
    } else {
        AgentConfig::default()
    };

    config.prompt = Some(body.trim().to_string());

    Some((name, config))
}

/// Split markdown content into optional YAML frontmatter and body.
pub(super) fn split_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let fm = after_first[..end_idx].trim().to_string();
        let body_start = end_idx + 4; // skip \n---
        let body = if body_start < after_first.len() {
            after_first[body_start..].to_string()
        } else {
            String::new()
        };
        (Some(fm), body)
    } else {
        (None, content.to_string())
    }
}

/// Fallback sanitization for invalid YAML frontmatter.
/// Matches TS `ConfigMarkdown.fallbackSanitization`: if a top-level value
/// contains a colon (which confuses simple YAML parsers), convert it to a
/// block scalar so the value is preserved verbatim.
pub(super) fn fallback_sanitize_yaml(yaml: &str) -> String {
    let mut result: Vec<String> = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim();
        // Pass through comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            result.push(line.to_string());
            continue;
        }
        // Pass through continuation/indented lines
        if line.starts_with(char::is_whitespace) {
            result.push(line.to_string());
            continue;
        }
        // Match top-level key: value
        let Some(caps) = YAML_TOP_LEVEL_KV_RE.captures(line) else {
            result.push(line.to_string());
            continue;
        };
        let key = &caps[1];
        let value = caps[2].trim();
        // Skip if value is empty, already quoted, or uses block scalar indicator
        if value.is_empty()
            || value == ">"
            || value == "|"
            || value == "|-"
            || value == ">-"
            || value.starts_with('"')
            || value.starts_with('\'')
        {
            result.push(line.to_string());
            continue;
        }
        // If value contains a colon, convert to block scalar
        if value.contains(':') {
            result.push(format!("{}: |-", key));
            result.push(format!("  {}", value));
            continue;
        }
        result.push(line.to_string());
    }
    result.join("\n")
}

/// Parse a YAML scalar value string into a JSON value.
fn yaml_scalar_to_json(value: &str) -> serde_json::Value {
    let value = value.trim();
    if value.is_empty() {
        return serde_json::Value::Null;
    }
    // Booleans
    if value == "true" || value == "True" || value == "TRUE" {
        return serde_json::Value::Bool(true);
    }
    if value == "false" || value == "False" || value == "FALSE" {
        return serde_json::Value::Bool(false);
    }
    if value == "null" || value == "Null" || value == "NULL" || value == "~" {
        return serde_json::Value::Null;
    }
    // Numbers
    if let Ok(n) = value.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(n) = value.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return serde_json::Value::Number(num);
        }
    }
    // Strip surrounding quotes
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        return serde_json::Value::String(value[1..value.len() - 1].to_string());
    }
    serde_json::Value::String(value.to_string())
}

/// Parse an inline YAML flow sequence like `[a, b, c]` into a JSON array.
fn parse_inline_list(value: &str) -> Option<serde_json::Value> {
    let trimmed = value.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Some(serde_json::Value::Array(Vec::new()));
    }
    let items: Vec<serde_json::Value> = inner
        .split(',')
        .map(|item| yaml_scalar_to_json(item.trim()))
        .collect();
    Some(serde_json::Value::Array(items))
}

/// Parse an inline YAML flow mapping like `{a: 1, b: true}` into a JSON object.
fn parse_inline_map(value: &str) -> Option<serde_json::Value> {
    let trimmed = value.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Some(serde_json::Value::Object(serde_json::Map::new()));
    }
    let mut map = serde_json::Map::new();
    for pair in inner.split(',') {
        if let Some((k, v)) = pair.split_once(':') {
            map.insert(k.trim().to_string(), yaml_scalar_to_json(v));
        }
    }
    Some(serde_json::Value::Object(map))
}

/// Compute the indentation level (number of leading spaces) of a line.
fn indent_level(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// YAML frontmatter to JSON conversion.
/// Handles: flat key-value, inline lists/maps, multi-line dash lists,
/// nested objects (indentation-based), and block scalars (| and >).
/// Falls back to sanitized re-parse on failure.
pub(super) fn serde_yaml_frontmatter_to_json(yaml: &str) -> serde_json::Value {
    match parse_yaml_mapping(yaml) {
        Some(value) => value,
        None => {
            // Fallback: sanitize and retry
            let sanitized = fallback_sanitize_yaml(yaml);
            parse_yaml_mapping(&sanitized)
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()))
        }
    }
}

/// Parse a YAML mapping (object) from a string. Returns None on structural failure.
fn parse_yaml_mapping(yaml: &str) -> Option<serde_json::Value> {
    let lines: Vec<&str> = yaml.lines().collect();
    let (map, _) = parse_yaml_mapping_lines(&lines, 0, 0)?;
    Some(serde_json::Value::Object(map))
}

/// Parse YAML mapping lines starting at `start` index with expected `base_indent`.
/// Returns the parsed map and the index of the next unconsumed line.
fn parse_yaml_mapping_lines(
    lines: &[&str],
    start: usize,
    base_indent: usize,
) -> Option<(serde_json::Map<String, serde_json::Value>, usize)> {
    let mut map = serde_json::Map::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let current_indent = indent_level(line);

        // If we've dedented past our base, we're done with this mapping
        if current_indent < base_indent {
            break;
        }

        // Handle list items at this level (shouldn't appear in a mapping context at same indent
        // unless it's a top-level list, which we don't support as root)
        if trimmed.starts_with("- ") && current_indent == base_indent {
            // This is a list at the mapping level -- not valid for our use case, skip
            break;
        }

        // Parse key: value
        if let Some((key_part, value_part)) = trimmed.split_once(':') {
            let key = key_part.trim().to_string();
            let value_str = value_part.trim();

            if value_str.is_empty() {
                // Value is on subsequent indented lines -- could be a nested map, list, or block scalar
                i += 1;
                if i < lines.len() {
                    let next_trimmed = lines[i].trim();
                    let next_indent = indent_level(lines[i]);
                    if next_indent > current_indent && next_trimmed.starts_with("- ") {
                        // It's a dash-list
                        let (list, next_i) = parse_yaml_list_lines(lines, i, next_indent);
                        map.insert(key, serde_json::Value::Array(list));
                        i = next_i;
                    } else if next_indent > current_indent {
                        // It's a nested mapping
                        if let Some((nested_map, next_i)) =
                            parse_yaml_mapping_lines(lines, i, next_indent)
                        {
                            map.insert(key, serde_json::Value::Object(nested_map));
                            i = next_i;
                        } else {
                            // Treat as string value from remaining indented lines
                            let (text, next_i) = collect_block_text(lines, i, current_indent);
                            map.insert(key, serde_json::Value::String(text));
                            i = next_i;
                        }
                    } else {
                        // Empty value, next line is at same or lower indent
                        map.insert(key, serde_json::Value::Null);
                    }
                } else {
                    map.insert(key, serde_json::Value::Null);
                }
            } else if value_str == "|" || value_str == "|-" || value_str == "|+" {
                // Block scalar (literal)
                i += 1;
                let (text, next_i) = collect_block_text(lines, i, current_indent);
                let text = if value_str == "|-" {
                    text.trim_end().to_string()
                } else {
                    text
                };
                map.insert(key, serde_json::Value::String(text));
                i = next_i;
            } else if value_str == ">" || value_str == ">-" || value_str == ">+" {
                // Block scalar (folded)
                i += 1;
                let (text, next_i) = collect_block_text(lines, i, current_indent);
                // Folded: join lines with spaces
                let folded = text.lines().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                let folded = if value_str == ">-" {
                    folded.trim_end().to_string()
                } else {
                    folded
                };
                map.insert(key, serde_json::Value::String(folded));
                i = next_i;
            } else if let Some(list) = parse_inline_list(value_str) {
                map.insert(key, list);
                i += 1;
            } else if let Some(obj) = parse_inline_map(value_str) {
                map.insert(key, obj);
                i += 1;
            } else {
                map.insert(key, yaml_scalar_to_json(value_str));
                i += 1;
            }
        } else {
            // Line doesn't match key: value pattern, skip
            i += 1;
        }
    }

    Some((map, i))
}

/// Parse YAML list lines (lines starting with "- ") at the given indent level.
/// Returns the list of values and the next unconsumed line index.
fn parse_yaml_list_lines(
    lines: &[&str],
    start: usize,
    base_indent: usize,
) -> (Vec<serde_json::Value>, usize) {
    let mut list = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        let current_indent = indent_level(line);
        if current_indent < base_indent {
            break;
        }
        if current_indent > base_indent {
            // Continuation of previous item, skip
            i += 1;
            continue;
        }

        if let Some(item_str) = trimmed.strip_prefix("- ") {
            let item_str = item_str.trim();
            // Check if item itself is a key: value (nested object in list)
            if item_str.contains(": ") {
                // Could be an inline object item like "- key: value"
                // For simplicity, treat as scalar string
                list.push(yaml_scalar_to_json(item_str));
            } else if let Some(inline_list) = parse_inline_list(item_str) {
                list.push(inline_list);
            } else {
                list.push(yaml_scalar_to_json(item_str));
            }
            i += 1;
        } else {
            break;
        }
    }

    (list, i)
}

/// Collect indented block text lines (for block scalars or multi-line values).
/// Stops when a line at or below `parent_indent` is encountered.
fn collect_block_text(lines: &[&str], start: usize, parent_indent: usize) -> (String, usize) {
    let mut text_lines = Vec::new();
    let mut i = start;
    let mut block_indent: Option<usize> = None;

    while i < lines.len() {
        let line = lines[i];
        // Empty lines are part of the block
        if line.trim().is_empty() {
            text_lines.push("");
            i += 1;
            continue;
        }
        let current_indent = indent_level(line);
        if current_indent <= parent_indent {
            break;
        }
        // Determine the block's base indent from the first non-empty line
        let bi = *block_indent.get_or_insert(current_indent);
        if current_indent >= bi {
            text_lines.push(&line[bi..]);
        } else {
            text_lines.push(line.trim());
        }
        i += 1;
    }

    // Trim trailing empty lines
    while text_lines.last() == Some(&"") {
        text_lines.pop();
    }

    (text_lines.join("\n"), i)
}

/// Derive a name from a file path relative to the base directory.
fn derive_name_from_path(path: &Path, base_dir: &Path, strip_prefixes: &[&str]) -> String {
    let rel = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    // Strip known directory prefixes
    let mut name = rel.as_str();
    for prefix in strip_prefixes {
        let with_sep = format!("{}/", prefix);
        if let Some(stripped) = name.strip_prefix(&with_sep) {
            name = stripped;
            break;
        }
        let with_rocode = format!(".agendao/{}/", prefix);
        if let Some(stripped) = name.strip_prefix(&with_rocode) {
            name = stripped;
            break;
        }
    }

    // Remove .md extension
    name.strip_suffix(".md").unwrap_or(name).to_string()
}
