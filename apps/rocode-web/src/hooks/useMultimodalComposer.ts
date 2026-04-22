import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  MultimodalCapabilitiesResponse,
  MultimodalCapabilityView,
  MultimodalPolicyView,
  MultimodalPreflightRequest,
  MultimodalPreflightResponse,
  SessionPartRecord,
} from "../lib/multimodal";

type ApiJson = <T>(path: string, options?: RequestInit) => Promise<T>;

function dedupeWarnings(values: string[]) {
  const normalized = values.map((value) => value.trim()).filter(Boolean);
  return normalized.filter(
    (value, index) =>
      normalized.findIndex((candidate) => candidate === value) === index,
  );
}

function fileSessionParts(parts: SessionPartRecord[]) {
  return parts.filter((part) => part.type === "file");
}

function acceptedCapabilityLabels(capability: MultimodalCapabilityView) {
  const accepted: string[] = [];
  if (capability.input.audio) accepted.push("audio");
  if (capability.input.image) accepted.push("image");
  if (capability.input.video) accepted.push("video");
  if (capability.input.pdf) accepted.push("pdf");
  if (capability.attachment) accepted.push("file");
  return accepted;
}

export function useMultimodalComposer({
  apiJson,
  selectedModel,
  attachments,
  scopeKey,
}: {
  apiJson: ApiJson;
  selectedModel: string;
  attachments: SessionPartRecord[];
  scopeKey: string;
}) {
  const [policy, setPolicy] = useState<MultimodalPolicyView | null>(null);
  const [capability, setCapability] = useState<MultimodalCapabilityView | null>(null);
  const [resolvedModel, setResolvedModel] = useState("");
  const [warnings, setWarnings] = useState<string[]>([]);

  useEffect(() => {
    let cancelled = false;

    const loadReadModels = async () => {
      try {
        const query =
          selectedModel.trim().length > 0
            ? `?model=${encodeURIComponent(selectedModel.trim())}`
            : "";
        const [policyResponse, capabilitiesResponse] = await Promise.all([
          apiJson<{ policy: MultimodalPolicyView }>("/multimodal/policy"),
          apiJson<MultimodalCapabilitiesResponse>(`/multimodal/capabilities${query}`),
        ]);
        if (cancelled) return;
        setPolicy(policyResponse.policy);
        setCapability(capabilitiesResponse.capability ?? null);
        setResolvedModel(capabilitiesResponse.resolved_model ?? "");
      } catch {
        if (cancelled) return;
        setCapability(null);
        setResolvedModel("");
      }
    };

    void loadReadModels();
    return () => {
      cancelled = true;
    };
  }, [apiJson, scopeKey, selectedModel]);

  useEffect(() => {
    let cancelled = false;

    const runAttachmentPreflight = async () => {
      const sessionParts = fileSessionParts(attachments);
      if (sessionParts.length === 0) {
        setWarnings([]);
        return;
      }
      try {
        const preflight = await apiJson<MultimodalPreflightResponse>("/multimodal/preflight", {
          method: "POST",
          body: JSON.stringify({
            model: selectedModel || undefined,
            parts: [],
            session_parts: sessionParts,
          } satisfies MultimodalPreflightRequest),
        });
        if (cancelled) return;
        setWarnings(
          dedupeWarnings([
            ...(preflight.warnings ?? []),
            ...(preflight.result?.warnings ?? []),
          ]),
        );
      } catch {
        if (cancelled) return;
        setWarnings([]);
      }
    };

    void runAttachmentPreflight();
    return () => {
      cancelled = true;
    };
  }, [apiJson, attachments, selectedModel]);

  const preflightBeforeSubmit = useCallback(async () => {
    const sessionParts = fileSessionParts(attachments);
    if (sessionParts.length === 0) {
      return { blocked: false as const, banner: null as string | null };
    }

    const request: MultimodalPreflightRequest = {
      model: selectedModel || undefined,
      parts: [],
      session_parts: sessionParts,
    };

    const preflight = await apiJson<MultimodalPreflightResponse>("/multimodal/preflight", {
      method: "POST",
      body: JSON.stringify(request),
    });
    const nextWarnings = dedupeWarnings([
      ...(preflight.warnings ?? []),
      ...(preflight.result?.warnings ?? []),
    ]);
    setWarnings(nextWarnings);

    if (preflight.result?.hard_block) {
      return {
        blocked: true as const,
        banner:
          nextWarnings[0] ??
          preflight.result.recommended_downgrade ??
          "Current multimodal policy blocked this input.",
      };
    }

    return {
      blocked: false as const,
      banner: nextWarnings.length > 0 ? nextWarnings.join(" ") : null,
    };
  }, [apiJson, attachments, selectedModel]);

  const hints = useMemo(() => {
    const next: Array<{ tone: "info" | "warning"; text: string }> = [];
    const seen = new Set<string>();
    const pushHint = (tone: "info" | "warning", text: string) => {
      const normalized = text.trim();
      if (!normalized || seen.has(`${tone}:${normalized}`)) return;
      seen.add(`${tone}:${normalized}`);
      next.push({ tone, text: normalized });
    };

    if (policy) {
      if (!policy.allow_audio_input) {
        pushHint("warning", "Voice input disabled by current multimodal policy.");
      }
      if (typeof policy.max_attachments_per_prompt === "number") {
        const attachmentCount = fileSessionParts(attachments).length;
        if (attachmentCount > policy.max_attachments_per_prompt) {
          pushHint(
            "warning",
            `Attachments exceed policy limit (${attachmentCount}/${policy.max_attachments_per_prompt}).`,
          );
        }
      }
    }

    for (const warning of warnings) {
      pushHint("warning", warning);
    }

    if (attachments.length > 0 && capability) {
      const accepted = acceptedCapabilityLabels(capability);
      pushHint(
        "info",
        accepted.length > 0
          ? `Current model accepts ${accepted.join(", ")} input.`
          : "Current model is effectively text-only for multimodal input.",
      );
    } else if (!attachments.length && resolvedModel) {
      pushHint("info", `Current multimodal target: ${resolvedModel}.`);
    }

    return next;
  }, [attachments, capability, policy, resolvedModel, warnings]);

  return {
    policy,
    capability,
    resolvedModel,
    warnings,
    hints,
    preflightBeforeSubmit,
  };
}
