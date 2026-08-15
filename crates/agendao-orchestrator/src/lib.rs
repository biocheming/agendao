pub mod agent_loop;
pub mod blueprint;
pub mod catalog;
pub mod context;
pub mod engine;
pub mod events;
mod model_request;
pub mod model_resolution;
pub mod output_projection;
pub mod policy;
pub mod selector;
pub mod templates;

pub use blueprint::CapabilityId;
pub use engine::{
    ArtifactRequest, CapabilityBackend, CheckpointHandle, CheckpointRequest, RestoreRequest,
    RunDisposition,
};
pub use policy::WorkspaceLimits;

#[cfg(test)]
mod scheduler_tests;
