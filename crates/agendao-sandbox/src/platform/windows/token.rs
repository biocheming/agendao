//! Restricted-token model for the Windows backend.
//!
//! Pure planning only — no Win32 calls in this phase. When the kernel
//! path integrates, this plan feeds `CreateRestrictedToken` (deny-only
//! SIDs + `LuaToken`) and the launch goes through
//! `CreateProcessAsUserW`; until then the backend fails closed (see
//! `super::wfp`).

use crate::model::ProcessMode;
use crate::plan::SandboxPlan;

/// Well-known SIDs the plan can reference (string forms are the SDDL
/// abbreviations; integration maps them to `CreateWellKnownSid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellKnownSid {
    /// `WD` — World; deny-only here blocks world-writable escalation.
    Everyone,
    /// `BA` — built-in Administrators.
    Administrators,
    /// `BU` — built-in Users (the token's working identity).
    Users,
}

impl WellKnownSid {
    pub fn sddl_abbreviation(&self) -> &'static str {
        match self {
            Self::Everyone => "WD",
            Self::Administrators => "BA",
            Self::Users => "BU",
        }
    }
}

/// What the restricted token must look like for one plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestrictedTokenPlan {
    /// SIDs kept in the token but converted to deny-only, so any ACE
    /// granting them is neutralized (the restricted-token containment
    /// primitive).
    pub deny_only_sids: Vec<WellKnownSid>,
    /// SIDs removed from the token entirely.
    pub drop_sids: Vec<WellKnownSid>,
    /// `LuaToken`: keep only integrity levels the caller can hold,
    /// dropping elevated group memberships.
    pub lua_token: bool,
}

/// Derive the restricted-token plan for one sandbox plan. Contained
/// plans neutralize administrative write reachability and keep the
/// plain Users identity; native plans never reach this backend.
pub fn restricted_token_plan(plan: &SandboxPlan) -> RestrictedTokenPlan {
    debug_assert_eq!(
        plan.process.mode,
        ProcessMode::Contained,
        "the registry only routes contained plans to platform backends"
    );
    RestrictedTokenPlan {
        deny_only_sids: vec![WellKnownSid::Administrators, WellKnownSid::Everyone],
        drop_sids: Vec::new(),
        lua_token: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FilesystemMode, TrustClass};
    use crate::plan::{FilesystemPlan, ProcessPlan};
    use crate::request::ProfileKind;

    fn contained_plan() -> SandboxPlan {
        SandboxPlan {
            execution_id: "exec-test".into(),
            trust_class: TrustClass::ModelReachable,
            requested_kind: ProfileKind::WorkspaceWrite,
            filesystem: FilesystemPlan {
                mode: FilesystemMode::WorkspaceWrite,
                workspace_root: crate::plan::CanonicalPathValue("/ws".into()),
                writable_roots: vec![crate::plan::CanonicalPathValue("/ws".into())],
                read_only_roots: Vec::new(),
            },
            network: crate::network::NetworkPolicy::disabled(),
            environment: crate::environment::EnvironmentPolicy::default(),
            process: ProcessPlan {
                mode: ProcessMode::Contained,
                term_grace_secs: 5,
            },
            fingerprint: "fp".into(),
            session_origin: None,
        }
    }

    #[test]
    fn contained_plan_neutralizes_administrators_and_world() {
        let plan = restricted_token_plan(&contained_plan());
        assert!(plan.deny_only_sids.contains(&WellKnownSid::Administrators));
        assert!(plan.deny_only_sids.contains(&WellKnownSid::Everyone));
        assert!(plan.lua_token);
        assert!(plan.drop_sids.is_empty());
    }

    #[test]
    fn sddl_abbreviations_are_stable() {
        assert_eq!(WellKnownSid::Everyone.sddl_abbreviation(), "WD");
        assert_eq!(WellKnownSid::Administrators.sddl_abbreviation(), "BA");
        assert_eq!(WellKnownSid::Users.sddl_abbreviation(), "BU");
    }
}
