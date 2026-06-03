use super::super::super::{
    SchedulerProfileConfig, SchedulerProfileOrchestrator, SchedulerProfilePlan,
};
use super::super::{orchestrator_from_definition, plan_from_definition};
use super::definition::{prometheus_default_stages, PROMETHEUS_PRESET};
use crate::tool_runner::ToolRunner;

pub type PrometheusPlan = SchedulerProfilePlan;
pub type PrometheusOrchestrator = SchedulerProfileOrchestrator;

pub fn prometheus_plan() -> PrometheusPlan {
    SchedulerProfilePlan::new(prometheus_default_stages()).with_orchestrator("prometheus")
}

pub fn prometheus_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> PrometheusPlan {
    plan_from_definition(profile_name, profile, PROMETHEUS_PRESET)
}

pub fn prometheus_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> PrometheusOrchestrator {
    orchestrator_from_definition(profile_name, profile, tool_runner, PROMETHEUS_PRESET)
}
