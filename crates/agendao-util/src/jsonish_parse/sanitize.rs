// ============================================================================
// Phase 0: SANITIZE — strip framing noise around the JSON
// =============================================================================

/// Remove non-JSON framing that LLMs wrap around tool-call output.
/// Handles: BOM (A5), ANSI escapes (A6), XML/HTML wrappers (A7),
/// markdown fences (A1/A8), trailing semicolons (D7).
pub(super) fn sanitize_input(input: &str, repairs: &mut Vec<String>) -> String {
    let mut s = input.to_string();

    // ── BOM (A5) ──
    if s.starts_with('\u{feff}') {
        s = s.trim_start_matches('\u{feff}').to_string();
        repairs.push("stripped BOM".into());
    }

    // ── ANSI escape sequences (A6) ──
    // Matches: ESC[ ... m  (SGR sequences — the vast majority of ANSI codes)
    // Also: ESC[ ... [A-Z] for cursor movement, etc.
    let ansi_len = s.len();
    s = strip_ansi_escapes(&s);
    if s.len() != ansi_len {
        repairs.push("stripped ANSI escape sequences".into());
    }

    // ── XML/HTML tool wrappers (A7) ──
    // Common patterns: <tool_call>...</tool_call>, <function_call>...</function_call>,
    // <json>...</json>, <tool_input>...</tool_input>
    let wrapper_tags = [
        "tool_call",
        "function_call",
        "tool_input",
        "json",
        "arguments",
        "tool_result",
    ];
    for tag in &wrapper_tags {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        // Also handle <tag ...> with attributes
        let open_attr = format!("<{} ", tag);
        if let Some(start) = s.find(&open).or_else(|| s.find(&open_attr)) {
            let content_start = s[start..].find('>').map(|i| start + i + 1);
            let content_end = s.rfind(&close);
            if let (Some(cs), Some(ce)) = (content_start, content_end) {
                if cs < ce {
                    s = s[cs..ce].to_string();
                    repairs.push(format!("stripped <{}> wrapper", tag));
                    break;
                }
            }
        }
    }

    // ── Markdown code fences (A1/A8) ──
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        // Find end of first fence line
        if let Some(nl) = trimmed.find('\n') {
            let after_fence = &trimmed[nl + 1..];
            // Find closing fence
            if let Some(close_pos) = after_fence.rfind("```") {
                s = after_fence[..close_pos].trim().to_string();
                repairs.push("stripped markdown code fences".into());
            } else {
                // No closing fence — just strip the opening line
                s = after_fence.trim().to_string();
                repairs.push("stripped markdown opening fence".into());
            }
        }
    }

    // ── Trailing semicolons (D7) ──
    let trimmed = s.trim_end();
    if trimmed.ends_with(';') {
        s = trimmed.trim_end_matches(';').to_string();
        repairs.push("stripped trailing semicolons".into());
    }

    s.trim().to_string()
}

/// Strip ANSI escape sequences (CSI sequences: ESC[ ... final_byte)
fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Check for CSI sequence: ESC [
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Consume parameter bytes (0x30-0x3F) and intermediate bytes (0x20-0x2F)
                              // until final byte (0x40-0x7E)
                loop {
                    match chars.next() {
                        Some(c) if ('\x40'..='\x7e').contains(&c) => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
                continue;
            }
            // OSC sequence: ESC ]
            if chars.peek() == Some(&']') {
                chars.next();
                // Consume until ST (ESC \ or BEL \x07)
                loop {
                    match chars.next() {
                        Some('\x07') => break,
                        Some('\x1b') if chars.peek() == Some(&'\\') => {
                            chars.next();
                            break;
                        }
                        Some(_) => continue,
                        None => break,
                    }
                }
                continue;
            }
            // Simple two-byte escape: ESC + single char
            if chars.peek().is_some() {
                chars.next();
            }
            continue;
        }
        out.push(ch);
    }
    out
}

// =============================================================================
// Phase 1: NORMALIZE — convert non-standard syntax to valid JSON
// =============================================================================

/// Convert non-standard JSON-like syntax to valid JSON.
/// Handles: D2 (unquoted keys), D3/D4 (comments), D5 (hex numbers),
/// D6 (Infinity/NaN), D8 (Python literals), D10 (plus prefix).
/// Single quotes (D1) are handled separately in convert_single_quotes.
pub(super) fn normalize_syntax(input: &str, repairs: &mut Vec<String>) -> String {
    let mut s = input.to_string();

    // ── Strip comments (D3/D4) — must run before other transforms ──
    let before_comments = s.len();
    s = strip_comments(&s);
    if s.len() != before_comments {
        repairs.push("stripped comments".into());
    }

    // ── Python literals (D8) — True/False/None → true/false/null ──
    // Only replace outside strings
    let before_py = s.clone();
    s = replace_python_literals(&s);
    if s != before_py {
        repairs.push("converted Python literals to JSON".into());
    }

    // ── Infinity/NaN (D6) → null ──
    let before_inf = s.clone();
    s = replace_special_numbers(&s);
    if s != before_inf {
        repairs.push("replaced Infinity/NaN with null".into());
    }

    // ── Unquoted keys (D2) ──
    let before_keys = s.clone();
    s = quote_unquoted_keys(&s);
    if s != before_keys {
        repairs.push("quoted unquoted keys".into());
    }

    // ── Hex numbers (D5) ──
    let before_hex = s.clone();
    s = convert_hex_numbers(&s);
    if s != before_hex {
        repairs.push("converted hex numbers to decimal".into());
    }

    // ── Plus prefix on numbers (D10) ──
    let before_plus = s.clone();
    s = strip_plus_prefix(&s);
    if s != before_plus {
        repairs.push("stripped plus prefix from numbers".into());
    }

    s
}

