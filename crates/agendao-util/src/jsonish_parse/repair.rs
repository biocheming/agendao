use super::sanitize::{normalize_syntax, sanitize_input};
use super::types::ToolSchema;
use serde_json::Value;

// =============================================================================
// Repair Layer — state-machine-aware JSON repair
// =============================================================================
//
// NOTE: The quote-tracking in these functions shares the fundamental limitation
// that unescaped `"` inside string values (e.g. HTML attributes) will desync
// the `in_string` state. This is acceptable because:
// 1. Most LLM output has properly escaped quotes in JSON strings
// 2. The ultra structural recovery handles the truly broken cases
// 3. These repairs handle the common cases (control chars, trailing commas, etc.)

/// Core repair pipeline. `aggressive` enables more forceful strategies (finalize).
pub(super) fn repair_json(input: &str, aggressive: bool, repairs: &mut Vec<String>) -> String {
    let mut s = input.to_string();

    // ═══ Phase 0: SANITIZE — strip framing noise ═══
    s = sanitize_input(&s, repairs);

    // ═══ Phase 1: NORMALIZE — non-standard syntax → JSON ═══
    s = convert_single_quotes(&s, repairs);
    s = normalize_syntax(&s, repairs);

    // ═══ Phase 2: REPAIR — fix broken JSON syntax ═══
    s = normalize_line_endings(&s, repairs);
    s = escape_control_chars_in_strings(&s, repairs);
    s = repair_unclosed_strings(&s, repairs);
    s = insert_missing_commas(&s, repairs);
    s = insert_missing_colons(&s, repairs);
    s = remove_trailing_commas(&s, repairs);

    // ═══ Phase 3: CLOSE — aggressive finalization ═══
    if aggressive {
        s = aggressive_close(&s, repairs);
    }

    // Always last: balance brackets (stack-ordered)
    s = balance_brackets_stateful(&s, repairs);

    s
}

// ─── Single quote conversion ────────────────────────────────────────────────

fn convert_single_quotes(input: &str, repairs: &mut Vec<String>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    let mut converted = false;

    for &ch in &chars {
        if escape {
            escape = false;
            out.push(ch);
            continue;
        }
        if ch == '\\' {
            escape = true;
            out.push(ch);
            continue;
        }

        if !in_double && !in_single {
            if ch == '\'' {
                out.push('"');
                in_single = true;
                converted = true;
                continue;
            }
            if ch == '"' {
                in_double = true;
                out.push(ch);
                continue;
            }
        } else if in_single {
            if ch == '\'' {
                out.push('"');
                in_single = false;
                continue;
            }
            if ch == '"' {
                // Double quote inside single-quoted string needs escaping
                out.push('\\');
                out.push('"');
                continue;
            }
        } else if in_double && ch == '"' {
            in_double = false;
        }

        out.push(ch);
    }

    if converted {
        repairs.push("converted single quotes to double quotes".into());
    }
    out
}
// ─── Control character escaping ──────────────────────────────────────────────

