import { useCallback, useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import type {
  SkillDetailResponseRecord,
  SkillHubGuardRunRequestRecord,
  SkillHubGuardRunResponseRecord,
  SkillHubIndexRefreshResponseRecord,
  SkillHubManagedDetachResponseRecord,
  SkillHubManagedRemoveResponseRecord,
  SkillHubSyncPlanResponseRecord,
  SkillManageResponseRecord,
  SkillMethodologyExtractResponseRecord,
  SkillMethodologyPreviewResponseRecord,
  SkillMethodologyTemplateRecord,
  SkillRemoteInstallPlanRecord,
  SkillRemoteInstallResponseRecord,
  SkillSourceRefRecord,
} from "@/lib/skill";
import {
  buildMethodologyTemplateFromDraft,
  emptySkillMethodologyDraft,
  methodologyDraftFromTemplate,
} from "../../settings/SkillMethodologyEditor";
import type { SkillMethodologyDraft } from "../../settings/SkillMethodologyEditor";
import { useI18n } from "@/i18n/I18nProvider";
import { formatError } from "../shared";
import type { SkillHubSettingsState } from "./useSkillHubSettingsState";

export interface SkillHubSettingsActionsDeps extends SkillHubSettingsState {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  onBanner: (message: string) => void;
  setFeedback: Dispatch<SetStateAction<string | null>>;
  setBusyKey: Dispatch<SetStateAction<string | null>>;
  selectedSessionId: string | null;
  reloadSettingsData: () => Promise<void>;
  runMutation: (
    key: string,
    action: () => Promise<string | void>,
    success: string,
  ) => Promise<void>;
}

export interface SkillHubSettingsActions {
  planSkillSync: () => Promise<void>;
  refreshSkillSourceIndex: () => Promise<void>;
  runGuard: (request: SkillHubGuardRunRequestRecord, targetLabel: string) => Promise<void>;
  applySkillSync: () => Promise<void>;
  planRemoteInstall: () => Promise<void>;
  applyRemoteInstall: () => Promise<void>;
  planRemoteUpdate: () => Promise<void>;
  applyRemoteUpdate: () => Promise<void>;
  detachManagedSkill: () => Promise<void>;
  removeManagedSkill: () => Promise<void>;
  createSkill: () => Promise<void>;
  saveSelectedSkill: () => Promise<void>;
  deleteSelectedSkill: () => Promise<void>;
  runSelectedSkillGuard: () => Promise<void>;
  runSelectedSourceGuard: () => Promise<void>;
}

export function useSkillHubSettingsActions({
  apiJson,
  onBanner,
  setFeedback,
  setBusyKey,
  selectedSessionId,
  reloadSettingsData,
  runMutation,
  skillCatalog,
  selectedSkillName,
  setSelectedSkillName,
  setSkillDetail,
  setSkillDetailLoading,
  skillEditorContent,
  setSkillEditorContent,
  editSkillEditorMode,
  setEditSkillEditorMode,
  editSkillDescription,
  setEditSkillDescription,
  editSkillMethodologyDraft,
  setEditSkillMethodologyDraft,
  setEditSkillMethodologyMatched,
  setEditSkillMethodologyPreview,
  setEditSkillMethodologyPreviewError,
  skillSourceIndices,
  selectedHubSourceSnapshot,
  skillSyncSourceId,
  skillSyncSourceKind,
  skillSyncLocator,
  skillSyncRevision,
  setSkillSyncSourceId,
  setSkillSyncSourceKind,
  setSkillSyncLocator,
  setSkillSyncRevision,
  setSkillSyncPlan,
  remoteInstallSkillName,
  setRemoteInstallSkillName,
  setRemoteInstallPlan,
  setSkillGuardReports,
  setSkillGuardTarget,
  newSkillName,
  newSkillDescription,
  newSkillCategory,
  newSkillBody,
  newSkillEditorMode,
  newSkillMethodologyDraft,
  setNewSkillName,
  setNewSkillDescription,
  setNewSkillCategory,
  setNewSkillBody,
  setNewSkillEditorMode,
  setNewSkillMethodologyDraft,
  setNewSkillMethodologyPreview,
  setNewSkillMethodologyPreviewError,
}: SkillHubSettingsActionsDeps): SkillHubSettingsActions {
  const { t } = useI18n();
  const requestSkillMethodologyPreview = useCallback(
    async (
      skillName: string,
      draft: SkillMethodologyDraft,
      applyPreview: (body: string, error: string | null) => void,
    ) => {
      try {
        const response = await apiJson<SkillMethodologyPreviewResponseRecord>(
          "/skill/methodology/preview",
          {
            method: "POST",
            body: JSON.stringify({
              skill_name: skillName.trim() || "draft-skill",
              methodology: buildMethodologyTemplateFromDraft(draft),
            }),
          },
        );
        applyPreview(response.body, null);
      } catch (error) {
        applyPreview("", formatError(error));
      }
    },
    [apiJson],
  );

  useEffect(() => {
    if (skillCatalog.length === 0) {
      setSelectedSkillName(null);
      setSkillDetail(null);
      setSkillDetailLoading(false);
      setSkillEditorContent("");
      setEditSkillDescription("");
      setEditSkillEditorMode("raw");
      setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
      setEditSkillMethodologyMatched(false);
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    // Only auto-select first skill when the current selection is stale
    // (was removed from catalog). Do NOT auto-select when nothing is selected
    // so the catalog list is visible to the user.
    if (!selectedSkillName) {
      return;
    }

    const current = selectedSkillName.trim().toLowerCase();
    const matched = skillCatalog.find(
      (skill) => skill.name.trim().toLowerCase() === current,
    );

    if (matched) {
      return;
    }

    setSelectedSkillName(skillCatalog[0].name);
  }, [
    selectedSkillName,
    skillCatalog,
    setSelectedSkillName,
    setSkillDetail,
    setSkillDetailLoading,
    setSkillEditorContent,
    setEditSkillDescription,
    setEditSkillEditorMode,
    setEditSkillMethodologyDraft,
    setEditSkillMethodologyMatched,
    setEditSkillMethodologyPreview,
    setEditSkillMethodologyPreviewError,
  ]);

  useEffect(() => {
    if (skillSyncSourceId.trim() || skillSourceIndices.length === 0) {
      return;
    }
    const firstSource = skillSourceIndices[0]?.source;
    if (!firstSource) {
      return;
    }
    setSkillSyncSourceId(firstSource.source_id);
    setSkillSyncSourceKind(firstSource.source_kind);
    setSkillSyncLocator(firstSource.locator);
    setSkillSyncRevision(firstSource.revision ?? "");
  }, [
    skillSourceIndices,
    skillSyncLocator,
    skillSyncRevision,
    skillSyncSourceId,
    setSkillSyncSourceId,
    setSkillSyncSourceKind,
    setSkillSyncLocator,
    setSkillSyncRevision,
  ]);

  useEffect(() => {
    if (!selectedHubSourceSnapshot) {
      return;
    }
    const current = remoteInstallSkillName.trim().toLowerCase();
    const exactMatch = selectedHubSourceSnapshot.entries.some(
      (entry) => entry.skill_name.trim().toLowerCase() === current,
    );
    if (!current || !exactMatch) {
      setRemoteInstallSkillName(selectedHubSourceSnapshot.entries[0]?.skill_name ?? "");
    }
  }, [remoteInstallSkillName, selectedHubSourceSnapshot, setRemoteInstallSkillName]);

  useEffect(() => {
    if (!selectedSkillName) {
      setSkillDetail(null);
      setSkillDetailLoading(false);
      setSkillEditorContent("");
      setEditSkillDescription("");
      setEditSkillEditorMode("raw");
      setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
      setEditSkillMethodologyMatched(false);
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    let cancelled = false;
    setSkillDetailLoading(true);

    void (async () => {
      try {
        const detailPath = selectedSessionId
          ? `/skill/detail?name=${encodeURIComponent(selectedSkillName)}&session_id=${encodeURIComponent(selectedSessionId)}`
          : `/skill/detail?name=${encodeURIComponent(selectedSkillName)}`;
        const detail = await apiJson<SkillDetailResponseRecord>(
          detailPath,
        );
        if (cancelled) return;
        let extractedMethodology: SkillMethodologyTemplateRecord | null = null;
        try {
          const extracted = await apiJson<SkillMethodologyExtractResponseRecord>(
            "/skill/methodology/extract",
            {
              method: "POST",
              body: JSON.stringify({
                content: detail.source ?? "",
              }),
            },
          );
          extractedMethodology = extracted.matched ? extracted.methodology ?? null : null;
        } catch {
          extractedMethodology = null;
        }
        if (cancelled) return;
        setSkillDetail(detail);
        setSkillEditorContent(detail.source ?? "");
        setEditSkillDescription(detail.skill.meta.description ?? "");
        setEditSkillMethodologyMatched(Boolean(extractedMethodology));
        setEditSkillMethodologyDraft(
          extractedMethodology
            ? methodologyDraftFromTemplate(extractedMethodology)
            : emptySkillMethodologyDraft(),
        );
        setEditSkillEditorMode(extractedMethodology ? "methodology" : "raw");
      } catch (error) {
        if (cancelled) return;
        const message = t("settings.feedback.skillLoadFailed", { name: selectedSkillName, error: formatError(error) });
        setSkillDetail(null);
        setSkillEditorContent("");
        setEditSkillDescription("");
        setEditSkillEditorMode("raw");
        setEditSkillMethodologyDraft(emptySkillMethodologyDraft());
        setEditSkillMethodologyMatched(false);
        setEditSkillMethodologyPreview("");
        setEditSkillMethodologyPreviewError(null);
        setFeedback(message);
        onBanner(message);
      } finally {
        if (!cancelled) {
          setSkillDetailLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    apiJson,
    onBanner,
    selectedSessionId,
    selectedSkillName,
    setFeedback,
    setSkillDetail,
    setSkillDetailLoading,
    setSkillEditorContent,
    setEditSkillDescription,
    setEditSkillEditorMode,
    setEditSkillMethodologyDraft,
    setEditSkillMethodologyMatched,
    setEditSkillMethodologyPreview,
    setEditSkillMethodologyPreviewError,
    t,
  ]);

  useEffect(() => {
    if (newSkillEditorMode !== "methodology") {
      setNewSkillMethodologyPreview("");
      setNewSkillMethodologyPreviewError(null);
      return;
    }

    const timer = window.setTimeout(() => {
      void requestSkillMethodologyPreview(
        newSkillName,
        newSkillMethodologyDraft,
        (body, error) => {
          setNewSkillMethodologyPreview(body);
          setNewSkillMethodologyPreviewError(error);
        },
      );
    }, 120);

    return () => window.clearTimeout(timer);
  }, [
    newSkillEditorMode,
    newSkillMethodologyDraft,
    newSkillName,
    requestSkillMethodologyPreview,
    setNewSkillMethodologyPreview,
    setNewSkillMethodologyPreviewError,
  ]);

  useEffect(() => {
    if (editSkillEditorMode !== "methodology" || !selectedSkillName) {
      setEditSkillMethodologyPreview("");
      setEditSkillMethodologyPreviewError(null);
      return;
    }

    const timer = window.setTimeout(() => {
      void requestSkillMethodologyPreview(
        selectedSkillName,
        editSkillMethodologyDraft,
        (body, error) => {
          setEditSkillMethodologyPreview(body);
          setEditSkillMethodologyPreviewError(error);
        },
      );
    }, 120);

    return () => window.clearTimeout(timer);
  }, [
    editSkillEditorMode,
    editSkillMethodologyDraft,
    requestSkillMethodologyPreview,
    selectedSkillName,
    setEditSkillMethodologyPreview,
    setEditSkillMethodologyPreviewError,
  ]);

  const buildSkillSyncSource = useCallback((): SkillSourceRefRecord => {
    if (!skillSyncSourceId.trim()) {
      throw new Error(t("settings.feedback.skillSourceIdRequired"));
    }
    if (!skillSyncLocator.trim()) {
      throw new Error(t("settings.feedback.skillLocatorRequired"));
    }
    return {
      source_id: skillSyncSourceId.trim(),
      source_kind: skillSyncSourceKind,
      locator: skillSyncLocator.trim(),
      revision: skillSyncRevision.trim() || undefined,
    };
  }, [skillSyncLocator, skillSyncRevision, skillSyncSourceId, skillSyncSourceKind, t]);

  const planSkillSync = async () => {
    const source = buildSkillSyncSource();
    setBusyKey(`skill:sync:plan:${source.source_id}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillHubSyncPlanResponseRecord>("/skill/hub/sync/plan", {
        method: "POST",
        body: JSON.stringify({ source }),
      });
      setSkillSyncPlan(response.plan);
      await reloadSettingsData();
      setFeedback(t("settings.feedback.syncPlanBuilt", { source: source.source_id }));
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const refreshSkillSourceIndex = async () => {
    const source = buildSkillSyncSource();
    setBusyKey(`skill:index:refresh:${source.source_id}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillHubIndexRefreshResponseRecord>("/skill/hub/index/refresh", {
        method: "POST",
        body: JSON.stringify({ source }),
      });
      await reloadSettingsData();
      setFeedback(
        t("settings.feedback.sourceIndexRefreshed", {
          source: response.snapshot.source.source_id,
          count: response.snapshot.entries.length,
        }),
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const runGuard = async (request: SkillHubGuardRunRequestRecord, targetLabel: string) => {
    await runMutation(
      `skill:guard:${targetLabel}`,
      async () => {
        const response = await apiJson<SkillHubGuardRunResponseRecord>("/skill/hub/guard/run", {
          method: "POST",
          body: JSON.stringify(request),
        });
        setSkillGuardTarget(targetLabel);
        setSkillGuardReports(response.reports);
        const violationCount = response.reports.reduce(
          (total, report) => total + report.violations.length,
          0,
        );
        return t("settings.feedback.guardScannedDetail", { target: targetLabel, reports: response.reports.length, violations: violationCount });
      },
      t("settings.feedback.guardScanned", { target: targetLabel }),
    );
  };

  const applySkillSync = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    await runMutation(
      `skill:sync:apply:${source.source_id}`,
      async () => {
        const response = await apiJson<SkillHubSyncPlanResponseRecord>("/skill/hub/sync/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
          }),
        });
        setSkillSyncPlan(response.plan);
        if ((response.guard_reports?.length ?? 0) > 0) {
          return t("settings.feedback.syncAppliedWithWarnings", { source: source.source_id, count: response.guard_reports?.length ?? 0 });
        }
      },
      t("settings.feedback.syncApplied", { source: source.source_id }),
    );
  };

  const planRemoteInstall = async () => {
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    setBusyKey(`skill:install:plan:${source.source_id}:${skillName}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillRemoteInstallPlanRecord>("/skill/hub/install/plan", {
        method: "POST",
        body: JSON.stringify({
          source,
          skill_name: skillName,
        }),
      });
      setRemoteInstallPlan(response);
      await reloadSettingsData();
      setFeedback(
        t("settings.feedback.installPlanBuilt", {
          name: response.entry.skill_name,
          source: source.source_id,
          action: response.entry.action,
        }),
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const applyRemoteInstall = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    await runMutation(
      `skill:install:apply:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillRemoteInstallResponseRecord>("/skill/hub/install/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        setRemoteInstallPlan(response.plan);
        const violationCount = response.guard_report?.violations.length ?? 0;
        if (violationCount > 0) {
          return t("settings.feedback.remoteActionAppliedWithWarnings", { action: response.plan.entry.action, name: response.result.skill_name, count: violationCount });
        }
        return t("settings.feedback.remoteActionApplied", { action: response.plan.entry.action, name: response.result.skill_name });
      },
      t("settings.feedback.installApplied", { name: skillName }),
    );
  };

  const planRemoteUpdate = async () => {
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    setBusyKey(`skill:update:plan:${source.source_id}:${skillName}`);
    setFeedback(null);
    try {
      const response = await apiJson<SkillRemoteInstallPlanRecord>("/skill/hub/update/plan", {
        method: "POST",
        body: JSON.stringify({
          source,
          skill_name: skillName,
        }),
      });
      setRemoteInstallPlan(response);
      await reloadSettingsData();
      setFeedback(
        t("settings.feedback.updatePlanBuilt", {
          name: response.entry.skill_name,
          source: source.source_id,
          action: response.entry.action,
        }),
      );
    } catch (error) {
      const message = formatError(error);
      setFeedback(message);
      onBanner(message);
    } finally {
      setBusyKey(null);
    }
  };

  const applyRemoteUpdate = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    await runMutation(
      `skill:update:apply:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillRemoteInstallResponseRecord>("/skill/hub/update/apply", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        setRemoteInstallPlan(response.plan);
        const violationCount = response.guard_report?.violations.length ?? 0;
        if (violationCount > 0) {
          return t("settings.feedback.remoteActionAppliedWithWarnings", { action: response.plan.entry.action, name: response.result.skill_name, count: violationCount });
        }
        return t("settings.feedback.remoteActionApplied", { action: response.plan.entry.action, name: response.result.skill_name });
      },
      t("settings.feedback.updateApplied", { name: skillName }),
    );
  };

  const detachManagedSkill = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    await runMutation(
      `skill:detach:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillHubManagedDetachResponseRecord>("/skill/hub/detach", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        return t("settings.feedback.skillDetached", { name: response.lifecycle.skill_name });
      },
      t("settings.feedback.skillDetachedFallback", { name: skillName }),
    );
  };

  const removeManagedSkill = async () => {
    if (!selectedSessionId) return;
    const source = buildSkillSyncSource();
    const skillName = remoteInstallSkillName.trim();
    if (!skillName) {
      throw new Error(t("settings.feedback.remoteSkillRequired"));
    }
    await runMutation(
      `skill:remove:${source.source_id}:${skillName}`,
      async () => {
        const response = await apiJson<SkillHubManagedRemoveResponseRecord>("/skill/hub/remove", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            source,
            skill_name: skillName,
          }),
        });
        if (response.deleted_from_workspace) {
          return t("settings.feedback.skillRemovedWithDelete", { name: response.lifecycle.skill_name });
        }
        return t("settings.feedback.skillRemovedNoDelete", { name: response.lifecycle.skill_name });
      },
      t("settings.feedback.skillRemovedFallback", { name: skillName }),
    );
  };

  const createSkill = async () => {
    if (!selectedSessionId) return;
    await runMutation(
      `skill:create:${newSkillName.trim() || "new"}`,
      async () => {
        const methodology =
          newSkillEditorMode === "methodology"
            ? buildMethodologyTemplateFromDraft(newSkillMethodologyDraft)
            : undefined;
        const response = await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: "create",
            name: newSkillName,
            description: newSkillDescription,
            category: newSkillCategory.trim() || undefined,
            body: newSkillEditorMode === "raw" ? newSkillBody : undefined,
            methodology,
          }),
        });
        setSelectedSkillName(response.result.skill_name);
        setNewSkillName("");
        setNewSkillDescription("");
        setNewSkillCategory("");
        setNewSkillBody("");
        setNewSkillEditorMode("methodology");
        setNewSkillMethodologyDraft(emptySkillMethodologyDraft());
        setNewSkillMethodologyPreview("");
        setNewSkillMethodologyPreviewError(null);
        if (response.guard_report) {
          return t("settings.feedback.skillCreatedWithWarnings", { name: response.result.skill_name, count: response.guard_report.violations.length });
        }
      },
      t("settings.feedback.skillCreated", { name: newSkillName.trim() }),
    );
  };

  const saveSelectedSkill = async () => {
    if (!selectedSessionId || !selectedSkillName) return;
    await runMutation(
      `skill:edit:${selectedSkillName}`,
      async () => {
        const structuredMode = editSkillEditorMode === "methodology";
        const response = await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: structuredMode ? "patch" : "edit",
            name: selectedSkillName,
            description: structuredMode ? editSkillDescription : undefined,
            methodology: structuredMode
              ? buildMethodologyTemplateFromDraft(editSkillMethodologyDraft)
              : undefined,
            content: structuredMode ? undefined : skillEditorContent,
          }),
        });
        if (response.guard_report) {
          return t("settings.feedback.skillSavedWithWarnings", { name: selectedSkillName, count: response.guard_report.violations.length });
        }
      },
      t("settings.feedback.skillSaved", { name: selectedSkillName }),
    );
  };

  const deleteSelectedSkill = async () => {
    if (!selectedSessionId || !selectedSkillName) return;
    const deletedSkillName = selectedSkillName;
    await runMutation(
      `skill:delete:${deletedSkillName}`,
      async () => {
        await apiJson<SkillManageResponseRecord>("/skill/manage", {
          method: "POST",
          body: JSON.stringify({
            session_id: selectedSessionId,
            action: "delete",
            name: deletedSkillName,
          }),
        });
        setSelectedSkillName(null);
      },
      t("settings.feedback.skillDeleted", { name: deletedSkillName }),
    );
  };

  const runSelectedSkillGuard = async () => {
    if (!selectedSkillName) return;
    await runGuard({ skill_name: selectedSkillName }, `skill ${selectedSkillName}`);
  };

  const runSelectedSourceGuard = async () => {
    const source = buildSkillSyncSource();
    await runGuard({ source }, `source ${source.source_id}`);
  };

  return {
    planSkillSync,
    refreshSkillSourceIndex,
    runGuard,
    applySkillSync,
    planRemoteInstall,
    applyRemoteInstall,
    planRemoteUpdate,
    applyRemoteUpdate,
    detachManagedSkill,
    removeManagedSkill,
    createSkill,
    saveSelectedSkill,
    deleteSelectedSkill,
    runSelectedSkillGuard,
    runSelectedSourceGuard,
  };
}
