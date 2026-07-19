import type {
  ManagedSkillRecord,
  SkillDistributionRecord,
  SkillGuardReportRecord,
  SkillManagedLifecycleRecord,
  SkillOperationalSnapshotRecord,
  SkillRetirementReasonKindRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
} from "@/lib/skill";

export type SkillSubtabId = "overview" | "hub" | "catalog" | "governance";

export type SkillVitalityStateValue = NonNullable<
  SkillOperationalSnapshotRecord["vitality"]
>["state"];

export type SkillReviewReasonKindValue = SkillRetirementReasonKindRecord;

export interface SkillReviewCandidateView {
  entry: SkillOperationalSnapshotRecord;
  reasonKind: SkillReviewReasonKindValue;
  relatedSkillName: string | null;
  summary: string;
  evidenceBadges: string[];
  evidenceLines: string[];
}

export interface SkillsTabStyles {
  primaryButtonClass: string;
  secondaryButtonClass: string;
  summaryCardClass: string;
  sectionCardClass: string;
  mutedCardClass: string;
  editorTextareaClass: string;
}

export interface SkillHubSearchResult {
  distribution: SkillDistributionRecord | null;
  entry: SkillSourceIndexSnapshotRecord["entries"][number];
  installedRecord: ManagedSkillRecord | null;
  lifecycle: SkillManagedLifecycleRecord | null;
  score: number;
  searchText: string;
  source: SkillSourceRefRecord;
}

export function managedSkillStateLabel(
  record: ManagedSkillRecord,
  t: (key: string) => string,
): string {
  if (record.deleted_locally) return t("settings.skills.state.deletedLocally");
  if (record.locally_modified) return t("settings.skills.state.locallyModified");
  return t("settings.skills.state.managedClean");
}

export function latestGuardStatusLabel(
  report: SkillGuardReportRecord,
  t: (key: string) => string,
): string {
  switch (report.status) {
    case "blocked":
      return t("settings.skills.guard.blocked");
    case "warn":
      return t("settings.skills.guard.warn");
    default:
      return t("settings.skills.guard.passed");
  }
}

export function lifecycleStatusClass(state: string): string {
  const normalized = state.trim().toLowerCase();
  if (normalized.includes("failed") || normalized === "diverged") {
    return "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)";
  }
  if (
    normalized === "updateavailable" ||
    normalized === "update_available" ||
    normalized === "plannedinstall" ||
    normalized === "planned_install" ||
    normalized === "removepending" ||
    normalized === "remove_pending"
  ) {
    return "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)";
  }
  if (normalized === "installed" || normalized === "fetched" || normalized === "resolved") {
    return "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)";
  }
  return "border-border/40 bg-muted text-muted-foreground";
}

export function formatHubDurationSeconds(value?: number | null): string {
  if (!value) return "--";
  if (value % 86400 === 0) return `${value / 86400}d`;
  if (value % 3600 === 0) return `${value / 3600}h`;
  if (value % 60 === 0) return `${value / 60}m`;
  return `${value}s`;
}

export function formatHubDurationMs(value?: number | null): string {
  if (!value) return "--";
  if (value % 1000 === 0) return `${value / 1000}s`;
  return `${value}ms`;
}

export function formatHubBytes(value?: number | null): string {
  if (!value) return "--";
  if (value >= 1024 * 1024 && value % (1024 * 1024) === 0) {
    return `${value / (1024 * 1024)} MiB`;
  }
  if (value >= 1024 && value % 1024 === 0) {
    return `${value / 1024} KiB`;
  }
  return `${value} bytes`;
}

export function unixTimeLabel(value?: number | null): string {
  if (!value) return "--";
  try {
    const timestamp = value > 1_000_000_000_000 ? value : value * 1000;
    return new Date(timestamp).toLocaleString();
  } catch {
    return String(value);
  }
}

export function usageWriteCount(entry: SkillOperationalSnapshotRecord): number {
  const writes = entry.writes;
  if (!writes) return 0;
  return (
    writes.create_count +
    writes.patch_count +
    writes.edit_count +
    writes.supporting_file_write_count +
    writes.supporting_file_remove_count +
    writes.install_count +
    writes.update_count +
    writes.detach_count +
    writes.remove_count +
    writes.delete_count
  );
}

export function governanceSeverityClass(severity: "info" | "warn"): string {
  if (severity === "warn") {
    return "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)";
  }
  return "border-border/40 bg-muted text-muted-foreground";
}

export function formatVitalityStateLabel(
  state: SkillVitalityStateValue | null | undefined,
  t: (key: string) => string,
): string {
  switch (state ?? "active") {
    case "review_candidate":
      return t("settings.skills.vitality.reviewCandidate");
    case "retired":
      return t("settings.skills.vitality.retired");
    case "archived":
      return t("settings.skills.vitality.archived");
    default:
      return t("settings.skills.vitality.active");
  }
}

export function vitalityStateClass(state?: SkillVitalityStateValue | null): string {
  switch (state ?? "active") {
    case "review_candidate":
      return "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)";
    case "retired":
      return "border-(--ds-error)/40 bg-(--ds-error)/12 text-(--ds-error)";
    case "archived":
      return "border-border/40 bg-muted text-muted-foreground";
    default:
      return "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)";
  }
}

export function reviewReasonKindLabel(
  kind: SkillReviewReasonKindValue,
  t: (key: string) => string,
): string {
  switch (kind) {
    case "negative_entropy":
      return t("settings.skills.reason.negativeEntropy");
    case "semantic_conflict":
      return t("settings.skills.reason.semanticConflict");
    case "manual_override":
      return t("settings.skills.reason.manualOverride");
    case "restored":
      return t("settings.skills.reason.restored");
    default:
      return kind;
  }
}

export function reviewReasonKindClass(kind: SkillReviewReasonKindValue): string {
  switch (kind) {
    case "negative_entropy":
      return "border-(--ds-warn)/40 bg-(--ds-warn)/12 text-(--ds-warn)";
    case "semantic_conflict":
      return "border-(--ds-info)/40 bg-(--ds-info)/12 text-(--ds-info)";
    case "manual_override":
      return "border-border/40 bg-muted text-muted-foreground";
    case "restored":
      return "border-(--ds-ok)/40 bg-(--ds-ok)/12 text-(--ds-ok)";
    default:
      return "border-border/40 bg-muted text-muted-foreground";
  }
}

export function negativeEntropySignalLabel(signal: string): string {
  return signal.replaceAll("_", " ");
}

export function semanticConflictKindLabel(kind: string): string {
  return kind.replaceAll("_", " ");
}

export function skillNameKey(value: string): string {
  return value.trim().toLowerCase();
}