/// Strip JavaScript-style comments: /* ... */ and // ... \n
fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Block comment: /* ... */
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() {
                if chars[i] == '*' && chars[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            // If we ran off the end without finding */, skip remaining
            if i >= chars.len() {
                break;
            }
            out.push(' '); // Replace comment with space to preserve token separation
            continue;
        }

        // Line comment: // ... \n
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            // Keep the newline
            if i < chars.len() {
                out.push('\n');
                i += 1;
            }
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replace Python True/False/None with JSON true/false/null (outside strings).
fn replace_python_literals(input: &str) -> String {
    // We need to be careful to only replace these as standalone tokens,
    // not inside strings or as parts of identifiers.
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    // Pre-compute byte offsets to avoid O(n²) char_indices().nth(i) lookups
    let byte_offsets: Vec<usize> = input.char_indices().map(|(b, _)| b).collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
            } else if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Check for Python literals at word boundary
        let rest = &input[byte_offsets[i]..];
        if let Some((py, json)) = match_python_literal(rest) {
            // Verify it's at a word boundary (not part of a larger identifier)
            let prev_is_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
            let next_idx = i + py.chars().count();
            let next_is_boundary = next_idx >= chars.len() || !chars[next_idx].is_alphanumeric();

            if prev_is_boundary && next_is_boundary {
                out.push_str(json);
                i = next_idx;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

fn match_python_literal(s: &str) -> Option<(&str, &str)> {
    if s.starts_with("True") {
        Some(("True", "true"))
    } else if s.starts_with("False") {
        Some(("False", "false"))
    } else if s.starts_with("None") {
        Some(("None", "null"))
    } else {
        None
    }
}

/// Replace Infinity, -Infinity, NaN with null (outside strings).
fn replace_special_numbers(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    // Pre-compute byte offsets to avoid O(n²) char_indices().nth(i) lookups
    let byte_offsets: Vec<usize> = input.char_indices().map(|(b, _)| b).collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
            } else if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        let rest = &input[byte_offsets[i]..];

        // -Infinity
        if rest.starts_with("-Infinity") {
            let next_idx = i + 9;
            let next_is_boundary = next_idx >= chars.len() || !chars[next_idx].is_alphanumeric();
            if next_is_boundary {
                out.push_str("null");
                i = next_idx;
                continue;
            }
        }
        // Infinity
        if rest.starts_with("Infinity") {
            let next_idx = i + 8;
            let next_is_boundary = next_idx >= chars.len() || !chars[next_idx].is_alphanumeric();
            if next_is_boundary {
                out.push_str("null");
                i = next_idx;
                continue;
            }
        }
        // NaN
        if rest.starts_with("NaN") {
            let next_idx = i + 3;
            let next_is_boundary = next_idx >= chars.len() || !chars[next_idx].is_alphanumeric();
            if next_is_boundary {
                out.push_str("null");
                i = next_idx;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Quote unquoted keys: `{key: "value"}` → `{"key": "value"}`
/// Detects patterns like `{ identifier :` outside strings.
fn quote_unquoted_keys(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 32);
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
            } else if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // After `{` or `,`, look for unquoted key pattern: identifier followed by `:`
        if chars[i] == '{' || chars[i] == ',' {
            out.push(chars[i]);
            i += 1;
            // Skip whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                out.push(chars[i]);
                i += 1;
            }
            // Check if next token is an unquoted identifier (not `"`, `{`, `[`, etc.)
            if i < chars.len() && (chars[i].is_alphabetic() || chars[i] == '_' || chars[i] == '$') {
                // Collect the identifier
                let key_start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
                {
                    i += 1;
                }
                let key: String = chars[key_start..i].iter().collect();
                // Skip whitespace after key
                let mut j = i;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                // If followed by `:`, this is an unquoted key
                if j < chars.len() && chars[j] == ':' {
                    out.push('"');
                    out.push_str(&key);
                    out.push('"');
                    // Push the whitespace we skipped
                    for ch in chars.iter().take(j).skip(i) {
                        out.push(*ch);
                    }
                    i = j;
                    continue;
                } else {
                    // Not a key — push as-is
                    out.push_str(&key);
                    i = key_start + key.len();
                    continue;
                }
            }
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Convert hex numbers (0xFF) to decimal (255) outside strings.
fn convert_hex_numbers(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
            } else if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Detect 0x or 0X prefix
        if chars[i] == '0' && i + 1 < chars.len() && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
            let hex_start = i + 2;
            let mut hex_end = hex_start;
            while hex_end < chars.len() && chars[hex_end].is_ascii_hexdigit() {
                hex_end += 1;
            }
            if hex_end > hex_start {
                let hex_str: String = chars[hex_start..hex_end].iter().collect();
                if let Ok(val) = u64::from_str_radix(&hex_str, 16) {
                    out.push_str(&val.to_string());
                    i = hex_end;
                    continue;
                }
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Strip `+` prefix from numbers: `+1` → `1`, `+3.14` → `3.14`
fn strip_plus_prefix(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        if in_string {
            if escape {
                escape = false;
                out.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '\\' {
                escape = true;
            } else if chars[i] == '"' {
                in_string = false;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '"' {
            in_string = true;
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // `+` followed by digit, after `:` or `,` or `[`
        if chars[i] == '+' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let prev_non_ws = chars[..i]
                .iter()
                .rev()
                .find(|c| !c.is_whitespace())
                .copied();
            if matches!(prev_non_ws, Some(':') | Some(',') | Some('[') | None)
                || matches!(prev, Some(c) if c.is_whitespace())
            {
                // Skip the `+`
                i += 1;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}
