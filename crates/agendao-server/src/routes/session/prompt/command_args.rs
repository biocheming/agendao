use std::collections::HashMap;

use agendao_command::{CommandArgumentField, CommandArgumentKind};

use crate::Result;

pub(super) fn normalize_field_key(key: &str) -> String {
    key.trim()
        .trim_start_matches('-')
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escape = false;

    for ch in raw.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            _ if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '*' | ':'))
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn parse(
    raw_arguments: Option<&str>,
    fields: &[CommandArgumentField],
) -> HashMap<String, Vec<String>> {
    let mut values = HashMap::new();
    let Some(raw_arguments) = raw_arguments.filter(|value| !value.trim().is_empty()) else {
        return values;
    };
    let field_map = fields
        .iter()
        .map(|field| (normalize_field_key(&field.key), field))
        .collect::<HashMap<_, _>>();
    let tokens = tokenize(raw_arguments);
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if !token.starts_with("--") {
            index += 1;
            continue;
        }
        let key = normalize_field_key(token.trim_start_matches("--"));
        let Some(field) = field_map.get(&key) else {
            index += 1;
            continue;
        };
        let mut captured = Vec::new();
        let mut cursor = index + 1;
        while cursor < tokens.len() && !tokens[cursor].starts_with("--") {
            captured.push(tokens[cursor].clone());
            cursor += 1;
            if !field.repeatable && !matches!(field.kind, CommandArgumentKind::GlobList) {
                break;
            }
        }
        if matches!(field.kind, CommandArgumentKind::Boolean) && captured.is_empty() {
            captured.push("true".to_string());
        }
        if !captured.is_empty() {
            values.entry(key).or_default().extend(captured);
        }
        index = cursor.max(index + 1);
    }
    values
}

pub(super) fn flatten(
    fields: &[CommandArgumentField],
    arguments: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    fields
        .iter()
        .flat_map(|field| {
            arguments
                .get(&normalize_field_key(&field.key))
                .into_iter()
                .flat_map(|values| values.iter().cloned())
        })
        .collect()
}

pub(super) fn hydrate(
    raw_arguments: &str,
    fields: &[CommandArgumentField],
) -> Result<(HashMap<String, Vec<String>>, String)> {
    let parsed = parse(Some(raw_arguments), fields);
    let mut parts = Vec::new();
    for field in fields {
        let Some(values) = parsed.get(&normalize_field_key(&field.key)) else {
            continue;
        };
        let values: Vec<_> = values
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(shell_quote)
            .collect();
        if !values.is_empty() {
            parts.push(format!("--{} {}", field.key, values.join(" ")));
        }
    }
    Ok((parsed, parts.join(" ")))
}

pub(super) fn missing_required(
    fields: &[CommandArgumentField],
    parsed_arguments: &HashMap<String, Vec<String>>,
) -> Vec<CommandArgumentField> {
    fields
        .iter()
        .filter(|field| field.required)
        .filter(|field| {
            parsed_arguments
                .get(&normalize_field_key(&field.key))
                .is_none_or(|values| values.iter().all(|value| value.trim().is_empty()))
        })
        .cloned()
        .collect()
}
