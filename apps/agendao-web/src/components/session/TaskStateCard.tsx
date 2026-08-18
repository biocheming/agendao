import { ChevronDownIcon, ChevronUpIcon, ShieldCheckIcon } from "lucide-react";
import { useState } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import { useAgendaoStore } from "@/store";
import { activeCheckpoints, openQuestions } from "@/lib/taskLedger";

const STATUS_LABEL_KEYS: Record<string, string> = {
  active: "taskLedger.statusActive",
  awaiting_user: "taskLedger.statusAwaiting",
  blocked: "taskLedger.statusBlocked",
  interrupted: "taskLedger.statusInterrupted",
  completed: "taskLedger.statusCompleted",
};

/**
 * Compact task-governance view: Goal, the single Next, open questions and
 * the latest verified checkpoint. Typed fields only — no inferred state,
 * no hidden reasoning. Hidden entirely until a ledger exists.
 */
export function TaskStateCard() {
  const { t } = useI18n();
  const selectedSessionId = useAgendaoStore((s) => s.selectedSessionId);
  const ledger = useAgendaoStore((s) =>
    selectedSessionId ? s.taskLedgers[selectedSessionId] : undefined,
  );
  const [expanded, setExpanded] = useState(false);

  if (!ledger || ledger.revision === 0 || !ledger.goal) return null;

  const open = openQuestions(ledger);
  const verified = activeCheckpoints(ledger);
  const latest = verified[verified.length - 1];
  const liveCore = ledger.core.filter((entry) => entry.live);

  return (
    <div
      className="mx-auto w-full max-w-[88rem] px-4 md:px-5"
      data-testid="task-state-card"
    >
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
            <span
              className="hidden max-w-[40%] truncate text-foreground md:inline"
              data-testid="task-state-next"
            >
              <span className="text-muted-foreground">{t("taskLedger.next")}: </span>
              {ledger.next.statement}
            </span>
          ) : null}
          {expanded ? (
            <ChevronUpIcon className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" />
          )}
        </button>

        {expanded ? (
          <div className="mt-3 grid gap-2 border-t border-border pt-3 text-[13px] leading-6">
            {ledger.next ? (
              <p className="md:hidden" data-testid="task-state-next-mobile">
                <span className="text-muted-foreground">{t("taskLedger.next")}: </span>
                {ledger.next.statement}
              </p>
            ) : null}
            {liveCore.length > 0 ? (
              <p className="text-muted-foreground">
                {t("taskLedger.core")}: {liveCore.map((entry) => entry.statement).join(" · ")}
              </p>
            ) : null}
            {open.length > 0 ? (
              <div data-testid="task-state-open">
                <p className="text-muted-foreground">{t("taskLedger.open")}</p>
                <ul className="ml-4 list-disc">
                  {open.slice(0, 4).map((question) => (
                    <li key={question.id}>
                      <span className="text-foreground/85">{question.question}</span>
                      <span className="text-muted-foreground">
                        {" "}
                    — {question.settled_by}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {latest ? (
              <p className="flex items-start gap-2" data-testid="task-state-verified">
                <ShieldCheckIcon className="mt-1 size-3.5 shrink-0 text-(--ds-success, currentColor)" />
                <span>
                  <span className="text-muted-foreground">
                    {latest.id} {t("taskLedger.verified")}:{" "}
                  </span>
                  {latest.claim}
                  <span className="text-muted-foreground"> — {latest.coverage.scope}</span>
                </span>
              </p>
            ) : null}
            {ledger.blocked_reason ? (
              <p className="text-(--ds-error, currentColor)" data-testid="task-state-blocked">
                {t("taskLedger.blocked")}: {ledger.blocked_reason}
              </p>
            ) : null}
            {ledger.uncovered_criteria && ledger.uncovered_criteria.length > 0 ? (
              <div className="text-(--ds-warning, currentColor)" data-testid="task-state-uncovered">
                <p>{t("taskLedger.uncovered")}</p>
                <ul className="ml-4 list-disc">
                  {ledger.uncovered_criteria.map((criterion) => (
                    <li key={criterion}>{criterion}</li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
