use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{TodoItemData, Tool, ToolContext, ToolError, ToolResult};

pub struct TodoReadTool;

pub struct TodoWriteTool;

#[derive(Debug, Serialize, Deserialize)]
struct TodoReadInput {
    #[serde(default, alias = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TodoWriteInput {
    todos: Vec<TodoWriteItem>,
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TodoWriteItem {
    id: Option<String>,
    content: String,
    status: Option<String>,
    priority: Option<String>,
}

#[async_trait]
impl Tool for TodoReadTool {
    fn id(&self) -> &str {
        "todoread"
    }

    fn description(&self) -> &str {
        "Read the current todo list for the session. Returns all todo items with their status."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Optional session ID. If not provided, uses current session."
                }
            }
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: TodoReadInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id.clone());

        ctx.ask_permission(
            crate::PermissionRequest::new("todoread")
                .with_metadata("session_id", serde_json::json!(&session_id))
                .always_allow(),
        )
        .await?;

        let todos = ctx.do_todo_get().await?;

        let output = format_todos_from_data(&todos);

        let todos_json: Vec<serde_json::Value> = todos
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "status": t.status,
                    "priority": t.priority
                })
            })
            .collect();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("todos".to_string(), serde_json::json!(todos_json));
        metadata.insert("count".to_string(), serde_json::json!(todos.len()));

        Ok(ToolResult {
            title: format!("Todo List ({} items)", todos.len()),
            output,
            metadata,
            truncated: false,
        })
    }
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn id(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Create or update the todo list for the session. Use this to track tasks and progress. Only call when the list actually changes; do not repeat identical updates."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "content": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
                            "priority": { "type": "string" }
                        },
                        "required": ["content"]
                    },
                    "description": "List of todo items"
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session ID"
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input: TodoWriteInput =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;

        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| ctx.session_id.clone());

        ctx.ask_permission(
            crate::PermissionRequest::new("todowrite")
                .with_metadata("session_id", serde_json::json!(&session_id))
                .with_metadata("count", serde_json::json!(input.todos.len()))
                .always_allow(),
        )
        .await?;

        let mut new_todos: Vec<TodoItemData> = Vec::new();

        for item in input.todos {
            let status =
                normalize_todo_status(item.status.as_deref().unwrap_or("pending")).to_string();

            let _id = item
                .id
                .unwrap_or_else(|| format!("todo_{}", &uuid::Uuid::new_v4().to_string()[..8]));

            new_todos.push(TodoItemData {
                content: item.content,
                status,
                priority: normalize_todo_priority(item.priority.as_deref().unwrap_or("medium"))
                    .to_string(),
            });
        }

        let existing_todos = ctx.do_todo_get().await?;
        if todos_equivalent(&existing_todos, &new_todos) {
            tracing::info!(
                session_id = %ctx.session_id,
                count = new_todos.len(),
                "todowrite deduplicated unchanged todo payload"
            );
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("count".to_string(), serde_json::json!(new_todos.len()));
            metadata.insert("no_op".to_string(), serde_json::json!(true));
            metadata.insert(
                "display.summary".to_string(),
                serde_json::json!(format!(
                    "Todo list unchanged ({} items), skipped duplicate update",
                    new_todos.len()
                )),
            );

            return Ok(ToolResult {
                title: format!("Todo List unchanged ({} items)", new_todos.len()),
                output: "No todo changes detected. Skipped duplicate update.".to_string(),
                metadata,
                truncated: false,
            });
        }

        ctx.do_todo_update(new_todos.clone()).await?;

        let output = format_todos_from_data(&new_todos);

        let todos_json: Vec<serde_json::Value> = new_todos
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "status": t.status,
                    "priority": t.priority
                })
            })
            .collect();

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("todos".to_string(), serde_json::json!(todos_json));
        metadata.insert("count".to_string(), serde_json::json!(new_todos.len()));

        Ok(ToolResult {
            title: format!("Updated Todo List ({} items)", new_todos.len()),
            output,
            metadata,
            truncated: false,
        })
    }
}

fn todos_equivalent(current: &[TodoItemData], next: &[TodoItemData]) -> bool {
    current.len() == next.len()
        && current.iter().zip(next.iter()).all(|(a, b)| {
            normalize_todo_content(&a.content) == normalize_todo_content(&b.content)
                && normalize_todo_status(&a.status) == normalize_todo_status(&b.status)
                && normalize_todo_priority(&a.priority) == normalize_todo_priority(&b.priority)
        })
}

fn normalize_todo_content(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_todo_status(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "in_progress" | "in-progress" | "in progress" | "doing" => "in_progress",
        "completed" | "done" => "completed",
        "cancelled" | "canceled" => "cancelled",
        _ => "pending",
    }
}

fn normalize_todo_priority(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "high" => "high",
        "low" => "low",
        _ => "medium",
    }
}

