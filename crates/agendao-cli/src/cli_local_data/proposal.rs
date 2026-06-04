use agendao_storage::{Database, SkillEvolutionProposalRepository};
use agendao_types::{ProposalStatus, SkillEvolutionProposal};

pub(crate) async fn list_skill_evolution_proposals(
    status: &ProposalStatus,
) -> anyhow::Result<Vec<SkillEvolutionProposal>> {
    let db = Database::new().await?;
    let repo = SkillEvolutionProposalRepository::new(db.pool().clone());
    Ok(repo.list_by_status(status).await?)
}

pub(crate) async fn get_skill_evolution_proposal(
    id: &str,
) -> anyhow::Result<Option<SkillEvolutionProposal>> {
    let db = Database::new().await?;
    let repo = SkillEvolutionProposalRepository::new(db.pool().clone());
    Ok(repo.get_by_id(id).await?)
}

pub(crate) async fn transition_skill_evolution_proposal(
    id: &str,
    next: &ProposalStatus,
) -> anyhow::Result<()> {
    let db = Database::new().await?;
    let repo = SkillEvolutionProposalRepository::new(db.pool().clone());
    repo.transition_status(id, next).await?;
    Ok(())
}
