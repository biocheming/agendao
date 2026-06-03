use agendao_provider::{Content, Message, Role};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;

const SKILL_TREE_TRUNCATION_MARKER: &str = "[... skill tree truncated ...]";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTreeNode {
    pub node_id: String,
    pub markdown_path: String,
    pub children: Vec<SkillTreeNode>,
}

impl SkillTreeNode {
    pub fn new(node_id: impl Into<String>, markdown_path: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            markdown_path: markdown_path.into(),
            children: Vec::new(),
        }
    }

    pub fn with_children(mut self, children: Vec<SkillTreeNode>) -> Self {
        self.children = children;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledSkillNode {
    pub node_id: String,
    pub parent_id: Option<String>,
    pub depth: usize,
    pub lineage: Vec<String>,
    pub source_paths: Vec<String>,
    pub context_markdown: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompiledSkillTree {
    pub nodes: Vec<CompiledSkillNode>,
}

impl CompiledSkillTree {
    pub fn node(&self, node_id: &str) -> Option<&CompiledSkillNode> {
        self.nodes.iter().find(|n| n.node_id == node_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillTreeCompileError {
    #[error("skill tree references missing markdown path: {path}")]
    MissingMarkdown { path: String },

    #[error("skill tree has duplicate node_id: {node_id}")]
    DuplicateNodeId { node_id: String },
}

#[derive(Debug, Clone)]
pub struct SkillTreeCompiler {
    context_separator: String,
    token_budget: Option<usize>,
    truncation_strategy: SkillTreeTruncationStrategy,
}

impl Default for SkillTreeCompiler {
    fn default() -> Self {
        Self {
            context_separator: "\n\n---\n\n".to_string(),
            token_budget: None,
            truncation_strategy: SkillTreeTruncationStrategy::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillTreeTruncationStrategy {
    Head,
    Tail,
    #[default]
    HeadTail,
}

impl SkillTreeTruncationStrategy {
    pub fn from_label(value: &str) -> Option<Self> {
        match value.trim() {
            "head" => Some(Self::Head),
            "tail" => Some(Self::Tail),
            "head-tail" => Some(Self::HeadTail),
            _ => None,
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Tail => "tail",
            Self::HeadTail => "head-tail",
        }
    }
}

impl SkillTreeCompiler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.context_separator = separator.into();
        self
    }

    pub fn with_token_budget(mut self, token_budget: Option<usize>) -> Self {
        self.token_budget = token_budget;
        self
    }

    pub fn with_truncation_strategy(
        mut self,
        truncation_strategy: SkillTreeTruncationStrategy,
    ) -> Self {
        self.truncation_strategy = truncation_strategy;
        self
    }

    pub fn compile(
        &self,
        root: &SkillTreeNode,
        markdown_repo: &HashMap<String, String>,
    ) -> Result<CompiledSkillTree, SkillTreeCompileError> {
        let mut visited = HashSet::new();
        let mut compiled_nodes = Vec::new();
        let mut lineage = Vec::<String>::new();
        let mut inherited_paths = Vec::<String>::new();
        let mut inherited_segments = Vec::<String>::new();
        let mut traversal = SkillTreeTraversal {
            markdown_repo,
            visited: &mut visited,
            lineage: &mut lineage,
            inherited_paths: &mut inherited_paths,
            inherited_segments: &mut inherited_segments,
            out: &mut compiled_nodes,
        };

        self.compile_node(root, None, 0, &mut traversal)?;

        Ok(CompiledSkillTree {
            nodes: compiled_nodes,
        })
    }

    fn compile_node(
        &self,
        node: &SkillTreeNode,
        parent_id: Option<&str>,
        depth: usize,
        traversal: &mut SkillTreeTraversal<'_>,
    ) -> Result<(), SkillTreeCompileError> {
        if !traversal.visited.insert(node.node_id.clone()) {
            return Err(SkillTreeCompileError::DuplicateNodeId {
                node_id: node.node_id.clone(),
            });
        }

        let markdown = traversal
            .markdown_repo
            .get(&node.markdown_path)
            .ok_or_else(|| SkillTreeCompileError::MissingMarkdown {
                path: node.markdown_path.clone(),
            })?
            .clone();

        traversal.lineage.push(node.node_id.clone());
        traversal.inherited_paths.push(node.markdown_path.clone());
        traversal.inherited_segments.push(markdown);

        let context_markdown = truncate_skill_tree_context(
            &traversal.inherited_segments.join(&self.context_separator),
            self.token_budget,
            self.truncation_strategy,
        );
        traversal.out.push(CompiledSkillNode {
            node_id: node.node_id.clone(),
            parent_id: parent_id.map(str::to_string),
            depth,
            lineage: traversal.lineage.clone(),
            source_paths: traversal.inherited_paths.clone(),
            context_markdown,
        });

        for child in &node.children {
            self.compile_node(child, Some(&node.node_id), depth + 1, traversal)?;
        }

        traversal.lineage.pop();
        traversal.inherited_paths.pop();
        traversal.inherited_segments.pop();

        Ok(())
    }
}

struct SkillTreeTraversal<'a> {
    markdown_repo: &'a HashMap<String, String>,
    visited: &'a mut HashSet<String>,
    lineage: &'a mut Vec<String>,
    inherited_paths: &'a mut Vec<String>,
    inherited_segments: &'a mut Vec<String>,
    out: &'a mut Vec<CompiledSkillNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillTreeRequestPlan {
    #[serde(alias = "contextMarkdown")]
    pub context_markdown: String,
    #[serde(
        default,
        alias = "tokenBudget",
        skip_serializing_if = "Option::is_none"
    )]
    pub token_budget: Option<usize>,
    #[serde(
        default,
        alias = "truncationStrategy",
        skip_serializing_if = "skill_tree_truncation_strategy_is_default"
    )]
    pub truncation_strategy: SkillTreeTruncationStrategy,
}

impl SkillTreeRequestPlan {
    const CONTEXT_HEADER: &'static str = "Skill Tree Context (Inherited):";

    pub fn from_tree(
        root: &SkillTreeNode,
        markdown_repo: &HashMap<String, String>,
    ) -> Result<Option<Self>, SkillTreeCompileError> {
        Self::from_tree_with_options(root, markdown_repo, None, None, None)
    }

    pub fn from_tree_with_separator(
        root: &SkillTreeNode,
        markdown_repo: &HashMap<String, String>,
        separator: Option<&str>,
    ) -> Result<Option<Self>, SkillTreeCompileError> {
        Self::from_tree_with_options(root, markdown_repo, separator, None, None)
    }

    pub fn from_tree_with_options(
        root: &SkillTreeNode,
        markdown_repo: &HashMap<String, String>,
        separator: Option<&str>,
        token_budget: Option<usize>,
        truncation_strategy: Option<SkillTreeTruncationStrategy>,
    ) -> Result<Option<Self>, SkillTreeCompileError> {
        let mut compiler = SkillTreeCompiler::new().with_token_budget(token_budget);
        if let Some(separator) = separator {
            compiler = compiler.with_separator(separator.to_string());
        }
        if let Some(truncation_strategy) = truncation_strategy {
            compiler = compiler.with_truncation_strategy(truncation_strategy);
        }
        let compiled = compiler.compile(root, markdown_repo)?;
        Ok(Self::from_compiled_with_options(
            compiled,
            token_budget,
            truncation_strategy.unwrap_or_default(),
        ))
    }

    pub fn from_compiled(compiled: CompiledSkillTree) -> Option<Self> {
        Self::from_compiled_with_options(compiled, None, SkillTreeTruncationStrategy::default())
    }

    pub fn from_compiled_with_options(
        compiled: CompiledSkillTree,
        token_budget: Option<usize>,
        truncation_strategy: SkillTreeTruncationStrategy,
    ) -> Option<Self> {
        let root = compiled
            .nodes
            .iter()
            .find(|node| node.depth == 0)
            .or_else(|| compiled.nodes.first())?;
        let context_markdown = root.context_markdown.trim().to_string();
        if context_markdown.is_empty() {
            None
        } else {
            Some(Self {
                context_markdown,
                token_budget,
                truncation_strategy,
            })
        }
    }

    pub fn append_context(&mut self, context: &str) {
        let context = context.trim();
        if context.is_empty() {
            return;
        }

        let combined = if self.context_markdown.trim().is_empty() {
            context.to_string()
        } else {
            format!("{}\n\n{}", self.context_markdown.trim_end(), context)
        };
        self.context_markdown =
            truncate_skill_tree_context(&combined, self.token_budget, self.truncation_strategy);
    }

    pub fn compose_system_prompt(&self, base: Option<&str>) -> Option<String> {
        let context = self.context_markdown.trim();
        let base = base.unwrap_or("").trim();

        match (base.is_empty(), context.is_empty()) {
            (true, true) => None,
            (false, true) => Some(base.to_string()),
            (true, false) => Some(format!("{}\n{}", Self::CONTEXT_HEADER, context)),
            (false, false) => Some(format!("{}\n\n{}\n{}", base, Self::CONTEXT_HEADER, context)),
        }
    }

    pub fn apply_to_messages(&self, mut messages: Vec<Message>) -> Vec<Message> {
        let existing_system =
            messages
                .first()
                .and_then(|message| match (&message.role, &message.content) {
                    (Role::System, Content::Text(text)) => Some(text.as_str()),
                    _ => None,
                });

        let Some(system_prompt) = self.compose_system_prompt(existing_system) else {
            return messages;
        };

        if let Some(first) = messages.first_mut() {
            if matches!(first.role, Role::System) {
                first.content = Content::Text(system_prompt);
                return messages;
            }
        }

        messages.insert(0, Message::system(system_prompt));
        messages
    }

    pub fn estimated_tokens(&self) -> usize {
        let char_count = self.context_markdown.trim().chars().count();
        char_count.saturating_add(3) / 4
    }

    pub fn is_truncated(&self) -> bool {
        self.context_markdown.contains(SKILL_TREE_TRUNCATION_MARKER)
    }
}

fn skill_tree_truncation_strategy_is_default(strategy: &SkillTreeTruncationStrategy) -> bool {
    *strategy == SkillTreeTruncationStrategy::default()
}

fn truncate_skill_tree_context(
    markdown: &str,
    token_budget: Option<usize>,
    truncation_strategy: SkillTreeTruncationStrategy,
) -> String {
    let trimmed = markdown.trim();
    let Some(token_budget) = token_budget else {
        return trimmed.to_string();
    };
    if trimmed.is_empty() {
        return String::new();
    }
    if token_budget == 0 {
        return String::new();
    }

    let char_budget = token_budget.saturating_mul(4);
    let total_chars = trimmed.chars().count();
    if total_chars <= char_budget {
        return trimmed.to_string();
    }

    let marker_chars = SKILL_TREE_TRUNCATION_MARKER.chars().count();
    if char_budget <= marker_chars {
        return first_chars(SKILL_TREE_TRUNCATION_MARKER, char_budget);
    }

    let remaining = char_budget.saturating_sub(marker_chars);
    let truncated = match truncation_strategy {
        SkillTreeTruncationStrategy::Head => format!(
            "{}{}",
            first_chars(trimmed, remaining).trim_end(),
            SKILL_TREE_TRUNCATION_MARKER
        ),
        SkillTreeTruncationStrategy::Tail => format!(
            "{}{}",
            SKILL_TREE_TRUNCATION_MARKER,
            last_chars(trimmed, remaining).trim_start()
        ),
        SkillTreeTruncationStrategy::HeadTail => {
            let head_chars = remaining / 2;
            let tail_chars = remaining.saturating_sub(head_chars);
            let head = first_chars(trimmed, head_chars).trim_end().to_string();
            let tail = last_chars(trimmed, tail_chars).trim_start().to_string();
            if head.is_empty() {
                format!("{}{}", SKILL_TREE_TRUNCATION_MARKER, tail)
            } else if tail.is_empty() {
                format!("{}{}", head, SKILL_TREE_TRUNCATION_MARKER)
            } else {
                format!("{head}{SKILL_TREE_TRUNCATION_MARKER}{tail}")
            }
        }
    };

    truncated.trim().to_string()
}

fn first_chars(input: &str, count: usize) -> String {
    input.chars().take(count).collect()
}

fn last_chars(input: &str, count: usize) -> String {
    let total = input.chars().count();
    if count >= total {
        return input.to_string();
    }
    input.chars().skip(total - count).collect()
}

pub fn resolve_skill_markdown_repo(
    skill_paths: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut repo = HashMap::with_capacity(skill_paths.len());
    for (key, value) in skill_paths {
        let trimmed = value.trim();
        let looks_inline =
            trimmed.contains('\n') || trimmed.starts_with('#') || trimmed.starts_with("```");
        if looks_inline {
            repo.insert(key.clone(), value.clone());
            continue;
        }

        let raw_path = trimmed.strip_prefix("file://").unwrap_or(trimmed);
        match fs::read_to_string(raw_path) {
            Ok(content) => {
                repo.insert(key.clone(), content);
            }
            Err(_) => {
                repo.insert(key.clone(), value.clone());
            }
        }
    }
    repo
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn repo(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn request_plan_composes_system_prompt() {
        let plan = SkillTreeRequestPlan {
            context_markdown: "ROOT".to_string(),
            token_budget: None,
            truncation_strategy: SkillTreeTruncationStrategy::default(),
        };

        assert_eq!(
            plan.compose_system_prompt(Some("BASE")).as_deref(),
            Some("BASE\n\nSkill Tree Context (Inherited):\nROOT")
        );
        assert_eq!(
            plan.compose_system_prompt(None).as_deref(),
            Some("Skill Tree Context (Inherited):\nROOT")
        );
    }

    #[test]
    fn request_plan_applies_to_messages() {
        let plan = SkillTreeRequestPlan {
            context_markdown: "ROOT".to_string(),
            token_budget: None,
            truncation_strategy: SkillTreeTruncationStrategy::default(),
        };

        let messages = plan.apply_to_messages(vec![Message::user("hello")]);
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].role, Role::System));
    }

    #[test]
    fn compile_single_node_tree() {
        let root = SkillTreeNode::new("root", "docs/root.md");
        let compiler = SkillTreeCompiler::new();
        let tree = compiler
            .compile(&root, &repo(&[("docs/root.md", "# Root Rule")]))
            .unwrap();

        assert_eq!(tree.nodes.len(), 1);
        let compiled = tree.node("root").unwrap();
        assert_eq!(compiled.depth, 0);
        assert_eq!(compiled.parent_id, None);
        assert_eq!(compiled.lineage, vec!["root".to_string()]);
        assert_eq!(compiled.source_paths, vec!["docs/root.md".to_string()]);
        assert_eq!(compiled.context_markdown, "# Root Rule");
    }

    #[test]
    fn compile_inherits_context_depth_first() {
        let root = SkillTreeNode::new("root", "docs/root.md")
            .with_children(vec![SkillTreeNode::new("child", "docs/child.md")
                .with_children(vec![SkillTreeNode::new("leaf", "docs/leaf.md")])]);

        let compiler = SkillTreeCompiler::new();
        let tree = compiler
            .compile(
                &root,
                &repo(&[
                    ("docs/root.md", "ROOT"),
                    ("docs/child.md", "CHILD"),
                    ("docs/leaf.md", "LEAF"),
                ]),
            )
            .unwrap();

        let leaf = tree.node("leaf").unwrap();
        assert_eq!(leaf.depth, 2);
        assert_eq!(
            leaf.lineage,
            vec!["root".to_string(), "child".to_string(), "leaf".to_string()]
        );
        assert_eq!(
            leaf.source_paths,
            vec![
                "docs/root.md".to_string(),
                "docs/child.md".to_string(),
                "docs/leaf.md".to_string()
            ]
        );
        assert_eq!(leaf.context_markdown, "ROOT\n\n---\n\nCHILD\n\n---\n\nLEAF");
    }

    #[test]
    fn compile_sibling_context_is_isolated() {
        let root = SkillTreeNode::new("root", "docs/root.md").with_children(vec![
            SkillTreeNode::new("a", "docs/a.md"),
            SkillTreeNode::new("b", "docs/b.md"),
        ]);
        let compiler = SkillTreeCompiler::new();
        let tree = compiler
            .compile(
                &root,
                &repo(&[
                    ("docs/root.md", "ROOT"),
                    ("docs/a.md", "A"),
                    ("docs/b.md", "B"),
                ]),
            )
            .unwrap();

        let a = tree.node("a").unwrap();
        let b = tree.node("b").unwrap();
        assert_eq!(a.context_markdown, "ROOT\n\n---\n\nA");
        assert_eq!(b.context_markdown, "ROOT\n\n---\n\nB");
    }

    #[test]
    fn compile_rejects_duplicate_node_id() {
        let root = SkillTreeNode::new("root", "docs/root.md").with_children(vec![
            SkillTreeNode::new("dup", "docs/a.md"),
            SkillTreeNode::new("dup", "docs/b.md"),
        ]);
        let compiler = SkillTreeCompiler::new();
        let err = compiler
            .compile(
                &root,
                &repo(&[
                    ("docs/root.md", "ROOT"),
                    ("docs/a.md", "A"),
                    ("docs/b.md", "B"),
                ]),
            )
            .unwrap_err();

        match err {
            SkillTreeCompileError::DuplicateNodeId { node_id } => assert_eq!(node_id, "dup"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn compile_rejects_missing_markdown_path() {
        let root = SkillTreeNode::new("root", "docs/root.md")
            .with_children(vec![SkillTreeNode::new("child", "docs/missing.md")]);
        let compiler = SkillTreeCompiler::new();
        let err = compiler
            .compile(&root, &repo(&[("docs/root.md", "ROOT")]))
            .unwrap_err();

        match err {
            SkillTreeCompileError::MissingMarkdown { path } => {
                assert_eq!(path, "docs/missing.md")
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn compile_applies_head_tail_token_budget() {
        let root = SkillTreeNode::new("root", "docs/root.md")
            .with_children(vec![SkillTreeNode::new("leaf", "docs/leaf.md")]);
        let tree = SkillTreeCompiler::new()
            .with_token_budget(Some(10))
            .compile(
                &root,
                &repo(&[
                    ("docs/root.md", "ROOT-ROOT-ROOT-ROOT"),
                    ("docs/leaf.md", "LEAF-LEAF-LEAF-LEAF"),
                ]),
            )
            .unwrap();

        let leaf = tree.node("leaf").expect("leaf should compile");
        assert!(leaf.context_markdown.contains("skill tree truncated"));
        assert!(leaf.context_markdown.starts_with("ROOT"));
        assert!(leaf.context_markdown.ends_with("LEAF"));
    }

    #[test]
    fn request_plan_append_context_reapplies_token_budget() {
        let mut plan = SkillTreeRequestPlan {
            context_markdown: "AAAAAAAAAAAAAAAAAAAA".to_string(),
            token_budget: Some(10),
            truncation_strategy: SkillTreeTruncationStrategy::Tail,
        };

        plan.append_context("BBBBBBBBBBBBBBBBBBBB");

        assert!(plan.context_markdown.contains("skill tree truncated"));
        assert!(plan.context_markdown.ends_with("BBBBBBBBBB"));
    }

    #[test]
    fn from_tree_with_options_preserves_budget_metadata() {
        let root = SkillTreeNode::new("root", "docs/root.md");
        let plan = SkillTreeRequestPlan::from_tree_with_options(
            &root,
            &repo(&[("docs/root.md", "ROOT")]),
            Some("\n--\n"),
            Some(128),
            Some(SkillTreeTruncationStrategy::Head),
        )
        .unwrap()
        .expect("plan should compile");

        assert_eq!(plan.context_markdown, "ROOT");
        assert_eq!(plan.token_budget, Some(128));
        assert_eq!(plan.truncation_strategy, SkillTreeTruncationStrategy::Head);
    }

    #[test]
    fn request_plan_reports_observability_fields() {
        let plan = SkillTreeRequestPlan {
            context_markdown: format!("ROOT{SKILL_TREE_TRUNCATION_MARKER}TAIL"),
            token_budget: Some(64),
            truncation_strategy: SkillTreeTruncationStrategy::Tail,
        };

        assert_eq!(plan.estimated_tokens(), 10);
        assert!(plan.is_truncated());
        assert_eq!(plan.truncation_strategy.as_label(), "tail");
    }
}