fn escape_control_chars_in_strings(input: &str, repairs: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut fixed = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            // ── Handle escape sequences ──
            if ch == '\\' {
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    match next {
                        // Valid JSON escapes — pass through
                        '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' => {
                            out.push('\\');
                            out.push(next);
                            i += 2;
                            continue;
                        }
                        // Unicode escape — validate length and content (C7, C8)
                        'u' => {
                            if i + 5 < chars.len()
                                && chars[i + 2].is_ascii_hexdigit()
                                && chars[i + 3].is_ascii_hexdigit()
                                && chars[i + 4].is_ascii_hexdigit()
                                && chars[i + 5].is_ascii_hexdigit()
                            {
                                let hex: String = chars[i + 2..i + 6].iter().collect();
                                if let Ok(code) = u16::from_str_radix(&hex, 16) {
                                    // C8: Lone high surrogate (D800-DBFF)
                                    if (0xD800..=0xDBFF).contains(&code) {
                                        // Check if followed by low surrogate \uDCxx-\uDFxx
                                        let has_low = i + 11 < chars.len()
                                            && chars[i + 6] == '\\'
                                            && chars[i + 7] == 'u';
                                        if has_low {
                                            // Valid surrogate pair — pass through all 12 chars
                                            for j in 0..12 {
                                                out.push(chars[i + j]);
                                            }
                                            i += 12;
                                            continue;
                                        } else {
                                            // Lone surrogate → replacement char
                                            out.push_str("\\uFFFD");
                                            i += 6;
                                            fixed = true;
                                            continue;
                                        }
                                    }
                                    // C8: Lone low surrogate (DC00-DFFF)
                                    if (0xDC00..=0xDFFF).contains(&code) {
                                        out.push_str("\\uFFFD");
                                        i += 6;
                                        fixed = true;
                                        continue;
                                    }
                                }
                                // Valid \uXXXX — pass through
                                for j in 0..6 {
                                    out.push(chars[i + j]);
                                }
                                i += 6;
                                continue;
                            } else {
                                // C7: Truncated unicode escape — pad with zeros
                                out.push_str("\\u");
                                i += 2;
                                let mut hex_count = 0;
                                while i < chars.len()
                                    && hex_count < 4
                                    && chars[i].is_ascii_hexdigit()
                                {
                                    out.push(chars[i]);
                                    hex_count += 1;
                                    i += 1;
                                }
                                for _ in hex_count..4 {
                                    out.push('0');
                                }
                                fixed = true;
                                continue;
                            }
                        }
                        // C6: Invalid escape sequence — double the backslash
                        _ => {
                            out.push_str("\\\\");
                            // Don't consume `next` — it will be processed normally
                            i += 1;
                            fixed = true;
                            continue;
                        }
                    }
                } else {
                    // Trailing backslash at end of input — escape it
                    out.push_str("\\\\");
                    i += 1;
                    fixed = true;
                    continue;
                }
            }

            if ch == '"' {
                in_string = false;
                out.push(ch);
                i += 1;
                continue;
            }

            // ── Control characters (C1, C2, C3, C10) ──
            match ch {
                '\n' => {
                    out.push_str("\\n");
                    fixed = true;
                }
                '\r' => {
                    out.push_str("\\r");
                    fixed = true;
                }
                '\t' => {
                    out.push_str("\\t");
                    fixed = true;
                }
                '\x08' => {
                    out.push_str("\\b");
                    fixed = true;
                }
                '\x0C' => {
                    out.push_str("\\f");
                    fixed = true;
                }
                c if c.is_control() => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                    fixed = true;
                }
                _ => out.push(ch),
            }
            i += 1;
            continue;
        }

        // ── Outside string ──
        if ch == '"' {
            in_string = true;
        }
        out.push(ch);
        i += 1;
    }

    if fixed {
        repairs.push("escaped control characters / fixed escape sequences in strings".into());
    }
    out
}

// ─── Unclosed string repair ─────────────────────────────────────────────────

fn repair_unclosed_strings(input: &str, repairs: &mut Vec<String>) -> String {
    let mut in_string = false;
    let mut escape = false;

    for ch in input.chars() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
                continue;
            }
        } else if ch == '"' {
            in_string = true;
        }
    }

    if in_string {
        let mut out = input.to_string();
        // Count consecutive trailing backslashes. An odd count means the last
        // one is unescaped and would re-escape the closing quote we're about
        // to add. An even count means they're all escaped pairs — safe to keep.
        let trailing_backslashes = out.chars().rev().take_while(|&c| c == '\\').count();
        if trailing_backslashes % 2 == 1 {
            out.pop();
        }
        out.push('"');
        repairs.push("closed unclosed string".into());
        return out;
    }

    input.to_string()
}
// ─── Trailing comma removal ─────────────────────────────────────────────────

