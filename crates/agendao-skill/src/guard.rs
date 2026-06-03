use crate::discovery::is_valid_relative_skill_path;
use crate::methodology::SkillQualityRubric;
use crate::write::{
    build_skill_document, parse_skill_document, read_frontmatter_value, validate_skill_description,
    validate_skill_name,
};
use agendao_types::{SkillGuardReport, SkillGuardSeverity, SkillGuardStatus, SkillGuardViolation};

const MAX_SKILL_MARKDOWN_BYTES: usize = 256 * 1024;
const MAX_SUPPORTING_FILE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillGuardMode {
    Off,
    Warn,
    Enforce,
}

impl Default for SkillGuardMode {
    fn default() -> Self {
        Self::Warn
    }
}

#[derive(Debug, Default)]
pub struct SkillGuardEngine {
    mode: SkillGuardMode,
}

impl SkillGuardEngine {
    pub fn new() -> Self {
        Self {
            mode: SkillGuardMode::Warn,
        }
    }

    pub fn with_mode(mode: SkillGuardMode) -> Self {
        Self { mode }
    }

    pub fn mode(&self) -> SkillGuardMode {
        self.mode
    }

    pub fn evaluate_create(
        &self,
        skill_name: &str,
        description: &str,
        body: &str,
        duplicate_conflict: bool,
        scanned_at: i64,
    ) -> SkillGuardReport {
        let mut violations = Vec::new();

        if validate_skill_name(skill_name).is_err()
            || validate_skill_description(skill_name, description).is_err()
        {
            violations.push(error_violation(
                "frontmatter.required_fields",
                "skill create request must provide a valid `name` and `description`.",
                None,
            ));
        }

        let markdown = build_skill_document(
            &crate::write::build_create_frontmatter(skill_name, description, None)
                .expect("guard create frontmatter should be valid"),
            body,
        )
        .expect("guard create markdown should render");
        evaluate_markdown_content(&mut violations, skill_name, &markdown, Some(body));
        if duplicate_conflict {
            violations.push(error_violation(
                "duplicate.skill_name_conflict",
                format!(
                    "a skill named `{}` already exists in the resolved catalog.",
                    skill_name
                ),
                None,
            ));
        }

        finalize_report(self.mode, skill_name, scanned_at, violations)
    }

    pub fn evaluate_patch(
        &self,
        current_name: &str,
        next_name: &str,
        body: Option<&str>,
        duplicate_conflict: bool,
        scanned_at: i64,
    ) -> SkillGuardReport {
        let mut violations = Vec::new();
        if validate_skill_name(next_name).is_err() {
            violations.push(error_violation(
                "frontmatter.required_fields",
                "patch request would leave the skill without a valid `name`.",
                None,
            ));
        }
        if let Some(body) = body {
            evaluate_markdown_content(
                &mut violations,
                next_name,
                &build_skill_document(
                    &crate::write::build_create_frontmatter(next_name, "patched", None)
                        .expect("guard patch frontmatter should be valid"),
                    body,
                )
                .expect("guard patch markdown should render"),
                Some(body),
            );
        }
        if duplicate_conflict {
            violations.push(error_violation(
                "duplicate.skill_name_conflict",
                format!(
                    "patch would rename `{}` to `{}`, but that target name already exists.",
                    current_name, next_name
                ),
                None,
            ));
        }
        finalize_report(self.mode, next_name, scanned_at, violations)
    }

