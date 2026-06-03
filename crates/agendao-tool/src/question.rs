use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use crate::{Tool, ToolContext, ToolError, ToolResult};

pub struct QuestionTool;

impl QuestionTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct QuestionInput {
    #[serde(rename = "questions")]
    questions: Vec<QuestionDef>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QuestionDef {
    #[serde(rename = "question")]
    question: String,
    #[serde(rename = "header")]
    header: Option<String>,
    #[serde(rename = "options", default)]
    options: Vec<QuestionOption>,
    #[serde(rename = "multiple", default)]
    multiple: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct QuestionOption {
    #[serde(rename = "label")]
    label: String,
    #[serde(rename = "description", default)]
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QuestionResponse {
    answers: Vec<String>,
}

#[async_trait]
impl Tool for QuestionTool {
    fn id(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user clarifying questions during execution. Use to gather preferences, clarify ambiguous requests, or get decisions on implementation choices."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The complete question to ask"
                            },
                            "header": {
                                "type": "string",
                                "description": "Short label for the question (max 30 chars)"
                            },
                            "multiple": {
                                "type": "boolean",
                                "default": false,
                                "description": "Allow selecting multiple options"
                            },
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {"type": "string"},
                                        "description": {"type": "string"}
                                    },
                                    "required": ["label"]
                                },
                                "description": "Available choices for the user"
                            }
                        },
                        "required": ["question"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let input = parse_question_input(args)?;

        let context_questions = input
            .questions
            .iter()
            .map(|q| crate::QuestionDef {
                question: q.question.clone(),
                header: q.header.clone(),
                options: q
                    .options
                    .iter()
                    .map(|opt| crate::QuestionOption {
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                    })
                    .collect(),
                multiple: q.multiple,
            })
            .collect::<Vec<_>>();

        let answers_by_question = match ctx.question(context_questions).await {
            Ok(answers) => answers,
            Err(ToolError::ExecutionError(msg))
                if msg.contains("Question callback not configured") =>
            {
                let mut manual_answers = Vec::with_capacity(input.questions.len());
                for q in &input.questions {
                    manual_answers.push(ask_question(q)?);
                }
                manual_answers
            }
            Err(e) => return Err(e),
        };

        let mut all_answers: Vec<String> = Vec::new();
        let mut display_fields = Vec::new();
        for (idx, q) in input.questions.iter().enumerate() {
            let answers = answers_by_question.get(idx).cloned().unwrap_or_default();
            let answer_text = answers.join(", ");
            all_answers.extend(answers);
            display_fields.push(serde_json::json!({
                "key": q.question,
                "value": answer_text,
            }));
        }

        let response = QuestionResponse {
            answers: all_answers.clone(),
        };

        let output = serde_json::to_string_pretty(&response)
            .unwrap_or_else(|_| format!("{:?}", response.answers));

        // Build display hints for TUI rendering
        let mut metadata = std::collections::HashMap::new();

        metadata.insert(
            "display.fields".to_string(),
            serde_json::Value::Array(display_fields),
        );

        // display.summary
        let summary = if input.questions.len() == 1 {
            "1 question answered".to_string()
        } else {
            format!("{} questions answered", input.questions.len())
        };
        metadata.insert(
            "display.summary".to_string(),
            serde_json::Value::String(summary),
        );

        Ok(ToolResult {
            title: "User response received".to_string(),
            output,
            metadata,
            truncated: false,
        })
    }
}

fn ask_question(q: &QuestionDef) -> Result<Vec<String>, ToolError> {
    println!();

    if let Some(ref header) = q.header {
        println!("┌─ {} ─────────────────", header);
    } else {
        println!("┌─ Question ─────────────────");
    }
    println!("│");
    println!("│ {}", q.question);
    println!("│");

    if q.options.is_empty() {
        println!("└─ Type your answer: ");
        print!("> ");
        io::stdout()
            .flush()
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

        let stdin = io::stdin();
        let mut answer = String::new();
        stdin
            .lock()
            .read_line(&mut answer)
            .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

        return Ok(vec![answer.trim().to_string()]);
    }

    println!("│ Options:");
    for (i, opt) in q.options.iter().enumerate() {
        let num = i + 1;
        if let Some(ref desc) = opt.description {
            println!("│   {}. {} - {}", num, opt.label, desc);
        } else {
            println!("│   {}. {}", num, opt.label);
        }
    }
    println!("│");

    if q.multiple {
        println!("└─ Enter choices (comma-separated, e.g., 1,3): ");
    } else {
        println!("└─ Enter your choice: ");
    }

    print!("> ");
    io::stdout()
        .flush()
        .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

    let stdin = io::stdin();
    let mut input = String::new();
    stdin
        .lock()
        .read_line(&mut input)
        .map_err(|e| ToolError::ExecutionError(e.to_string()))?;

    let input = input.trim();
    let answers: Vec<String> = input
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }

            if let Ok(num) = s.parse::<usize>() {
                if num > 0 && num <= q.options.len() {
                    return Some(q.options[num - 1].label.clone());
                }
            }

            Some(s.to_string())
        })
        .collect();

    if answers.is_empty() && !q.options.is_empty() {
        return Ok(vec![q.options[0].label.clone()]);
    }

    Ok(answers)
}

