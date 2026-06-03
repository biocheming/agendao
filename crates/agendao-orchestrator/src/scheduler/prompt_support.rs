use super::prompt_context::{AvailableAgentMeta, AvailableCategoryMeta};
use super::SchedulerSkillRef;
use std::collections::BTreeMap;

const COMPACT_SKILLS_PER_CATEGORY: usize = 12;
const COMPACT_SKILL_DESCRIPTION_CHARS: usize = 96;

pub(crate) const TONE_AND_STYLE: &str = r#"<Tone_and_Style>
## Communication Style

### Be Concise
- Start work immediately. No acknowledgments ("I'm on it", "Let me...", "I'll start...")
- Answer directly without preamble
- Don't summarize what you did unless asked
- Don't explain your code unless asked
- One word answers are acceptable when appropriate

### No Flattery
Never start responses with praise of the user's input. Just respond directly to the substance.

### No Status Updates
Never start responses with casual acknowledgments. Just start working. Use tasks for progress tracking.

### When User is Wrong
If the user's approach seems problematic:
- Don't blindly implement it
- Don't lecture or be preachy
- Concisely state your concern and alternative
- Ask if they want to proceed anyway

### Match User's Style
- If user is terse, be terse
- If user wants detail, provide detail
- Adapt to their communication preference
</Tone_and_Style>"#;

pub(crate) const HARD_BLOCKS: &str = r#"## Hard Blocks (NEVER violate)

- Commit without explicit request — **Never**
- Speculate about unread code — **Never**
- Leave code in broken state after failures — **Never**
- Delivering final answer before collecting Oracle result — **Never**"#;

pub(crate) const ANTI_PATTERNS: &str = r#"## Anti-Patterns (BLOCKING violations)

- **Error Handling**: Empty catch blocks
- **Testing**: Deleting failing tests to "pass"
- **Search**: Firing agents for single-line typos or obvious syntax errors
- **Debugging**: Shotgun debugging, random changes
- **Background Tasks**: Polling background tasks — end response and wait for notification
- **Oracle**: Delivering answer without collecting Oracle results"#;

pub(crate) const SOFT_GUIDELINES: &str = r#"## Soft Guidelines

- Prefer existing libraries over new dependencies
- Prefer small, focused changes over large refactors
- When uncertain about scope, ask"#;

pub(crate) const SKILLS_GUIDANCE: &str = r#"After completing a complex task (5+ tool calls), fixing a tricky error, or discovering a non-trivial workflow, save the approach as a skill with `skill_manage` so you can reuse it next time.

When using a skill and finding it outdated, incomplete, or wrong, patch it immediately with `skill_manage` instead of waiting to be asked."#;

pub(crate) fn push_session_context_section(
    sections: &mut Vec<String>,
    session_context: Option<&str>,
) {
    if let Some(context) = session_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sections.push(context.to_string());
    }
}

// ===========================================================================
// Dynamic section builders (mirrors OMO dynamic-agent-prompt-builder.ts)
// ===========================================================================

pub(crate) fn build_tool_selection_table(
    agents: &[AvailableAgentMeta],
    _skill_list: &[SchedulerSkillRef],
) -> String {
    // PLACEHOLDER_TOOL_SELECTION_BODY
    let mut rows = Vec::new();
    rows.push("### Tool & Agent Selection:".to_string());
    rows.push(String::new());

    // Built-in tools are always FREE
    rows.push("- `grep`, `glob`, `read`, `bash` — **FREE** — Not Complex, Scope Clear".to_string());

    // Sort agents by cost: FREE < CHEAP < EXPENSIVE, exclude utility agents
    let mut sorted: Vec<&AvailableAgentMeta> =
        agents.iter().filter(|a| a.mode != "primary").collect();
    sorted.sort_by_key(|a| match a.cost.as_str() {
        "FREE" => 0,
        "CHEAP" => 1,
        _ => 2,
    });

    for agent in &sorted {
        let short_desc = agent
            .description
            .split('.')
            .next()
            .unwrap_or(&agent.description);
        rows.push(format!(
            "- `{}` agent — **{}** — {}",
            agent.name, agent.cost, short_desc
        ));
    }

    rows.push(String::new());
    rows.push("**Default flow**: explore (background) + tools → oracle (if required)".to_string());

    rows.join("\n")
}

