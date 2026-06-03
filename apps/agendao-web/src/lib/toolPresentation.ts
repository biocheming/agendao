import { hasDisplayContract, type OutputField, type OutputPreview, type ToolOutputBlock } from "./history";
import { isSkillToolName } from "./toolLabels";

export function toolDisplayTitle(block: ToolOutputBlock): string {
  const header = block.display?.header?.trim();
  if (header) {
    if (!/^[A-Za-z0-9._:-]+$/.test(header)) {
      return header;
    }
    return header;
  }
  return block.title?.trim() ?? block.name?.trim() ?? block.kind;
}

export function toolDisplayRawLabelKey(block: ToolOutputBlock): string {
  return block.display?.header?.trim()
    ?? block.title?.trim()
    ?? block.name?.trim()
    ?? block.kind;
}

export function toolExecutionKind(block: ToolOutputBlock): "tool" | "skill" {
  return isSkillToolName(block.name ?? block.title ?? "") ? "skill" : "tool";
}

export function toolDisplaySummary(block: ToolOutputBlock): string | null {
  const displaySummary = block.display?.summary?.trim();
  if (displaySummary) return displaySummary;

  const legacySummary = block.summary?.trim();
  if (legacySummary) return legacySummary;

  // Compatibility fallback: older payloads lack display.summary
  if (!hasDisplayContract(block)) {
    return block.detail?.trim() ?? block.text?.trim() ?? null;
  }

  return null;
}

export function toolDisplayFields(block: ToolOutputBlock): OutputField[] | undefined {
  if (block.display?.fields?.length) {
    return block.display.fields;
  }
  if (block.fields?.length) {
    return block.fields;
  }
  return undefined;
}

export function toolDisplayPreview(block: ToolOutputBlock): {
  previewText: string | null;
  previewKind: string | null;
  previewTruncated: boolean;
} {
  const displayPreview = block.display?.preview;
  if (displayPreview?.text?.trim()) {
    return {
      previewText: displayPreview.text.trim(),
      previewKind: displayPreview.kind?.trim() ?? null,
      previewTruncated: Boolean(displayPreview.truncated),
    };
  }

  if (!hasDisplayContract(block) && block.preview?.trim()) {
    return {
      previewText: block.preview.trim(),
      previewKind: "text",
      previewTruncated: false,
    };
  }

  return { previewText: null, previewKind: null, previewTruncated: false };
}

export function toolCompatDetail(block: ToolOutputBlock): string | null {
  if (!hasDisplayContract(block) && !block.fields?.length) {
    return block.detail?.trim() ?? null;
  }
  return null;
}
