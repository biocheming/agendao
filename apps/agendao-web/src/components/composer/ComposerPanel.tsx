"use client";

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, ClipboardEvent, DragEvent, KeyboardEvent } from "react";
import type { BreadcrumbProvenance } from "../../hooks/useSchedulerNavigation";
import { AttachmentDetailsPanel } from "../chat/AttachmentDetailsPanel";
import { ComposerContextStrip } from "./ComposerContextStrip";
import { SlashCommandMenu } from "./SlashCommandMenu";
import type { ComposerAttachmentRecord } from "../../lib/composerContext";
import {
  filterSlashCommands,
  slashCommandQueryFromText,
  type CommandApiSpec,
} from "../../lib/command";
import { contextPressureLabel, contextPressureTone } from "../../lib/contextPressure";
import type {
  ProviderModelCapabilitiesRecord,
  ProviderModelRecord,
  ProviderRecord,
} from "../../lib/provider";
import type { RecentModelRecord } from "../../lib/workspace";
import { useI18n } from "@/i18n/I18nProvider";
import { cn } from "@/lib/utils";
import {
  AudioLinesIcon,
  BrainCircuitIcon,
  CheckIcon,
  ChevronsUpDownIcon,
  EyeIcon,
  FileTextIcon,
  ImageIcon,
  MicIcon,
  PaperclipIcon,
  PlusIcon,
  SendIcon,
  SquareIcon,
  VideoIcon,
  WrenchIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

const AUTO_MODEL_VALUE = "__auto__";

function parseModeKind(modeKey: string): string | null {
  const colonIdx = modeKey.indexOf(":");
  if (colonIdx < 1) return null;
  return modeKey.slice(0, colonIdx);
}

function compactOptionLabel(label: string) {
  const trimmed = label.trim();
  if (!trimmed) return trimmed;
  const slashParts = trimmed.split("/").map((part) => part.trim()).filter(Boolean);
  if (slashParts.length > 1) return slashParts[slashParts.length - 1];
  const separatorParts = trimmed.split("·").map((part) => part.trim()).filter(Boolean);
  if (separatorParts.length > 1) return separatorParts[0];
  return trimmed;
}

function formatCompactCapacity(value?: number | null) {
  return formatCompactTokenCount(value).replace(".0K", "K").replace(".0M", "M");
}

function findProviderModel(
  providers: ProviderRecord[],
  selectedModel: string,
): { provider: ProviderRecord; model: ProviderModelRecord; key: string } | null {
  const target = selectedModel.trim();
  if (!target) return null;

  for (const provider of providers) {
    for (const model of provider.models ?? []) {
      const key = `${provider.id}/${model.id}`;
      if (
        key === target ||
        model.id === target ||
        key.endsWith(`/${target}`)
      ) {
        return { provider, model, key };
      }
    }
  }

  return null;
}

function capabilityBadges(capabilities?: ProviderModelCapabilitiesRecord | null) {
  if (!capabilities) return [];

  const badges: Array<{
    key: string;
    label: string;
    icon: React.ComponentType<{ className?: string }>;
  }> = [];
  const push = (
    enabled: boolean | null | undefined,
    key: string,
    label: string,
    icon: React.ComponentType<{ className?: string }>,
  ) => {
    if (!enabled || badges.some((badge) => badge.key === key)) return;
    badges.push({ key, label, icon });
  };

  push(capabilities.input?.image || capabilities.output?.image, "vision", "Vision", EyeIcon);
  push(capabilities.input?.audio || capabilities.output?.audio, "audio", "Audio", AudioLinesIcon);
  push(capabilities.input?.video || capabilities.output?.video, "video", "Video", VideoIcon);
  push(capabilities.input?.pdf || capabilities.output?.pdf, "pdf", "PDF", FileTextIcon);
  push(capabilities.attachment, "files", "Files", PaperclipIcon);
  push(capabilities.tool_call, "tools", "Tools", WrenchIcon);
  push(capabilities.reasoning, "reasoning", "Reasoning", BrainCircuitIcon);

  return badges;
}

const MODEL_META_BADGE_CLASS =
  "inline-flex h-5 items-center gap-1 rounded-full border border-border/45 bg-background/72 px-2 text-[10px] font-medium leading-none text-muted-foreground";

function renderModelBadge({
  label,
  icon: Icon,
}: {
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}) {
  return (
    <span
      key={label}
      className={MODEL_META_BADGE_CLASS}
      title={label}
    >
      <Icon className="size-3" />
      <span>{label}</span>
    </span>
  );
}

function formatCompactTokenCount(value?: number | null) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return "0";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(Math.round(value));
}

function formatCompactPrice(value?: number | null) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return value.toFixed(2);
}

const VOICE_MIME_CANDIDATES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/ogg;codecs=opus",
  "audio/ogg",
  "audio/mp4",
] as const;

function resolveVoiceRecordingMimeType() {
  if (typeof window === "undefined" || typeof window.MediaRecorder === "undefined") {
    return null;
  }
  const ctor = window.MediaRecorder as typeof MediaRecorder & {
    isTypeSupported?: (mimeType: string) => boolean;
  };
  for (const candidate of VOICE_MIME_CANDIDATES) {
    if (typeof ctor.isTypeSupported !== "function" || ctor.isTypeSupported(candidate)) {
      return candidate;
    }
  }
  return "";
}

