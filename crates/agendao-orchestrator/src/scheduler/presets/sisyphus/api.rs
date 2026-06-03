use super::super::super::{
    SchedulerProfileConfig, SchedulerProfileOrchestrator, SchedulerProfilePlan,
};
use super::super::{orchestrator_from_definition, plan_from_definition};
use super::definition::{sisyphus_default_stages, SISYPHUS_PRESET};
use crate::tool_runner::ToolRunner;

pub type SisyphusPlan = SchedulerProfilePlan;
pub type SisyphusOrchestrator = SchedulerProfileOrchestrator;

pub fn sisyphus_plan() -> SisyphusPlan {
    SchedulerProfilePlan::new(sisyphus_default_stages()).with_orchestrator("sisyphus")
}

pub fn sisyphus_plan_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
) -> SisyphusPlan {
    plan_from_definition(profile_name, profile, SISYPHUS_PRESET)
}

pub fn sisyphus_orchestrator_from_profile(
    profile_name: Option<String>,
    profile: &SchedulerProfileConfig,
    tool_runner: ToolRunner,
) -> SisyphusOrchestrator {
    orchestrator_from_definition(profile_name, profile, tool_runner, SISYPHUS_PRESET)
}
