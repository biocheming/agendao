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