fn remove_trailing_commas(input: &str, repairs: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut in_string = false;
    let mut escape = false;
    let mut fixed = false;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            if escape {
                escape = false;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '\\' {
                escape = true;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == ',' {
            let rest = &chars[i + 1..];
            let next_non_ws = rest.iter().find(|c| !c.is_whitespace());
            // B9: trailing comma before } or ]
            if next_non_ws == Some(&'}') || next_non_ws == Some(&']') {
                fixed = true;
                i += 1;
                continue;
            }
            // B10: consecutive commas — skip extra commas
            if next_non_ws == Some(&',') {
                fixed = true;
                i += 1;
                continue;
            }
        }

        out.push(ch);
        i += 1;
    }

    if fixed {
        repairs.push("removed trailing commas".into());
    }
    out
}

// ─── Missing comma insertion (B11) ──────────────────────────────────────────

/// Insert missing commas between fields: `"a":"1" "b":"2"` → `"a":"1", "b":"2"`
/// Detects pattern: `"value" "key"` (string end followed by string start without comma).
fn insert_missing_commas(input: &str, repairs: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(input.len() + 32);
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut fixed = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            if escape {
                escape = false;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '\\' {
                escape = true;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '"' {
                in_string = false;
                out.push(ch);
                i += 1;

                // After closing a string, check if next non-whitespace is `"`
                // without a comma, colon, }, ] in between
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '"' {
                    // Look back: was the string before this a value (preceded by `:`)?
                    // If so, the next `"` starts a new key — insert comma.
                    // Simple heuristic: if we see `"value" "key":`, insert comma.
                    // Check if after the upcoming `"..."` there's a `:`
                    let mut k = j + 1;
                    let mut k_escape = false;
                    while k < chars.len() {
                        if k_escape {
                            k_escape = false;
                            k += 1;
                            continue;
                        }
                        if chars[k] == '\\' {
                            k_escape = true;
                            k += 1;
                            continue;
                        }
                        if chars[k] == '"' {
                            k += 1;
                            break;
                        }
                        k += 1;
                    }
                    // Skip whitespace after the closing quote
                    while k < chars.len() && chars[k].is_whitespace() {
                        k += 1;
                    }
                    // If followed by `:`, this is a key — insert comma
                    if k < chars.len() && chars[k] == ':' {
                        out.push(',');
                        fixed = true;
                    }
                }
                continue;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
        }
        out.push(ch);
        i += 1;
    }

    if fixed {
        repairs.push("inserted missing commas between fields".into());
    }
    out
}

// ─── Missing colon insertion (B12) ──────────────────────────────────────────

/// Insert missing colons: `{"a" "value"}` → `{"a": "value"}`
/// Detects pattern: `"key" "value"` where the first string is a key (after `{` or `,`).
fn insert_missing_colons(input: &str, repairs: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut fixed = false;
    // Track whether we expect a key (after `{` or `,`)
    let mut expect_key = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            if escape {
                escape = false;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '\\' {
                escape = true;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '"' {
                in_string = false;
                out.push(ch);
                i += 1;

                if expect_key {
                    // We just closed a key string. Check if next non-ws is `"` (missing colon)
                    let mut j = i;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if j < chars.len()
                        && (chars[j] == '"'
                            || chars[j] == '{'
                            || chars[j] == '['
                            || chars[j].is_ascii_digit()
                            || chars[j] == '-')
                    {
                        // Check it's not already followed by `:`
                        if j < chars.len() && chars[j] != ':' {
                            out.push(':');
                            fixed = true;
                        }
                    }
                    expect_key = false;
                }
                continue;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '{' | ',' => {
                expect_key = true;
                out.push(ch);
            }
            ':' | '}' | ']' => {
                expect_key = false;
                out.push(ch);
            }
            _ => {
                out.push(ch);
            }
        }
        i += 1;
    }

    if fixed {
        repairs.push("inserted missing colons".into());
    }
    out
}

// ─── Line ending normalization (E4) ─────────────────────────────────────────

/// Normalize `\r\n` → `\n` and lone `\r` → `\n` in string values.
fn normalize_line_endings(input: &str, repairs: &mut Vec<String>) -> String {
    if !input.contains('\r') {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut fixed = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_string {
            if escape {
                escape = false;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '\\' {
                escape = true;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '"' {
                in_string = false;
                out.push(ch);
                i += 1;
                continue;
            }
            if ch == '\r' {
                // \r\n → \n, lone \r → \n
                if i + 1 < chars.len() && chars[i + 1] == '\n' {
                    i += 1; // skip \r, the \n will be processed next iteration
                } else {
                    out.push('\n');
                    i += 1;
                    fixed = true;
                }
                continue;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
        }
        out.push(ch);
        i += 1;
    }

    if fixed {
        repairs.push("normalized line endings in strings".into());
    }
    out
}

// ─── State-machine-aware bracket balancing ──────────────────────────────────

fn balance_brackets_stateful(input: &str, repairs: &mut Vec<String>) -> String {
    // Track the actual nesting order so we close in correct reverse order
    let mut stack: Vec<char> = Vec::new(); // '{' or '['
    let mut in_string = false;
    let mut escape = false;

    for ch in input.chars() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('{'),
            '}' => {
                // Pop matching '{', or ignore stray '}'
                if let Some(pos) = stack.iter().rposition(|&c| c == '{') {
                    stack.remove(pos);
                }
            }
            '[' => stack.push('['),
            ']' => {
                if let Some(pos) = stack.iter().rposition(|&c| c == '[') {
                    stack.remove(pos);
                }
            }
            _ => {}
        }
    }

    if stack.is_empty() {
        return input.to_string();
    }

    let mut out = input.to_string();
    // Close in reverse nesting order
    for &opener in stack.iter().rev() {
        match opener {
            '{' => out.push('}'),
            '[' => out.push(']'),
            _ => {}
        }
    }

    repairs.push(format!(
        "balanced brackets (unclosed: {})",
        stack.iter().collect::<String>()
    ));
    out
}
// ─── Aggressive close ───────────────────────────────────────────────────────

fn aggressive_close(input: &str, repairs: &mut Vec<String>) -> String {
    let trimmed = input.trim_end();
    if trimmed.ends_with('}') || trimmed.ends_with(']') {
        return input.to_string();
    }

    let last_significant = trimmed.chars().rev().find(|c| !c.is_whitespace());

    match last_significant {
        // B2: truncated after colon — add null placeholder
        Some(':') => {
            repairs.push("added null for truncated value".into());
            format!("{}null", trimmed)
        }
        // B3: truncated after comma — remove dangling comma
        Some(',') => {
            repairs.push("removed dangling comma".into());
            trimmed.trim_end_matches(',').to_string()
        }
        // B14: truncated mid-escape — `"hello\` → remove trailing backslash
        Some('\\') => {
            let mut s = trimmed.to_string();
            s.pop(); // remove `\`
            repairs.push("removed truncated escape sequence".into());
            s
        }
        // B17: truncated mid-number — `{"a": 3.1` → number is valid, just needs closing
        // Also handles truncated after `"` (string just closed, needs comma or close)
        Some(c) if c.is_ascii_digit() || c == '.' => {
            // Number is fine as-is, bracket balancing will close it
            trimmed.to_string()
        }
        _ => trimmed.to_string(),
    }
}

// =============================================================================
// Tool Detection — scored schema matching
// =============================================================================

pub(super) fn detect_tool(value: &Value, schemas: &[ToolSchema]) -> Option<String> {
    let obj = value.as_object()?;

    // Direct name field takes priority
    if let Some(name_val) = obj.get("name").or_else(|| obj.get("tool")) {
        if let Some(name_str) = name_val.as_str() {
            if let Some(schema) = schemas.iter().find(|s| s.name == name_str) {
                return Some(schema.name.clone());
            }
        }
    }

    let mut best: Option<&ToolSchema> = None;
    let mut best_score: i32 = 0;
    let mut ambiguous = false;

    for schema in schemas {
        let mut score: i32 = 0;

        for key in &schema.required_keys {
            if obj.contains_key(key) {
                score += 3;
            } else {
                score -= 1;
            }
        }

        for key in &schema.optional_keys {
            if obj.contains_key(key) {
                score += 1;
            }
        }

        if score > best_score {
            best_score = score;
            best = Some(schema);
            ambiguous = false;
        } else if score == best_score && score > 0 {
            ambiguous = true;
        }
    }

    // Ambiguous low-score matches are rejected
    if ambiguous && best_score < 3 {
        return None;
    }

    best.map(|s| s.name.clone())
}

// =============================================================================
// Public API — standalone repair for external callers
// =============================================================================

/// Apply the full repair pipeline to a raw JSON string without requiring
/// a streaming parser or tool schemas. Returns the repaired string and
/// a list of repair operations applied.
///
/// This is the standalone entry point for callers like `recover_tool_call_ultra`
/// that want to use the repair pipeline without the streaming parser.
pub fn repair_json_standalone(input: &str, aggressive: bool) -> (String, Vec<String>) {
    let mut repairs = Vec::new();
    let repaired = repair_json(input, aggressive, &mut repairs);
    (repaired, repairs)
}

/// Apply only Phase 0 (sanitize) — strip framing noise without modifying
/// the JSON structure itself. Use this when you need clean input for
/// structural recovery that searches for field boundaries.
pub fn sanitize_standalone(input: &str) -> (String, Vec<String>) {
    let mut repairs = Vec::new();
    let sanitized = sanitize_input(input, &mut repairs);
    (sanitized, repairs)
}