impl Default for QuestionTool {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_question_input(args: serde_json::Value) -> Result<QuestionInput, ToolError> {
    if let Ok(input) = serde_json::from_value::<QuestionInput>(args.clone()) {
        return Ok(input);
    }

    let obj = args
        .as_object()
        .ok_or_else(|| ToolError::InvalidArguments("question input must be an object".into()))?;
    let questions_value = obj
        .get("questions")
        .ok_or_else(|| ToolError::InvalidArguments("questions is required".into()))?;
    let questions = parse_questions_value(questions_value).map_err(ToolError::InvalidArguments)?;
    Ok(QuestionInput { questions })
}

fn parse_questions_value(value: &serde_json::Value) -> Result<Vec<QuestionDef>, String> {
    match value {
        serde_json::Value::Array(_) => serde_json::from_value::<Vec<QuestionDef>>(value.clone())
            .map_err(|e| format!("failed to parse questions array: {}", e)),
        serde_json::Value::Object(_) => serde_json::from_value::<QuestionDef>(value.clone())
            .map(|q| vec![q])
            .map_err(|e| format!("failed to parse question object: {}", e)),
        serde_json::Value::String(raw) => {
            if let Ok(list) = serde_json::from_str::<Vec<QuestionDef>>(raw) {
                return Ok(list);
            }
            if let Ok(single) = serde_json::from_str::<QuestionDef>(raw) {
                return Ok(vec![single]);
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(raw) {
                return parse_questions_value(&parsed);
            }
            Err("questions must be an array/object or a JSON string representing them".into())
        }
        _ => Err("questions must be an array/object or a JSON string".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_question_input, QuestionInput, QuestionTool};
    use crate::{Tool, ToolContext};

    #[test]
    fn parse_question_input_accepts_questions_array() {
        let input = parse_question_input(serde_json::json!({
            "questions": [
                { "question": "继续吗？", "options": [{ "label": "是" }, { "label": "否" }] }
            ]
        }))
        .expect("questions array should parse");
        assert_eq!(input.questions.len(), 1);
        assert_eq!(input.questions[0].question, "继续吗？");
    }

    #[test]
    fn parse_question_input_accepts_stringified_questions_array() {
        let raw = r#"[{"question":"继续吗？","options":[{"label":"是"},{"label":"否"}]}]"#;
        let input = parse_question_input(serde_json::json!({
            "questions": raw
        }))
        .expect("stringified questions should parse");
        assert_eq!(input.questions.len(), 1);
        assert_eq!(input.questions[0].question, "继续吗？");
    }

    #[test]
    fn parse_question_input_rejects_non_collection_questions() {
        let err = parse_question_input(serde_json::json!({ "questions": 1 }))
            .expect_err("numeric questions should fail");
        let msg = err.to_string();
        assert!(msg.contains("questions must be an array/object"));
    }

    #[test]
    fn parse_question_input_still_parses_direct_shape() {
        let parsed: QuestionInput = parse_question_input(serde_json::json!({
            "questions": [{ "question": "Q1" }]
        }))
        .expect("direct shape should parse");
        assert_eq!(parsed.questions.len(), 1);
    }

    #[tokio::test]
    async fn question_tool_uses_ctx_callback_when_available() {
        let tool = QuestionTool::new();
        let ctx = ToolContext::new("s".to_string(), "m".to_string(), ".".to_string())
            .with_ask_question(|questions| async move {
                assert_eq!(questions.len(), 1);
                Ok(vec![vec!["确认计划".to_string()]])
            });

        let result = tool
            .execute(
                serde_json::json!({
                    "questions": [
                        {
                            "question": "计划已生成并通过自我审查。您希望如何继续？",
                            "options": [{ "label": "确认计划" }, { "label": "需要修改" }]
                        }
                    ]
                }),
                ctx,
            )
            .await
            .expect("question execution should succeed");

        assert!(result.output.contains("确认计划"));
        assert_eq!(
            result
                .metadata
                .get("display.summary")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "1 question answered"
        );
    }
}
