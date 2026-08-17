/// Build the user-visible prompt message from input text and optional command.
///
/// # Authority (AgenDao 木律, P2.3)
///
/// This is the CLI adapter's pre-formatting step.  The result is sent as
/// `PromptRequest.message` (the full `/command args` text).
/// `PromptRequest.command` is also set when available.
///
/// **Current division of labor:**
/// - CLI adapter: pre-formats `/command input` for backward compatibility.
/// - Session ingress (`normalize_ingress_source`): canonical authority
///   for ingress source classification.
/// - `PromptRequest.message` takes precedence for model-visible text.
///
/// **Structured command preservation (P2.3):**
/// - **Direct transport**: `PromptOptions.command` flows through to
///   `PromptExecutionOptions.command`, preserved end-to-end.
/// - **HTTP / Unix transport**: `PromptOptions.command` is forwarded to
///   `PromptRequest.command` / JSON-RPC params, so session ingress sees
///   both the full text and the structured hint.
///
/// **Future**: move the command/input concatenation to the session
/// ingress layer so that `message` is always the authoritative text and
/// `command` is a structured hint for diagnostics/routing.
pub(super) fn build_prompt_message(input: &str, command: Option<&str>) -> String {
    if let Some(cmd) = command {
        if input.trim().is_empty() {
            format!("/{}", cmd)
        } else {
            format!("/{} {}", cmd, input)
        }
    } else {
        input.to_string()
    }
}

/// Machine-readable result for `run --format json`: one JSON object with the
/// session id and the final text so scripts can consume it deterministically.
pub(super) fn print_json_prompt_result(session_id: &str, text: &str) {
    println!(
        "{}",
        serde_json::json!({
            "session_id": session_id,
            "text": text,
        })
    );
}

pub(super) fn print_assistant_messages(messages: &[agendao_client::MessageInfo]) {
    for msg in messages {
        if msg.role != "user" {
            for part in &msg.parts {
                if let Some(text) = part.text.as_deref() {
                    print!("{}", text);
                }
            }
        }
    }
    println!();
}
