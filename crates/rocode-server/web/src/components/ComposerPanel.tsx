"use client";

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { FormEvent, ClipboardEvent, DragEvent } from "react";
import type { BreadcrumbProvenance } from "../hooks/useSchedulerNavigation";
import {
  browserSpeechRecognitionConstructor,
  type BrowserSpeechRecognition,
} from "../lib/browserSpeech";
import { AttachmentDetailsPanel } from "./AttachmentDetailsPanel";
import { ComposerContextStrip } from "./ComposerContextStrip";
import type { ComposerAttachmentRecord } from "../lib/composerContext";
import type {
  ProviderModelCapabilitiesRecord,
  ProviderModelRecord,
  ProviderRecord,
} from "../lib/provider";
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
  return formatCompactTokenCount(value).replace(".0k", "k").replace(".0M", "M");
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
      className="inline-flex items-center gap-1 rounded-full border border-border/45 bg-background/72 px-2 py-0.75 text-[10px] font-medium text-muted-foreground"
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
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(Math.round(value));
}

function formatCompactMoney(value?: number | null) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "$0";
  return `$${value.toFixed(2)}`;
}

function formatCompactPrice(value?: number | null) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return value.toFixed(2);
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
  selectedModel: string;
  onModelChange: (value: string) => void;
  references: string[];
  attachments: ComposerAttachmentRecord[];
  selectedAttachmentIndex: number | null;
  selectedAttachment: ComposerAttachmentRecord | null;
  selectedWorkspacePath: string | null;
  workspaceRootPath: string;
  contextTokensUsed?: number | null;
  contextTokensLimit?: number | null;
  sessionCost?: number | null;
  inputPricePerMillion?: number | null;
  outputPricePerMillion?: number | null;
  activeStageId: string | null;
  provenance: BreadcrumbProvenance | null;
  onPreviewStage?: (stageId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
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
  selectedModel,
  onModelChange,
  references,
  attachments,
  selectedAttachmentIndex,
  selectedAttachment,
  selectedWorkspacePath,
  workspaceRootPath,
  contextTokensUsed = null,
  contextTokensLimit = null,
  sessionCost = null,
  inputPricePerMillion = null,
  outputPricePerMillion = null,
  activeStageId,
  provenance,
  onPreviewStage,
  onSubmit,
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
  onFileChange,
  onPaste,
  onComposerChange,
}: ComposerPanelProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const imageInputRef = useRef<HTMLInputElement>(null);
  const recognitionRef = useRef<BrowserSpeechRecognition | null>(null);
  const voiceBaseTextRef = useRef("");
  const [voiceSupported, setVoiceSupported] = useState(false);
  const [voiceListening, setVoiceListening] = useState(false);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);

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
    const RecognitionCtor = browserSpeechRecognitionConstructor(window);
    setVoiceSupported(Boolean(RecognitionCtor));

    return () => {
      recognitionRef.current?.stop();
      recognitionRef.current = null;
    };
  }, []);

  const stopVoiceRecognition = () => {
    recognitionRef.current?.stop();
    recognitionRef.current = null;
    setVoiceListening(false);
  };

  const startVoiceRecognition = () => {
    if (typeof window === "undefined") return;
    const RecognitionCtor = browserSpeechRecognitionConstructor(window);
    if (!RecognitionCtor) {
      setVoiceSupported(false);
      setVoiceError("This browser does not support speech recognition.");
      return;
    }

    setVoiceError(null);
    voiceBaseTextRef.current = composer.trimEnd();

    const recognition = new RecognitionCtor();
    recognition.continuous = false;
    recognition.interimResults = true;
    recognition.lang =
      typeof navigator !== "undefined" && navigator.language
        ? navigator.language
        : "en-US";
    recognition.onresult = (event) => {
      let finalTranscript = "";
      let interimTranscript = "";

      for (let index = event.resultIndex; index < event.results.length; index += 1) {
        const result = event.results[index];
        const transcript = result[0]?.transcript ?? result.item(0)?.transcript ?? "";
        if (!transcript) continue;
        if (result.isFinal) {
          finalTranscript += transcript;
        } else {
          interimTranscript += transcript;
        }
      }

      const spokenText = [finalTranscript, interimTranscript]
        .map((value) => value.trim())
        .filter(Boolean)
        .join(" ")
        .trim();

      const base = voiceBaseTextRef.current;
      if (!spokenText) {
        onComposerChange(base);
        return;
      }

      onComposerChange(base ? `${base}\n${spokenText}` : spokenText);
    };
    recognition.onerror = (event) => {
      if (event.error === "no-speech") {
        setVoiceError("No speech detected.");
      } else if (event.error === "not-allowed") {
        setVoiceError("Microphone permission was denied.");
      } else {
        setVoiceError(`Voice input failed: ${event.error}`);
      }
      setVoiceListening(false);
      recognitionRef.current = null;
    };
    recognition.onend = () => {
      setVoiceListening(false);
      recognitionRef.current = null;
    };

    recognitionRef.current = recognition;
    setVoiceListening(true);
    recognition.start();
  };

  const contextCount = references.length + attachments.length;
  const modeValue = selectedMode || "";
  const modelValue = selectedModel || "";
  const selectedProviderModel = findProviderModel(providers, modelValue);
  const selectedModelBadges = capabilityBadges(selectedProviderModel?.model.capabilities);
  const activityHint = voiceError
    ? voiceError
    : composerDragActive
      ? "Drop files or images to attach them to this turn."
      : streaming
        ? "ROCode is responding. You can stop the current turn."
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
  const contextPercent =
    contextRatio === null ? null : `${Math.max(1, Math.round(contextRatio * 100))}%`;
  const pricingLabel = (() => {
    const input = formatCompactPrice(inputPricePerMillion);
    const output = formatCompactPrice(outputPricePerMillion);
    if (!input || !output) return null;
    return `$${input} in · $${output} out / 1M`;
  })();
  const contextSummary = hasContextEstimate
    ? hasContextLimit
      ? `${formatCompactTokenCount(contextTokensUsed)} / ${formatCompactTokenCount(contextTokensLimit)}`
      : formatCompactTokenCount(contextTokensUsed)
    : null;
  const selectedModelTitle = selectedProviderModel
    ? selectedProviderModel.model.name?.trim() || selectedProviderModel.model.id
    : modelValue.trim()
      ? compactOptionLabel(modelValue)
      : "Auto";
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
      ? "Selected explicitly"
      : "Use session or workspace default";
  const modelSearchValue = modelValue || AUTO_MODEL_VALUE;

  return (
    <div className="flex flex-col gap-2.5" data-testid="composer-form">
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

      <div className="roc-composer-shell" data-drag-active={composerDragActive ? "true" : "false"}>
        <form className="w-full" onSubmit={onSubmit} data-testid="composer-dropzone">
          <div
            className="flex flex-col"
            onDragEnter={onDragEnter}
            onDragOver={onDragOver}
            onDragLeave={onDragLeave}
            onDrop={onDrop}
          >
            <div className="px-4 pt-4 pb-3 md:px-5">
              <textarea
                ref={textareaRef}
                name="message"
                rows={1}
                placeholder="Ask ROCode"
                value={composer}
                onChange={(e) => onComposerChange(e.target.value)}
                onPaste={onPaste}
                disabled={streaming}
                className="h-auto min-h-0 w-full resize-none border-0 bg-transparent py-0.5 text-[15px] leading-[1.65] text-foreground outline-none placeholder:text-muted-foreground/50"
              />
            </div>

            <div className="border-t border-border/55 px-4 py-2.5 md:px-5">
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
              <div className="flex flex-col gap-2">
                <div className="flex flex-col gap-2.5 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <label className="roc-toolbar-field max-w-full">
                        <span className="roc-toolbar-label">Mode</span>
                        <select
                          aria-label="Execution mode"
                          className="roc-toolbar-select max-w-[8rem]"
                          value={modeValue}
                          onChange={(event) => onModeChange(event.target.value)}
                        >
                          <option value="">Auto</option>
                          {modeOptions.map((mode) => (
                            <option key={mode.key} value={mode.key}>
                              {compactOptionLabel(mode.label)}
                            </option>
                          ))}
                        </select>
                      </label>

                      <div className="roc-toolbar-field min-w-[16rem] max-w-full">
                        <span className="roc-toolbar-label">Model</span>
                        <Popover open={modelPickerOpen} onOpenChange={setModelPickerOpen}>
                          <PopoverTrigger asChild>
                            <button
                              type="button"
                              aria-label="Model"
                              aria-expanded={modelPickerOpen}
                              className="flex min-w-0 flex-1 items-center gap-2 border-0 bg-transparent px-0 py-0 text-left shadow-none outline-none"
                            >
                              <div className="min-w-0 flex flex-1 items-center gap-2">
                                <div className="min-w-0 flex flex-1 flex-col overflow-hidden">
                                  <div className="truncate text-[12px] font-medium leading-4.5 text-foreground">
                                    {selectedModelTitle}
                                  </div>
                                  <div className="flex min-w-0 items-center gap-1.5">
                                    <div className="truncate text-[10px] leading-4 text-muted-foreground">
                                      {selectedModelSubtitle}
                                    </div>
                                    {selectedModelBadges.length > 0 ? (
                                      <div className="flex shrink-0 items-center gap-1">
                                        {selectedModelBadges.slice(0, 2).map(renderModelBadge)}
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
                            className="w-[26rem] max-w-[calc(100vw-2rem)] overflow-hidden p-0"
                          >
                            <Command
                              shouldFilter
                              className="max-h-[24rem] rounded-[24px] bg-transparent"
                            >
                              <CommandInput placeholder="Filter models, providers, capabilities..." />
                              <CommandList className="max-h-[19rem]">
                                <CommandEmpty>No matching model.</CommandEmpty>
                                <CommandGroup heading="Automatic">
                                  <CommandItem
                                    value={AUTO_MODEL_VALUE}
                                    keywords={["auto", "default", "automatic", "workspace", "session"]}
                                    className="items-start rounded-xl px-3 py-2.5"
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
                                          Auto
                                        </span>
                                        <span className="text-[11px] leading-5 text-muted-foreground">
                                          Use the session or workspace default model.
                                        </span>
                                      </div>
                                    </div>
                                  </CommandItem>
                                </CommandGroup>
                                {providers.map((provider) => (
                                  <CommandGroup key={provider.id} heading={provider.name}>
                                    {(provider.models ?? []).map((model) => {
                                      const optionKey = `${provider.id}/${model.id}`;
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
                                          className="items-start rounded-xl px-3 py-2.5"
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
                                                  modelSearchValue === optionKey
                                                    ? "opacity-100"
                                                    : "opacity-0",
                                                )}
                                              />
                                            </div>
                                            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
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
                                              <div className="flex flex-wrap items-center gap-1.5">
                                                {model.context_window ? (
                                                  <span className="rounded-full border border-border/45 bg-background/72 px-2 py-0.75 text-[10px] font-medium text-muted-foreground">
                                                    {formatCompactCapacity(model.context_window)} ctx
                                                  </span>
                                                ) : null}
                                                {badges.map(renderModelBadge)}
                                              </div>
                                            </div>
                                          </div>
                                        </CommandItem>
                                      );
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
                          title="Stop voice input"
                          onClick={stopVoiceRecognition}
                        >
                          <SquareIcon className="size-3.5 fill-current" />
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center gap-1.5 lg:pb-0.5">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="h-8.5 rounded-full border-border/45 px-3 text-[11px] text-muted-foreground"
                          title="Add attachment"
                        >
                          <PlusIcon className="mr-1.5 size-3.5" />
                          Add
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="min-w-[8rem]">
                        <DropdownMenuItem
                          disabled={!allowAudioInput || !voiceSupported}
                          onClick={startVoiceRecognition}
                          className="gap-2 text-xs"
                        >
                          <MicIcon className="size-3.5" />
                          Voice
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          disabled={!allowFileInput}
                          onClick={() => fileInputRef.current?.click()}
                          className="gap-2 text-xs"
                        >
                          <PaperclipIcon className="size-3.5" />
                          File
                          {attachments.length > 0 ? (
                            <span className="ml-auto rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-foreground">
                              {attachments.length}
                            </span>
                          ) : null}
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          disabled={!allowImageInput}
                          onClick={() => imageInputRef.current?.click()}
                          className="gap-2 text-xs"
                        >
                          <ImageIcon className="size-3.5" />
                          Image
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>

                    {streaming ? (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-8.5 gap-1.5 rounded-full border-destructive/50 px-3 text-[11px] text-destructive hover:bg-destructive/10"
                        onClick={() => {
                          window.dispatchEvent(new CustomEvent("rocode:stop-streaming"));
                        }}
                      >
                        <span className="size-2 rounded-sm bg-current" />
                        Stop
                      </Button>
                    ) : null}

                    <Button
                      type="submit"
                      variant="default"
                      size="sm"
                      disabled={!composer.trim() && attachments.length === 0}
                      className="h-9.5 rounded-full px-4 shadow-sm"
                    >
                      <span className="mr-1 text-[11px] font-medium">Send</span>
                      <SendIcon className="size-3.25" />
                    </Button>
                  </div>
                </div>

                <div className="flex flex-col gap-2 pt-0.25 lg:flex-row lg:items-end lg:justify-between">
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[11px] leading-5">
                      {contextSummary ? (
                        <span className="font-medium text-foreground/88">
                          {contextSummary}
                          {contextPercent ? <span className="text-muted-foreground"> · {contextPercent}</span> : null}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">Awaiting telemetry</span>
                      )}
                      {contextCount > 0 ? (
                        <span className="roc-chip-subtle">
                          {references.length} refs · {attachments.length} files
                        </span>
                      ) : null}
                      {sessionCost != null && sessionCost > 0 ? (
                        <span className="text-muted-foreground">
                          ${sessionCost.toFixed(2)}
                        </span>
                      ) : null}
                      {pricingLabel ? (
                        <span className="text-muted-foreground" title="Model pricing per million tokens">
                          {pricingLabel}
                        </span>
                      ) : null}
                      {multimodalHints.map((hint, index) => (
                        <span
                          key={`${hint.tone}:${hint.text}:${index}`}
                          className={cn(
                            hint.tone === "warning"
                              ? "text-amber-700 dark:text-amber-300"
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
                    </div>
                    <div className="mt-1.5 h-2 overflow-hidden rounded-full bg-muted/60">
                      <div
                        className={cn(
                          "h-full rounded-full transition-[width,background-color] duration-200 ease-out",
                          contextRatio === null
                            ? "bg-muted-foreground/35"
                            : contextRatio >= 0.8
                              ? "bg-amber-500/85"
                              : "bg-primary/75",
                        )}
                        style={{ width: contextRatio !== null ? `${Math.max(6, Math.round(contextRatio * 100))}%` : "0%" }}
                      />
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}
