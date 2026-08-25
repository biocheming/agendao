//! Network enforcement status for the Windows backend.
//!
//! The intended design (documented so the integration is a plan, not a
//! mystery): a per-launch WFP sublayer with `FWPM_LAYER_ALE_AUTH_
//! CONNECT_V4/V6` BLOCK filters keyed on the child's program path, so
//! `NetworkMode::Disabled` is kernel-enforced for the whole process
//! tree the way seccomp denies socket calls on Linux. A restricted
//! token alone cannot deny network access — SIDs say nothing about
//! sockets — which is exactly why the backend stays fail-closed until
//! this layer lands.

/// Why the Windows backend is not selectable yet. Flows into every
/// `SandboxUnavailable` denial so users on Windows see one actionable
/// reason instead of a silent capability gap.
pub const NETWORK_ENFORCEMENT_REASON: &str = "Windows sandbox backend is not integrated yet: \
     restricted-token + job-object + WFP kernel enforcement is planned; contained launches fail \
     closed on Windows until it ships (native launches are unaffected)";

#[cfg(test)]
mod tests {
    #[test]
    fn reason_names_the_missing_layer_and_fail_closed_semantics() {
        let reason = super::NETWORK_ENFORCEMENT_REASON;
        assert!(
            reason.contains("WFP"),
            "must name the missing layer: {reason}"
        );
        assert!(
            reason.contains("fail closed"),
            "must state the failure semantics: {reason}"
        );
    }
}