pub(crate) fn build_explore_section(agents: &[AvailableAgentMeta]) -> String {
    let has_explore = agents.iter().any(|a| a.name == "explore");
    if !has_explore {
        return String::new();
    }

    r#"### Explore Agent = Contextual Grep

Use it as a **peer tool**, not a fallback. Fire liberally.

**Use Direct Tools when:**
- You know the exact file path
- Simple keyword search
- Single-file inspection

**Use Explore Agent when:**
- Cross-file pattern discovery
- Understanding module relationships
- Finding implementations across the codebase
- Any question requiring 3+ file reads"#
        .to_string()
}

pub(crate) fn build_librarian_section(agents: &[AvailableAgentMeta]) -> String {
    let has_librarian = agents.iter().any(|a| a.name == "librarian");
    if !has_librarian {
        return String::new();
    }

    r#"### Librarian Agent = External Reference Grep

Use it when the task needs **external documentation or best practices**, not just local codebase facts.

**Use Librarian when:**
- Security, framework, or library best practices matter
- You need external docs, standards, or battle-tested patterns
- Repo inspection alone is insufficient for a safe decision

**Do NOT use Librarian when:**
- The answer is already in the repo
- The task is a purely local refactor or bug fix with enough internal evidence
- You only need a quick code search"#
        .to_string()
}

pub(crate) fn build_category_skills_guide(
    categories: &[AvailableCategoryMeta],
    skill_list: &[SchedulerSkillRef],
) -> String {
    if categories.is_empty() && skill_list.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push("### Category + Skills Delegation System".to_string());
    lines.push(String::new());
    lines.push("**task() combines categories and skills for optimal task execution.**".to_string());

    if !categories.is_empty() {
        lines.push(String::new());
        lines.push("#### Available Categories (Domain-Optimized Models)".to_string());
        lines.push(String::new());
        lines.push("Each category is configured with a model optimized for that domain. Read the description to understand when to use it.".to_string());
        lines.push(String::new());
        for cat in categories {
            lines.push(format!("- `{}` — {}", cat.name, cat.description));
        }
    }

    if !skill_list.is_empty() {
        lines.push(String::new());
        lines.push("#### Available Skills".to_string());
        lines.push(String::new());
        lines.push(render_compact_skill_catalog(skill_list));
        lines.push(String::new());
        lines.push(
            "> Read this index first, then use `skill_view(name)` before EVERY delegation. If a skill exposes supporting files, continue with `skill_view(name, file_path)`."
                .to_string(),
        );
        lines.push(String::new());
        lines.push(SKILLS_GUIDANCE.to_string());
    }

    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());
    lines.push("### MANDATORY: Category + Skill Selection Protocol".to_string());
    lines.push(String::new());
    lines.push("**STEP 1: Select Category**".to_string());
    lines.push("- Read each category's description".to_string());
    lines.push("- Match task requirements to category domain".to_string());
    lines.push("- Select the category whose domain BEST fits the task".to_string());
    lines.push(String::new());
    lines.push("**STEP 2: Evaluate ALL Skills**".to_string());
    lines.push(
        "For EVERY skill, ask: \"Does this skill's expertise domain overlap with my task?\""
            .to_string(),
    );
    lines.push("- If YES → INCLUDE in `load_skills=[...]`".to_string());
    lines.push("- If NO → OMIT".to_string());
    lines.push(String::new());
    lines.push("### Delegation Pattern".to_string());
    lines.push(String::new());
    lines.push("```".to_string());
    lines.push("task(".to_string());
    lines.push("  category=\"[selected-category]\",".to_string());
    lines.push("  load_skills=[\"skill-1\", \"skill-2\"],".to_string());
    lines.push("  prompt=\"...\"".to_string());
    lines.push(")".to_string());
    lines.push("```".to_string());

    lines.join("\n")
}

