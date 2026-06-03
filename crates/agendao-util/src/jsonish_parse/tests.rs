use super::*;

fn test_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "write_file".into(),
            required_keys: vec!["file_path".into(), "content".into()],
            optional_keys: vec!["description".into()],
        },
        ToolSchema {
            name: "run_command".into(),
            required_keys: vec!["command".into()],
            optional_keys: vec!["working_dir".into()],
        },
        ToolSchema {
            name: "search".into(),
            required_keys: vec!["query".into(), "path".into()],
            optional_keys: vec![],
        },
    ]
}
#[test]
fn test_complete_json() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"{"file_path": "/a/b.rs", "content": "hello world"}"#);

    let result = parser.try_parse().unwrap();
    assert_eq!(result.tool_name, "write_file");
    assert_eq!(result.value["file_path"], "/a/b.rs");
}

#[test]
fn test_streaming_chunks() {
    let mut parser = StreamingToolParser::new(test_schemas());

    parser.push(r#"{"file_pa"#);
    parser.push(r#"th": "/a/b.rs", "#);
    parser.push(r#""content": "hello "#);
    parser.push(r#"world"}"#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "write_file");
    assert_eq!(result.value["content"], "hello world");
}

#[test]
fn test_truncated_string() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"{"file_path": "/a/b.rs", "content": "hello wor"#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "write_file");
    assert!(result.value["content"]
        .as_str()
        .unwrap()
        .starts_with("hello wor"));
}

#[test]
fn test_truncated_after_colon() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"{"file_path": "/a/b.rs", "content":"#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "write_file");
}

#[test]
fn test_html_content_with_escaped_quotes() {
    // This tests properly escaped HTML quotes — the normal case that works fine
    let mut parser = StreamingToolParser::new(test_schemas());
    let input = r#"{"file_path": "/index.html", "content": "<html lang=\"zh-CN\">\n<head>\n<title>Test</title>\n</head>\n</html>"}"#;
    parser.push(input);

    let result = parser.try_parse().unwrap();
    assert_eq!(result.tool_name, "write_file");
}

#[test]
fn test_unescaped_newlines() {
    let mut parser = StreamingToolParser::new(test_schemas());
    let input = "{\"file_path\": \"/a.txt\", \"content\": \"line1\nline2\nline3\"}";
    parser.push(input);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "write_file");
}
#[test]
fn test_trailing_comma() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"{"command": "ls -la", "working_dir": "/tmp",}"#);

    let result = parser.try_parse().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

#[test]
fn test_multiple_objects() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(
        r#"{"command": "mkdir /tmp/test"} {"file_path": "/tmp/test/a.txt", "content": "hi"}"#,
    );

    assert_eq!(parser.object_count(), 2);

    let results = parser.finalize_all();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap().tool_name, "run_command");
    assert_eq!(results[1].as_ref().unwrap().tool_name, "write_file");
}

#[test]
fn test_stray_closing_brace() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"some text } more text {"command": "echo hi"}"#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

#[test]
fn test_utf8_content() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"这是一些前缀文本 {"file_path": "/中文路径/测试.txt", "content": "你好世界🌍"}"#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "write_file");
    assert_eq!(result.value["content"], "你好世界🌍");
}

#[test]
fn test_array_value_truncated() {
    let schemas = vec![ToolSchema {
        name: "multi_edit".into(),
        required_keys: vec!["edits".into()],
        optional_keys: vec![],
    }];
    let mut parser = StreamingToolParser::new(schemas);
    parser.push(r#"{"edits": [{"file": "a.txt", "content": "hello"}, {"file": "b.txt""#);

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "multi_edit");
}

#[test]
fn test_single_quotes() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push("{'command': 'echo hello'}");

    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

#[test]
fn test_reset() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(r#"{"command": "ls"}"#);
    assert_eq!(parser.object_count(), 1);

    parser.reset();
    assert_eq!(parser.object_count(), 0);
    assert_eq!(parser.buffer(), "");
}

#[test]
fn test_repair_diagnostics() {
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push("{\"command\": \"echo hello\nworld\",}");

    let result = parser.finalize().unwrap();
    assert!(!result.repairs.is_empty());
}

// =========================================================================
// Taxonomy Tests — one per malformation ID
// =========================================================================
//
// Each test uses repair_json_standalone to verify the repair in isolation,
// then (where applicable) runs through StreamingToolParser for integration.

// ── Category A: Framing ──────────────────────────────────────────────────

#[test]
fn test_a5_bom_prefix() {
    let input = "\u{feff}{\"command\": \"ls\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("BOM")));
}

#[test]
fn test_a6_ansi_escape_sequences() {
    let input = "\x1b[32m{\"command\": \"ls\"}\x1b[0m";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("ANSI")));
}

