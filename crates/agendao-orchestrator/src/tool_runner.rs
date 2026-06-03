use crate::traits::ToolExecutor;
use crate::types::{ExecutionContext, ToolOutput};
use crate::ToolExecError;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolCallInput {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolCallOutput {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
    pub is_error: bool,
    pub title: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone)]
pub struct ToolRunner {
    executor: Arc<dyn ToolExecutor>,
}

impl ToolRunner {
    pub fn new(executor: Arc<dyn ToolExecutor>) -> Self {
        Self { executor }
    }

    pub fn repair_tool_call_name(name: &str, available_tools: &[String]) -> Option<String> {
        if available_tools.iter().any(|tool| tool == name) {
            return None;
        }

        let lower = name.to_ascii_lowercase();
        if lower != name && available_tools.iter().any(|tool| tool == &lower) {
            return Some(lower);
        }

        None
    }

    pub async fn execute_tool_call(
        &self,
        call: ToolCallInput,
        exec_ctx: &ExecutionContext,
    ) -> ToolCallOutput {
        let available = self.executor.list_ids().await;
        let repaired_name =
            Self::repair_tool_call_name(&call.name, &available).unwrap_or(call.name.clone());

        let result = self
            .executor
            .execute(&repaired_name, call.arguments, exec_ctx)
            .await;

        Self::to_output(&call.id, &repaired_name, result)
    }

    fn to_output(
        tool_call_id: &str,
        tool_name: &str,
        result: Result<ToolOutput, ToolExecError>,
    ) -> ToolCallOutput {
        match result {
            Ok(output) => ToolCallOutput {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                content: output.output,
                is_error: output.is_error,
                title: output.title,
                metadata: output.metadata,
            },
            Err(error) => ToolCallOutput {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                content: error.to_string(),
                is_error: true,
                title: None,
                metadata: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolRunner;

    #[test]
    fn repair_tool_call_name_fixes_case_when_lower_tool_exists() {
        let available = vec!["read".to_string(), "write".to_string()];
        let repaired = ToolRunner::repair_tool_call_name("Read", &available);
        assert_eq!(repaired, Some("read".to_string()));
    }

    #[test]
    fn repair_tool_call_name_preserves_unknown_tool_name() {
        let available = vec!["read".to_string(), "invalid".to_string()];
        let repaired = ToolRunner::repair_tool_call_name("missing_tool", &available);
        assert_eq!(repaired, None);
    }
}
