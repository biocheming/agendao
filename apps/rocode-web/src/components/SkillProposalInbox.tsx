import { useCallback, useEffect, useState } from "react";
import {
  listSkillProposals,
  getSkillProposal,
  updateSkillProposalStatus,
  type SkillEvolutionProposalRecord,
  type SuggestedSkillChangeRecord,
} from "../lib/skillProposal";
import { cn } from "../lib/utils";

const STATUS_LABELS: Record<string, string> = {
  draft: "Draft",
  accepted: "Accepted",
  rejected: "Rejected",
  superseded: "Superseded",
  applied: "Applied",
};

const KIND_LABELS: Record<string, string> = {
  patch_existing_skill: "Patch",
  create_new_skill: "Create",
};

const FILTERS = ["draft", "accepted", "rejected", "superseded", "applied"] as const;

export function SkillProposalInbox() {
  const [filter, setFilter] = useState<string>("draft");
  const [proposals, setProposals] = useState<SkillEvolutionProposalRecord[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<SkillEvolutionProposalRecord | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadList = useCallback(async (status: string) => {
    setLoading(true);
    setError(null);
    try {
      const items = await listSkillProposals(status);
      setProposals(items);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const loadDetail = useCallback(async (id: string) => {
    try {
      const record = await getSkillProposal(id);
      setDetail(record);
      setSelectedId(id);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const handleStatusChange = useCallback(
    async (id: string, nextStatus: string) => {
      try {
        await updateSkillProposalStatus(id, nextStatus);
        await loadList(filter);
        if (selectedId === id) {
          await loadDetail(id);
        }
      } catch (e) {
        setError(String(e));
      }
    },
    [filter, selectedId, loadList, loadDetail]
  );

  useEffect(() => {
    loadList(filter);
  }, [filter, loadList]);

  const selected = proposals.find((p) => p.id === selectedId) ?? detail;

  return (
    <div className="flex flex-col h-full gap-2 p-2">
      {/* filter bar */}
      <div className="flex gap-1">
        {FILTERS.map((s) => (
          <button
            key={s}
            onClick={() => {
              setFilter(s);
              setSelectedId(null);
              setDetail(null);
            }}
            className={cn(
              "px-2 py-1 text-sm rounded",
              filter === s
                ? "bg-primary text-primary-foreground"
                : "bg-muted hover:bg-muted-foreground/20"
            )}
          >
            {STATUS_LABELS[s]}
          </button>
        ))}
      </div>

      {error && (
        <div className="text-sm text-destructive p-1">{error}</div>
      )}

      <div className="flex gap-2 flex-1 min-h-0">
        {/* proposal list */}
        <div className="w-80 overflow-auto border rounded">
          {loading && (
            <div className="text-sm text-muted-foreground p-2">Loading...</div>
          )}
          {!loading && proposals.length === 0 && (
            <div className="text-sm text-muted-foreground p-2">
              No {STATUS_LABELS[filter]?.toLowerCase() ?? filter} proposals
            </div>
          )}
          {proposals.map((p) => (
            <button
              key={p.id}
              onClick={() => loadDetail(p.id)}
              className={cn(
                "w-full text-left p-2 border-b text-sm hover:bg-accent",
                selectedId === p.id && "bg-accent"
              )}
            >
              <div className="font-medium truncate">{p.title}</div>
              <div className="text-xs text-muted-foreground">
                {KIND_LABELS[p.proposal_kind] ?? p.proposal_kind}
                {" · "}
                {p.linked_skill_name ?? "(new skill)"}
              </div>
            </button>
          ))}
        </div>

        {/* proposal detail */}
        <div className="flex-1 overflow-auto border rounded p-3">
          {!selected && (
            <div className="text-sm text-muted-foreground">
              Select a proposal to review
            </div>
          )}
          {selected && (
            <div className="space-y-3 text-sm">
              <h2 className="font-semibold">{selected.title}</h2>
              <div className="flex gap-2 text-xs text-muted-foreground">
                <span>{STATUS_LABELS[selected.status]}</span>
                <span>·</span>
                <span>{KIND_LABELS[selected.proposal_kind]}</span>
                <span>·</span>
                <span>{selected.linked_skill_name ?? "(new skill)"}</span>
              </div>

              <div>
                <div className="font-medium mb-1">Rationale</div>
                <div className="text-muted-foreground whitespace-pre-wrap">
                  {selected.rationale}
                </div>
              </div>

              <div>
                <div className="font-medium mb-1">Evidence</div>
                <div className="text-xs text-muted-foreground space-y-0.5">
                  {selected.memory_record_ids.map((rid) => (
                    <div key={rid}>memory: {rid}</div>
                  ))}
                </div>
              </div>

              <div>
                <div className="font-medium mb-1">Suggested Changes</div>
                <ChangeList changes={selected.suggested_changes} />
              </div>

              <div className="text-xs text-muted-foreground">
                Session: {selected.session_id}
                {" · "}
                Created: {new Date(selected.created_at_ms).toLocaleString()}
              </div>

              {/* actions */}
              <div className="flex gap-2 pt-2 border-t">
                {selected.status === "draft" && (
                  <>
                    <button
                      onClick={() => handleStatusChange(selected.id, "accepted")}
                      className="px-3 py-1 text-sm rounded bg-primary text-primary-foreground"
                    >
                      Accept
                    </button>
                    <button
                      onClick={() => handleStatusChange(selected.id, "rejected")}
                      className="px-3 py-1 text-sm rounded bg-destructive text-destructive-foreground"
                    >
                      Reject
                    </button>
                  </>
                )}
                {selected.status === "accepted" && (
                  <>
                    <button
                      onClick={() => handleStatusChange(selected.id, "rejected")}
                      className="px-3 py-1 text-sm rounded bg-destructive text-destructive-foreground"
                    >
                      Reject
                    </button>
                    <span className="text-xs text-muted-foreground self-center">
                      Accepted does not modify SKILL.md.
                    </span>
                  </>
                )}
                {(selected.status === "rejected" ||
                  selected.status === "superseded" ||
                  selected.status === "applied") && (
                  <span className="text-xs text-muted-foreground">Read-only</span>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ChangeList({ changes }: { changes: SuggestedSkillChangeRecord[] }) {
  return (
    <div className="space-y-2">
      {changes.map((c, i) => (
        <ChangeItem key={i} change={c} />
      ))}
    </div>
  );
}

function ChangeItem({ change }: { change: SuggestedSkillChangeRecord }) {
  switch (change.kind) {
    case "add_trigger_condition":
      return (
        <div className="border-l-2 border-green-500 pl-2">
          <div className="text-xs text-green-600">+ Trigger</div>
          <div>{change.text}</div>
          <RefsList refs={change.evidence_refs} />
        </div>
      );
    case "add_core_step":
      return (
        <div className="border-l-2 border-blue-500 pl-2">
          <div className="text-xs text-blue-600">+ Step</div>
          <div>{change.text}</div>
          <RefsList refs={change.evidence_refs} />
        </div>
      );
    case "add_boundary":
      return (
        <div className="border-l-2 border-orange-500 pl-2">
          <div className="text-xs text-orange-600">+ Boundary</div>
          <div>{change.text}</div>
          <RefsList refs={change.evidence_refs} />
        </div>
      );
    case "add_validation_step":
      return (
        <div className="border-l-2 border-purple-500 pl-2">
          <div className="text-xs text-purple-600">+ Validation</div>
          <div>{change.text}</div>
          <RefsList refs={change.evidence_refs} />
        </div>
      );
    case "create_skill_draft":
      return (
        <div className="border-l-2 border-teal-500 pl-2 space-y-1">
          <div className="text-xs text-teal-600">
            = Create skill: {change.suggested_name}
          </div>
          {change.when_to_use && change.when_to_use.length > 0 && (
            <div>
              <div className="text-xs text-muted-foreground">When to use:</div>
              {change.when_to_use.map((w, i) => (
                <div key={i} className="text-xs ml-2">· {w}</div>
              ))}
            </div>
          )}
          {change.core_steps && change.core_steps.length > 0 && (
            <div>
              <div className="text-xs text-muted-foreground">Steps:</div>
              {change.core_steps.map((s, i) => (
                <div key={i} className="text-xs ml-2">· {s}</div>
              ))}
            </div>
          )}
          {change.boundaries && change.boundaries.length > 0 && (
            <div>
              <div className="text-xs text-muted-foreground">Boundaries:</div>
              {change.boundaries.map((b, i) => (
                <div key={i} className="text-xs ml-2">· {b}</div>
              ))}
            </div>
          )}
        </div>
      );
    default:
      return <div className="text-xs text-muted-foreground">Unknown change kind: {change.kind}</div>;
  }
}

function RefsList({ refs }: { refs?: string[] }) {
  if (!refs || refs.length === 0) return null;
  return (
    <div className="text-xs text-muted-foreground mt-0.5">
      refs: {refs.join(", ")}
    </div>
  );
}

export default SkillProposalInbox;
