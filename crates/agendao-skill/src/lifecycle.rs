use agendao_types::{ManagedSkillRecord, SkillManagedLifecycleRecord, SkillManagedLifecycleState};

#[derive(Debug, Default)]
pub struct SkillLifecycleCoordinator;

impl SkillLifecycleCoordinator {
    pub fn new() -> Self {
        Self
    }

    pub fn managed_runtime_state(
        &self,
        record: &ManagedSkillRecord,
        latest_revision: Option<&str>,
    ) -> SkillManagedLifecycleState {
        if record.deleted_locally || record.locally_modified {
            SkillManagedLifecycleState::Diverged
        } else if self.update_available(record.installed_revision.as_deref(), latest_revision) {
            SkillManagedLifecycleState::UpdateAvailable
        } else {
            SkillManagedLifecycleState::Installed
        }
    }

    pub fn build_record(
        &self,
        distribution_id: String,
        source_id: String,
        skill_name: String,
        state: SkillManagedLifecycleState,
        updated_at: i64,
        error: Option<String>,
    ) -> SkillManagedLifecycleRecord {
        SkillManagedLifecycleRecord {
            distribution_id,
            source_id,
            skill_name,
            state,
            updated_at,
            error,
        }
    }

    pub fn update_available(
        &self,
        installed_revision: Option<&str>,
        latest_revision: Option<&str>,
    ) -> bool {
        match latest_revision
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(latest) => {
                installed_revision
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    != Some(latest)
            }
            None => false,
        }
    }
}
