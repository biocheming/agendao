import type {
  SessionCacheExplainabilityContractRecord,
  SessionCompactionContinuityInspectionRecord,
  SessionCompactionBoundaryContractRecord,
  SessionContextClosureContractRecord,
  SessionPrefixStabilityContractRecord,
} from "./sessionActivity";
import {
  compactionContinuitySourceLabel as generatedCompactionContinuitySourceLabel,
  contextClosureBoundaryStatusLabel as generatedContextClosureBoundaryStatusLabel,
  contextClosureCacheStatusLabel as generatedContextClosureCacheStatusLabel,
  contextClosureCoarseDiagnosticLabel as generatedContextClosureCoarseDiagnosticLabel,
  contextClosureExplainabilitySourceLabel as generatedContextClosureExplainabilitySourceLabel,
  contextClosureGovernanceStatusLabel as generatedContextClosureGovernanceStatusLabel,
  contextClosurePrefixStatusLabel as generatedContextClosurePrefixStatusLabel,
  contextClosureSeverityLabel as generatedContextClosureSeverityLabel,
} from "../generated/contextClosure.generated";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === "object" && !Array.isArray(value));
}

function readString(value: unknown) {
  return typeof value === "string" ? value : null;
}

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readBoolean(value: unknown) {
  return typeof value === "boolean" ? value : null;
}

function prefixStabilityRecord(value: unknown): SessionPrefixStabilityContractRecord | null {
  if (!isRecord(value)) return null;
  const basis = readString(value.basis);
  const trackedOnApiView = readBoolean(value.tracked_on_api_view);
  const apiViewMessages = readNumber(value.api_view_messages);
  const trimmedModelVisibleMessages = readNumber(value.trimmed_model_visible_messages);
  const prefixChangeDetected = readBoolean(value.prefix_change_detected);
  if (
    !basis ||
    trackedOnApiView == null ||
    apiViewMessages == null ||
    trimmedModelVisibleMessages == null ||
    prefixChangeDetected == null
  ) {
    return null;
  }
  return {
    basis,
    tracked_on_api_view: trackedOnApiView,
    api_view_messages: apiViewMessages,
    trimmed_model_visible_messages: trimmedModelVisibleMessages,
    prefix_change_detected: prefixChangeDetected,
    explanation: readString(value.explanation),
  };
}

function compactionBoundaryRecord(
  value: unknown,
): SessionCompactionBoundaryContractRecord | null {
  if (!isRecord(value)) return null;
  const boundaryRecorded = readBoolean(value.boundary_recorded);
  const compactionAttempted = readBoolean(value.compaction_attempted);
  const compactionSucceeded = readBoolean(value.compaction_succeeded);
  const blocking = readBoolean(value.blocking);
  if (
    boundaryRecorded == null ||
    compactionAttempted == null ||
    compactionSucceeded == null ||
    blocking == null
  ) {
    return null;
  }
  return {
    boundary_recorded: boundaryRecorded,
    phase: readString(value.phase),
    trigger: readString(value.trigger),
    reason: readString(value.reason),
    governance_status: readString(value.governance_status),
    request_pressure_percent: readNumber(value.request_pressure_percent),
    live_pressure_percent: readNumber(value.live_pressure_percent),
    compaction_attempted: compactionAttempted,
    compaction_succeeded: compactionSucceeded,
    blocking,
  };
}

function cacheExplainabilityRecord(
  value: unknown,
): SessionCacheExplainabilityContractRecord | null {
  if (!isRecord(value)) return null;
  const issuePresent = readBoolean(value.issue_present);
  const explained = readBoolean(value.explained);
  const source = readString(value.source);
  if (issuePresent == null || explained == null || !source) return null;
  return {
    issue_present: issuePresent,
    explained,
    source,
    severity: readString(value.severity),
    explanation: readString(value.explanation),
  };
}

export function contextClosureContractFromTelemetry(
  telemetry: unknown,
): SessionContextClosureContractRecord | null {
  if (!isRecord(telemetry)) return null;
  const contract = telemetry.context_closure_contract;
  if (!isRecord(contract)) return null;
  const prefixStability = prefixStabilityRecord(contract.prefix_stability);
  const compactionBoundary = compactionBoundaryRecord(contract.compaction_boundary);
  const cacheExplainability = cacheExplainabilityRecord(contract.cache_explainability);
  if (!prefixStability || !compactionBoundary || !cacheExplainability) {
    return null;
  }
  return {
    prefix_stability: prefixStability,
    compaction_boundary: compactionBoundary,
    cache_explainability: cacheExplainability,
  };
}

export function contextClosureGovernanceStatusLabel(value: string | null | undefined) {
  return generatedContextClosureGovernanceStatusLabel(value);
}

export function contextClosureExplainabilitySourceLabel(
  value: string | null | undefined,
) {
  return generatedContextClosureExplainabilitySourceLabel(value);
}

export function contextClosureSeverityLabel(value: string | null | undefined) {
  return generatedContextClosureSeverityLabel(value);
}

export function contextClosurePrefixStatusLabel(
  prefix: SessionPrefixStabilityContractRecord,
) {
  return generatedContextClosurePrefixStatusLabel(prefix);
}

export function contextClosureBoundaryStatusLabel(
  boundary: SessionCompactionBoundaryContractRecord,
) {
  return generatedContextClosureBoundaryStatusLabel(boundary);
}

export function contextClosureCacheStatusLabel(
  cache: SessionCacheExplainabilityContractRecord,
) {
  return generatedContextClosureCacheStatusLabel(cache);
}

export function compactionContinuitySourceLabel(
  continuity: SessionCompactionContinuityInspectionRecord,
) {
  return generatedCompactionContinuitySourceLabel(continuity);
}

export function contextClosureCoarseDiagnosticLabel(
  contract: SessionContextClosureContractRecord | null | undefined,
) {
  return generatedContextClosureCoarseDiagnosticLabel(contract);
}
