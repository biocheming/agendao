import { ChevronDownIcon } from "lucide-react";
import { useI18n } from "../../i18n/I18nProvider";

interface StructuredDataViewProps {
  value: unknown;
  emptyLabel?: string;
  onNavigateKeyValue?: (key: string, value: string) => void;
}

type TranslateFn = (key: string, params?: Record<string, string | number>) => string;

function valueTypeLabel(value: unknown) {
  if (Array.isArray(value)) return `Array(${value.length})`;
  if (value === null) return "null";
  return typeof value;
}

function PrimitiveValue({ value }: { value: unknown }) {
  if (typeof value === "string") {
    return <pre className="roc-structured-value">{value}</pre>;
  }
  return <code className="roc-inline-fact font-mono">{String(value)}</code>;
}

function nestedValueSummary(value: unknown, t: TranslateFn) {
  if (Array.isArray(value)) {
    return t("execution.structured.items", { count: value.length });
  }
  if (value && typeof value === "object") {
    const size = Object.keys(value as Record<string, unknown>).length;
    return t("execution.structured.fields", { count: size });
  }
  return valueTypeLabel(value);
}

export function StructuredDataView({
  value,
  emptyLabel,
  onNavigateKeyValue,
}: StructuredDataViewProps) {
  const { t } = useI18n();
  const resolvedEmptyLabel = emptyLabel ?? t("execution.structured.empty");
  if (value === null || value === undefined) {
    return <div className="roc-structured-empty">{resolvedEmptyLabel}</div>;
  }

  if (typeof value !== "object") {
    return (
      <div className="roc-structured-list">
        <PrimitiveValue value={value} />
      </div>
    );
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return <div className="roc-structured-empty">{resolvedEmptyLabel}</div>;
    }

    return (
      <div className="roc-structured-list">
        {value.map((entry, index) => (
          <details
            key={`array-entry-${index}`}
            className="roc-structured-disclosure group"
            open={index < 2}
          >
            <summary className="roc-structured-summary">
              <div className="roc-structured-summary-copy">
                <span className="roc-structured-summary-label">[{index}]</span>
                <span className="roc-structured-summary-note">{nestedValueSummary(entry, t)}</span>
              </div>
              <span className="inline-flex items-center gap-2">
                <span className="roc-structured-summary-meta">{valueTypeLabel(entry)}</span>
                <ChevronDownIcon className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
              </span>
            </summary>
            <div className="roc-structured-body">
              <StructuredDataView value={entry} emptyLabel={t("execution.structured.emptyItem")} />
            </div>
          </details>
        ))}
      </div>
    );
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0) {
    return <div className="roc-structured-empty">{resolvedEmptyLabel}</div>;
  }

  const scalarEntries = entries.filter(([, entry]) => entry === null || typeof entry !== "object");
  const nestedEntries = entries.filter(([, entry]) => entry !== null && typeof entry === "object");

  return (
    <div className="roc-structured-list">
      {scalarEntries.length ? (
        <dl className="roc-structured-dl">
          {scalarEntries.map(([key, entry]) => (
            <div key={key} className="roc-structured-row">
              <dt className="roc-structured-key">{key}</dt>
              <dd className="grid gap-2">
                <PrimitiveValue value={entry} />
                {onNavigateKeyValue && typeof entry === "string" ? (
                  <button
                    className="roc-rail-link justify-self-start"
                    type="button"
                    onClick={() => onNavigateKeyValue(key, entry)}
                  >
                    {t("execution.open")}
                  </button>
                ) : null}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
      {nestedEntries.map(([key, entry]) => (
        <details key={key} className="roc-structured-disclosure group" open>
          <summary className="roc-structured-summary">
            <div className="roc-structured-summary-copy">
              <span className="roc-structured-summary-label">{key}</span>
              <span className="roc-structured-summary-note">{nestedValueSummary(entry, t)}</span>
            </div>
            <span className="inline-flex items-center gap-2">
              <span className="roc-structured-summary-meta">{valueTypeLabel(entry)}</span>
              <ChevronDownIcon className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
            </span>
          </summary>
          <div className="roc-structured-body">
            <StructuredDataView value={entry} emptyLabel={t("execution.structured.noDataIn", { key })} />
          </div>
        </details>
      ))}
    </div>
  );
}
