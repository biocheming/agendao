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
  onQuestionAnswerChange: (index: number, value: QuestionAnswerValue) => void;
  onRejectQuestion: () => void;
  onSubmitQuestion: () => void;
  onReplyPermission: (reply: "once" | "always" | "reject") => void;
}

export function InteractionOverlays({
  question,
  permission,
  questionAnswers,
  questionSubmitting,
  permissionSubmitting,
  onQuestionAnswerChange,
  onRejectQuestion,
  onSubmitQuestion,
  onReplyPermission,
}: InteractionOverlaysProps) {
  return (
    <>
      {question ? (
        <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-center justify-center" data-testid="question-overlay" onClick={onRejectQuestion}>
          <section className="w-full max-w-lg bg-card rounded-3xl border border-border shadow-2xl p-6 flex flex-col gap-5" data-testid="question-modal" onClick={(event) => event.stopPropagation()}>
            <header className="flex items-center justify-between">
              <h2>Question</h2>
            </header>
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
            <footer className="flex items-center justify-end gap-3 pt-3 border-t border-border">
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
          </section>
        </div>
      ) : null}

      {permission ? (
        <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm flex items-center justify-center" data-testid="permission-overlay" onClick={() => onReplyPermission("reject")}>
          <section className="w-full max-w-lg bg-card rounded-3xl border border-border shadow-2xl p-6 flex flex-col gap-5" data-testid="permission-modal" onClick={(event) => event.stopPropagation()}>
            <header className="flex items-center justify-between">
              <h2>Permission</h2>
            </header>
            <div className="flex flex-col gap-5">
              {permission.message ? <p>{permission.message}</p> : null}
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
                {permission.permission ? (
                  <div>
                    <dt>Permission</dt>
                    <dd>{permission.permission}</dd>
                  </div>
                ) : null}
                {permission.command ? (
                  <div>
                    <dt>Command</dt>
                    <dd>{permission.command}</dd>
                  </div>
                ) : null}
                {permission.filepath ? (
                  <div>
                    <dt>Path</dt>
                    <dd>{permission.filepath}</dd>
                  </div>
                ) : null}
              </dl>
            </div>
            <footer className="flex items-center justify-end gap-3 pt-3 border-t border-border">
              <button
                className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                type="button"
                data-testid="permission-reject"
                disabled={permissionSubmitting}
                onClick={() => onReplyPermission("reject")}
              >
                Reject
              </button>
              <button
                className="min-h-[36px] rounded-full px-4 border border-border bg-card/70 text-foreground text-sm inline-flex items-center justify-center cursor-pointer transition-all duration-150 hover:-translate-y-px hover:bg-accent"
                type="button"
                data-testid="permission-always"
                disabled={permissionSubmitting}
                onClick={() => onReplyPermission("always")}
              >
                Allow Always
              </button>
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
          </section>
        </div>
      ) : null}
    </>
  );
}