    pub fn evaluate_edit(
        &self,
        skill_name: &str,
        content: &str,
        duplicate_conflict: bool,
        scanned_at: i64,
    ) -> SkillGuardReport {
        let mut violations = Vec::new();
        match parse_skill_document(content) {
            Ok(document) => {
                let next_name = read_frontmatter_value(&document.frontmatter_lines, "name");
                let description =
                    read_frontmatter_value(&document.frontmatter_lines, "description");
                if next_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                    || description
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .is_none()
                {
                    violations.push(error_violation(
                        "frontmatter.required_fields",
                        "edited SKILL.md must include both `name` and `description` frontmatter.",
                        None,
                    ));
                }
            }
            Err(_) => violations.push(error_violation(
                "frontmatter.required_fields",
                "edited SKILL.md could not be parsed into a valid frontmatter/body document.",
                None,
            )),
        }
        evaluate_markdown_content(&mut violations, skill_name, content, None);
        if duplicate_conflict {
            violations.push(error_violation(
                "duplicate.skill_name_conflict",
                format!(
                    "edited content would conflict with another resolved skill named `{}`.",
                    skill_name
                ),
                None,
            ));
        }
        finalize_report(self.mode, skill_name, scanned_at, violations)
    }

    pub fn evaluate_supporting_file(
        &self,
        skill_name: &str,
        file_path: &str,
        content: &str,
        scanned_at: i64,
    ) -> SkillGuardReport {
        let mut violations = Vec::new();
        if !is_valid_relative_skill_path(file_path) || file_path.eq_ignore_ascii_case("SKILL.md") {
            violations.push(error_violation(
                "path.escape",
                format!(
                    "supporting file path `{}` escapes the allowed skill-relative sandbox.",
                    file_path
                ),
                Some(file_path.to_string()),
            ));
        }
        if content.len() > MAX_SUPPORTING_FILE_BYTES {
            violations.push(error_violation(
                "file.size_limit",
                format!(
                    "supporting file `{}` exceeds {} bytes.",
                    file_path, MAX_SUPPORTING_FILE_BYTES
                ),
                Some(file_path.to_string()),
            ));
        }
        evaluate_suspicious_content(&mut violations, content, Some(file_path));
        finalize_report(self.mode, skill_name, scanned_at, violations)
    }

    pub fn evaluate_imported_skill(
        &self,
        skill_name: &str,
        markdown_content: &str,
        supporting_files: &[(String, String)],
        duplicate_conflict: bool,
        scanned_at: i64,
    ) -> SkillGuardReport {
        let mut violations = Vec::new();
        evaluate_markdown_content(&mut violations, skill_name, markdown_content, None);
        if duplicate_conflict {
            violations.push(error_violation(
                "duplicate.skill_name_conflict",
                format!(
                    "imported skill `{}` conflicts with an existing non-managed skill.",
                    skill_name
                ),
                None,
            ));
        }
        for (file_path, content) in supporting_files {
            if !is_valid_relative_skill_path(file_path)
                || file_path.eq_ignore_ascii_case("SKILL.md")
            {
                violations.push(error_violation(
                    "path.escape",
                    format!(
                        "imported supporting file `{}` escapes the skill sandbox.",
                        file_path
                    ),
                    Some(file_path.clone()),
                ));
            }
            if content.len() > MAX_SUPPORTING_FILE_BYTES {
                violations.push(error_violation(
                    "file.size_limit",
                    format!(
                        "imported supporting file `{}` exceeds {} bytes.",
                        file_path, MAX_SUPPORTING_FILE_BYTES
                    ),
                    Some(file_path.clone()),
                ));
            }
            evaluate_suspicious_content(&mut violations, content, Some(file_path.as_str()));
        }
        finalize_report(self.mode, skill_name, scanned_at, violations)
    }
}

fn evaluate_markdown_content(
    violations: &mut Vec<SkillGuardViolation>,
    skill_name: &str,
    markdown_content: &str,
    body_hint: Option<&str>,
) {
    if markdown_content.len() > MAX_SKILL_MARKDOWN_BYTES {
        violations.push(error_violation(
            "content.size_limit",
            format!(
                "skill `{}` exceeds the markdown size limit of {} bytes.",
                skill_name, MAX_SKILL_MARKDOWN_BYTES
            ),
            Some("SKILL.md".to_string()),
        ));
    }
    let content = body_hint.unwrap_or(markdown_content);
    evaluate_suspicious_content(violations, content, Some("SKILL.md"));
    evaluate_methodology_quality(violations, skill_name, content);
}

