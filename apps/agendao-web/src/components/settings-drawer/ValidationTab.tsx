import type {
  ConfigPolicyValidationItemRecord,
  ConfigPolicyValidationOwnerRecord,
  ConfigPolicyValidationSnapshotRecord,
} from "@/lib/configPolicy";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import { validationJumpTarget } from "./shared";

interface ValidationTabStyles {
  summaryCardClass: string;
  mutedCardClass: string;
  secondaryButtonClass: string;
}

export interface ValidationTabProps {
  styles: ValidationTabStyles;
  configValidation: ConfigPolicyValidationSnapshotRecord | null;
  validationReports: ConfigPolicyValidationItemRecord[];
  validationGroups: Array<{
    owner: ConfigPolicyValidationOwnerRecord;
    items: ConfigPolicyValidationItemRecord[];
  }>;
  validationErrorCount: number;
  validationWarningCount: number;
  onJumpToValidationTarget: (item: ConfigPolicyValidationItemRecord) => void;
}

function formatDateTime(ts?: number | null) {
  if (!ts) return "--";
  return new Date(ts).toLocaleString();
}

function validationSeverityTone(severity: string | null | undefined) {
  switch ((severity || "").toLowerCase()) {
    case "error":
      return "danger";
    case "warning":
      return "warn";
    default:
      return "muted";
  }
}

function validationOwnerLabel(owner: ConfigPolicyValidationOwnerRecord, t: (key: string) => string) {
  switch (owner) {
    case "provider_profile":
      return t("settings.validation.owner.providerProfile");
    case "external_adapter":
      return t("settings.validation.owner.externalAdapter");
  }
}

function validationEffectLabel(effect: string | null | undefined, t: (key: string) => string) {
  switch ((effect || "").toLowerCase()) {
    case "fail_closed_bootstrap":
      return t("settings.validation.effect.failClosedBootstrap");
    case "fail_closed_request_gate":
      return t("settings.validation.effect.failClosedRequestGate");
    default:
      return effect || "--";
  }
}

function validationScopeLabel(scopeKind: string | null | undefined, t: (key: string) => string) {
  switch ((scopeKind || "").toLowerCase()) {
    case "provider":
      return t("settings.validation.scope.provider");
    case "external_adapter":
      return t("settings.validation.scope.externalAdapter");
    default:
      return scopeKind || "--";
  }
}

export function ValidationTab({
  styles,
  configValidation,
  validationReports,
  validationGroups,
  validationErrorCount,
  validationWarningCount,
  onJumpToValidationTarget,
}: ValidationTabProps) {
  const {
    summaryCardClass,
    mutedCardClass,
    secondaryButtonClass,
  } = styles;
  const { t } = useI18n();
  return (
    <div className="grid gap-6" data-testid="settings-panel-validation">
      <div className="grid gap-3 sm:grid-cols-4">
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.validation.configRevision")}</span>
          <strong>{configValidation?.revision ?? "--"}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.validation.generated")}</span>
          <strong className="text-sm">{formatDateTime(configValidation?.generated_at_ms)}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.validation.errors")}</span>
          <strong>{configValidation ? validationErrorCount : "--"}</strong>
        </div>
        <div className={summaryCardClass}>
          <span className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">{t("settings.validation.warnings")}</span>
          <strong>{configValidation ? validationWarningCount : "--"}</strong>
        </div>
      </div>

      {!configValidation ? (
        <div className={mutedCardClass} data-testid="settings-validation-unavailable">
          {t("settings.validation.unavailable")}
        </div>
      ) : null}

      {configValidation && validationReports.length === 0 ? (
        <div className={mutedCardClass} data-testid="settings-validation-empty">
          {t("settings.validation.empty")}
        </div>
      ) : null}

      {configValidation && validationGroups.length > 0 ? (
        validationGroups.map((group) => (
          <div key={group.owner} className="roc-section" data-testid="settings-validation-group">
            <div className="flex items-center justify-between gap-3">
              <p className="text-xs tracking-widest uppercase text-muted-foreground font-semibold">
                {validationOwnerLabel(group.owner, t)}
              </p>
              <span className="text-sm text-muted-foreground">
                {t("settings.validation.findings", { count: group.items.length })}
              </span>
            </div>
            <div className="grid gap-3">
              {group.items.map((item) => {
                const jumpTarget = validationJumpTarget(item);
                return (
                  <div
                    key={`${item.owner}:${item.path}:${item.code}:${item.scope.subject_id ?? ""}`}
                    className="rounded-lg border border-border/35 bg-card p-4 grid gap-3"
                  >
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="grid gap-1">
                        <strong>{item.code}</strong>
                        <p className="m-0 text-sm text-muted-foreground leading-relaxed break-all">
                          <code>{item.path}</code>
                        </p>
                      </div>
                      <div className="flex flex-wrap items-center justify-end gap-2">
                        <span
                          className={cn(
                            "rounded-full border px-3 py-1.5 text-xs font-semibold",
                            validationSeverityTone(item.severity) === "danger"
                              ? "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)"
                              : validationSeverityTone(item.severity) === "warn"
                                ? "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)"
                                : "border-border bg-muted text-muted-foreground",
                          )}
                        >
                          {item.severity}
                        </span>
                        <span className="rounded-full border border-border bg-muted px-3 py-1.5 text-xs font-semibold text-muted-foreground">
                          {validationEffectLabel(item.effect, t)}
                        </span>
                        {jumpTarget ? (
                          <button
                            className={secondaryButtonClass}
                            type="button"
                            onClick={() => onJumpToValidationTarget(item)}
                          >
                            {t(jumpTarget.label)}
                          </button>
                        ) : null}
                      </div>
                    </div>
                    <p className="m-0 text-sm leading-relaxed text-foreground">
                      {item.message}
                    </p>
                    <div className="grid gap-1 text-sm text-muted-foreground">
                      <p className="m-0">
                        {t("settings.validation.scopeLine", { kind: validationScopeLabel(item.scope.kind, t) })}
                        {item.scope.subject_id ? ` · ${item.scope.subject_id}` : ""}
                      </p>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        ))
      ) : null}
    </div>
  );
}
