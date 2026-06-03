import type {
  PermissionInteractionRecord,
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
  onReplyPermission: (reply: "once" | "turn" | "session" | "reject") => void;
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

  return (
    <>
      {question ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm" data-testid="question-overlay" onClick={onRejectQuestion}>
          <section className={overlayShellClassName} data-testid="question-modal" onClick={(event) => event.stopPropagation()}>
            <div className="flex max-h-[inherit] flex-col gap-5 p-5 sm:p-6">
            <header className="flex shrink-0 items-center justify-between">
              <h2>Question</h2>
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
                Reject
              </button>
              <button
                className="min-h-[36px] rounded-full px-5 bg-foreground border-foreground text-background text-sm font-semibold inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px"
                type="button"
                data-testid="question-submit"
                disabled={questionSubmitting}
                onClick={onSubmitQuestion}
              >
                Submit
              </button>
            </footer>
            </div>
          </section>
        </div>
      ) : null}

      {permission ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm" data-testid="permission-overlay">
          <section className={overlayShellClassName} data-testid="permission-modal" onClick={(event) => event.stopPropagation()}>
            <div className="flex max-h-[inherit] flex-col gap-5 p-5 sm:p-6">
            <header className="flex shrink-0 items-center justify-between">
              <h2>Permission</h2>
            </header>
            <div className="min-h-0 overflow-y-auto pr-1">
              <div className="flex flex-col gap-5">
              {permission.message ? <p>{permission.message}</p> : null}
              {permissionSubmitError ? (
                <p
                  className="rounded-2xl border border-rose-400/35 bg-rose-500/10 px-4 py-3 text-sm text-rose-100"
                  data-testid="permission-submit-error"
                >
                  {permissionSubmitError}
                </p>
              ) : null}
              {permissionSubmitStartedAt || permissionSubmitCompletedAt ? (
                <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm text-muted-foreground">
                  {permissionSubmitStartedAt ? (
                    <>
                      <dt>Submit started</dt>
                      <dd data-testid="permission-submit-started">{permissionSubmitStartedAt}</dd>
                    </>
                  ) : null}
                  {permissionSubmitCompletedAt ? (
                    <>
                      <dt>Last submit done</dt>
                      <dd data-testid="permission-submit-completed">{permissionSubmitCompletedAt}</dd>
                    </>
                  ) : null}
                </dl>
              ) : null}
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
                {permission.permission ? (
                  <div>
                    <dt>Permission</dt>
                    <dd>{permission.permission}</dd>
                  </div>
                ) : null}
                {permission.permission_class_label ? (
                  <div>
                    <dt>Class</dt>
                    <dd>{permission.permission_class_label}</dd>
                  </div>
                ) : null}
                {permission.scope_label || permission.scope_key ? (
                  <div>
                    <dt>Scope</dt>
                    <dd>{permission.scope_label ?? permission.scope_key}</dd>
                  </div>
                ) : null}
                {permission.grant_target_summary ? (
                  <div>
                    <dt>Target</dt>
                    <dd>{permission.grant_target_summary}</dd>
                  </div>
                ) : null}
                {permission.matcher_label ? (
                  <div>
                    <dt>Match</dt>
                    <dd>{permission.matcher_label}</dd>
                  </div>
                ) : null}
                {permission.grant_hint ? (
                  <div>
                    <dt>Grant</dt>
                    <dd>{permission.grant_hint}</dd>
                  </div>
                ) : null}
                {permission.risk_tags?.length ? (
                  <div>
                    <dt>Risk</dt>
                    <dd>{permission.risk_tags.join(", ")}</dd>
                  </div>
                ) : null}
                {permission.command ? (
                  <div>
                    <dt>Command</dt>
                    <dd className="mt-1">
                      <CollapsibleCodeValue value={permission.command} testId="permission-command" />
                    </dd>
                  </div>
                ) : null}
                {permission.filepath ? (
                  <div>
                    <dt>Path</dt>
                    <dd className="mt-1">
                      <CollapsibleCodeValue value={permission.filepath} testId="permission-path" />
                    </dd>
                  </div>
                ) : null}
              </dl>
              </div>
            </div>
            <footer className="flex shrink-0 flex-wrap items-center justify-end gap-3 border-t border-border pt-3">
              <button
                className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                type="button"
                data-testid="permission-reject"
                disabled={permissionSubmitting}
                onClick={() => onReplyPermission("reject")}
              >
                Reject
              </button>
              {permission.supported_lifetimes?.includes("turn") ? (
                <button
                  className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                  type="button"
                  data-testid="permission-turn"
                  disabled={permissionSubmitting}
                  onClick={() => onReplyPermission("turn")}
                >
                  Allow Turn
                </button>
              ) : null}
              {permission.supported_lifetimes?.includes("session") ? (
                <button
                  className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                  type="button"
                  data-testid="permission-session"
                  disabled={permissionSubmitting}
                  onClick={() => onReplyPermission("session")}
                >
                  Allow Session
                </button>
              ) : null}
              <button
                className="min-h-[36px] rounded-full px-5 bg-foreground border-foreground text-background text-sm font-semibold inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px"
                type="button"
                data-testid="permission-once"
                disabled={permissionSubmitting}
                onClick={() => onReplyPermission("once")}
              >
                Allow Once
              </button>
            </footer>
            </div>
          </section>
        </div>
      ) : null}
    </>
  );
}