pub(crate) fn build_delegation_table(agents: &[AvailableAgentMeta]) -> String {
    let delegatable: Vec<&AvailableAgentMeta> = agents
        .iter()
        .filter(|a| a.mode == "subagent" || a.mode == "all")
        .collect();

    if delegatable.is_empty() {
        return String::new();
    }

    let mut rows = Vec::new();
    rows.push("### Delegation Table:".to_string());
    rows.push(String::new());

    for agent in &delegatable {
        let short_desc = agent
            .description
            .split('.')
            .next()
            .unwrap_or(&agent.description);
        rows.push(format!("- **{}** → `{}` agent", short_desc, agent.name));
    }

    rows.join("\n")
}

pub(crate) fn build_oracle_section(agents: &[AvailableAgentMeta]) -> String {
    let has_oracle = agents.iter().any(|a| a.name == "oracle");
    if !has_oracle {
        return String::new();
    }

    r#"<Oracle_Usage>
## Oracle — Read-Only High-IQ Consultant

Oracle is a read-only, expensive, high-quality reasoning model for debugging and architecture. Consultation only.

### WHEN to Consult (Oracle FIRST, then implement):

- Architecture decisions affecting 3+ files
- Debugging failures after 2+ failed attempts
- Performance optimization strategy
- Security-sensitive design choices

### WHEN NOT to Consult:

- Simple code questions answerable by reading
- Straightforward implementation tasks
- When you already have a clear approach

### Usage Pattern:
Briefly announce "Consulting Oracle for [reason]" before invocation.

### Oracle Background Task Policy:

**Collect Oracle results before your final answer. No exceptions.**

- Oracle takes time. When done with your own work: **end your response** — wait for the notification.
- Do NOT poll background tasks on a running Oracle. The notification will come.
- Never cancel Oracle.
</Oracle_Usage>"#
        .to_string()
}

pub(crate) fn build_task_management_section() -> String {
    r#"<Task_Management>
## Task Management (CRITICAL)

**DEFAULT BEHAVIOR**: Create tasks BEFORE starting any non-trivial task. This is your PRIMARY coordination mechanism.

### When to Create Tasks (MANDATORY)

- Multi-step task (2+ steps) → ALWAYS `TaskCreate` first
- Uncertain scope → ALWAYS (tasks clarify thinking)
- User request with multiple items → ALWAYS
- Complex single task → `TaskCreate` to break down

### Workflow (NON-NEGOTIABLE)

1. **IMMEDIATELY on receiving request**: `TaskCreate` to plan atomic steps.
   - ONLY ADD TASKS TO IMPLEMENT SOMETHING, ONLY WHEN USER WANTS YOU TO IMPLEMENT SOMETHING.
2. **Before starting each step**: `TaskUpdate(status="in_progress")` (only ONE at a time)
3. **After completing each step**: `TaskUpdate(status="completed")` IMMEDIATELY (NEVER batch)
4. **If scope changes**: Update tasks before proceeding

### Anti-Patterns (BLOCKING)

- Skipping tasks on multi-step tasks — user has no visibility, steps get forgotten
- Batch-completing multiple tasks — defeats real-time tracking purpose
- Proceeding without marking in_progress — no indication of what you're working on
- Finishing without completing tasks — task appears incomplete to user

**FAILURE TO USE TASKS ON NON-TRIVIAL TASKS = INCOMPLETE WORK.**
</Task_Management>"#
        .to_string()
}

