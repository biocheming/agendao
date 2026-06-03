use std::collections::HashMap;

pub async fn print_openapi_spec() -> anyhow::Result<()> {
    let mut paths: HashMap<String, serde_json::Map<String, serde_json::Value>> = HashMap::new();
    let operations: &[(&str, &str, &str)] = &[
        ("/health", "get", "health"),
        ("/event", "get", "eventSubscribe"),
        ("/path", "get", "pathsGet"),
        ("/vcs", "get", "vcsGet"),
        ("/command", "get", "commandList"),
        ("/agent", "get", "agentList"),
        ("/skill", "get", "skillList"),
        ("/lsp", "get", "lspStatus"),
        ("/formatter", "get", "formatterStatus"),
        ("/auth/{id}", "put", "authSet"),
        ("/auth/{id}", "delete", "authDelete"),
        ("/session", "get", "sessionList"),
        ("/session", "post", "sessionCreate"),
        ("/session/status", "get", "sessionStatus"),
        ("/session/{id}", "get", "sessionGet"),
        ("/session/{id}", "patch", "sessionUpdate"),
        ("/session/{id}", "delete", "sessionDelete"),
        ("/session/{id}/attached", "get", "sessionAttachedSessions"),
        ("/session/{id}/executions", "get", "sessionExecutions"),
        ("/session/{id}/recovery", "get", "sessionRecovery"),
        (
            "/session/{id}/recovery/execute",
            "post",
            "sessionRecoveryExecute",
        ),
        ("/session/{id}/todo", "get", "sessionTodo"),
        ("/session/{id}/fork", "post", "sessionFork"),
        ("/session/{id}/abort", "post", "sessionAbort"),
        (
            "/session/{id}/scheduler/stage/abort",
            "post",
            "sessionSchedulerStageAbort",
        ),
        ("/session/{id}/share", "post", "sessionShare"),
        ("/session/{id}/share", "delete", "sessionUnshare"),
        ("/session/{id}/archive", "post", "sessionArchive"),
        ("/session/{id}/title", "patch", "sessionSetTitle"),
        ("/session/{id}/permission", "patch", "sessionSetPermission"),
        ("/session/{id}/summary", "get", "sessionSummaryGet"),
        ("/session/{id}/summary", "patch", "sessionSummarySet"),
        ("/session/{id}/revert", "post", "sessionRevert"),
        ("/session/{id}/revert", "delete", "sessionRevertClear"),
        ("/session/{id}/unrevert", "post", "sessionUnrevert"),
        ("/session/{id}/compact", "post", "sessionCompaction"),
        ("/session/{id}/summarize", "post", "sessionSummarize"),
        ("/session/{id}/init", "post", "sessionInit"),
        ("/session/{id}/command", "post", "sessionCommand"),
        ("/session/{id}/shell", "post", "sessionShell"),
        ("/session/{id}/message", "get", "sessionMessageList"),
        ("/session/{id}/message", "post", "sessionMessageCreate"),
        ("/session/{id}/message/{msgID}", "get", "sessionMessageGet"),
        (
            "/session/{id}/message/{msgID}",
            "delete",
            "sessionMessageDelete",
        ),
        (
            "/session/{id}/message/{msgID}/part",
            "post",
            "sessionPartAdd",
        ),
        (
            "/session/{id}/message/{msgID}/part/{partID}",
            "patch",
            "sessionPartUpdate",
        ),
        (
            "/session/{id}/message/{msgID}/part/{partID}",
            "delete",
            "sessionPartDelete",
        ),
        ("/session/{id}/stream", "post", "sessionStream"),
        ("/session/{id}/prompt", "post", "sessionPrompt"),
        ("/session/{id}/prompt/abort", "post", "sessionPromptAbort"),
        ("/session/{id}/prompt_async", "post", "sessionPromptAsync"),
        ("/session/{id}/diff", "get", "sessionDiff"),
        ("/provider/", "get", "providerList"),
        ("/provider/refresh", "post", "providerRefresh"),
        ("/provider/auth", "get", "providerAuth"),
        (
            "/provider/{id}/oauth/authorize",
            "post",
            "providerOAuthAuthorize",
        ),
        (
            "/provider/{id}/oauth/callback",
            "post",
            "providerOAuthCallback",
        ),
        ("/config/", "get", "configGet"),
        ("/config/", "patch", "configPatch"),
        ("/config/providers", "get", "configProviderGet"),
        ("/mcp", "get", "mcpList"),
        ("/mcp", "post", "mcpAdd"),
        ("/mcp/{name}/connect", "post", "mcpConnect"),
        ("/mcp/{name}/disconnect", "post", "mcpDisconnect"),
        ("/mcp/{name}/auth", "post", "mcpAuthStart"),
        ("/mcp/{name}/auth", "delete", "mcpAuthDelete"),
        ("/mcp/{name}/auth/callback", "post", "mcpAuthCallback"),
        (
            "/mcp/{name}/auth/authenticate",
            "post",
            "mcpAuthAuthenticate",
        ),
        ("/file/read", "post", "fileRead"),
        ("/file/write", "post", "fileWrite"),
        ("/file/status", "get", "fileStatus"),
        ("/find", "post", "find"),
        ("/permission", "get", "permissionList"),
        ("/permission/{id}/reply", "post", "permissionReply"),
        ("/project", "get", "projectList"),
        ("/project/current", "get", "projectCurrent"),
        ("/project/current", "patch", "projectCurrentPatch"),
        ("/pty", "post", "ptyCreate"),
        ("/pty/{id}", "get", "ptyRead"),
        ("/pty/{id}", "delete", "ptyDelete"),
        ("/question", "get", "questionList"),
        ("/question", "post", "questionReply"),
        ("/tui/session", "get", "tuiSessionList"),
        ("/tui/session", "post", "tuiSessionCreate"),
        ("/global/event", "get", "globalEventSubscribe"),
    ];

    for (path, method, operation_id) in operations {
        let entry = paths.entry((*path).to_string()).or_default();
        entry.insert(
            (*method).to_string(),
            serde_json::json!({
                "operationId": operation_id,
                "responses": { "200": { "description": "OK" } },
                "x-codeSamples": [
                    {
                        "lang": "js",
                        "source": format!(
                            "import {{ createOpencodeClient }} from \"@opencode-ai/sdk\"\n\nconst client = createOpencodeClient()\nawait client.{}({{\n  ...\n}})",
                            operation_id
                        )
                    }
                ]
            }),
        );
    }

    let spec = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "AGENDAO API",
            "version": env!("CARGO_PKG_VERSION")
        },
        "paths": paths
    });
    println!("{}", serde_json::to_string_pretty(&spec)?);
    Ok(())
}
