use agendao_agent::AgentExecutor;

pub(crate) async fn stream_prompt_to_text(
    executor: &mut AgentExecutor,
    prompt: &str,
) -> anyhow::Result<String> {
    executor
        .execute_text_response(prompt.to_string())
        .await
        .map_err(|err| anyhow::anyhow!("{}", err))
}