/// Compact capabilities summary for prompt contexts that need awareness of
/// available agents, categories, and skills but don't need the full delegation
/// protocol (e.g. route stage, profile prompt suffix, planner stages).
pub(crate) fn build_capabilities_summary(
    agents: &[AvailableAgentMeta],
    categories: &[AvailableCategoryMeta],
    skill_list: &[SchedulerSkillRef],
) -> String {
    if agents.is_empty() && categories.is_empty() && skill_list.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();
    sections.push("### Available Capabilities".to_string());

    if !agents.is_empty() {
        sections.push(String::new());
        sections.push("**Agents:**".to_string());
        for agent in agents {
            let short_desc = agent
                .description
                .split('.')
                .next()
                .unwrap_or(&agent.description);
            sections.push(format!("- `{}` — {}", agent.name, short_desc));
        }
    }

    if !categories.is_empty() {
        sections.push(String::new());
        sections.push("**Task Categories:**".to_string());
        for cat in categories {
            sections.push(format!("- `{}` — {}", cat.name, cat.description));
        }
    }

    if !skill_list.is_empty() {
        sections.push(String::new());
        sections.push("**Skills:**".to_string());
        sections.push(render_compact_skill_catalog(skill_list));
        sections.push(
            "Use `skill_view(name)` to inspect any relevant skill before relying on it."
                .to_string(),
        );
        sections.push(SKILLS_GUIDANCE.to_string());
    }

    sections.join("\n")
}

pub(crate) fn render_compact_skill_catalog(skill_list: &[SchedulerSkillRef]) -> String {
    if skill_list.is_empty() {
        return "<available_skills compact=\"true\" />".to_string();
    }

    let mut grouped = BTreeMap::<String, Vec<&SchedulerSkillRef>>::new();
    for skill in skill_list {
        let category = skill
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("general")
            .to_string();
        grouped.entry(category).or_default().push(skill);
    }

    let mut output = String::from("<available_skills compact=\"true\">\n");
    for (category, mut skills) in grouped {
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        output.push_str(&format!("  {}:", category));
        let names = skills
            .iter()
            .take(COMPACT_SKILLS_PER_CATEGORY)
            .map(|skill| compact_skill_entry(skill))
            .collect::<Vec<_>>()
            .join(", ");
        output.push(' ');
        output.push_str(&names);
        let omitted = skills.len().saturating_sub(COMPACT_SKILLS_PER_CATEGORY);
        if omitted > 0 {
            output.push_str(&format!(" (+{} more; use skill_list/skill_view)", omitted));
        }
        output.push('\n');
    }
    output.push_str("</available_skills>");
    output
}

fn compact_skill_entry(skill: &SchedulerSkillRef) -> String {
    let description = skill.description.trim();
    if description.is_empty() {
        return skill.name.clone();
    }

    let first_sentence = description.split('.').next().unwrap_or(description).trim();
    let short = first_sentence
        .chars()
        .take(COMPACT_SKILL_DESCRIPTION_CHARS)
        .collect::<String>();
    if short.is_empty() {
        skill.name.clone()
    } else {
        format!("{}: {}", skill.name, short)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_summary_includes_skills_guidance_after_skill_catalog() {
        let summary = build_capabilities_summary(
            &[],
            &[],
            &[SchedulerSkillRef {
                name: "debug-linux".to_string(),
                description: "Investigate Linux-specific failures".to_string(),
                category: Some("debug".to_string()),
            }],
        );

        assert!(summary.contains("<available_skills compact=\"true\">"));
        assert!(summary.contains(
            "Use `skill_view(name)` to inspect any relevant skill before relying on it."
        ));
        assert!(summary.contains("After completing a complex task (5+ tool calls)"));
        assert!(summary.contains("patch it immediately with `skill_manage`"));
    }

    #[test]
    fn compact_skill_catalog_limits_large_categories() {
        let skills = (0..20)
            .map(|index| SchedulerSkillRef {
                name: format!("skill-{index:02}"),
                description: "A very long skill description that should be shortened before being placed in hot scheduler prompts.".to_string(),
                category: Some("debug".to_string()),
            })
            .collect::<Vec<_>>();

        let summary = render_compact_skill_catalog(&skills);

        assert!(summary.contains("compact=\"true\""));
        assert!(summary.contains("skill-00"));
        assert!(summary.contains("(+8 more; use skill_list/skill_view)"));
        assert!(!summary.contains("skill-19"));
    }
}