#[test]
fn test_a6_ansi_osc_sequence() {
    // OSC sequence: ESC ] ... BEL
    let input = "\x1b]0;title\x07{\"command\": \"pwd\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "pwd");
    assert!(repairs.iter().any(|r| r.contains("ANSI")));
}

#[test]
fn test_a7_xml_tool_call_wrapper() {
    let input = "<tool_call>{\"command\": \"echo hi\"}</tool_call>";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "echo hi");
    assert!(repairs.iter().any(|r| r.contains("tool_call")));
}

#[test]
fn test_a7_xml_function_call_wrapper() {
    let input = "<function_call>{\"command\": \"date\"}</function_call>";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "date");
    assert!(repairs.iter().any(|r| r.contains("function_call")));
}

#[test]
fn test_a7_xml_json_wrapper_with_attrs() {
    let input = "<json type=\"tool\">{\"command\": \"whoami\"}</json>";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "whoami");
    assert!(repairs.iter().any(|r| r.contains("<json>")));
}

#[test]
fn test_a1_markdown_code_fence() {
    let input = "```json\n{\"command\": \"ls -la\"}\n```";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls -la");
    assert!(repairs.iter().any(|r| r.contains("markdown")));
}

#[test]
fn test_a1_markdown_fence_no_closing() {
    let input = "```json\n{\"command\": \"ls\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("markdown")));
}

#[test]
fn test_d7_trailing_semicolons() {
    let input = "{\"command\": \"echo ok\"};";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "echo ok");
    assert!(repairs.iter().any(|r| r.contains("semicolon")));
}

// ── Category D: Syntax Sugar ─────────────────────────────────────────────

#[test]
fn test_d2_unquoted_keys() {
    let input = "{command: \"ls\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("unquoted")));
}

#[test]
fn test_d2_unquoted_keys_multiple() {
    let input = "{file_path: \"/a.txt\", content: \"hello\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["file_path"], "/a.txt");
    assert_eq!(v["content"], "hello");
    assert!(repairs.iter().any(|r| r.contains("unquoted")));
}

#[test]
fn test_d3_block_comments() {
    let input = "{\"command\": \"ls\" /* list files */}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("comment")));
}

#[test]
fn test_d4_line_comments() {
    let input = "{\"command\": \"ls\" // list files\n}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("comment")));
}

#[test]
fn test_d5_hex_numbers() {
    let input = "{\"a\": 0xFF}";
    let (repaired, _) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["a"], 255);
}

#[test]
fn test_d6_infinity() {
    let input = "{\"a\": Infinity}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert!(v["a"].is_null());
    assert!(repairs
        .iter()
        .any(|r| r.contains("Infinity") || r.contains("NaN")));
}

#[test]
fn test_d6_negative_infinity() {
    let input = "{\"a\": -Infinity}";
    let (repaired, _) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert!(v["a"].is_null());
}

#[test]
fn test_d6_nan() {
    let input = "{\"a\": NaN}";
    let (repaired, _) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert!(v["a"].is_null());
}

#[test]
fn test_d8_python_true_false_none() {
    let input = "{\"a\": True, \"b\": False, \"c\": None}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["a"], true);
    assert_eq!(v["b"], false);
    assert!(v["c"].is_null());
    assert!(repairs.iter().any(|r| r.contains("Python")));
}

#[test]
fn test_d8_python_literals_not_in_strings() {
    // "True" inside a string value should NOT be converted
    let input = "{\"command\": \"echo True\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "echo True");
}

#[test]
fn test_d10_plus_prefix() {
    let input = "{\"a\": +42}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["a"], 42);
    assert!(repairs.iter().any(|r| r.contains("plus")));
}

// ── Category B: Structural ───────────────────────────────────────────────

#[test]
fn test_b10_consecutive_commas() {
    let input = "{\"command\": \"ls\",,,}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs
        .iter()
        .any(|r| r.contains("trailing comma") || r.contains("commas")));
}

#[test]
fn test_b11_missing_comma_between_fields() {
    let input = "{\"file_path\": \"/a.txt\" \"content\": \"hello\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["file_path"], "/a.txt");
    assert_eq!(v["content"], "hello");
    assert!(repairs.iter().any(|r| r.contains("missing comma")));
}

#[test]
fn test_b12_missing_colon() {
    let input = "{\"command\" \"ls\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("missing colon")));
}

#[test]
fn test_b14_truncated_mid_escape() {
    let input = "{\"command\": \"hello\\";
    let (repaired, _) = repair_json_standalone(input, true);
    // Should be parseable after aggressive close + bracket balance
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
}

#[test]
fn test_b17_truncated_mid_number() {
    let input = "{\"a\": 3.14";
    let (repaired, _) = repair_json_standalone(input, true);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["a"].as_f64(), Some(157.0 / 50.0));
}