fn format_todos_from_data(todos: &[TodoItemData]) -> String {
    if todos.is_empty() {
        return "No todos in the list.".to_string();
    }

    let mut output = String::new();
    output.push_str("# Todo List\n\n");

    for (i, todo) in todos.iter().enumerate() {
        let status_icon = match todo.status.as_str() {
            "in_progress" => "🔄",
            "completed" => "✅",
            _ => "⬜",
        };

        let priority_str = if !todo.priority.is_empty() {
            format!(" [{}]", todo.priority)
        } else {
            String::new()
        };

        output.push_str(&format!(
            "{} todo_{}{}\n   {}\n\n",
            status_icon, i, priority_str, todo.content
        ));
    }

    let pending = todos.iter().filter(|t| t.status == "pending").count();
    let in_progress = todos.iter().filter(|t| t.status == "in_progress").count();
    let completed = todos.iter().filter(|t| t.status == "completed").count();

    output.push_str(&format!(
        "Summary: {} pending, {} in progress, {} completed",
        pending, in_progress, completed
    ));

    output
}

impl Default for TodoReadTool {
    fn default() -> Self {
        Self
    }
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn todowrite_skips_duplicate_updates() {
        let update_calls = Arc::new(AtomicUsize::new(0));
        let update_calls_clone = Arc::clone(&update_calls);

        let existing = vec![TodoItemData {
            content: "分析 t2.html 当前内容和结构".to_string(),
            status: "completed".to_string(),
            priority: "high".to_string(),
        }];

        let ctx = ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            ".".to_string(),
        )
        .with_todo_get({
            let existing = existing.clone();
            move |_| {
                let existing = existing.clone();
                async move { Ok(existing) }
            }
        })
        .with_todo_update(move |_, _| {
            let update_calls_clone = Arc::clone(&update_calls_clone);
            async move {
                update_calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = TodoWriteTool
            .execute(
                serde_json::json!({
                    "todos": [{
                        "content": "分析 t2.html 当前内容和结构",
                        "status": "completed",
                        "priority": "high"
                    }]
                }),
                ctx,
            )
            .await
            .expect("todowrite should succeed");

        assert_eq!(update_calls.load(Ordering::SeqCst), 0);
        assert_eq!(result.metadata.get("no_op"), Some(&serde_json::json!(true)));
        assert!(
            result.output.contains("No todo changes detected"),
            "expected no-op output"
        );
    }

    #[tokio::test]
    async fn todowrite_updates_when_content_changes() {
        let update_calls = Arc::new(AtomicUsize::new(0));
        let update_calls_clone = Arc::clone(&update_calls);

        let existing = vec![TodoItemData {
            content: "分析 t2.html 当前内容和结构".to_string(),
            status: "pending".to_string(),
            priority: "high".to_string(),
        }];

        let ctx = ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            ".".to_string(),
        )
        .with_todo_get({
            let existing = existing.clone();
            move |_| {
                let existing = existing.clone();
                async move { Ok(existing) }
            }
        })
        .with_todo_update(move |_, _| {
            let update_calls_clone = Arc::clone(&update_calls_clone);
            async move {
                update_calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = TodoWriteTool
            .execute(
                serde_json::json!({
                    "todos": [{
                        "content": "分析 t2.html 当前内容和结构",
                        "status": "completed",
                        "priority": "high"
                    }]
                }),
                ctx,
            )
            .await
            .expect("todowrite should succeed");

        assert_eq!(update_calls.load(Ordering::SeqCst), 1);
        assert_ne!(result.metadata.get("no_op"), Some(&serde_json::json!(true)));
    }

    #[tokio::test]
    async fn todowrite_skips_duplicates_with_whitespace_or_case_differences() {
        let update_calls = Arc::new(AtomicUsize::new(0));
        let update_calls_clone = Arc::clone(&update_calls);

        let existing = vec![TodoItemData {
            content: "Analyze   t2.html content".to_string(),
            status: "in_progress".to_string(),
            priority: "HIGH".to_string(),
        }];

        let ctx = ToolContext::new(
            "session-1".to_string(),
            "message-1".to_string(),
            ".".to_string(),
        )
        .with_todo_get({
            let existing = existing.clone();
            move |_| {
                let existing = existing.clone();
                async move { Ok(existing) }
            }
        })
        .with_todo_update(move |_, _| {
            let update_calls_clone = Arc::clone(&update_calls_clone);
            async move {
                update_calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });

        let result = TodoWriteTool
            .execute(
                serde_json::json!({
                    "todos": [{
                        "content": "Analyze t2.html    content",
                        "status": "in progress",
                        "priority": "high"
                    }]
                }),
                ctx,
            )
            .await
            .expect("todowrite should succeed");

        assert_eq!(update_calls.load(Ordering::SeqCst), 0);
        assert_eq!(result.metadata.get("no_op"), Some(&serde_json::json!(true)));
    }
}
