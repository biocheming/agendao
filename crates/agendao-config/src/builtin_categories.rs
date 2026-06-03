use std::collections::HashMap;

use crate::category::TaskCategoryDef;

/// Built-in categories that are always available, even without a task.jsonc.
/// Users can override any of these by defining a category with the same name
/// in their task.jsonc file.
pub fn builtin_categories() -> HashMap<String, TaskCategoryDef> {
    let mut cats = HashMap::new();

    cats.insert(
        "quick".to_string(),
        TaskCategoryDef {
            description: "Trivial tasks — single file changes, typo fixes, simple modifications"
                .to_string(),
            model: None,
            prompt_suffix: Some(
                "<Category_Context>\n\
                 You are working on SMALL / QUICK tasks.\n\n\
                 Efficient execution mindset:\n\
                 - Fast, focused, minimal overhead\n\
                 - Get to the point immediately\n\
                 - No over-engineering\n\
                 - Simple solutions for simple problems\n\
                 </Category_Context>"
                    .to_string(),
            ),
            variant: None,
        },
    );

    cats.insert(
        "deep".to_string(),
        TaskCategoryDef {
            description:
                "Goal-oriented autonomous problem-solving requiring thorough research before action"
                    .to_string(),
            model: None,
            prompt_suffix: Some(
                "<Category_Context>\n\
                 You are working on GOAL-ORIENTED AUTONOMOUS tasks.\n\n\
                 Autonomous executor mindset:\n\
                 - You receive a GOAL, not step-by-step instructions\n\
                 - Figure out HOW to achieve the goal yourself\n\
                 - Thorough research before any action\n\
                 - Explore extensively, understand deeply, then act decisively\n\
                 - Prefer comprehensive solutions over quick patches\n\
                 </Category_Context>"
                    .to_string(),
            ),
            variant: None,
        },
    );

    cats.insert(
        "visual-engineering".to_string(),
        TaskCategoryDef {
            description: "Frontend, UI/UX, design, styling, animation".to_string(),
            model: None,
            prompt_suffix: Some(
                "<Category_Context>\n\
                 You are working on VISUAL/UI tasks.\n\n\
                 Design-first mindset:\n\
                 - Bold aesthetic choices over safe defaults\n\
                 - Cohesive color palettes with sharp accents\n\
                 - High-impact animations with staggered reveals\n\
                 - AVOID: generic fonts, predictable layouts, cookie-cutter patterns\n\
                 </Category_Context>"
                    .to_string(),
            ),
            variant: None,
        },
    );

    cats.insert(
        "writing".to_string(),
        TaskCategoryDef {
            description: "Documentation, prose, technical writing".to_string(),
            model: None,
            prompt_suffix: Some(
                "<Category_Context>\n\
                 You are working on WRITING / PROSE tasks.\n\n\
                 Wordsmith mindset:\n\
                 - Clear, flowing prose with appropriate tone\n\
                 - Engaging and readable\n\
                 - Proper structure and organization\n\
                 - ANTI-AI-SLOP: no em dashes, no \"delve\"/\"leverage\"/\"utilize\", \
                 use plain words and contractions naturally\n\
                 </Category_Context>"
                    .to_string(),
            ),
            variant: None,
        },
    );

    cats.insert(
        "general".to_string(),
        TaskCategoryDef {
            description: "Tasks that don't fit other categories".to_string(),
            model: None,
            prompt_suffix: None,
            variant: None,
        },
    );

    cats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_categories_include_core_set() {
        let cats = builtin_categories();
        assert!(cats.contains_key("quick"));
        assert!(cats.contains_key("deep"));
        assert!(cats.contains_key("visual-engineering"));
        assert!(cats.contains_key("writing"));
        assert!(cats.contains_key("general"));
        assert_eq!(cats.len(), 5);
    }
}
