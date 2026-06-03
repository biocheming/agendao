export interface MultimodalAttachmentInfo {
  filename: string;
  mime: string;
}

export interface MultimodalPolicyView {
  voice: {
    duration_seconds: number;
    attach_audio: boolean;
    mime: string;
    language?: string | null;
  };
  max_input_bytes?: number | null;
  max_attachments_per_prompt?: number | null;
  allow_audio_input: boolean;
  allow_image_input: boolean;
  allow_file_input: boolean;
}

export interface MultimodalCapabilityView {
  provider_id: string;
  model_id: string;
  attachment: boolean;
  tool_call: boolean;
  reasoning: boolean;
  temperature: boolean;
  input: {
    text: boolean;
    audio: boolean;
    image: boolean;
    video: boolean;
    pdf: boolean;
  };
  output: {
    text: boolean;
    audio: boolean;
    image: boolean;
    video: boolean;
    pdf: boolean;
  };
}

export interface MultimodalPreflightResult {
  warnings: string[];
  unsupported_parts: string[];
  recommended_downgrade?: string | null;
  hard_block: boolean;
}

export interface MultimodalPreflightResponse {
  policy: MultimodalPolicyView;
  resolved_model?: string | null;
  capability?: MultimodalCapabilityView | null;
  result: MultimodalPreflightResult;
  warnings: string[];
}

export interface MultimodalCapabilitiesResponse {
  resolved_model?: string | null;
  capability?: MultimodalCapabilityView | null;
  warnings: string[];
}

export interface SessionPartRecord {
  type: string;
  url?: string;
  filename?: string;
  mime?: string;
}

export interface MultimodalPreflightRequest {
  model?: string;
  parts: unknown[];
  session_parts?: SessionPartRecord[];
}

export interface PersistedMultimodalExplain {
  attachment_count: number;
  kinds: string[];
  badges: string[];
  compact_label?: string | null;
  resolved_model?: string | null;
  warnings: string[];
  unsupported_parts: string[];
  recommended_downgrade?: string | null;
  hard_block: boolean;
  transport_replaced_parts: string[];
  transport_warnings: string[];
  attachments: MultimodalAttachmentInfo[];
}

export interface SessionMultimodalInsight extends PersistedMultimodalExplain {
  user_message_id: string;
}

export interface MultimodalInfoField {
  label?: string;
  value?: string;
}

export interface MultimodalExplainBlock {
  id: string;
  kind: "multimodal_info";
  role?: string;
  title: string;
  text: string;
  fields: MultimodalInfoField[];
}

export interface MultimodalHistoryBlock {
  id: string;
  kind: "message" | "multimodal_info" | "status";
  role?: string;
  title?: string;
  text: string;
  fields?: MultimodalInfoField[];
  tone?: "warning";
}

export function multimodalDisplayLabel(multimodal?: PersistedMultimodalExplain | null) {
  const compactLabel = multimodal?.compact_label?.trim();
  if (compactLabel) return compactLabel;
  const attachmentCount = multimodal?.attachment_count ?? 0;
  if (attachmentCount <= 0) return null;
  return attachmentCount === 1 ? "[1 attachment]" : `[${attachmentCount} attachments]`;
}

export function multimodalCombinedWarnings(multimodal?: PersistedMultimodalExplain | null) {
  const merged = [
    ...(multimodal?.warnings ?? []),
    ...(multimodal?.transport_warnings ?? []),
  ];
  return merged.filter((warning, index) => {
    const normalized = warning.trim();
    if (!normalized) return false;
    return merged.findIndex((candidate) => candidate.trim() === normalized) === index;
  });
}

export function buildMultimodalExplainBlocks(message: {
  id: string;
  role?: string;
  multimodal?: PersistedMultimodalExplain | null;
}): MultimodalExplainBlock[] {
  const multimodal = message.multimodal;
  if (!multimodal || multimodal.attachment_count <= 0) {
    return [];
  }

  const fields: MultimodalInfoField[] = [
    { label: "Attachments", value: String(multimodal.attachment_count) },
  ];
  if (multimodal.kinds.length > 0) {
    fields.push({ label: "Kinds", value: multimodal.kinds.join(", ") });
  }
  if (multimodal.resolved_model?.trim()) {
    fields.push({ label: "Target Model", value: multimodal.resolved_model.trim() });
  }
  if (multimodal.transport_replaced_parts.length > 0) {
    fields.push({
      label: "Transport Replaced",
      value: multimodal.transport_replaced_parts.join(", "),
    });
  }
  const attachmentNames = multimodal.attachments
    .map((file) => file.filename)
    .filter(Boolean)
    .join(", ");
  if (attachmentNames) {
    fields.push({ label: "Files", value: attachmentNames });
  }

  return [
    {
      id: `${message.id}:multimodal-info`,
      kind: "multimodal_info",
      role: message.role,
      title: "Multimodal Input",
      text: multimodalDisplayLabel(multimodal) ?? "[attachment input]",
      fields,
    },
  ];
}

export function buildMultimodalHistoryBlocks(message: {
  id: string;
  role?: string;
  multimodal?: PersistedMultimodalExplain | null;
}) {
  const blocks: MultimodalHistoryBlock[] = [];
  const fallbackText = multimodalDisplayLabel(message.multimodal);
  if (fallbackText) {
    blocks.push({
      id: `${message.id}:message`,
      kind: "message",
      role: message.role,
      text: fallbackText,
    });
  }

  blocks.push(...buildMultimodalExplainBlocks(message));

  const warnings = multimodalCombinedWarnings(message.multimodal);
  if (warnings.length > 0) {
    blocks.push({
      id: `${message.id}:multimodal-preflight`,
      kind: "status",
      tone: "warning",
      text: warnings.join(" "),
    });
  }

  return blocks;
}
