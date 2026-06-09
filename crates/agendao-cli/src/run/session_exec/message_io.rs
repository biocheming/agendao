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
/// - **HTTP / Unix transport**: `PromptOptions.command` is present in
///   the `PromptOptions` struct but `send_prompt` does not yet forward
///   it to the wire `PromptRequest.command`.  The full `message` text
///   is always sent; only the structured hint is deferred to a future
///   transport protocol update.
///
/// **Future**: move the command/input concatenation to the session
/// ingress layer so that `message` is always the authoritative text and
/// `command` is a structured hint for diagnostics/routing across all
/// transport paths.
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