fn evaluate_methodology_quality(
    violations: &mut Vec<SkillGuardViolation>,
    skill_name: &str,
    markdown_content: &str,
) {
    let rubric = SkillQualityRubric::default();
    let normalized = markdown_content.to_ascii_lowercase();
    let line_count = markdown_content.lines().count();
    if line_count > rubric.recommended_max_lines {
        violations.push(warn_violation(
            "quality.length_budget",
            format!(
                "skill `{}` is {} lines long; consider keeping the core SKILL under {} lines and moving detail into references or scripts.",
                skill_name, line_count, rubric.recommended_max_lines
            ),
            Some("SKILL.md".to_string()),
        ));
    }

    for rule in rubric.required_rules {
        let matched = rule
            .heading_hints
            .iter()
            .any(|hint| normalized.contains(&hint.to_ascii_lowercase()));
        if !matched {
            violations.push(warn_violation(
                &rule.rule_id,
                rule.message,
                Some("SKILL.md".to_string()),
            ));
        }
    }
}

fn evaluate_suspicious_content(
    violations: &mut Vec<SkillGuardViolation>,
    content: &str,
    file_path: Option<&str>,
) {
    let lower = content.to_ascii_lowercase();

    if lower.contains("ignore previous instructions")
        || lower.contains("ignore all previous instructions")
        || lower.contains("override system prompt")
        || lower.contains("<system>")
        || lower.contains("<developer>")
    {
        violations.push(warn_violation(
            "suspicious.inline_instruction_override",
            "content looks like it tries to override higher-priority instructions.",
            file_path.map(str::to_string),
        ));
    }

    if lower.contains("curl ")
        && (lower.contains("| sh") || lower.contains("|bash") || lower.contains("| bash"))
        || lower.contains("wget ")
            && (lower.contains("| sh") || lower.contains("|bash") || lower.contains("| bash"))
        || lower.contains("bash <(")
        || lower.contains("sh -c \"$(curl")
    {
        violations.push(error_violation(
            "suspicious.shell_autorun_snippet",
            "content contains an inline remote shell autorun snippet.",
            file_path.map(str::to_string),
        ));
    }

    if lower.contains("curl http")
        || lower.contains("curl https")
        || lower.contains("wget http")
        || lower.contains("wget https")
        || lower.contains("invoke-webrequest http")
        || lower.contains("fetch('http")
        || lower.contains("fetch(\"http")
    {
        violations.push(warn_violation(
            "suspicious.remote_fetch_without_context",
            "content performs remote fetching and should be reviewed for provenance/context.",
            file_path.map(str::to_string),
        ));
    }
}

fn finalize_report(
    mode: SkillGuardMode,
    skill_name: &str,
    scanned_at: i64,
    violations: Vec<SkillGuardViolation>,
) -> SkillGuardReport {
    let status = if violations.is_empty() {
        SkillGuardStatus::Passed
    } else if mode == SkillGuardMode::Enforce
        && violations
            .iter()
            .any(|violation| violation.severity == SkillGuardSeverity::Error)
    {
        SkillGuardStatus::Blocked
    } else {
        SkillGuardStatus::Warn
    };

    SkillGuardReport {
        skill_name: skill_name.to_string(),
        status,
        violations,
        scanned_at,
    }
}

fn warn_violation(
    rule_id: &str,
    message: impl Into<String>,
    file_path: Option<String>,
) -> SkillGuardViolation {
    SkillGuardViolation {
        rule_id: rule_id.to_string(),
        severity: SkillGuardSeverity::Warn,
        message: message.into(),
        file_path,
    }
}

fn error_violation(
    rule_id: &str,
    message: impl Into<String>,
    file_path: Option<String>,
) -> SkillGuardViolation {
    SkillGuardViolation {
        rule_id: rule_id.to_string(),
        severity: SkillGuardSeverity::Error,
        message: message.into(),
        file_path,
    }
}
