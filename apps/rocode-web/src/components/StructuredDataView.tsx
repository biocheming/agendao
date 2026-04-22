import { ChevronDownIcon } from "lucide-react";

interface StructuredDataViewProps {
  value: unknown;
  emptyLabel?: string;
  onNavigateKeyValue?: (key: string, value: string) => void;
}

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

function nestedValueSummary(value: unknown) {
  if (Array.isArray(value)) {
    return value.length === 1 ? "1 item" : `${value.length} items`;
  }
  if (value && typeof value === "object") {
    const size = Object.keys(value as Record<string, unknown>).length;
    return size === 1 ? "1 field" : `${size} fields`;
  }
  return valueTypeLabel(value);
}

export function StructuredDataView({
  value,
  emptyLabel = "No structured data.",
  onNavigateKeyValue,
}: StructuredDataViewProps) {
  if (value === null || value === undefined) {
    return <div className="roc-structured-empty">{emptyLabel}</div>;
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
      return <div className="roc-structured-empty">{emptyLabel}</div>;
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
                <span className="roc-structured-summary-note">{nestedValueSummary(entry)}</span>
              </div>
              <span className="inline-flex items-center gap-2">
                <span className="roc-structured-summary-meta">{valueTypeLabel(entry)}</span>
                <ChevronDownIcon className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
              </span>
            </summary>
            <div className="roc-structured-body">
              <StructuredDataView value={entry} emptyLabel="Empty item." />
            </div>
          </details>
        ))}
      </div>
    );
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (entries.length === 0) {
    return <div className="roc-structured-empty">{emptyLabel}</div>;
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
                    Open
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
              <span className="roc-structured-summary-note">{nestedValueSummary(entry)}</span>
            </div>
            <span className="inline-flex items-center gap-2">
              <span className="roc-structured-summary-meta">{valueTypeLabel(entry)}</span>
              <ChevronDownIcon className="size-4 text-muted-foreground transition-transform group-open:rotate-180" />
            </span>
          </summary>
          <div className="roc-structured-body">
            <StructuredDataView value={entry} emptyLabel={`No data in ${key}.`} />
          </div>
        </details>
      ))}
    </div>
  );
}
