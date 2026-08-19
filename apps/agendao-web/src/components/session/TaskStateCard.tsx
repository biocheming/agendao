import {
  CheckCircleIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  ExternalLinkIcon,
  PencilIcon,
  PlusIcon,
  ShieldCheckIcon,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useI18n } from "@/i18n/I18nProvider";
import { ApiHttpError, apiJson } from "@/lib/api";
import {
  type SessionTaskLedger,
  type TaskLedgerWriteResponse,
} from "@/lib/taskLedger";
import { useAgendaoStore } from "@/store";

const STATUS_LABEL_KEYS: Record<string, string> = {
  active: "taskLedger.statusActive",
  awaiting_user: "taskLedger.statusAwaiting",
  blocked: "taskLedger.statusBlocked",
  interrupted: "taskLedger.statusInterrupted",
  completed: "taskLedger.statusCompleted",
};

type EditTarget =
  | { kind: "goal" }
  | { kind: "next" }
  | { kind: "core"; slot?: number }
  | { kind: "close_open"; openId: string };

function lines(value: string): string[] {
  return value
    .split("\n")
    .map((entry) => entry.trim())
    .filter(Boolean);
}

function isRevisionConflict(error: unknown): boolean {
  return (
    (error instanceof ApiHttpError && error.status === 409) ||
    (error instanceof Error && /revision conflict/i.test(error.message))
  );
}

interface TaskStateCardProps {
  onNavigateStage?: (stageId: string) => void;
}

