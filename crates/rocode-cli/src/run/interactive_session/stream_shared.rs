use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct InteractiveSessionStream {
    pub(super) sse_rx: mpsc::UnboundedReceiver<CliServerEvent>,
    pub(super) sse_cancel: CancellationToken,
}

pub(super) async fn bootstrap_interactive_stream(
    server_url: &str,
    server_session_id: &str,
    api_client: &Arc<CliApiClient>,
    runtime: &CliExecutionRuntime,
    local: bool,
    local_state: &Option<Arc<rocode_server::ServerState>>,
) -> InteractiveSessionStream {
    let (sse_tx, sse_rx) = mpsc::unbounded_channel::<CliServerEvent>();
    let sse_cancel = CancellationToken::new();

    if local {
        // Direct mode: no server, no SSE. Run a local poll loop that
        // translates session state changes into frontend events.
        if let Some(state) = local_state {
            let session_id = server_session_id.to_string();
            let state = Arc::clone(state);
            let tx = sse_tx.clone();
            let cancel = sse_cancel.clone();
            tokio::spawn(async move {
                local_poll_loop(&state, &session_id, tx, cancel).await;
            });
        }
        return InteractiveSessionStream { sse_rx, sse_cancel };
    }

    let _sse_handle = event_stream::spawn_sse_subscriber(
        server_url.to_string(),
        server_session_id.to_string(),
        sse_tx,
        sse_cancel.clone(),
    );

    cli_refresh_server_info(
        api_client,
        &runtime.frontend_projection,
        Some(server_session_id),
    )
    .await;

    InteractiveSessionStream { sse_rx, sse_cancel }
}

/// Poll local session state and emit synthetic SSE-compatible events.
/// Uses message count for SessionUpdated; uses runtime run status
/// (via RuntimeControlRegistry) to decide when the run is truly idle.
async fn local_poll_loop(
    state: &Arc<rocode_server::ServerState>,
    session_id: &str,
    tx: mpsc::UnboundedSender<CliServerEvent>,
    cancel: CancellationToken,
) {
    let mut last_message_count = 0usize;
    let mut stale_ticks = 0u32;
    let mut was_idle = false;
    let mut pending_question_ids = HashSet::new();
    let mut pending_permission_ids: HashMap<String, String> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(300));

    // Emit Busy upfront so the interactive loop shows a spinner.
    let _ = tx.send(CliServerEvent::SessionBusy {
        session_id: session_id.to_string(),
    });

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {},
        }

        // Read messages for new-content detection.
        let Ok(messages) = rocode_server::local_list_messages(
            Arc::clone(state),
            session_id,
            None,
            None,
        )
        .await
        else {
            break;
        };

        if let Ok(questions) = rocode_server::local_list_questions(Arc::clone(state)).await {
            let mut current_question_ids = HashSet::new();
            for question in questions
                .into_iter()
                .filter(|question| question.session_id == session_id)
            {
                current_question_ids.insert(question.id.clone());
                if pending_question_ids.insert(question.id.clone()) {
                    let questions_json = serde_json::to_value(local_question_defs_from_info(&question))
                        .unwrap_or(serde_json::Value::Null);
                    let _ = tx.send(CliServerEvent::QuestionCreated {
                        request_id: question.id,
                        session_id: question.session_id,
                        questions_json,
                    });
                }
            }
            for resolved_id in pending_question_ids
                .iter()
                .filter(|id| !current_question_ids.contains(*id))
                .cloned()
                .collect::<Vec<_>>()
            {
                pending_question_ids.remove(&resolved_id);
                let _ = tx.send(CliServerEvent::QuestionResolved {
                    request_id: resolved_id,
                });
            }
        }

        if let Ok(permissions) = rocode_server::local_list_permissions(Arc::clone(state)).await {
            let mut current_permission_ids = HashMap::new();
            for permission in permissions
                .into_iter()
                .filter(|permission| permission.session_id == session_id)
            {
                current_permission_ids
                    .insert(permission.id.clone(), permission.session_id.clone());
                if !pending_permission_ids.contains_key(&permission.id) {
                    let info_json =
                        serde_json::to_value(&permission).unwrap_or(serde_json::Value::Null);
                    let _ = tx.send(CliServerEvent::PermissionRequested {
                        session_id: permission.session_id.clone(),
                        permission_id: permission.id.clone(),
                        info_json,
                    });
                }
            }
            for (resolved_id, resolved_session_id) in pending_permission_ids
                .iter()
                .filter(|(id, _)| !current_permission_ids.contains_key(*id))
                .map(|(id, session_id)| (id.clone(), session_id.clone()))
                .collect::<Vec<_>>()
            {
                let _ = tx.send(CliServerEvent::PermissionResolved {
                    session_id: resolved_session_id,
                    permission_id: resolved_id,
                });
            }
            pending_permission_ids = current_permission_ids;
        }

        let count = messages.len();
        if count > last_message_count {
            last_message_count = count;
            stale_ticks = 0;
            was_idle = false;
            let _ = tx.send(CliServerEvent::SessionUpdated {
                session_id: session_id.to_string(),
                source: Some("local_poll".to_string()),
            });
            continue;
        }

        // No new messages — check if the run is still in progress.
        // A missing or non-terminal finish on the last assistant message
        // means the run may be in a long tool call / thinking phase.
        let has_terminal_finish = messages
            .last()
            .filter(|m| m.role == "assistant")
            .and_then(|m| m.finish.as_deref())
            .map(|f| f != "tool_calls" && f != "unknown")
            .unwrap_or(false);

        if !has_terminal_finish {
            // Still running — long tool call or reasoning.
            stale_ticks = 0;
            if was_idle {
                was_idle = false;
                let _ = tx.send(CliServerEvent::SessionBusy {
                    session_id: session_id.to_string(),
                });
            }
            continue;
        }

        stale_ticks += 1;
        // After ~3s of silence + terminal finish, emit Idle.
        if stale_ticks >= 10 && !was_idle {
            was_idle = true;
            let _ = tx.send(CliServerEvent::SessionIdle {
                session_id: session_id.to_string(),
            });
        }
    }
}

fn local_question_defs_from_info(
    info: &crate::api_client::QuestionInfo,
) -> Vec<rocode_tool::QuestionDef> {
    if !info.items.is_empty() {
        return info
            .items
            .iter()
            .map(|item| rocode_tool::QuestionDef {
                question: item.question.clone(),
                header: item.header.clone(),
                options: item
                    .options
                    .iter()
                    .map(|option| rocode_tool::QuestionOption {
                        label: option.label.clone(),
                        description: option.description.clone(),
                    })
                    .collect(),
                multiple: item.multiple,
            })
            .collect();
    }

    info.questions
        .iter()
        .enumerate()
        .map(|(index, question)| rocode_tool::QuestionDef {
            question: question.clone(),
            header: None,
            options: info
                .options
                .as_ref()
                .and_then(|all| all.get(index))
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|label| rocode_tool::QuestionOption {
                    label,
                    description: None,
                })
                .collect(),
            multiple: false,
        })
        .collect()
}
