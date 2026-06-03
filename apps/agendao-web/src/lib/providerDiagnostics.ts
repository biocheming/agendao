export interface ProviderDiagnosticSummaryRecord {
  severity?: string | null;
  source?: string | null;
  code?: string | null;
  provider_id?: string | null;
  model_id?: string | null;
  message?: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

export function providerDiagnosticFromMetadata(
  metadata: Record<string, unknown> | null | undefined,
): ProviderDiagnosticSummaryRecord | null {
  const summary = metadata?.provider_diagnostic;
  if (!isRecord(summary)) return null;
  return {
    severity: typeof summary.severity === "string" ? summary.severity : null,
    source: typeof summary.source === "string" ? summary.source : null,
    code: typeof summary.code === "string" ? summary.code : null,
    provider_id: typeof summary.provider_id === "string" ? summary.provider_id : null,
    model_id: typeof summary.model_id === "string" ? summary.model_id : null,
    message: typeof summary.message === "string" ? summary.message : null,
  };
}

export function providerDiagnosticLabel(
  summary: ProviderDiagnosticSummaryRecord | null | undefined,
) {
  const code = summary?.code?.trim();
  if (!code) return null;
  switch (code) {
    case "thinking_replay_missing":
      return "thinking replay missing";
    case "thinking_replay_rejected":
      return "thinking replay rejected";
    default:
      return code.replace(/[_-]+/g, " ").trim();
  }
}