/** Typed task-governance view. All mutations use server CAS authority. */
export function TaskStateCard({ onNavigateStage }: TaskStateCardProps) {
  const { t } = useI18n();
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const ledger = useAgendaoStore((s) =>
    selectedSessionId ? s.taskLedgers[selectedSessionId] : undefined,
  );
  const setTaskLedger = useAgendaoStore((s) => s.setTaskLedger);
  const setBanner = useAgendaoStore((s) => s.setBanner);
  const setSelectedFilePath = useAgendaoStore((s) => s.setSelectedFilePath);
  const setWorkspacePanelTab = useAgendaoStore((s) => s.setWorkspacePanelTab);
  const setRightSidebarOpen = useAgendaoStore((s) => s.setRightSidebarOpen);
  const [expanded, setExpanded] = useState(false);
  const [editTarget, setEditTarget] = useState<EditTarget | null>(null);
  const [primary, setPrimary] = useState("");
  const [secondary, setSecondary] = useState("");
  const [selectedCriteria, setSelectedCriteria] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);

  if (!ledger || ledger.revision === 0 || !ledger.goal || !selectedSessionId) return null;

  const open = ledger.projection.open_questions;
  const verified = ledger.projection.current_checkpoints;
  const latest = verified[verified.length - 1];
  const live = ledger.projection.live_core;

  const openEditor = (target: EditTarget) => {
    setEditTarget(target);
    setSelectedCriteria([]);
    if (target.kind === "goal") {
      setPrimary(ledger.goal?.statement ?? "");
      setSecondary((ledger.goal?.acceptance_criteria ?? []).join("\n"));
    } else if (target.kind === "next") {
      setPrimary(ledger.next?.statement ?? "");
      setSecondary("");
    } else if (target.kind === "core") {
      setPrimary(target.slot ? live[target.slot - 1]?.statement ?? "" : "");
      setSecondary("");
    } else {
      setPrimary("");
      setSecondary("");
    }
  };

  const refreshAfterConflict = async () => {
    const latestLedger = await apiJson<SessionTaskLedger>(
      `/session/${selectedSessionId}/task-ledger`,
    );
    setTaskLedger(selectedSessionId, latestLedger);
    setBanner(t("taskLedger.conflict"), "error");
  };

  const commit = async (path: string, body: unknown) => {
    try {
      const response = await apiJson<TaskLedgerWriteResponse>(path, {
        method: path.endsWith("/task-ledger") ? "PATCH" : "POST",
        body: JSON.stringify(body),
      });
      setTaskLedger(selectedSessionId, response.ledger);
      setBanner(t("taskLedger.saved"), "success");
      setEditTarget(null);
    } catch (error) {
      if (isRevisionConflict(error)) {
        await refreshAfterConflict();
        return;
      }
      setBanner(error instanceof Error ? error.message : t("taskLedger.saveFailed"), "error");
    }
  };

  const submitEdit = async () => {
    if (!editTarget || !primary.trim()) return;
    setSubmitting(true);
    try {
      if (editTarget.kind === "goal") {
        const acceptanceCriteria = lines(secondary);
        await commit(`/session/${selectedSessionId}/task-ledger`, {
          expected_revision: ledger.revision,
          op: {
            op: "set_goal",
            goal: {
              statement: primary.trim(),
              acceptance_criteria: acceptanceCriteria,
              criterion_checks: (ledger.goal?.criterion_checks ?? []).filter((check) =>
                acceptanceCriteria.includes(check.criterion),
              ),
              set_by: "user",
              set_at: Date.now(),
            },
          },
        });
      } else if (editTarget.kind === "next") {
        await commit(`/session/${selectedSessionId}/task-ledger`, {
          expected_revision: ledger.revision,
          op: { op: "set_next", statement: primary.trim(), actor: "user" },
        });
      } else if (editTarget.kind === "core") {
        await commit(`/session/${selectedSessionId}/task-ledger`, {
          expected_revision: ledger.revision,
          op: editTarget.slot
            ? {
                op: "swap_core_live",
                slot: editTarget.slot,
                statement: primary.trim(),
                actor: "user",
              }
            : { op: "add_core", statement: primary.trim(), live: true, actor: "user" },
        });
      } else {
        await commit(
          `/session/${selectedSessionId}/task-ledger/open/${encodeURIComponent(editTarget.openId)}/close`,
          {
            expected_revision: ledger.revision,
            claim: primary.trim(),
            verifier: { user_confirmation: { actor: "user" } },
            coverage: { scope: secondary.trim() },
            covered_criteria: selectedCriteria,
            evidence_artifact_ids: [],
            source_stage_id: null,
          },
        );
      }
    } finally {
      setSubmitting(false);
    }
  };

  const navigateArtifact = (artifactId: string) => {
    setSelectedFilePath(artifactId);
    setWorkspacePanelTab("files");
    setRightSidebarOpen(true);
  };

  return (
    <div className="mx-auto w-full max-w-[88rem] px-4 md:px-5" data-testid="task-state-card">
      <div className="rounded-xl border border-border bg-card/70 px-4 py-3 text-sm">
        <button
          type="button"
          className="flex w-full items-center gap-2 text-left"
          onClick={() => setExpanded((value) => !value)}
          aria-expanded={expanded}
        >
          <span
            className="rounded-full border border-border/60 px-2 py-0.5 text-[11px] uppercase text-muted-foreground"
            data-testid="task-state-status"
          >
            {STATUS_LABEL_KEYS[ledger.status]
              ? t(STATUS_LABEL_KEYS[ledger.status])
              : ledger.status}
          </span>
          <span className="min-w-0 flex-1 truncate text-foreground/85">
            <span className="text-muted-foreground">{t("taskLedger.goal")}: </span>
            {ledger.goal.statement}
          </span>
          {ledger.next ? (
            <span className="hidden max-w-[40%] truncate text-foreground md:inline" data-testid="task-state-next">
              <span className="text-muted-foreground">{t("taskLedger.next")}: </span>
              {ledger.next.statement}
            </span>
          ) : null}
          {expanded ? <ChevronUpIcon className="size-4 shrink-0" /> : <ChevronDownIcon className="size-4 shrink-0" />}
        </button>

        {expanded ? (
          <div className="mt-3 grid gap-3 border-t border-border pt-3 text-[13px] leading-6">
            <section>
              <div className="flex items-start gap-2">
                <p className="min-w-0 flex-1">
                  <span className="text-muted-foreground">{t("taskLedger.goal")}: </span>
                  {ledger.goal.statement}
                  <span className="ml-2 text-xs text-muted-foreground">{ledger.goal.set_by}</span>
                </p>
                <Button variant="ghost" size="icon-xs" onClick={() => openEditor({ kind: "goal" })} title={t("taskLedger.editGoal")}>
                  <PencilIcon />
                </Button>
              </div>
            </section>

            {ledger.next ? (
              <div className="flex items-start gap-2" data-testid="task-state-next-expanded">
                <p className="min-w-0 flex-1 break-words">
                  <span className="text-muted-foreground">{t("taskLedger.next")}: </span>
                  {ledger.next.statement}
                  <span className="ml-2 text-xs text-muted-foreground">
                    {ledger.next.provenance.actor} · r{ledger.revision}
                  </span>
                </p>
                <Button variant="ghost" size="icon-xs" onClick={() => openEditor({ kind: "next" })} title={t("taskLedger.editNext")}>
                  <PencilIcon />
                </Button>
              </div>
            ) : null}

            <section>
              <div className="flex items-center justify-between">
                <p className="text-muted-foreground">{t("taskLedger.core")}</p>
                {live.length < 2 ? (
                  <Button variant="ghost" size="icon-xs" onClick={() => openEditor({ kind: "core" })} title={t("taskLedger.addCore")}>
                    <PlusIcon />
                  </Button>
                ) : null}
              </div>
              {live.map((entry, index) => (
                <div key={entry.id} className="flex items-start gap-2 pl-3">
                  <p className="min-w-0 flex-1 break-words">
                    {entry.id} · {entry.statement}
                    <span className="ml-2 text-xs text-muted-foreground">{entry.set_by ?? "system"}</span>
                  </p>
                  <Button variant="ghost" size="icon-xs" onClick={() => openEditor({ kind: "core", slot: index + 1 })} title={t("taskLedger.editCore")}>
                    <PencilIcon />
                  </Button>
                </div>
              ))}
            </section>

            {open.length > 0 ? (
              <section data-testid="task-state-open">
                <p className="text-muted-foreground">{t("taskLedger.open")}</p>
                <ul className="grid gap-1 pl-3">
                  {open.slice(0, 6).map((question) => (
                    <li key={question.id} className="flex items-start gap-2">
                      <span className="min-w-0 flex-1 break-words">
                        <strong>{question.id}</strong> · {question.question}
                        <span className="text-muted-foreground"> — {question.settled_by}</span>
                      </span>
                      <Button variant="ghost" size="icon-xs" onClick={() => openEditor({ kind: "close_open", openId: question.id })} title={t("taskLedger.closeOpen")}>
                        <CheckCircleIcon />
                      </Button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}

            {latest ? (
              <section className="flex items-start gap-2" data-testid="task-state-verified">
                <ShieldCheckIcon className="mt-1 size-3.5 shrink-0 text-(--ds-success,currentColor)" />
                <div className="min-w-0 flex-1">
                  <p className="break-words">
                    <span className="text-muted-foreground">{latest.id} {t("taskLedger.verified")}: </span>
                    {latest.claim}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {latest.verifier_label} · {latest.coverage.scope}
                  </p>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {latest.source_stage_id && onNavigateStage ? (
                      <Button variant="outline" size="xs" onClick={() => onNavigateStage(latest.source_stage_id!)}>
                        <ExternalLinkIcon /> {t("taskLedger.stageEvidence")} {latest.source_stage_id}
                      </Button>
                    ) : null}
                    {(latest.evidence_artifact_ids ?? []).map((artifactId) => (
                      <Button key={artifactId} variant="outline" size="xs" onClick={() => navigateArtifact(artifactId)}>
                        <ExternalLinkIcon /> {artifactId}
                      </Button>
                    ))}
                  </div>
                </div>
              </section>
            ) : null}

            {ledger.blocked_reason ? <p className="text-(--ds-error,currentColor)">{t("taskLedger.blocked")}: {ledger.blocked_reason}</p> : null}
            {ledger.uncovered_criteria?.length ? (
              <div className="text-(--ds-warning,currentColor)" data-testid="task-state-uncovered">
                <p>{t("taskLedger.uncovered")}</p>
                <ul className="list-disc pl-5">{ledger.uncovered_criteria.map((criterion) => <li key={criterion}>{criterion}</li>)}</ul>
              </div>
            ) : null}
            {ledger.projection.missing_acceptance_criteria.length ? (
              <div className="text-(--ds-warning,currentColor)" data-testid="task-state-missing-evidence">
                <p>{t("taskLedger.missingEvidence")}</p>
                <ul className="list-disc pl-5">
                  {ledger.projection.missing_acceptance_criteria.map((criterion) => (
                    <li key={criterion}>{criterion}</li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      <Dialog open={editTarget !== null} onOpenChange={(value) => !value && setEditTarget(null)}>
        <DialogContent data-testid="task-ledger-edit-dialog">
          <DialogHeader>
            <DialogTitle>
              {editTarget?.kind === "close_open" ? t("taskLedger.closeOpen") : t("taskLedger.editState")}
            </DialogTitle>
            <DialogDescription>{t("taskLedger.revisionHint", { revision: ledger.revision })}</DialogDescription>
          </DialogHeader>
          <label className="grid gap-1 text-sm">
            <span>{editTarget?.kind === "close_open" ? t("taskLedger.claim") : t("taskLedger.statement")}</span>
            <Textarea value={primary} onChange={(event) => setPrimary(event.target.value)} data-testid="task-ledger-primary" />
          </label>
          {editTarget?.kind === "goal" ? (
            <label className="grid gap-1 text-sm">
              <span>{t("taskLedger.criteria")}</span>
              <Textarea value={secondary} onChange={(event) => setSecondary(event.target.value)} />
            </label>
          ) : null}
          {editTarget?.kind === "close_open" ? (
            <>
              <label className="grid gap-1 text-sm">
                <span>{t("taskLedger.coverage")}</span>
                <Input value={secondary} onChange={(event) => setSecondary(event.target.value)} data-testid="task-ledger-coverage" />
              </label>
              {(ledger.goal.acceptance_criteria ?? []).length ? (
                <fieldset className="grid gap-2 text-sm">
                  <legend>{t("taskLedger.coveredCriteria")}</legend>
                  {ledger.goal.acceptance_criteria?.map((criterion) => (
                    <label key={criterion} className="flex items-start gap-2">
                      <input
                        type="checkbox"
                        checked={selectedCriteria.includes(criterion)}
                        onChange={(event) => setSelectedCriteria((current) =>
                          event.target.checked ? [...current, criterion] : current.filter((entry) => entry !== criterion),
                        )}
                      />
                      <span>{criterion}</span>
                    </label>
                  ))}
                </fieldset>
              ) : null}
            </>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditTarget(null)}>{t("taskLedger.cancel")}</Button>
            <Button
              onClick={() => void submitEdit()}
              disabled={submitting || !primary.trim() || (editTarget?.kind === "close_open" && !secondary.trim())}
              data-testid="task-ledger-save"
            >
              {submitting ? t("taskLedger.saving") : t("taskLedger.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