function voiceFileExtension(mimeType: string) {
  if (mimeType.includes("ogg")) return "ogg";
  if (mimeType.includes("mp4")) return "m4a";
  return "webm";
}

interface ComposerPanelProps {
  composer: string;
  composerDragActive: boolean;
  streaming: boolean;
  multimodalHints: Array<{ tone: "info" | "warning"; text: string }>;
  allowAudioInput: boolean;
  allowImageInput: boolean;
  allowFileInput: boolean;
  modeOptions: Array<{ key: string; label: string }>;
  selectedMode: string;
  onModeChange: (value: string) => void;
  providers: ProviderRecord[];
  recentModels: RecentModelRecord[];
  selectedModel: string;
  onModelChange: (value: string) => void;
  references: string[];
  slashCommands: CommandApiSpec[];
  attachments: ComposerAttachmentRecord[];
  selectedAttachmentIndex: number | null;
  selectedAttachment: ComposerAttachmentRecord | null;
  selectedWorkspacePath: string | null;
  workspaceRootPath: string;
  contextTokensUsed?: number | null;
  contextTokensLimit?: number | null;
  lastTurnInputTokens?: number | null;
  lastTurnOutputTokens?: number | null;
  cacheReadTokens?: number | null;
  cacheMissTokens?: number | null;
  cacheWriteTokens?: number | null;
  closureDiagnosticLabel?: string | null;
  ingressDiagnosticLabel?: string | null;
  providerDiagnosticLabel?: string | null;
  inputPricePerMillion?: number | null;
  outputPricePerMillion?: number | null;
  composerNotice?: { id: number; text: string; count: number } | null;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  permissionStatusLabel?: string | null;
  permissionStatusTone?: "muted" | "warning" | "destructive";
  onPreviewStage?: (stageId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onStopStreaming: () => void | Promise<void>;
  onRemoveReference: (reference: string) => void;
  onRemoveAttachment: (index: number) => void;
  onSelectAttachment: (index: number, attachment: ComposerAttachmentRecord) => void;
  onLocateAttachment: (attachment: ComposerAttachmentRecord) => void;
  onNavigateStage: (stageId: string) => void;
  onNavigateProvenanceSession: () => void;
  onNavigateProvenanceStage: () => void;
  onNavigateProvenanceToolCall: () => void;
  onDragEnter: (event: DragEvent<HTMLDivElement>) => void;
  onDragOver: (event: DragEvent<HTMLDivElement>) => void;
  onDragLeave: (event: DragEvent<HTMLDivElement>) => void;
  onDrop: (event: DragEvent<HTMLDivElement>) => void;
  onAttachFiles: (files: File[], failurePrefix: string) => void | Promise<void>;
  onFileChange: (event: React.ChangeEvent<HTMLInputElement>) => void | Promise<void>;
  onPaste: (event: ClipboardEvent<HTMLTextAreaElement>) => void | Promise<void>;
  onComposerChange: (value: string) => void;
}

export function ComposerPanel({
  composer,
  composerDragActive,
  streaming,
  multimodalHints,
  allowAudioInput,
  allowImageInput,
  allowFileInput,
  modeOptions,
  selectedMode,
  onModeChange,
  providers,
  recentModels,
  selectedModel,
  onModelChange,
  references,
  slashCommands,
  attachments,
  selectedAttachmentIndex,
  selectedAttachment,
  selectedWorkspacePath,
  workspaceRootPath,
  contextTokensUsed = null,
  contextTokensLimit = null,
  lastTurnInputTokens = null,
  lastTurnOutputTokens = null,
  cacheReadTokens = null,
  cacheMissTokens = null,
  cacheWriteTokens = null,
  closureDiagnosticLabel = null,
  ingressDiagnosticLabel = null,
  providerDiagnosticLabel = null,
  inputPricePerMillion = null,
  outputPricePerMillion = null,
  composerNotice = null,
  activeStageId,
  provenance,
  permissionStatusLabel,
  permissionStatusTone = "muted",
  onPreviewStage,
  onSubmit,
  onStopStreaming,
  onRemoveReference,
  onRemoveAttachment,
  onSelectAttachment,
  onLocateAttachment,
  onNavigateStage,
  onNavigateProvenanceSession,
  onNavigateProvenanceStage,
  onNavigateProvenanceToolCall,
  onDragEnter,
  onDragOver,
  onDragLeave,
  onDrop,
  onAttachFiles,
  onFileChange,
  onPaste,
  onComposerChange,
}: ComposerPanelProps) {
  const { t } = useI18n();
  type NoticePhase = "idle" | "enter" | "exit";
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  const voiceRecorderRef = useRef<MediaRecorder | null>(null);
  const voiceStreamRef = useRef<MediaStream | null>(null);
  const voiceChunksRef = useRef<BlobPart[]>([]);
  const voiceMimeTypeRef = useRef("audio/webm");
  const previousAttachmentCountRef = useRef(attachments.length);
  const [voiceSupported, setVoiceSupported] = useState(false);
  const [voiceListening, setVoiceListening] = useState(false);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const [displayNotice, setDisplayNotice] = useState(composerNotice);
  const [noticePhase, setNoticePhase] = useState<NoticePhase>(composerNotice ? "enter" : "idle");
  const [countPulse, setCountPulse] = useState(false);
  const [slashActiveIndex, setSlashActiveIndex] = useState(0);
  const [slashDismissedQuery, setSlashDismissedQuery] = useState<string | null>(null);
  const slashCaretRef = useRef<number | null>(null);

  const slashQuery = slashCommandQueryFromText(composer);
  const slashItems = useMemo(
    () => (slashQuery === null ? [] : filterSlashCommands(slashCommands, slashQuery)),
    [slashCommands, slashQuery],
  );
  const slashMenuOpen =
    !streaming &&
    slashQuery !== null &&
    slashQuery !== slashDismissedQuery &&
    slashItems.length > 0;
  const slashMenuActiveIndex = slashItems.length
    ? Math.min(slashActiveIndex, slashItems.length - 1)
    : 0;

  useEffect(() => {
    setSlashActiveIndex(0);
  }, [slashQuery]);

  useLayoutEffect(() => {
    if (slashCaretRef.current === null) return;
    const caret = slashCaretRef.current;
    slashCaretRef.current = null;
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.focus();
    textarea.setSelectionRange(caret, caret);
  }, [composer]);

  useEffect(() => {
    if (!slashMenuOpen) return;
    const activeItem = textareaRef.current
      ?.closest("form")
      ?.querySelector<HTMLElement>(
        '[data-testid="slash-command-menu"] [data-active="true"]',
      );
    activeItem?.scrollIntoView?.({ block: "nearest" });
  }, [slashMenuOpen, slashMenuActiveIndex, slashItems]);

  const selectSlashCommand = (command: CommandApiSpec) => {
    slashCaretRef.current = command.name.length + 2;
    setSlashDismissedQuery(null);
    setSlashActiveIndex(0);
    onComposerChange(`/${command.name} `);
  };

  const handleComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!slashMenuOpen) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSlashActiveIndex((current) => (current + 1) % slashItems.length);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSlashActiveIndex(
        (current) => (current - 1 + slashItems.length) % slashItems.length,
      );
      return;
    }
    if (event.key === "Enter" || event.key === "Tab") {
      event.preventDefault();
      const command = slashItems[slashMenuActiveIndex];
      if (command) selectSlashCommand(command);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setSlashDismissedQuery(slashQuery);
    }
  };

  useLayoutEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;

    textarea.style.height = "auto";

    const computed = window.getComputedStyle(textarea);
    const lineHeight = Number.parseFloat(computed.lineHeight) || 24;
    const paddingTop = Number.parseFloat(computed.paddingTop) || 0;
    const paddingBottom = Number.parseFloat(computed.paddingBottom) || 0;
    const borderTop = Number.parseFloat(computed.borderTopWidth) || 0;
    const borderBottom = Number.parseFloat(computed.borderBottomWidth) || 0;
    const maxHeight =
      lineHeight * 10 + paddingTop + paddingBottom + borderTop + borderBottom;
    const nextHeight = Math.min(textarea.scrollHeight, maxHeight);

    textarea.style.height = `${nextHeight}px`;
    textarea.style.overflowY =
      textarea.scrollHeight > maxHeight ? "auto" : "hidden";
  }, [composer]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const supportsRecording =
      typeof window.MediaRecorder !== "undefined" &&
      typeof navigator !== "undefined" &&
      !!navigator.mediaDevices?.getUserMedia;
    setVoiceSupported(supportsRecording);

    return () => {
      const recorder = voiceRecorderRef.current;
      if (recorder && recorder.state !== "inactive") {
        recorder.stop();
      }
      voiceRecorderRef.current = null;
      voiceChunksRef.current = [];
      voiceStreamRef.current?.getTracks().forEach((track) => track.stop());
      voiceStreamRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (attachments.length > previousAttachmentCountRef.current) {
      setDetailsExpanded(true);
    }
    previousAttachmentCountRef.current = attachments.length;
  }, [attachments.length]);

  useEffect(() => {
    if (!composerNotice) {
      if (!displayNotice) return;
      setNoticePhase("exit");
      const timeoutId = window.setTimeout(() => {
        setDisplayNotice(null);
        setNoticePhase("idle");
      }, 140);
      return () => window.clearTimeout(timeoutId);
    }

    if (!displayNotice) {
      setDisplayNotice(composerNotice);
      setNoticePhase("enter");
      return;
    }

    if (displayNotice.id === composerNotice.id) return;

    setNoticePhase("exit");
    const timeoutId = window.setTimeout(() => {
      setDisplayNotice(composerNotice);
      setNoticePhase("enter");
    }, 130);
    return () => window.clearTimeout(timeoutId);
  }, [composerNotice, displayNotice]);

  useEffect(() => {
    if (noticePhase !== "enter") return;
    const timeoutId = window.setTimeout(() => {
      setNoticePhase("idle");
    }, 260);
    return () => window.clearTimeout(timeoutId);
  }, [noticePhase, displayNotice?.id]);

  useEffect(() => {
    if (!displayNotice || displayNotice.count <= 1) {
      setCountPulse(false);
      return;
    }
    setCountPulse(true);
    const timeoutId = window.setTimeout(() => {
      setCountPulse(false);
    }, 360);
    return () => window.clearTimeout(timeoutId);
  }, [displayNotice]);

  const stopVoiceCapture = () => {
    const recorder = voiceRecorderRef.current;
    if (!recorder || recorder.state === "inactive") {
      setVoiceListening(false);
      return;
    }
    recorder.stop();
    setVoiceListening(false);
  };

  const startVoiceCapture = async () => {
    if (typeof window === "undefined" || typeof navigator === "undefined") return;
    if (typeof window.MediaRecorder === "undefined" || !navigator.mediaDevices?.getUserMedia) {
      setVoiceSupported(false);
      setVoiceError(t("composer.voiceUnsupported"));
      return;
    }
    if (voiceRecorderRef.current && voiceRecorderRef.current.state !== "inactive") {
      stopVoiceCapture();
      return;
    }

    setVoiceError(null);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = resolveVoiceRecordingMimeType();
      const recorder = mimeType
        ? new window.MediaRecorder(stream, mimeType ? { mimeType } : undefined)
        : new window.MediaRecorder(stream);

      voiceStreamRef.current?.getTracks().forEach((track) => track.stop());
      voiceStreamRef.current = stream;
      voiceRecorderRef.current = recorder;
      voiceChunksRef.current = [];
      voiceMimeTypeRef.current = recorder.mimeType || mimeType || "audio/webm";

      recorder.ondataavailable = (event) => {
        if (event.data && event.data.size > 0) {
          voiceChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        setVoiceError(t("composer.voiceCaptureFailed"));
        setVoiceListening(false);
      };
      recorder.onstop = () => {
        const mimeType = voiceMimeTypeRef.current || recorder.mimeType || "audio/webm";
        const chunks = voiceChunksRef.current.slice();
        voiceChunksRef.current = [];
        voiceRecorderRef.current = null;
        const stream = voiceStreamRef.current;
        voiceStreamRef.current = null;
        stream?.getTracks().forEach((track) => track.stop());

        if (!chunks.length) {
          setVoiceError(t("composer.voiceNoAudioCaptured"));
          return;
        }

        const blob = new Blob(chunks, { type: mimeType });
        const extension = voiceFileExtension(mimeType);
        const file = new File([blob], `voice-${Date.now()}.${extension}`, {
          type: mimeType,
        });
        void Promise.resolve(onAttachFiles([file], t("composer.voiceCaptureFailedShort"))).catch((error) => {
          setVoiceError(
            error instanceof Error ? error.message : t("composer.voiceAttachFailed"),
          );
        });
      };

      recorder.start();
      setVoiceListening(true);
    } catch (error) {
      const message =
        error instanceof DOMException && error.name === "NotAllowedError"
          ? t("composer.voiceMicDenied")
          : error instanceof DOMException && error.name === "NotFoundError"
            ? t("composer.voiceNoMicFound")
            : error instanceof Error
              ? error.message
              : t("composer.voiceStartFailed");
      setVoiceError(message);
      setVoiceListening(false);
    }
  };

  const contextCount = references.length + attachments.length;
  const modeValue = selectedMode || "";
  const modelValue = selectedModel || "";
  const selectedProviderModel = findProviderModel(providers, modelValue);
  const selectedModelBadges = capabilityBadges(selectedProviderModel?.model.capabilities);
  const activityHint = voiceError
    ? voiceError
    : composerDragActive
      ? t("composer.dropFilesHint")
      : voiceListening
        ? t("composer.recordingHint")
      : streaming
        ? t("composer.respondingHint")
        : null;
  const hasContextEstimate =
    typeof contextTokensUsed === "number" &&
    Number.isFinite(contextTokensUsed) &&
    contextTokensUsed > 0;
  const hasContextLimit =
    typeof contextTokensLimit === "number" &&
    Number.isFinite(contextTokensLimit) &&
    contextTokensLimit > 0;
  const contextRatio =
    hasContextEstimate && hasContextLimit
      ? Math.max(0, Math.min(1, contextTokensUsed / contextTokensLimit))
      : null;
  const contextPercentValue =
    contextRatio === null ? null : Math.max(1, Math.round(contextRatio * 100));
  const contextPercent = contextPercentValue === null ? null : `${contextPercentValue}%`;
  const contextPressure = contextPressureLabel(contextPercentValue);
  const contextPressureClass = cn(
    "font-medium",
    contextPressureTone(contextPercentValue) === "critical"
      ? "text-destructive"
      : contextPressureTone(contextPercentValue) === "warning"
        ? "text-(--ds-warn)"
        : "text-muted-foreground",
  );
  const pricingLabel = (() => {
    const input = formatCompactPrice(inputPricePerMillion);
    const output = formatCompactPrice(outputPricePerMillion);
    if (!input || !output) return null;
    return t("composer.pricingLabel", { input, output });
  })();
  const turnUsageLabel =
    typeof lastTurnInputTokens === "number" &&
    typeof lastTurnOutputTokens === "number" &&
    (lastTurnInputTokens > 0 || lastTurnOutputTokens > 0)
      ? t("composer.turnUsage", {
          input: formatCompactTokenCount(lastTurnInputTokens),
          output: formatCompactTokenCount(lastTurnOutputTokens),
        })
      : null;
  const cacheUsageLabel =
    ((cacheReadTokens ?? 0) > 0 || (cacheMissTokens ?? 0) > 0 || (cacheWriteTokens ?? 0) > 0)
      ? t("composer.cacheUsage", {
          read: formatCompactTokenCount(cacheReadTokens),
          miss: formatCompactTokenCount(cacheMissTokens),
          write: formatCompactTokenCount(cacheWriteTokens),
        })
      : null;
  const contextBadgeLabel =
    contextCount > 0 ? t("composer.contextBadge", { refs: references.length, files: attachments.length }) : null;
  const contextSummary = hasContextEstimate
    ? hasContextLimit
      ? `${formatCompactTokenCount(contextTokensUsed)} / ${formatCompactTokenCount(contextTokensLimit)}`
      : formatCompactTokenCount(contextTokensUsed)
    : null;
  const selectedModelTitle = selectedProviderModel
    ? selectedProviderModel.model.name?.trim() || selectedProviderModel.model.id
    : modelValue.trim()
      ? compactOptionLabel(modelValue)
      : t("composer.auto");
  const selectedModelSubtitle = selectedProviderModel
    ? [
        selectedProviderModel.provider.name,
        selectedProviderModel.model.context_window
          ? `${formatCompactCapacity(selectedProviderModel.model.context_window)} ctx`
          : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : modelValue.trim()
      ? t("composer.modelSelectedExplicitly")
      : t("composer.modelDefaultSubtitle");
  const modelSearchValue = modelValue || AUTO_MODEL_VALUE;
  const recentProviderModels = useMemo(() => {
    const entries: Array<{ provider: ProviderRecord; model: ProviderModelRecord; key: string }> = [];
    const used = new Set<string>();
    for (const recent of recentModels) {
      const provider = providers.find((item) => item.id === recent.provider);
      const model = provider?.models?.find(
        (item) => item.id === recent.model && item.available !== false,
      );
      if (!provider || !model) continue;
      const key = `${provider.id}/${model.id}`;
      if (used.has(key)) continue;
      used.add(key);
      entries.push({ provider, model, key });
    }
    return entries;
  }, [providers, recentModels]);
  const recentModelKeys = useMemo(
    () => new Set(recentProviderModels.map((entry) => entry.key)),
    [recentProviderModels],
  );
  const hasDiagnostics =
    Boolean(closureDiagnosticLabel) ||
    Boolean(ingressDiagnosticLabel) ||
    Boolean(providerDiagnosticLabel) ||
    Boolean(pricingLabel) ||
    multimodalHints.length > 0 ||
    Boolean(activityHint) ||
    Boolean(permissionStatusLabel);
  const [telemetryExpanded, setTelemetryExpanded] = useState(false);

  useEffect(() => {
    if (hasDiagnostics) {
      setTelemetryExpanded(true);
    }
  }, [hasDiagnostics]);

  // Badge keys (vision/audio/...) map to composer.capability.* messages; the
  // English label stays in CommandItem keywords so search behavior is unchanged.
  const renderLocalizedModelBadge = (badge: {
    key: string;
    label: string;
    icon: React.ComponentType<{ className?: string }>;
  }) => renderModelBadge({ label: t(`composer.capability.${badge.key}`), icon: badge.icon });

  const renderModelOption = (
    provider: ProviderRecord,
    model: ProviderModelRecord,
    optionKey: string,
  ) => {
    const badges = capabilityBadges(model.capabilities);
    return (
      <CommandItem
        key={optionKey}
        value={optionKey}
        keywords={[
          provider.name,
          provider.id,
          model.id,
          model.name ?? "",
          ...badges.map((badge) => badge.label),
        ]}
        className="items-start rounded-2xl px-3 py-2"
        onSelect={() => {
          onModelChange(optionKey);
          setModelPickerOpen(false);
        }}
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <div className="pt-0.5">
            <CheckIcon
              className={cn(
                "size-4 text-foreground transition-opacity",
                modelSearchValue === optionKey ? "opacity-100" : "opacity-0",
              )}
            />
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium text-foreground">
                {model.name?.trim() || model.id}
              </span>
              {model.name?.trim() && model.name !== model.id ? (
                <span className="truncate text-[11px] text-muted-foreground">
                  {model.id}
                </span>
              ) : null}
            </div>
            <div className="flex flex-wrap items-center gap-1">
              {model.context_window ? (
                <span className={MODEL_META_BADGE_CLASS}>
                  {formatCompactCapacity(model.context_window)} ctx
                </span>
              ) : null}
              {badges.map(renderLocalizedModelBadge)}
            </div>
          </div>
        </div>
      </CommandItem>
    );
  };

  return (
    <div className="flex flex-col gap-2" data-testid="composer-form">
      {detailsExpanded ? (
        <>
          <ComposerContextStrip
            references={references}
            attachments={attachments}
            selectedAttachmentIndex={selectedAttachmentIndex}
            selectedWorkspacePath={selectedWorkspacePath}
            workspaceRootPath={workspaceRootPath}
            activeStageId={activeStageId}
            provenance={provenance}
            onPreviewStage={onPreviewStage}
            onRemoveReference={onRemoveReference}
            onRemoveAttachment={onRemoveAttachment}
            onSelectAttachment={onSelectAttachment}
          />
          <AttachmentDetailsPanel
            attachment={selectedAttachment}
            workspaceRootPath={workspaceRootPath}
            activeStageId={activeStageId}
            provenance={provenance}
            onLocateAttachment={onLocateAttachment}
            onNavigateStage={onNavigateStage}
            onNavigateProvenanceSession={onNavigateProvenanceSession}
            onNavigateProvenanceStage={onNavigateProvenanceStage}
            onNavigateProvenanceToolCall={onNavigateProvenanceToolCall}
          />
        </>
      ) : null}

      <div className="roc-composer-shell" data-drag-active={composerDragActive ? "true" : "false"}>
        <form className="w-full" onSubmit={onSubmit} data-testid="composer-dropzone">
          <div
            className="flex flex-col"
            onDragEnter={onDragEnter}
            onDragOver={onDragOver}
            onDragLeave={onDragLeave}
            onDrop={onDrop}
          >
            <input
              ref={fileInputRef}
              data-testid="composer-file-input"
              type="file"
              multiple
              className="hidden"
              onChange={onFileChange}
            />
            <input
              ref={imageInputRef}
              data-testid="composer-image-input"
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={onFileChange}
            />
            {displayNotice ? (
              <div
                className="px-4 pt-3 md:px-5"
                data-testid="composer-notice"
                role="status"
                aria-live="polite"
              >
                <div
                  key={displayNotice.id}
                  data-state={noticePhase}
                  className="roc-composer-notice-chip inline-flex max-w-full items-center gap-2 rounded-full border border-(--ds-ok)/20 bg-(--ds-ok)/8 px-2.5 py-1 text-[11px] leading-5 text-(--ds-ok) shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
                >
                  <span className="rounded-full bg-(--ds-ok)/14 px-1.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-(--ds-ok)">
                    {t("composer.attached")}
                  </span>
                  {displayNotice.count > 1 ? (
                    <span
                      data-pulse={countPulse ? "true" : "false"}
                      className="roc-composer-notice-count inline-flex min-w-5 items-center justify-center rounded-full bg-(--ds-ok)/16 px-1.5 text-[10px] font-semibold tabular-nums text-(--ds-ok)"
                    >
                      {displayNotice.count}
                    </span>
                  ) : null}
                  <CheckIcon className="size-3.5 shrink-0 text-(--ds-ok)" />
                  <span className="truncate">{displayNotice.text}</span>
                </div>
              </div>
            ) : null}
            <div className={cn("relative px-4 pb-3 md:px-5", displayNotice ? "pt-2.5" : "pt-4")}>
              {slashMenuOpen ? (
                <SlashCommandMenu
                  items={slashItems}
                  activeIndex={slashMenuActiveIndex}
                  onActiveIndexChange={setSlashActiveIndex}
                  onSelect={selectSlashCommand}
                />
              ) : null}
              <div className="flex items-end gap-2">
                <textarea
                  ref={textareaRef}
                  data-testid="composer-input"
                  name="message"
                  rows={1}
                  placeholder={t("composer.placeholder")}
                  value={composer}
                  onChange={(e) => onComposerChange(e.target.value)}
                  onKeyDown={handleComposerKeyDown}
                  onPaste={onPaste}
                  disabled={streaming}
                  className="h-auto min-h-0 flex-1 resize-none border-0 bg-transparent py-0.5 text-[15px] leading-[1.65] text-foreground outline-none placeholder:text-muted-foreground/50"
                />
                <div className="flex shrink-0 items-center gap-1.5 pb-0.5">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="roc-action h-9 w-9 rounded-full"
                        title={t("composer.addAttachment")}
                      >
                        <PlusIcon className="size-4" />
                        {contextCount > 0 ? (
                          <span className="absolute -right-0.5 -top-0.5 min-w-4 rounded-full bg-primary px-1 text-[9px] leading-4 text-primary-foreground">
                            {contextCount}
                          </span>
                        ) : null}
                      </Button>
                    </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="min-w-[8rem]">
                      <DropdownMenuItem
                        disabled={!allowAudioInput || !voiceSupported}
                        onClick={() => void (voiceListening ? stopVoiceCapture() : startVoiceCapture())}
                        className="gap-2 text-xs"
                      >
                        <MicIcon className="size-3.5" />
                        {voiceListening ? t("composer.stopRecording") : t("composer.voice")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={!allowFileInput}
                        onClick={() => fileInputRef.current?.click()}
                        className="gap-2 text-xs"
                      >
                        <PaperclipIcon className="size-3.5" />
                        {t("composer.file")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={!allowImageInput}
                        onClick={() => imageInputRef.current?.click()}
                        className="gap-2 text-xs"
                      >
                        <ImageIcon className="size-3.5" />
                        {t("composer.image")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                  {streaming ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      className="h-9 w-9 rounded-full border-destructive/50 text-destructive hover:bg-destructive/10"
                      data-testid="composer-stop"
                      title={t("composer.stopResponse")}
                      onClick={() => void onStopStreaming()}
                    >
                      <SquareIcon className="size-3.5 fill-current" />
                    </Button>
                  ) : voiceListening ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      className="h-9 w-9 rounded-full border-(--ds-warn)/55 text-(--ds-warn) hover:bg-(--ds-warn)/10"
                      data-testid="composer-voice-stop"
                      title={t("composer.stopVoiceRecording")}
                      onClick={stopVoiceCapture}
                    >
                      <MicIcon className="size-3.5 fill-current" />
                    </Button>
                  ) : null}
                  <Button
                    type="submit"
                    data-testid="composer-send"
                    variant="ghost"
                    size="icon"
                    disabled={!composer.trim() && attachments.length === 0}
                    className="roc-primary-action h-9 w-9 rounded-full"
                    title={t("composer.send")}
                  >
                    <SendIcon className="size-4" />
                  </Button>
                </div>
              </div>
            </div>

            <button
              type="button"
              className="roc-composer-meter group relative h-3 w-full overflow-hidden border-t border-border/55 bg-muted/35 text-left"
              title={t("composer.meterTitle")}
              aria-expanded={detailsExpanded}
              onClick={() => setDetailsExpanded((value) => !value)}
            >
              <span
                className={cn(
                  "absolute inset-y-0 left-0 transition-[width,background-color] duration-200 ease-out",
                  contextRatio === null
                    ? "bg-muted-foreground/20"
                    : contextRatio >= 0.8
                      ? "bg-(--ds-warn)/70"
                      : "bg-primary/65",
                )}
                style={{ width: contextRatio !== null ? `${Math.max(3, Math.round(contextRatio * 100))}%` : "0%" }}
              />
              <span className="absolute inset-0 opacity-0 transition-opacity group-hover:opacity-100" />
            </button>

            {detailsExpanded ? (
              <div className="border-t border-border/55 px-4 py-2.5 md:px-5">
              <div className="flex flex-col gap-2">
                <div className="flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <label className="roc-toolbar-field max-w-full">
                        <span className="roc-toolbar-label">{t("composer.mode")}</span>
                        <div className="flex min-w-0 flex-col">
                          <select
                            aria-label={t("composer.executionMode")}
                            className="roc-toolbar-select max-w-[8rem]"
                            value={modeValue}
                            onChange={(event) => onModeChange(event.target.value)}
                          >
                            <option value="">{t("composer.auto")}</option>
                            {modeOptions.map((mode) => (
                              <option key={mode.key} value={mode.key}>
                                {compactOptionLabel(mode.label)}
                              </option>
                            ))}
                          </select>
                          {modeValue ? (
                            <span className="truncate text-[10px] leading-4 text-muted-foreground">
                              {parseModeKind(modeValue) || "agent"}
                            </span>
                          ) : (
                            <span className="text-[10px] leading-4 text-muted-foreground">{t("composer.autoDetect")}</span>
                          )}
                        </div>
                      </label>

                      <div className="roc-toolbar-field min-w-[16rem] max-w-full">
                        <span className="roc-toolbar-label">{t("composer.model")}</span>
                        <Popover open={modelPickerOpen} onOpenChange={setModelPickerOpen}>
                          <PopoverTrigger asChild>
                            <button
                              type="button"
                              aria-label={t("composer.model")}
                              aria-expanded={modelPickerOpen}
                              className="flex min-w-0 flex-1 items-center gap-2 border-0 bg-transparent px-0 py-0 text-left shadow-none outline-none"
                            >
                              <div className="min-w-0 flex flex-1 items-center gap-2">
                                <div className="min-w-0 flex flex-1 flex-col overflow-hidden">
                                  <div className="truncate text-[12px] font-medium leading-4.5 text-foreground">
                                    {selectedModelTitle}
                                  </div>
                                  <div className="flex min-w-0 flex-wrap items-center gap-1">
                                    <div className="min-w-0 truncate text-[10px] leading-4 text-muted-foreground">
                                      {selectedModelSubtitle}
                                    </div>
                                    {selectedModelBadges.length > 0 ? (
                                      <div className="flex shrink-0 items-center gap-1">
                                        {selectedModelBadges.slice(0, 2).map(renderLocalizedModelBadge)}
                                      </div>
                                    ) : null}
                                  </div>
                                </div>
                                <ChevronsUpDownIcon className="size-3.5 shrink-0 text-muted-foreground/70" />
                              </div>
                            </button>
                          </PopoverTrigger>
                          <PopoverContent
                            align="start"
                            sideOffset={10}
                            className="w-[min(24rem,calc(100vw-1rem))] max-h-[min(30rem,calc(100vh-1rem))] overflow-hidden p-0"
                          >
                            <Command
                              shouldFilter
                              className="max-h-[22rem] rounded-4xl bg-transparent"
                            >
                              <CommandInput placeholder={t("composer.filterModels")} />
                              <CommandList className="max-h-[17.5rem]">
                                <CommandEmpty>{t("composer.noMatchingModel")}</CommandEmpty>
                                <CommandGroup heading={t("composer.automatic")}>
                                  <CommandItem
                                    value={AUTO_MODEL_VALUE}
                                    keywords={["auto", "default", "automatic", "workspace", "session"]}
                                    className="items-start rounded-2xl px-3 py-2"
                                    onSelect={() => {
                                      onModelChange("");
                                      setModelPickerOpen(false);
                                    }}
                                  >
                                    <div className="flex min-w-0 flex-1 items-start gap-3">
                                      <div className="pt-0.5">
                                        <CheckIcon
                                          className={cn(
                                            "size-4 text-foreground transition-opacity",
                                            modelSearchValue === AUTO_MODEL_VALUE
                                              ? "opacity-100"
                                              : "opacity-0",
                                          )}
                                        />
                                      </div>
                                      <div className="flex min-w-0 flex-1 flex-col gap-1">
                                        <span className="text-sm font-medium text-foreground">
                                          {t("composer.auto")}
                                        </span>
                                        <span className="text-[11px] leading-[1.35] text-muted-foreground">
                                          {t("composer.autoModelDescription")}
                                        </span>
                                      </div>
                                    </div>
                                  </CommandItem>
                                </CommandGroup>
                                {recentProviderModels.length > 0 ? (
                                  <CommandGroup heading={t("composer.recent")}>
                                    {recentProviderModels.map(({ provider, model, key }) =>
                                      renderModelOption(provider, model, key),
                                    )}
                                  </CommandGroup>
                                ) : null}
                                {providers.map((provider) => (
                                  <CommandGroup key={provider.id} heading={provider.name}>
                                    {(provider.models ?? [])
                                      .filter(
                                        (model) =>
                                          model.available !== false &&
                                          !recentModelKeys.has(`${provider.id}/${model.id}`),
                                      )
                                      .map((model) => {
                                        const optionKey = `${provider.id}/${model.id}`;
                                        return renderModelOption(provider, model, optionKey);
                                      })}
                                  </CommandGroup>
                                ))}
                              </CommandList>
                            </Command>
                          </PopoverContent>
                        </Popover>
                      </div>

                      {voiceListening ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-8 w-8 rounded-full text-foreground"
                          title={t("composer.stopVoiceRecording")}
                          onClick={stopVoiceCapture}
                        >
                          <SquareIcon className="size-3.5 fill-current" />
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center gap-1.5 lg:pb-0.5">
                    {contextBadgeLabel ? (
                      <span className="roc-badge px-3 py-1.5 text-[11px]">{contextBadgeLabel}</span>
                    ) : null}
                  </div>
                </div>

                <div className="flex flex-col gap-2 pt-0.25">
                  <div className="flex min-w-0 flex-col gap-1.5">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[11px] leading-5">
                      {contextSummary ? (
                        <span className="font-medium text-foreground/88">
                          {contextSummary}
                          {contextPercent ? <span className="text-muted-foreground"> · {contextPercent}</span> : null}
                          {contextPressure ? <span className={contextPressureClass}> · {contextPressure}</span> : null}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">{t("composer.awaitingTelemetry")}</span>
                      )}
                      {turnUsageLabel ? (
                        <span className="text-muted-foreground">{turnUsageLabel}</span>
                      ) : null}
                      {cacheUsageLabel ? (
                        <span className="text-muted-foreground">{cacheUsageLabel}</span>
                      ) : null}
                      {contextCount > 0 ? (
                        <span className="roc-badge">
                          {t("composer.contextBadge", { refs: references.length, files: attachments.length })}
                        </span>
                      ) : null}
                    </div>
                    {hasDiagnostics ? (
                      <div className="flex items-center justify-between gap-2">
                        <button
                          type="button"
                          className="text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
                          aria-expanded={telemetryExpanded}
                          onClick={() => setTelemetryExpanded((value) => !value)}
                        >
                          {telemetryExpanded ? t("composer.hideDiagnostics") : t("composer.showDiagnostics")}
                        </button>
                      </div>
                    ) : null}
                    {hasDiagnostics && telemetryExpanded ? (
                      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[11px] leading-5">
                        {closureDiagnosticLabel ? (
                          <span
                            className="text-(--ds-warn)"
                            title={t("composer.closureDiagnosticTitle")}
                          >
                            {t("composer.closureDiagnostic", { label: closureDiagnosticLabel })}
                          </span>
                        ) : null}
                        {ingressDiagnosticLabel ? (
                          <span className="text-muted-foreground" title={t("composer.ingressDiagnosticTitle")}>
                            {t("composer.ingressDiagnostic", { label: ingressDiagnosticLabel })}
                          </span>
                        ) : null}
                        {providerDiagnosticLabel ? (
                          <span className="text-(--ds-warn)" title={t("composer.providerDiagnosticTitle")}>
                            {t("composer.providerDiagnostic", { label: providerDiagnosticLabel })}
                          </span>
                        ) : null}
                        {pricingLabel ? (
                          <span className="text-muted-foreground" title={t("composer.pricingTitle")}>
                            {pricingLabel}
                          </span>
                        ) : null}
                        {multimodalHints.map((hint, index) => (
                          <span
                            key={`${hint.tone}:${hint.text}:${index}`}
                            className={cn(
                              hint.tone === "warning"
                                ? "text-(--ds-warn)"
                                : "text-muted-foreground",
                            )}
                          >
                            {hint.text}
                          </span>
                        ))}
                        {activityHint ? (
                          <span className={cn("font-medium", voiceError ? "text-destructive" : "text-muted-foreground")}>
                            {activityHint}
                          </span>
                        ) : null}
                        {permissionStatusLabel ? (
                          <span
                            className={cn(
                              "font-medium",
                              permissionStatusTone === "destructive"
                                ? "text-destructive"
                                : permissionStatusTone === "warning"
                                  ? "text-(--ds-warn)"
                                  : "text-muted-foreground",
                            )}
                          >
                            {permissionStatusLabel}
                          </span>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>
            ) : null}
          </div>
        </form>
      </div>
    </div>
  );
}
