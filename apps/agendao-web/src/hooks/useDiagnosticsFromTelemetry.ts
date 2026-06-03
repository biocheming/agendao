import { useMemo } from "react";
import type { CacheEvidenceSummaryRecord } from "../lib/cacheDiagnostics";
import {
  cacheBustSummaryFromMetadata,
  cacheBustSummaryLabel,
  cacheSemanticsFromTelemetry,
} from "../lib/cacheDiagnostics";
import {
  contextClosureCoarseDiagnosticLabel,
  contextClosureContractFromTelemetry,
} from "../lib/contextClosureDiagnostics";
import type { MessageRecord } from "../lib/history";
import {
  providerDiagnosticFromMetadata,
  providerDiagnosticLabel,
} from "../lib/providerDiagnostics";
import type { SessionTelemetrySnapshotRecord } from "../lib/sessionActivity";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function ingressStabilizationLabel(value: unknown): string | null {
  if (!isRecord(value)) return null;
  if (!value) return null;
  const sourceValue = value.source;
  const source =
    typeof sourceValue === "string"
      ? sourceValue
      : sourceValue && typeof sourceValue === "object" && "source" in sourceValue && typeof (sourceValue as Record<string, unknown>).source === "string"
        ? (sourceValue as Record<string, unknown>).source
        : "unknown";
  const policy = typeof value.policy === "string" ? value.policy : "metadata_only";
  const batchCount = typeof value.batch_count === "number" ? value.batch_count : 1;
  return batchCount > 1 ? `${source} · ${policy} · batch ${batchCount}` : `${source} · ${policy}`;
}

export interface TelemetryDiagnostics {
  latestClosureDiagnostic: string | null;
  latestIngressDiagnostic: string | null;
  latestProviderDiagnostic: string | null;
}

export function useDiagnosticsFromTelemetry(
  telemetry: SessionTelemetrySnapshotRecord | null | undefined,
  messageHistory: MessageRecord[],
): TelemetryDiagnostics {
  const latestClosureDiagnostic = useMemo(() => {
    const contextClosure = contextClosureContractFromTelemetry(telemetry);
    if (contextClosure) {
      return contextClosureCoarseDiagnosticLabel(contextClosure);
    }

    const semanticsLabel = cacheSemanticsFromTelemetry(telemetry)?.label;
    if (semanticsLabel) return semanticsLabel;

    const telemetrySummary =
      telemetry?.cache_evidence &&
      typeof telemetry.cache_evidence === "object"
        ? (telemetry.cache_evidence as CacheEvidenceSummaryRecord)
        : null;
    const telemetryLabel = cacheBustSummaryLabel(telemetrySummary);
    if (telemetryLabel) return telemetryLabel;

    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const label = cacheBustSummaryLabel(cacheBustSummaryFromMetadata(message.metadata));
      if (label) return label;
    }
    return null;
  }, [
    telemetry?.cache_evidence,
    telemetry?.context_closure_contract,
    telemetry?.cache_semantics,
    messageHistory,
  ]);

  const latestIngressDiagnostic = useMemo(
    () => ingressStabilizationLabel(telemetry?.ingress_stabilization ?? null),
    [telemetry?.ingress_stabilization],
  );

  const latestProviderDiagnostic = useMemo(() => {
    const telemetrySummary = providerDiagnosticFromMetadata({
      provider_diagnostic: telemetry?.provider_diagnostic_summary ?? null,
    });
    const telemetryLabel = providerDiagnosticLabel(telemetrySummary);
    if (telemetryLabel) return telemetryLabel;

    for (let index = messageHistory.length - 1; index >= 0; index -= 1) {
      const message = messageHistory[index];
      if (message?.role !== "assistant") continue;
      const label = providerDiagnosticLabel(providerDiagnosticFromMetadata(message.metadata));
      if (label) return label;
    }
    return null;
  }, [telemetry?.provider_diagnostic_summary, messageHistory]);

  return { latestClosureDiagnostic, latestIngressDiagnostic, latestProviderDiagnostic };
}
