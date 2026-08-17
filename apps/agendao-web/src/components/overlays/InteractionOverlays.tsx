import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/i18n/I18nProvider";
import { CheckIcon } from "lucide-react";
import type {
  PermissionInteractionRecord,
  PermissionReplyChoice,
  QuestionAnswerValue,
  QuestionInteractionRecord,
} from "@/lib/interaction";

interface InteractionOverlaysProps {
  question: QuestionInteractionRecord | null;
  permission: PermissionInteractionRecord | null;
  questionAnswers: Record<number, QuestionAnswerValue>;
  questionSubmitting: boolean;
  permissionSubmitting: boolean;
  permissionSubmitError: string | null;
  permissionSubmitStartedAt: string | null;
  permissionSubmitCompletedAt: string | null;
  onQuestionAnswerChange: (index: number, value: QuestionAnswerValue) => void;
  onRejectQuestion: () => void;
  onSubmitQuestion: () => void;
  onReplyPermission: (reply: PermissionReplyChoice) => void;
}

interface PermissionOption {
  reply: PermissionReplyChoice;
  label: string;
  dangerous?: boolean;
}

function shouldCollapseValue(value: string): boolean {
  return value.includes("\n") || value.length > 96;
}

function collapsedPreview(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function CollapsibleCodeValue({
  value,
  testId,
}: {
  value: string;
  testId?: string;
}) {
  if (!shouldCollapseValue(value)) {
    return (
      <code
        className="block overflow-hidden rounded-2xl border border-border/45 bg-background/72 px-3 py-2 font-mono text-[12px] leading-5 text-foreground break-all"
        data-testid={testId}
      >
        {value}
      </code>
    );
  }

  const preview = collapsedPreview(value);

  return (
    <details className="group rounded-2xl border border-border/45 bg-background/72" data-testid={testId}>
      <summary className="flex cursor-pointer list-none items-start justify-between gap-3 px-3 py-2.5">
        <code className="line-clamp-2 flex-1 font-mono text-[12px] leading-5 text-foreground break-all">
          {preview}
        </code>
        <span className="shrink-0 rounded-full border border-border/50 bg-background/80 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground transition-colors group-open:text-foreground">
          <span className="group-open:hidden">Expand</span>
          <span className="hidden group-open:inline">Collapse</span>
        </span>
      </summary>
      <pre className="max-h-56 overflow-auto border-t border-border/45 px-3 py-3 text-[12px] leading-5 text-foreground whitespace-pre-wrap break-all">
        <code>{value}</code>
      </pre>
    </details>
  );
}

export function InteractionOverlays({
  question,
  permission,
  questionAnswers,
  questionSubmitting,
  permissionSubmitting,
  permissionSubmitError,
  permissionSubmitStartedAt,
  permissionSubmitCompletedAt,
  onQuestionAnswerChange,
  onRejectQuestion,
  onSubmitQuestion,
  onReplyPermission,
}: InteractionOverlaysProps) {
  const overlayShellClassName =
    "w-full max-w-xl max-h-[min(42rem,calc(100vh-2rem))] overflow-hidden rounded-3xl border border-border bg-card shadow-2xl";
  const { t } = useI18n();
  const [selectedPermissionOption, setSelectedPermissionOption] = useState(0);
  // "Full access" mutates the session permission mode for the whole session;
  // require a second click so a stray click cannot flip the mode.
  const [confirmingFullAccess, setConfirmingFullAccess] = useState(false);
  const permissionOptionButtonsRef = useRef<Array<HTMLButtonElement | null>>([]);
  const permissionId = permission?.permission_id;
  const permissionTarget =
    permission?.grant_target_summary ?? permission?.scope_label ?? permission?.scope_key;
  const permissionOptions: PermissionOption[] = permission
    ? [
        ...(permission.supported_lifetimes ?? ["once"])
          .filter((lifetime, index, lifetimes) =>
            ["once", "turn", "session", "always"].includes(lifetime) &&
            lifetimes.findIndex((candidate) =>
              (candidate === "always" ? "session" : candidate) ===
              (lifetime === "always" ? "session" : lifetime),
            ) === index
          )
          .map((lifetime): PermissionOption => {
            if (lifetime === "turn") {
              return {
                reply: "turn",
                label: t("overlay.allowTurn", { target: permissionTarget ?? "" }),
              };
            }
            if (lifetime === "session" || lifetime === "always") {
              return {
                reply: "session",
                label: t("overlay.allowSession", { target: permissionTarget ?? "" }),
              };
            }
            return { reply: "once", label: t("overlay.allowOnce") };
          }),
        { reply: "trust_workspace", label: t("overlay.trustWorkspace") },
        {
          reply: "full_access",
          label: t("overlay.fullAccess"),
          dangerous: true,
        },
        { reply: "reject", label: t("overlay.deny"), dangerous: true },
      ]
    : [];

  useEffect(() => {
    if (!permissionId) return;
    setSelectedPermissionOption(0);
    setConfirmingFullAccess(false);
    const frame = requestAnimationFrame(() => permissionOptionButtonsRef.current[0]?.focus());
    return () => cancelAnimationFrame(frame);
  }, [permissionId]);

  useEffect(() => {
    if (!confirmingFullAccess) return;
    const timer = window.setTimeout(() => setConfirmingFullAccess(false), 4000);
    return () => window.clearTimeout(timer);
  }, [confirmingFullAccess]);

  const handlePermissionSelect = (reply: PermissionReplyChoice) => {
    if (reply === "full_access" && !confirmingFullAccess) {
      setConfirmingFullAccess(true);
      return;
    }
    onReplyPermission(reply);
  };

  const focusPermissionOption = (index: number) => {
    if (permissionOptions.length === 0) return;
    const nextIndex = Math.max(0, Math.min(index, permissionOptions.length - 1));
    setSelectedPermissionOption(nextIndex);
    permissionOptionButtonsRef.current[nextIndex]?.focus();
  };

  return (
    <>
      {question ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm"
          data-testid="question-overlay"
        >
          <section className={overlayShellClassName} data-testid="question-modal" onClick={(event) => event.stopPropagation()}>
            <div className="flex max-h-[inherit] flex-col gap-5 p-5 sm:p-6">
            <header className="flex shrink-0 items-center justify-between">
              <h2>{t("overlay.question")}</h2>
            </header>
            <div className="min-h-0 overflow-y-auto pr-1">
              <div className="flex flex-col gap-5">
              {question.questions.map((item, index) => (
                <div key={`question-${index}`} className="grid gap-3">
                  {item.header ? <p className="text-xs uppercase tracking-[0.2em] text-muted-foreground">{item.header}</p> : null}
                  <p>{item.question}</p>
                  {item.options?.length ? (
                    <div className="flex flex-wrap gap-2">
                      {item.options.map((option) => (
                        (() => {
                          const current = questionAnswers[index];
                          const selectedValues = Array.isArray(current)
                            ? current
                            : current
                              ? [current]
                              : [];
                          const isSelected = selectedValues.includes(option.label);
                          return (
                        <button
                          key={option.label}
                          type="button"
                          data-testid="question-option"
                          data-question-index={index}
                          data-option-value={option.label}
                          className={
                            isSelected ? "px-4 py-2 rounded-full border-0 cursor-pointer text-sm bg-foreground text-background font-semibold" : "px-4 py-2 rounded-full border border-border cursor-pointer text-sm bg-card/70 text-foreground hover:bg-accent"
                          }
                          title={option.description}
                          onClick={() => {
                            if (item.multiple) {
                              onQuestionAnswerChange(
                                index,
                                isSelected
                                  ? selectedValues.filter((value) => value !== option.label)
                                  : [...selectedValues, option.label],
                              );
                              return;
                            }
                            onQuestionAnswerChange(index, option.label);
                          }}
                        >
                          {option.label}
                        </button>
                          );
                        })()
                      ))}
                    </div>
                  ) : (
                    <textarea
                      data-testid="question-input"
                      data-question-index={index}
                      className="min-h-[96px] rounded-xl border border-border/45 bg-background/70 px-4 py-3 text-sm text-foreground"
                      value={
                        Array.isArray(questionAnswers[index])
                          ? questionAnswers[index].join("\n")
                          : (questionAnswers[index] ?? "")
                      }
                      onChange={(event) => onQuestionAnswerChange(index, event.target.value)}
                    />
                  )}
                </div>
              ))}
              </div>
            </div>
            <footer className="flex shrink-0 items-center justify-end gap-3 border-t border-border pt-3">
              <button
                className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                type="button"
                data-testid="question-reject"
                disabled={questionSubmitting}
                onClick={onRejectQuestion}
              >
                {t("overlay.reject")}
              </button>
              <button
                className="min-h-[36px] rounded-full px-5 bg-foreground border-foreground text-background text-sm font-semibold inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px"
                type="button"
                data-testid="question-submit"
                disabled={questionSubmitting}
                onClick={onSubmitQuestion}
              >
                {t("overlay.submit")}
              </button>
            </footer>
            </div>
          </section>
        </div>
      ) : null}

      {permission ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm" data-testid="permission-overlay">
          <section
            className={overlayShellClassName}
            data-testid="permission-modal"
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
              if (permissionSubmitting || permissionOptions.length === 0) return;
              if (event.key === "ArrowUp") {
                event.preventDefault();
                focusPermissionOption(selectedPermissionOption - 1);
              } else if (event.key === "ArrowDown") {
                event.preventDefault();
                focusPermissionOption(selectedPermissionOption + 1);
              } else if (event.key === "Home") {
                event.preventDefault();
                focusPermissionOption(0);
              } else if (event.key === "End") {
                event.preventDefault();
                focusPermissionOption(permissionOptions.length - 1);
              } else if (event.key === "Enter") {
                event.preventDefault();
                handlePermissionSelect(permissionOptions[selectedPermissionOption].reply);
              }
            }}
          >
            <div className="flex max-h-[inherit] flex-col gap-5 p-5 sm:p-6">
            <header className="flex shrink-0 items-center justify-between">
              <h2>{t("overlay.permission")}</h2>
            </header>
            <div className="min-h-0 overflow-y-auto pr-1">
              <div className="flex flex-col gap-5">
              {permission.message ? <p>{permission.message}</p> : null}
              {permissionSubmitError ? (
                <p
                  className="rounded-2xl border border-(--ds-error)/35 bg-(--ds-error)/10 px-4 py-3 text-sm text-(--ds-error)"
                  data-testid="permission-submit-error"
                >
                  {permissionSubmitError}
                </p>
              ) : null}
              {permissionSubmitStartedAt || permissionSubmitCompletedAt ? (
                <dl className="flex flex-col gap-3 text-sm text-muted-foreground">
                  {permissionSubmitStartedAt ? (
                    <div className="flex flex-col gap-1">
                      <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">
                        Submit started
                      </dt>
                      <dd data-testid="permission-submit-started">{permissionSubmitStartedAt}</dd>
                    </div>
                  ) : null}
                  {permissionSubmitCompletedAt ? (
                    <div className="flex flex-col gap-1">
                      <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">
                        Last submit done
                      </dt>
                      <dd data-testid="permission-submit-completed">{permissionSubmitCompletedAt}</dd>
                    </div>
                  ) : null}
                </dl>
              ) : null}
              <dl className="flex flex-col gap-3 text-sm">
                {permission.permission ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Permission</dt>
                    <dd className="leading-6 text-foreground">{permission.permission}</dd>
                  </div>
                ) : null}
                {permission.permission_class_label ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Class</dt>
                    <dd className="leading-6 text-foreground">{permission.permission_class_label}</dd>
                  </div>
                ) : null}
                {permission.scope_label || permission.scope_key ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Scope</dt>
                    <dd className="leading-6 text-foreground">{permission.scope_label ?? permission.scope_key}</dd>
                  </div>
                ) : null}
                {permission.grant_target_summary ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Target</dt>
                    <dd className="leading-6 text-foreground">{permission.grant_target_summary}</dd>
                  </div>
                ) : null}
                {permission.matcher_label ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Match</dt>
                    <dd className="leading-6 text-foreground">{permission.matcher_label}</dd>
                  </div>
                ) : null}
                {permission.grant_hint ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Grant</dt>
                    <dd className="leading-6 text-foreground">{permission.grant_hint}</dd>
                  </div>
                ) : null}
                {permission.risk_tags?.length ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Risk</dt>
                    <dd className="leading-6 text-foreground">{permission.risk_tags.join(", ")}</dd>
                  </div>
                ) : null}
                {permission.command ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Command</dt>
                    <dd className="mt-1">
                      <CollapsibleCodeValue value={permission.command} testId="permission-command" />
                    </dd>
                  </div>
                ) : null}
                {permission.filepath ? (
                  <div className="flex flex-col gap-1.5">
                    <dt className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground/75">Path</dt>
                    <dd className="mt-1">
                      <CollapsibleCodeValue value={permission.filepath} testId="permission-path" />
                    </dd>
                  </div>
                ) : null}
              </dl>
              </div>
            </div>
            <p
              className="shrink-0 text-xs leading-5 text-muted-foreground"
              data-testid="permission-timeout-hint"
            >
              {t("overlay.timeoutHint")}
            </p>
            <footer
              className="grid shrink-0 gap-1.5 border-t border-border pt-3"
              role="radiogroup"
              aria-label="Permission choices"
            >
              {permissionOptions.map((option, index) => {
                const selected = index === selectedPermissionOption;
                return (
                <button
                  key={option.reply}
                  ref={(button) => {
                    permissionOptionButtonsRef.current[index] = button;
                  }}
                  className={`grid min-h-[38px] w-full grid-cols-[1.25rem_minmax(0,1fr)] items-center gap-2 rounded-lg border px-3 py-2 text-left text-sm transition-colors ${
                    selected
                      ? option.dangerous
                        ? "border-(--ds-error)/55 bg-(--ds-error)/10 text-(--ds-error)"
                        : "border-foreground/45 bg-accent text-foreground"
                      : "border-border/55 bg-card/70 text-foreground hover:bg-accent/70"
                  }`}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  tabIndex={selected ? 0 : -1}
                  data-testid={`permission-${option.reply.replaceAll("_", "-")}`}
                  disabled={permissionSubmitting}
                  onFocus={() => setSelectedPermissionOption(index)}
                  onClick={() => handlePermissionSelect(option.reply)}
                >
                  <span
                    className={`flex size-4 items-center justify-center rounded-full border ${
                      selected ? "border-current" : "border-border"
                    }`}
                    aria-hidden="true"
                  >
                    {selected ? <CheckIcon className="size-3" strokeWidth={2.5} /> : null}
                  </span>
                  <span className="min-w-0 break-words">
                    {option.reply === "full_access" && confirmingFullAccess
                      ? t("overlay.fullAccessConfirm")
                      : option.label}
                  </span>
                </button>
                );
              })}
            </footer>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
