use anyhow::Error;
use agendao_session::SessionMessage;

pub(super) fn attach_provider_diagnostic_from_error(
    assistant: &mut SessionMessage,
    error: &Error,
    provider_id: &str,
    model_id: Option<&str>,
) {
    match agendao_session::prompt::provider_failure_from_anyhow(error) {
        Some(agendao_session::prompt::PromptProviderFailure::TypedSummary(summary)) => {
            summary.attach_to_metadata(&mut assistant.metadata);
            if let Some(diagnostic) = summary.provider_diagnostic.as_ref() {
                diagnostic.attach_to_metadata(&mut assistant.metadata);
            }
        }
        Some(agendao_session::prompt::PromptProviderFailure::UntypedMessage(message)) => {
            if let Some(summary) = agendao_provider::provider_diagnostic_from_error_text(
                provider_id,
                model_id,
                &message,
            ) {
                summary.attach_to_metadata(&mut assistant.metadata);
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_provider_diagnostic_from_error_prefers_typed_summary() {
        let mut assistant = agendao_session::SessionMessage::assistant("session-1".to_string());
        let error = anyhow::Error::new(agendao_session::prompt::PromptError::ProviderFailure(
            agendao_orchestrator::runtime::events::ModelFailure::Provider(
                agendao_provider::ProviderErrorSummary {
                    kind: agendao_provider::ProviderErrorKind::InvalidRequest,
                    provider_id: "deepseek".to_string(),
                    model_id: Some("deepseek-reasoner".to_string()),
                    message: "missing replay".to_string(),
                    status_code: Some(400),
                    standard_code: agendao_provider::error_code::StandardErrorCode::InvalidRequest,
                    retryable: false,
                    provider_diagnostic: Some(agendao_provider::ProviderDiagnosticSummary {
                        severity: agendao_provider::ProviderDiagnosticSeverity::HardFail,
                        source: agendao_provider::ProviderDiagnosticSource::RequestValidation,
                        code: "thinking_replay_missing".to_string(),
                        provider_id: "deepseek".to_string(),
                        model_id: Some("deepseek-reasoner".to_string()),
                        message: "missing replay".to_string(),
                    }),
                },
            ),
        ));

        attach_provider_diagnostic_from_error(
            &mut assistant,
            &error,
            "deepseek",
            Some("deepseek-reasoner"),
        );

        let summary = agendao_provider::provider_error_summary_from_metadata(&assistant.metadata)
            .expect("typed provider error summary should be attached to assistant metadata");
        assert_eq!(summary.provider_id, "deepseek");
        assert_eq!(summary.status_code, Some(400));
        let diagnostic = agendao_provider::provider_diagnostic_from_metadata(&assistant.metadata)
            .expect("provider diagnostic should be attached for legacy consumers");
        assert_eq!(diagnostic.code, "thinking_replay_missing");
    }

    #[test]
    fn attach_provider_diagnostic_from_error_uses_untyped_provider_message_fallback() {
        let mut assistant = agendao_session::SessionMessage::assistant("session-1".to_string());
        let error = anyhow::Error::new(agendao_session::prompt::PromptError::ProviderFailure(
            agendao_orchestrator::runtime::events::ModelFailure::Message(
                "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
                    .to_string(),
            ),
        ));

        attach_provider_diagnostic_from_error(
            &mut assistant,
            &error,
            "deepseek",
            Some("deepseek-reasoner"),
        );

        assert!(
            agendao_provider::provider_error_summary_from_metadata(&assistant.metadata).is_none()
        );
        let diagnostic = agendao_provider::provider_diagnostic_from_metadata(&assistant.metadata)
            .expect("untyped provider message should still attach fallback diagnostic");
        assert_eq!(diagnostic.code, "thinking_replay_rejected");
    }

    #[test]
    fn attach_provider_diagnostic_from_error_ignores_untyped_anyhow_text() {
        let mut assistant = agendao_session::SessionMessage::assistant("session-1".to_string());
        let error = anyhow::anyhow!(
            "provider `deepseek` rejected the request because thinking-mode reasoning replay was missing or incompatible: 400 Bad Request"
        );

        attach_provider_diagnostic_from_error(
            &mut assistant,
            &error,
            "deepseek",
            Some("deepseek-reasoner"),
        );

        assert!(
            agendao_provider::provider_error_summary_from_metadata(&assistant.metadata).is_none()
        );
        assert!(agendao_provider::provider_diagnostic_from_metadata(&assistant.metadata).is_none());
    }
}
