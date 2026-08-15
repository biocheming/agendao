import {
  type AuxiliaryOutputBlock,
  type DisplayContractOutputBlock,
  isMessageOutputBlock,
  isMultimodalInfoOutputBlock,
  isReasoningOutputBlock,
  isStatusOutputBlock,
  isToolOutputBlock,
  type OutputBlock,
  type OutputField,
} from "./history";

// P2-3: Centralized text-priority contract. Each block kind declares an
// explicit ordered list of text sources. Components must not re-derive this
// logic inline.
//
// Policy:
//   message/reasoning → text-first (raw text > display supplement)
//   tool              → display-first (display.* > direct detail/text)
//   status            → status-only (text/title/summary, no display-contract guess)
//   default           → display-first (safest for unknown future kinds)

// -- helpers ----------------------------------------------------------------

function joinedFieldText(fields?: OutputField[]): string | null {
  if (!fields?.length) return null;
  const text = fields
    .map((field) => `${field.label ?? "Field"}: ${String(field.value ?? "")}`)
    .join("\n")
    .trim();
  return text || null;
}

function nullable(block: OutputBlock, accessor: (b: OutputBlock) => string | null | undefined): string | null {
  const v = accessor(block);
  return typeof v === "string" && v.trim() ? v : null;
}

function nullableDisplay(
  block: DisplayContractOutputBlock,
  accessor: (b: DisplayContractOutputBlock) => string | null | undefined,
): string | null {
  const v = accessor(block);
  return typeof v === "string" && v.trim() ? v : null;
}

// -- text sources -----------------------------------------------------------

function rawText(block: OutputBlock) {
  return nullable(block, (b) => b.text);
}
function displaySummary(block: DisplayContractOutputBlock) {
  return nullableDisplay(block, (b) => b.display?.summary);
}
function displayFields(block: DisplayContractOutputBlock) {
  return joinedFieldText(block.display?.fields);
}
function displayPreview(block: DisplayContractOutputBlock) {
  return nullableDisplay(block, (b) => b.display?.preview?.text);
}
function blockSummary(block: OutputBlock) {
  return nullable(block, (b) => b.summary);
}
function directFields(block: OutputBlock) {
  return joinedFieldText(block.fields);
}
function blockBody(block: DisplayContractOutputBlock) {
  return nullableDisplay(block, (b) => b.body);
}
function blockDetail(block: DisplayContractOutputBlock) {
  return nullableDisplay(block, (b) => b.detail);
}
function blockPreview(block: DisplayContractOutputBlock) {
  return nullableDisplay(block, (b) => b.preview);
}
function blockTitle(block: OutputBlock) {
  return nullable(block, (b) => b.title);
}
function resolveChain(chain: Array<(b: OutputBlock) => string | null>, block: OutputBlock): string {
  for (const fn of chain) {
    const v = fn(block);
    if (v) return v;
  }
  return "";
}

function resolveDisplayChain(block: DisplayContractOutputBlock): string {
  const displayFirstChain: Array<(b: DisplayContractOutputBlock) => string | null> = [
    displaySummary,
    blockSummary,
    displayFields,
    directFields,
    displayPreview,
    blockBody,
    blockDetail,
    rawText,
    blockPreview,
  ];
  for (const fn of displayFirstChain) {
    const v = fn(block);
    if (v) return v;
  }
  return "";
}

function resolveTextFirstChain(block: OutputBlock): string {
  return resolveChain([rawText, blockSummary, directFields, blockTitle], block);
}

function isAuxiliaryOutputBlock(block: OutputBlock): block is AuxiliaryOutputBlock {
  return block.kind === "session_event" || block.kind === "queue_item" || block.kind === "inspect";
}

// -- policy -----------------------------------------------------------------

export function primaryDisplayText(block: OutputBlock): string {
  if (isMessageOutputBlock(block) || isReasoningOutputBlock(block)) {
    return resolveTextFirstChain(block);
  }
  if (isToolOutputBlock(block)) {
    return resolveDisplayChain(block);
  }
  if (isStatusOutputBlock(block)) {
    return resolveChain([rawText, blockTitle, blockSummary], block);
  }
  if (isMultimodalInfoOutputBlock(block)) {
    return resolveChain([blockSummary, rawText, directFields, blockTitle], block);
  }
  if (isAuxiliaryOutputBlock(block)) {
    return resolveDisplayChain(block);
  }
  return resolveTextFirstChain(block);
}
