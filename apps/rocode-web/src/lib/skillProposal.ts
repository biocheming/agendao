import { apiJson } from "./api";

// ── types (mirror rocode-types/src/memory.rs serde output) ────────────────

export interface SuggestedSkillChangeRecord {
  kind: string;
  text?: string;
  evidence_refs?: string[];
  suggested_name?: string;
  when_to_use?: string[];
  core_steps?: string[];
  boundaries?: string[];
  validation?: string[];
}

export interface SkillEvolutionProposalRecord {
  id: string;
  session_id: string;
  memory_record_ids: string[];
  linked_skill_name?: string | null;
  proposal_kind: "patch_existing_skill" | "create_new_skill";
  title: string;
  rationale: string;
  suggested_changes: SuggestedSkillChangeRecord[];
  status: "draft" | "accepted" | "rejected" | "superseded" | "applied";
  evidence_hash: string;
  created_at_ms: number;
  updated_at_ms: number;
}

// ── API ───────────────────────────────────────────────────────────────────

export function listSkillProposals(
  status: string = "draft"
): Promise<SkillEvolutionProposalRecord[]> {
  return apiJson(
    `/api/skill/proposal/?status=${encodeURIComponent(status)}`
  );
}

export function getSkillProposal(
  id: string
): Promise<SkillEvolutionProposalRecord> {
  return apiJson(`/api/skill/proposal/${encodeURIComponent(id)}`);
}

export function updateSkillProposalStatus(
  id: string,
  status: string
): Promise<SkillEvolutionProposalRecord> {
  return apiJson(`/api/skill/proposal/${encodeURIComponent(id)}/status`, {
    method: "POST",
    body: JSON.stringify({ status }),
  });
}
