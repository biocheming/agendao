//! RoleChip — message block 角色标签的单点。
//!
//! 替代 layout_block 里散落的 `format!(" {} ", chip)` + 各自配色。
//! role → (chip 文本, Semantic 色) 唯一权威在此。
//! 配色服从五行：User=木 / Assistant=金 / Tool=火 / Think·Skill·Todo=水 / System=土。

use crate::ds::color::Semantic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Tool,
    Think,
    System,
    Skill,
    Stage,
    Todo,
}

/// role → (chip 文本, 语义色) 单点。
pub fn role_chip(role: Role) -> (&'static str, Semantic) {
    match role {
        Role::User      => ("You",       Semantic::Wood),
        Role::Assistant => ("Assistant", Semantic::Metal),
        Role::Tool      => ("Tool",      Semantic::Fire),
        Role::Think     => ("Thinking",  Semantic::Water),
        Role::System    => ("System",    Semantic::Earth),
        Role::Skill     => ("Skill",     Semantic::Water),
        Role::Stage     => ("Stage",     Semantic::Fire),
        Role::Todo      => ("Tasks",     Semantic::Water),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_is_wood_assistant_is_metal() {
        assert_eq!(role_chip(Role::User).1, Semantic::Wood);
        assert_eq!(role_chip(Role::Assistant).1, Semantic::Metal);
    }

    #[test]
    fn all_roles_have_chip_text() {
        for r in [Role::User, Role::Assistant, Role::Tool, Role::Think,
                  Role::System, Role::Skill, Role::Stage, Role::Todo] {
            assert!(!role_chip(r).0.is_empty());
        }
    }
}
