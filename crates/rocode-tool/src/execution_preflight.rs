use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::attachment_metadata::{
    collect_attachments_from_metadata, strip_attachments_from_metadata,
};
use crate::{Metadata, ToolContext, ToolError, ToolRegistry, ToolResult};

pub const EXECUTION_PREFLIGHT_METADATA_KEY: &str = "preflight";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPreflightSeverity {
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPreflightStatus {
    Ready,
    Advisory,
    SoftWarn,
    HardFail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPreflightIssue {
    pub severity: ExecutionPreflightSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionPreflightMetadata {
    pub runner: String,
    pub subject: String,
    pub status: ExecutionPreflightStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ExecutionPreflightIssue>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
    #[serde(default)]
    pub attachment_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPreflightReport {
    pub runner: String,
    pub subject: String,
    pub output: String,
    pub metadata: Metadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ExecutionPreflightIssue>,
}

impl ExecutionPreflightReport {
    pub fn new(runner: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            runner: runner.into(),
            subject: subject.into(),
            output: String::new(),
            metadata: Metadata::new(),
            attachments: Vec::new(),
            issues: Vec::new(),
        }
    }

    pub fn from_tool_result(
        runner: impl Into<String>,
        subject: impl Into<String>,
        result: ToolResult,
    ) -> Self {
        let attachments = collect_attachments_from_metadata(&result.metadata);
        Self {
            runner: runner.into(),
            subject: subject.into(),
            output: result.output,
            metadata: strip_attachments_from_metadata(&result.metadata),
            attachments,
            issues: Vec::new(),
        }
    }

    pub fn add_issue(
        &mut self,
        severity: ExecutionPreflightSeverity,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.issues.push(ExecutionPreflightIssue {
            severity,
            code: code.into(),
            message: message.into(),
        });
    }

    pub fn advisory(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.add_issue(ExecutionPreflightSeverity::Advisory, code, message);
        self
    }

    pub fn soft_warn(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.add_issue(ExecutionPreflightSeverity::SoftWarn, code, message);
        self
    }

    #[allow(dead_code)]
    pub fn hard_fail(mut self, code: impl Into<String>, message: impl Into<String>) -> Self {
        self.add_issue(ExecutionPreflightSeverity::HardFail, code, message);
        self
    }

    pub fn status(&self) -> ExecutionPreflightStatus {
        if self
            .issues
            .iter()
            .any(|issue| matches!(issue.severity, ExecutionPreflightSeverity::HardFail))
        {
            ExecutionPreflightStatus::HardFail
        } else if self
            .issues
            .iter()
            .any(|issue| matches!(issue.severity, ExecutionPreflightSeverity::SoftWarn))
        {
            ExecutionPreflightStatus::SoftWarn
        } else if self
            .issues
            .iter()
            .any(|issue| matches!(issue.severity, ExecutionPreflightSeverity::Advisory))
        {
            ExecutionPreflightStatus::Advisory
        } else {
            ExecutionPreflightStatus::Ready
        }
    }

    pub fn has_guidance(&self) -> bool {
        !self.output.trim().is_empty() || !self.metadata.is_empty() || !self.attachments.is_empty()
    }

    #[allow(dead_code)]
    pub fn blocking_issue(&self) -> Option<&ExecutionPreflightIssue> {
        self.issues
            .iter()
            .find(|issue| matches!(issue.severity, ExecutionPreflightSeverity::HardFail))
    }

    #[allow(dead_code)]
    pub fn ensure_not_blocked(&self) -> Result<(), ToolError> {
        let Some(issue) = self.blocking_issue() else {
            return Ok(());
        };
        Err(ToolError::ExecutionError(format!(
            "execution preflight blocked {} [{}]: {}",
            self.subject, issue.code, issue.message
        )))
    }

    pub fn metadata_projection(&self) -> ExecutionPreflightMetadata {
        ExecutionPreflightMetadata {
            runner: self.runner.clone(),
            subject: self.subject.clone(),
            status: self.status(),
            issues: self.issues.clone(),
            output: self.output.clone(),
            metadata: self.metadata.clone(),
            attachment_count: self.attachments.len(),
        }
    }

    pub fn metadata_value(&self) -> serde_json::Value {
        serde_json::to_value(self.metadata_projection())
            .expect("execution preflight metadata projection should serialize")
    }

    pub fn attach_to_metadata(&self, metadata: &mut Metadata) {
        self.attach_to_metadata_with_key(metadata, EXECUTION_PREFLIGHT_METADATA_KEY);
    }

    pub fn attach_to_metadata_with_key(&self, metadata: &mut Metadata, key: &str) {
        metadata.insert(key.to_string(), self.metadata_value());
    }
}

#[async_trait]
pub trait ExecutionPreflightRunner {
    async fn run(&self, ctx: &ToolContext) -> Result<ExecutionPreflightReport, ToolError>;
}

pub async fn execute_registry_tool_execution_preflight(
    registry: &ToolRegistry,
    tool_id: &str,
    args: serde_json::Value,
    ctx: &ToolContext,
    runner: impl Into<String>,
    subject: impl Into<String>,
) -> Result<ExecutionPreflightReport, ToolError> {
    if registry.get(tool_id).await.is_none() {
        return Err(ToolError::ExecutionError(format!(
            "execution preflight requires `{}` tool but it is not registered",
            tool_id
        )));
    }

    let result = registry.execute(tool_id, args, ctx.clone()).await?;
    Ok(ExecutionPreflightReport::from_tool_result(
        runner, subject, result,
    ))
}

pub fn execution_preflight_from_value(
    value: &serde_json::Value,
) -> Option<ExecutionPreflightMetadata> {
    serde_json::from_value(value.clone()).ok()
}

pub fn execution_preflight_from_metadata(
    metadata: &Metadata,
) -> Option<ExecutionPreflightMetadata> {
    execution_preflight_from_value(metadata.get(EXECUTION_PREFLIGHT_METADATA_KEY)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_highest_severity_issue() {
        let report = ExecutionPreflightReport::new("read", "/tmp/a")
            .advisory("registry_unavailable", "registry unavailable")
            .soft_warn("attachment_missing", "attachment missing");
        assert_eq!(report.status(), ExecutionPreflightStatus::SoftWarn);

        let report = report.hard_fail("blocked", "blocked");
        assert_eq!(report.status(), ExecutionPreflightStatus::HardFail);
    }

    #[test]
    fn attach_to_metadata_only_projects_execution_report() {
        let mut report = ExecutionPreflightReport::new("read", "/tmp/sample.pdf");
        report.output = "PDF read successfully".to_string();
        report
            .metadata
            .insert("mime".to_string(), serde_json::json!("application/pdf"));
        report.attachments.push(serde_json::json!({
            "mime": "application/pdf",
            "filename": "sample.pdf",
            "url": "data:application/pdf;base64,AA=="
        }));

        let mut metadata = Metadata::new();
        report.attach_to_metadata(&mut metadata);

        assert_eq!(metadata["preflight"]["status"], serde_json::json!("ready"));
        assert_eq!(
            metadata["preflight"]["attachment_count"],
            serde_json::json!(1)
        );
        assert!(metadata["preflight"]["metadata"]
            .get("attachments")
            .is_none());
        assert!(!metadata.contains_key("attachments"));
        assert!(!metadata.contains_key("attachment"));
    }

    #[test]
    fn execution_preflight_round_trips_from_metadata() {
        let mut report = ExecutionPreflightReport::new("read", "/tmp/sample.pdf");
        report.output = "PDF read successfully".to_string();
        report
            .metadata
            .insert("mime".to_string(), serde_json::json!("application/pdf"));
        report.add_issue(
            ExecutionPreflightSeverity::SoftWarn,
            "attachment_missing",
            "attachment payload missing",
        );

        let mut metadata = Metadata::new();
        report.attach_to_metadata(&mut metadata);

        let parsed = execution_preflight_from_metadata(&metadata)
            .expect("execution preflight metadata should parse");

        assert_eq!(parsed.runner, "read");
        assert_eq!(parsed.subject, "/tmp/sample.pdf");
        assert_eq!(parsed.status, ExecutionPreflightStatus::SoftWarn);
        assert_eq!(parsed.issues.len(), 1);
        assert_eq!(parsed.metadata["mime"], "application/pdf");
    }

    #[test]
    fn ensure_not_blocked_returns_explicit_execution_error() {
        let report =
            ExecutionPreflightReport::new("read", "/tmp/sample.pdf").hard_fail("blocked", "denied");
        let error = report
            .ensure_not_blocked()
            .expect_err("hard fail should block execution");
        assert!(error.to_string().contains("execution preflight blocked"));
    }
}
