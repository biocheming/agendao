import { useMemo, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import type {
  ManagedSkillRecord,
  SkillArtifactCacheEntryRecord,
  SkillCatalogEntry,
  SkillDetailResponseRecord,
  SkillDistributionRecord,
  SkillGovernanceTimelineEntryRecord,
  SkillGuardReportRecord,
  SkillHubPolicyRecord,
  SkillManagedLifecycleRecord,
  SkillNegativeEntropyDiagnosticRecord,
  SkillOperationalSnapshotRecord,
  SkillSemanticConflictDiagnosticRecord,
  SkillRemoteInstallPlanRecord,
  SkillSourceIndexSnapshotRecord,
  SkillSourceRefRecord,
  SkillSyncPlanRecord,
} from "@/lib/skill";
import { emptySkillMethodologyDraft } from "../../settings/SkillMethodologyEditor";
import type { SkillMethodologyDraft } from "../../settings/SkillMethodologyEditor";
import type { SkillEditorMode } from "../types";

export interface SkillHubSettingsState {
  skillCatalog: SkillCatalogEntry[];
  setSkillCatalog: Dispatch<SetStateAction<SkillCatalogEntry[]>>;
  managedSkills: ManagedSkillRecord[];
  setManagedSkills: Dispatch<SetStateAction<ManagedSkillRecord[]>>;
  skillSourceIndices: SkillSourceIndexSnapshotRecord[];
  setSkillSourceIndices: Dispatch<SetStateAction<SkillSourceIndexSnapshotRecord[]>>;
  skillDistributions: SkillDistributionRecord[];
  setSkillDistributions: Dispatch<SetStateAction<SkillDistributionRecord[]>>;
  skillArtifactCache: SkillArtifactCacheEntryRecord[];
  setSkillArtifactCache: Dispatch<SetStateAction<SkillArtifactCacheEntryRecord[]>>;
  skillHubPolicy: SkillHubPolicyRecord | null;
  setSkillHubPolicy: Dispatch<SetStateAction<SkillHubPolicyRecord | null>>;
  skillLifecycle: SkillManagedLifecycleRecord[];
  setSkillLifecycle: Dispatch<SetStateAction<SkillManagedLifecycleRecord[]>>;
  skillUsageLedger: SkillOperationalSnapshotRecord[];
  setSkillUsageLedger: Dispatch<SetStateAction<SkillOperationalSnapshotRecord[]>>;
  skillNegativeEntropy: SkillNegativeEntropyDiagnosticRecord[];
  setSkillNegativeEntropy: Dispatch<SetStateAction<SkillNegativeEntropyDiagnosticRecord[]>>;
  skillSemanticConflicts: SkillSemanticConflictDiagnosticRecord[];
  setSkillSemanticConflicts: Dispatch<
    SetStateAction<SkillSemanticConflictDiagnosticRecord[]>
  >;
  skillGovernanceTimeline: SkillGovernanceTimelineEntryRecord[];
  setSkillGovernanceTimeline: Dispatch<SetStateAction<SkillGovernanceTimelineEntryRecord[]>>;
  skillSyncSourceId: string;
  setSkillSyncSourceId: Dispatch<SetStateAction<string>>;
  skillSyncSourceKind: SkillSourceRefRecord["source_kind"];
  setSkillSyncSourceKind: Dispatch<SetStateAction<SkillSourceRefRecord["source_kind"]>>;
  skillSyncLocator: string;
  setSkillSyncLocator: Dispatch<SetStateAction<string>>;
  skillSyncRevision: string;
  setSkillSyncRevision: Dispatch<SetStateAction<string>>;
  skillSyncPlan: SkillSyncPlanRecord | null;
  setSkillSyncPlan: Dispatch<SetStateAction<SkillSyncPlanRecord | null>>;
  remoteInstallSkillName: string;
  setRemoteInstallSkillName: Dispatch<SetStateAction<string>>;
  remoteInstallPlan: SkillRemoteInstallPlanRecord | null;
  setRemoteInstallPlan: Dispatch<SetStateAction<SkillRemoteInstallPlanRecord | null>>;
  skillGuardReports: SkillGuardReportRecord[];
  setSkillGuardReports: Dispatch<SetStateAction<SkillGuardReportRecord[]>>;
  skillGuardTarget: string | null;
  setSkillGuardTarget: Dispatch<SetStateAction<string | null>>;
  selectedSkillName: string | null;
  setSelectedSkillName: Dispatch<SetStateAction<string | null>>;
  skillDetail: SkillDetailResponseRecord | null;
  setSkillDetail: Dispatch<SetStateAction<SkillDetailResponseRecord | null>>;
  skillDetailLoading: boolean;
  setSkillDetailLoading: Dispatch<SetStateAction<boolean>>;
  skillEditorContent: string;
  setSkillEditorContent: Dispatch<SetStateAction<string>>;
  editSkillEditorMode: SkillEditorMode;
  setEditSkillEditorMode: Dispatch<SetStateAction<SkillEditorMode>>;
  editSkillDescription: string;
  setEditSkillDescription: Dispatch<SetStateAction<string>>;
  editSkillMethodologyDraft: SkillMethodologyDraft;
  setEditSkillMethodologyDraft: Dispatch<SetStateAction<SkillMethodologyDraft>>;
  editSkillMethodologyMatched: boolean;
  setEditSkillMethodologyMatched: Dispatch<SetStateAction<boolean>>;
  editSkillMethodologyPreview: string;
  setEditSkillMethodologyPreview: Dispatch<SetStateAction<string>>;
  editSkillMethodologyPreviewError: string | null;
  setEditSkillMethodologyPreviewError: Dispatch<SetStateAction<string | null>>;
  newSkillName: string;
  setNewSkillName: Dispatch<SetStateAction<string>>;
  newSkillDescription: string;
  setNewSkillDescription: Dispatch<SetStateAction<string>>;
  newSkillCategory: string;
  setNewSkillCategory: Dispatch<SetStateAction<string>>;
  newSkillBody: string;
  setNewSkillBody: Dispatch<SetStateAction<string>>;
  newSkillEditorMode: SkillEditorMode;
  setNewSkillEditorMode: Dispatch<SetStateAction<SkillEditorMode>>;
  newSkillMethodologyDraft: SkillMethodologyDraft;
  setNewSkillMethodologyDraft: Dispatch<SetStateAction<SkillMethodologyDraft>>;
  newSkillMethodologyPreview: string;
  setNewSkillMethodologyPreview: Dispatch<SetStateAction<string>>;
  newSkillMethodologyPreviewError: string | null;
  setNewSkillMethodologyPreviewError: Dispatch<SetStateAction<string | null>>;
  selectedSkillEntry: SkillCatalogEntry | null;
  managedRecordBySkill: Map<string, ManagedSkillRecord>;
  selectedHubSourceSnapshot: SkillSourceIndexSnapshotRecord | null;
  selectedRemoteSourceEntries: SkillSourceIndexSnapshotRecord["entries"];
  selectedRemoteSourceEntry: SkillSourceIndexSnapshotRecord["entries"][number] | null;
  selectedRemoteDistribution: SkillDistributionRecord | null;
  selectedRemoteLifecycle: SkillManagedLifecycleRecord | null;
  selectedRemoteArtifactCache: SkillArtifactCacheEntryRecord | null;
  latestGuardBySkill: Map<string, SkillGuardReportRecord>;
  skillWorkspaceRoot: string;
  skillsMutationsEnabled: boolean;
}

export function useSkillHubSettingsState({
  selectedSessionId,
  workspaceRootPath,
}: {
  selectedSessionId: string | null;
  workspaceRootPath: string;
}): SkillHubSettingsState {
  const [skillCatalog, setSkillCatalog] = useState<SkillCatalogEntry[]>([]);
  const [managedSkills, setManagedSkills] = useState<ManagedSkillRecord[]>([]);
  const [skillSourceIndices, setSkillSourceIndices] = useState<SkillSourceIndexSnapshotRecord[]>([]);
  const [skillDistributions, setSkillDistributions] = useState<SkillDistributionRecord[]>([]);
  const [skillArtifactCache, setSkillArtifactCache] = useState<SkillArtifactCacheEntryRecord[]>([]);
  const [skillHubPolicy, setSkillHubPolicy] = useState<SkillHubPolicyRecord | null>(null);
  const [skillLifecycle, setSkillLifecycle] = useState<SkillManagedLifecycleRecord[]>([]);
  const [skillUsageLedger, setSkillUsageLedger] = useState<SkillOperationalSnapshotRecord[]>([]);
  const [skillNegativeEntropy, setSkillNegativeEntropy] = useState<SkillNegativeEntropyDiagnosticRecord[]>([]);
  const [skillSemanticConflicts, setSkillSemanticConflicts] = useState<SkillSemanticConflictDiagnosticRecord[]>([]);
  const [skillGovernanceTimeline, setSkillGovernanceTimeline] = useState<SkillGovernanceTimelineEntryRecord[]>([]);
  const [skillSyncSourceId, setSkillSyncSourceId] = useState("");
  const [skillSyncSourceKind, setSkillSyncSourceKind] = useState<SkillSourceRefRecord["source_kind"]>("local_path");
  const [skillSyncLocator, setSkillSyncLocator] = useState("");
  const [skillSyncRevision, setSkillSyncRevision] = useState("");
  const [skillSyncPlan, setSkillSyncPlan] = useState<SkillSyncPlanRecord | null>(null);
  const [remoteInstallSkillName, setRemoteInstallSkillName] = useState("");
  const [remoteInstallPlan, setRemoteInstallPlan] = useState<SkillRemoteInstallPlanRecord | null>(null);
  const [skillGuardReports, setSkillGuardReports] = useState<SkillGuardReportRecord[]>([]);
  const [skillGuardTarget, setSkillGuardTarget] = useState<string | null>(null);
  const [selectedSkillName, setSelectedSkillName] = useState<string | null>(null);
  const [skillDetail, setSkillDetail] = useState<SkillDetailResponseRecord | null>(null);
  const [skillDetailLoading, setSkillDetailLoading] = useState(false);
  const [skillEditorContent, setSkillEditorContent] = useState("");
  const [editSkillEditorMode, setEditSkillEditorMode] = useState<SkillEditorMode>("raw");
  const [editSkillDescription, setEditSkillDescription] = useState("");
  const [editSkillMethodologyDraft, setEditSkillMethodologyDraft] =
    useState<SkillMethodologyDraft>(emptySkillMethodologyDraft);
  const [editSkillMethodologyMatched, setEditSkillMethodologyMatched] = useState(false);
  const [editSkillMethodologyPreview, setEditSkillMethodologyPreview] = useState("");
  const [editSkillMethodologyPreviewError, setEditSkillMethodologyPreviewError] =
    useState<string | null>(null);
  const [newSkillName, setNewSkillName] = useState("");
  const [newSkillDescription, setNewSkillDescription] = useState("");
  const [newSkillCategory, setNewSkillCategory] = useState("");
  const [newSkillBody, setNewSkillBody] = useState("");
  const [newSkillEditorMode, setNewSkillEditorMode] = useState<SkillEditorMode>("methodology");
  const [newSkillMethodologyDraft, setNewSkillMethodologyDraft] =
    useState<SkillMethodologyDraft>(emptySkillMethodologyDraft);
  const [newSkillMethodologyPreview, setNewSkillMethodologyPreview] = useState("");
  const [newSkillMethodologyPreviewError, setNewSkillMethodologyPreviewError] =
    useState<string | null>(null);

  const selectedSkillEntry = useMemo(
    () =>
      skillCatalog.find(
        (skill) =>
          skill.name.trim().toLowerCase() === (selectedSkillName ?? "").trim().toLowerCase(),
      ) ?? null,
    [selectedSkillName, skillCatalog],
  );
  const managedRecordBySkill = useMemo(
    () =>
      new Map(
        managedSkills.map((record) => [record.skill_name.trim().toLowerCase(), record] as const),
      ),
    [managedSkills],
  );
  const selectedHubSourceSnapshot = useMemo(
    () =>
      skillSourceIndices.find(
        (snapshot) =>
          snapshot.source.source_id.trim().toLowerCase() ===
          skillSyncSourceId.trim().toLowerCase(),
      ) ?? null,
    [skillSourceIndices, skillSyncSourceId],
  );
  const selectedRemoteSourceEntries = useMemo(
    () => selectedHubSourceSnapshot?.entries ?? [],
    [selectedHubSourceSnapshot],
  );
  const selectedRemoteSourceEntry = useMemo(
    () =>
      selectedRemoteSourceEntries.find(
        (entry) =>
          entry.skill_name.trim().toLowerCase() ===
          remoteInstallSkillName.trim().toLowerCase(),
      ) ?? null,
    [remoteInstallSkillName, selectedRemoteSourceEntries],
  );
  const selectedRemoteDistribution = useMemo(() => {
    const matches = skillDistributions
      .filter(
        (record) =>
          record.source.source_id.trim().toLowerCase() ===
            skillSyncSourceId.trim().toLowerCase() &&
          record.skill_name.trim().toLowerCase() ===
            remoteInstallSkillName.trim().toLowerCase(),
      )
      .sort(
        (left, right) =>
          (right.resolution?.resolved_at ?? 0) - (left.resolution?.resolved_at ?? 0),
      );
    return matches[0] ?? null;
  }, [remoteInstallSkillName, skillDistributions, skillSyncSourceId]);
  const selectedRemoteLifecycle = useMemo(() => {
    if (selectedRemoteDistribution) {
      return (
        skillLifecycle.find(
          (record) => record.distribution_id === selectedRemoteDistribution.distribution_id,
        ) ?? null
      );
    }
    const matches = skillLifecycle
      .filter(
        (record) =>
          record.source_id.trim().toLowerCase() === skillSyncSourceId.trim().toLowerCase() &&
          record.skill_name.trim().toLowerCase() ===
            remoteInstallSkillName.trim().toLowerCase(),
      )
      .sort((left, right) => right.updated_at - left.updated_at);
    return matches[0] ?? null;
  }, [remoteInstallSkillName, selectedRemoteDistribution, skillLifecycle, skillSyncSourceId]);
  const selectedRemoteArtifactCache = useMemo(() => {
    if (!selectedRemoteDistribution) {
      return null;
    }
    return (
      skillArtifactCache.find(
        (entry) =>
          entry.artifact.artifact_id ===
          selectedRemoteDistribution.resolution.artifact.artifact_id,
      ) ?? null
    );
  }, [selectedRemoteDistribution, skillArtifactCache]);
  const latestGuardBySkill = useMemo(() => {
    const result = new Map<string, SkillGuardReportRecord>();
    for (const entry of skillGovernanceTimeline) {
      const key = entry.skill_name?.trim().toLowerCase();
      if (!key || result.has(key) || !entry.guard_report) {
        continue;
      }
      result.set(key, entry.guard_report);
    }
    return result;
  }, [skillGovernanceTimeline]);
  const skillWorkspaceRoot = useMemo(() => {
    const trimmed = workspaceRootPath.trim();
    if (!trimmed) return ".agendao/skills";
    return `${trimmed.replace(/\/+$/, "")}/.agendao/skills`;
  }, [workspaceRootPath]);
  const skillsMutationsEnabled = Boolean(selectedSessionId);

  return {
    skillCatalog,
    setSkillCatalog,
    managedSkills,
    setManagedSkills,
    skillSourceIndices,
    setSkillSourceIndices,
    skillDistributions,
    setSkillDistributions,
    skillArtifactCache,
    setSkillArtifactCache,
    skillHubPolicy,
    setSkillHubPolicy,
    skillLifecycle,
    setSkillLifecycle,
    skillUsageLedger,
    setSkillUsageLedger,
    skillNegativeEntropy,
    setSkillNegativeEntropy,
    skillSemanticConflicts,
    setSkillSemanticConflicts,
    skillGovernanceTimeline,
    setSkillGovernanceTimeline,
    skillSyncSourceId,
    setSkillSyncSourceId,
    skillSyncSourceKind,
    setSkillSyncSourceKind,
    skillSyncLocator,
    setSkillSyncLocator,
    skillSyncRevision,
    setSkillSyncRevision,
    skillSyncPlan,
    setSkillSyncPlan,
    remoteInstallSkillName,
    setRemoteInstallSkillName,
    remoteInstallPlan,
    setRemoteInstallPlan,
    skillGuardReports,
    setSkillGuardReports,
    skillGuardTarget,
    setSkillGuardTarget,
    selectedSkillName,
    setSelectedSkillName,
    skillDetail,
    setSkillDetail,
    skillDetailLoading,
    setSkillDetailLoading,
    skillEditorContent,
    setSkillEditorContent,
    editSkillEditorMode,
    setEditSkillEditorMode,
    editSkillDescription,
    setEditSkillDescription,
    editSkillMethodologyDraft,
    setEditSkillMethodologyDraft,
    editSkillMethodologyMatched,
    setEditSkillMethodologyMatched,
    editSkillMethodologyPreview,
    setEditSkillMethodologyPreview,
    editSkillMethodologyPreviewError,
    setEditSkillMethodologyPreviewError,
    newSkillName,
    setNewSkillName,
    newSkillDescription,
    setNewSkillDescription,
    newSkillCategory,
    setNewSkillCategory,
    newSkillBody,
    setNewSkillBody,
    newSkillEditorMode,
    setNewSkillEditorMode,
    newSkillMethodologyDraft,
    setNewSkillMethodologyDraft,
    newSkillMethodologyPreview,
    setNewSkillMethodologyPreview,
    newSkillMethodologyPreviewError,
    setNewSkillMethodologyPreviewError,
    selectedSkillEntry,
    managedRecordBySkill,
    selectedHubSourceSnapshot,
    selectedRemoteSourceEntries,
    selectedRemoteSourceEntry,
    selectedRemoteDistribution,
    selectedRemoteLifecycle,
    selectedRemoteArtifactCache,
    latestGuardBySkill,
    skillWorkspaceRoot,
    skillsMutationsEnabled,
  };
}
