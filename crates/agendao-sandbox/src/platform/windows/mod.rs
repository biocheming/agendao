//! Windows platform backend: restricted-token containment (Phase 7).
//!
//! Delivery shape is deliberately honest: the *model* layer — the
//! restricted-token plan, the protected-metadata DACL text, the job
//! object configuration, and the network enforcement status — compiles
//! and is contract-tested on every host. The kernel enforcement path
//! (`CreateRestrictedToken` + `CreateProcessAsUserW` + Job Object +
//! WFP) is **not integrated yet**, so the backend probes unavailable
//! and every contained launch on Windows fails closed with an
//! actionable reason instead of silently weakening containment.
//! `docs/sandbox.md` documents this state and the integration plan.

pub mod acl;
pub mod job;
pub mod token;
pub mod wfp;

use async_trait::async_trait;

use crate::backend::{
    BackendChild, BackendProbe, BackendViolationToken, ChildEnvironment, SandboxBackend, StdioPlan,
};
use crate::model::ProcessMode;
use crate::plan::SandboxPlan;
use crate::request::SpawnSpec;
use crate::violation::SandboxExecutionError;

pub struct WindowsSandboxBackend;

#[async_trait]
impl SandboxBackend for WindowsSandboxBackend {
    fn name(&self) -> &'static str {
        "windows-restricted-token"
    }

    fn probe(&self) -> BackendProbe {
        BackendProbe::unavailable(wfp::NETWORK_ENFORCEMENT_REASON)
    }

    fn supports(&self, plan: &SandboxPlan) -> bool {
        // The capability envelope this backend will enforce once the
        // kernel path lands; probe() keeps it unselectable until then.
        plan.process.mode == ProcessMode::Contained
    }

    async fn spawn(
        &self,
        _plan: &SandboxPlan,
        _spec: &SpawnSpec,
        _env: &ChildEnvironment,
        _stdio: &StdioPlan,
        _violation_token: BackendViolationToken,
    ) -> Result<Box<dyn BackendChild>, SandboxExecutionError> {
        // Fail-closed floor: even if a future registry change selects
        // this backend before enforcement is integrated, no launch
        // escapes without the restricted token.
        Err(SandboxExecutionError::SandboxUnavailable {
            backend: self.name().to_string(),
            reason: wfp::NETWORK_ENFORCEMENT_REASON.to_string(),
        })
    }
}

/// Convenience constructor matching the registry's expected type.
pub fn windows_backend() -> std::sync::Arc<dyn SandboxBackend> {
    std::sync::Arc::new(WindowsSandboxBackend)
}
