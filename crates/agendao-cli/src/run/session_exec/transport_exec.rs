use super::message_io::build_prompt_message;

pub(in crate::run) async fn run_cli_prompt_transport(
    transport: &agendao_client::FrontendTransport,
    input: &str,
    command: Option<&str>,
    model: Option<&str>,
    agent: Option<&str>,
    variant: Option<&str>,
) -> anyhow::Result<()> {
    let session_id = agendao_core::id::create(agendao_core::id::Prefix::Session, false, None);
    let message = build_prompt_message(input, command);
    let response = transport
        .prompt(
            &session_id,
            &message,
            agendao_client::transport::PromptOptions {
                agent_id: agent.map(|s| s.to_string()),
                model: model.map(|s| s.to_string()),
                variant: variant.map(|s| s.to_string()),
                source_origin: Some(agendao_types::MessageSourceOrigin::Operator),
                source_surface: Some(agendao_types::MessageSourceSurface::Cli),
                ..Default::default()
            },
        )
        .await?;
    println!("{}", response.text);
    Ok(())
}