// ── Category C: String Content ───────────────────────────────────────────

#[test]
fn test_c5_unescaped_backslash() {
    // \q is not a valid JSON escape — should become \\q
    let input = r#"{"command": "echo \q"}"#;
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert!(v["command"].as_str().unwrap().contains("\\q"));
    assert!(repairs.iter().any(|r| r.contains("escape")));
}

#[test]
fn test_c6_invalid_escape_windows_path() {
    // Windows path: C:\Users\name → should escape the backslashes
    let input = "{\"file_path\": \"C:\\Users\\name\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
}

#[test]
fn test_c7_truncated_unicode_escape() {
    // \u00 is only 2 hex digits — should be padded to \u0000
    let input = "{\"a\": \"text\\u00\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
    assert!(repairs.iter().any(|r| r.contains("escape")));
}

#[test]
fn test_c7_truncated_unicode_escape_one_digit() {
    let input = "{\"a\": \"text\\uA\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
}

#[test]
fn test_c8_lone_high_surrogate() {
    // \uD83D without a following low surrogate → \uFFFD
    let input = "{\"a\": \"emoji \\uD83D end\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
    assert!(repaired.contains("uFFFD"));
    assert!(repairs.iter().any(|r| r.contains("escape")));
}

#[test]
fn test_c8_lone_low_surrogate() {
    // \uDC00 (low surrogate without preceding high) → \uFFFD
    let input = "{\"a\": \"bad \\uDC00 end\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
    assert!(repaired.contains("uFFFD"));
}

#[test]
fn test_c8_valid_surrogate_pair_preserved() {
    // Valid surrogate pair should pass through unchanged
    let input = "{\"a\": \"emoji \\uD83D\\uDE00 end\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
    assert!(repaired.contains("\\uD83D\\uDE00"));
}

// ── Category E: Encoding ─────────────────────────────────────────────────

#[test]
fn test_e4_crlf_in_strings() {
    let input = "{\"command\": \"line1\r\nline2\"}";
    let (repaired, _) = repair_json_standalone(input, false);
    // After repair, \r should be gone or normalized
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
}

#[test]
fn test_e4_lone_cr_in_strings() {
    let input = "{\"command\": \"line1\rline2\"}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let result = serde_json::from_str::<Value>(&repaired);
    assert!(result.is_ok(), "Failed to parse: {}", repaired);
    assert!(repairs.iter().any(|r| r.contains("line ending")));
}

// ── Integration: Combined malformations ──────────────────────────────────

#[test]
fn test_combined_bom_plus_trailing_comma() {
    // BOM is before the `{`, so the scanner extracts the object without BOM.
    // The standalone API handles BOM; the streaming parser only sees the trailing comma.
    let input = "\u{feff}{\"command\": \"ls\",}";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.len() >= 2); // BOM + trailing comma
}

#[test]
fn test_combined_markdown_fence_plus_single_quotes() {
    let input = "```json\n{'command': 'echo hello'}\n```";
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(input);
    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

#[test]
fn test_combined_xml_wrapper_plus_python_literals() {
    let input = "<tool_call>{\"a\": True, \"command\": \"test\"}</tool_call>";
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(input);
    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

#[test]
fn test_combined_ansi_plus_unquoted_keys() {
    // ANSI escape `[` confuses the scanner's bracket depth, so this goes
    // through the standalone API (the correct path for pre-framed input).
    let input = "\x1b[1m{command: \"ls -la\"}\x1b[0m";
    let (repaired, repairs) = repair_json_standalone(input, false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls -la");
    assert!(repairs.iter().any(|r| r.contains("ANSI")));
    assert!(repairs.iter().any(|r| r.contains("unquoted")));
}

#[test]
fn test_combined_comments_plus_trailing_comma() {
    let input = "{\"command\": \"ls\" /* list */, }";
    let mut parser = StreamingToolParser::new(test_schemas());
    parser.push(input);
    let result = parser.finalize().unwrap();
    assert_eq!(result.tool_name, "run_command");
}

// ── Standalone API tests ─────────────────────────────────────────────────

#[test]
fn test_standalone_api_non_aggressive() {
    let (repaired, repairs) = repair_json_standalone("```\n{\"command\": \"ls\",}\n```", false);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(!repairs.is_empty());
}

#[test]
fn test_standalone_api_aggressive() {
    let (repaired, repairs) =
        repair_json_standalone("{\"command\": \"ls\", \"working_dir\":", true);
    let v: Value = serde_json::from_str(&repaired).unwrap();
    assert_eq!(v["command"], "ls");
    assert!(repairs.iter().any(|r| r.contains("null")));
}
