use agendao_types::{MemoryKind, MemoryRuleDefinition, MemoryRulePack, MemoryRulePackKind};

pub fn builtin_rule_packs(now: i64) -> Vec<MemoryRulePack> {
    vec![
        MemoryRulePack {
            id: "builtin.memory.validation.core".to_string(),
            rule_pack_kind: MemoryRulePackKind::Validation,
            version: "2026.04.13".to_string(),
            rules: vec![
                MemoryRuleDefinition {
                    id: "boundary.require.validation".to_string(),
                    description:
                        "Durable memory must carry explicit boundaries, triggers, and evidence."
                            .to_string(),
                    tags: vec![
                        "validation".to_string(),
                        "boundary".to_string(),
                        "evidence".to_string(),
                    ],
                    promotion_target: None,
                },
                MemoryRuleDefinition {
                    id: "scope.require.workspace_identity".to_string(),
                    description:
                        "Workspace-scoped memory must resolve to one workspace authority key."
                            .to_string(),
                    tags: vec!["validation".to_string(), "scope".to_string()],
                    promotion_target: None,
                },
            ],
            created_at: now,
            updated_at: now,
        },
        MemoryRulePack {
            id: "builtin.memory.consolidation.core".to_string(),
            rule_pack_kind: MemoryRulePackKind::Consolidation,
            version: "2026.04.13".to_string(),
            rules: vec![
                MemoryRuleDefinition {
                    id: "merge.similar.summary".to_string(),
                    description:
                        "Merge overlapping records that describe the same normalized workflow or fact."
                            .to_string(),
                    tags: vec!["merge".to_string(), "summary".to_string()],
                    promotion_target: None,
                },
                MemoryRuleDefinition {
                    id: "promotion.pattern.from_repeated_lessons".to_string(),
                    description:
                        "Promote repeated validated lessons into a consolidated pattern."
                            .to_string(),
                    tags: vec!["promotion".to_string(), "pattern".to_string()],
                    promotion_target: Some(MemoryKind::Pattern),
                },
                MemoryRuleDefinition {
                    id: "promotion.methodology.from_structured_pattern".to_string(),
                    description:
                        "Promote well-structured consolidated patterns into methodology candidates."
                            .to_string(),
                    tags: vec![
                        "promotion".to_string(),
                        "methodology".to_string(),
                        "reflection".to_string(),
                    ],
                    promotion_target: Some(MemoryKind::MethodologyCandidate),
                },
            ],
            created_at: now,
            updated_at: now,
        },
        MemoryRulePack {
            id: "builtin.memory.reflection.core".to_string(),
            rule_pack_kind: MemoryRulePackKind::Reflection,
            version: "2026.04.13".to_string(),
            rules: vec![
                MemoryRuleDefinition {
                    id: "reflection.expand_validation_recipe".to_string(),
                    description:
                        "When a memory cluster is repeatedly useful, capture its validation recipe explicitly."
                            .to_string(),
                    tags: vec!["reflection".to_string(), "validation".to_string()],
                    promotion_target: None,
                },
                MemoryRuleDefinition {
                    id: "reflection.extract_methodology_scope".to_string(),
                    description:
                        "Methodology candidates should state triggers, reusable facts, and non-goals."
                            .to_string(),
                    tags: vec!["reflection".to_string(), "methodology".to_string()],
                    promotion_target: None,
                },
            ],
            created_at: now,
            updated_at: now,
        },
    ]
}
