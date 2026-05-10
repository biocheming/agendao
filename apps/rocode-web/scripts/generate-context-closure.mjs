import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "../../..");
const rustSourcePath = resolve(repoRoot, "crates/rocode-types/src/session.rs");
const outputPath = resolve(repoRoot, "apps/rocode-web/src/generated/contextClosure.generated.ts");

function readRustStringLiteral(source, pattern, description) {
  const match = source.match(pattern);
  if (!match) throw new Error(`Missing Rust ${description}`);
  return match[1];
}

function rustStringLiteralToJson(value) {
  if (!value.startsWith('"') || !value.endsWith('"')) {
    throw new Error(`Expected Rust string literal, got ${value}`);
  }
  return JSON.stringify(JSON.parse(value));
}

function generatedContent(source) {
  const governanceReady = rustStringLiteralToJson(
    readRustStringLiteral(
      source,
      /Self::Ready => ("[^"]+")/,
      "context pressure governance ready label",
    ),
  );
  const governanceCompacted = rustStringLiteralToJson(
    readRustStringLiteral(
      source,
      /Self::Compacted => ("[^"]+")/,
      "context pressure governance compacted label",
    ),
  );
  const governanceDeferred = rustStringLiteralToJson(
    readRustStringLiteral(
      source,
      /Self::Deferred => ("[^"]+")/,
      "context pressure governance deferred label",
    ),
  );
  const governanceBlocked = rustStringLiteralToJson(
    readRustStringLiteral(
      source,
      /Self::Blocked => ("[^"]+")/,
      "context pressure governance blocked label",
    ),
  );

  const severityStable = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::Stable => ("[^"]+")/, "cache severity stable label"),
  );
  const severityLow = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::LowChange => ("[^"]+")/, "cache severity low label"),
  );
  const severityMedium = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::MediumChange => ("[^"]+")/, "cache severity medium label"),
  );
  const severityHigh = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::HighChange => ("[^"]+")/, "cache severity high label"),
  );

  const sourceNone = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::None => ("[^"]+")/, "cache source none label"),
  );
  const sourceCache = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::CacheEvidence => ("[^"]+")/, "cache source cache label"),
  );
  const sourceSurface = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::SurfaceEvidence => ("[^"]+")/, "cache source surface label"),
  );
  const sourceBoundary = rustStringLiteralToJson(
    readRustStringLiteral(source, /Self::BoundaryEvidence => ("[^"]+")/, "cache source boundary label"),
  );

  const prefixChanged = rustStringLiteralToJson(
    readRustStringLiteral(source, /("prefix changed")/, "prefix changed label"),
  );
  const stablePrefix = rustStringLiteralToJson(
    readRustStringLiteral(source, /("stable prefix")/, "stable prefix label"),
  );
  const boundaryRecorded = rustStringLiteralToJson(
    readRustStringLiteral(source, /("boundary recorded")/, "boundary recorded label"),
  );
  const boundaryClear = rustStringLiteralToJson(
    readRustStringLiteral(source, /("boundary clear")/, "boundary clear label"),
  );
  const cacheStable = rustStringLiteralToJson(
    readRustStringLiteral(source, /("cache stable")/, "cache stable label"),
  );
  const cacheExplained = rustStringLiteralToJson(
    readRustStringLiteral(source, /("cache explained")/, "cache explained label"),
  );
  const cacheUnexplained = rustStringLiteralToJson(
    readRustStringLiteral(source, /("cache unexplained")/, "cache unexplained label"),
  );
  const leakDetected = rustStringLiteralToJson(
    readRustStringLiteral(source, /("leak detected")/, "leak detected label"),
  );
  const isolated = rustStringLiteralToJson(
    readRustStringLiteral(source, /("isolated")/, "isolated label"),
  );
  const notOwnerLocal = rustStringLiteralToJson(
    readRustStringLiteral(source, /("not owner-local")/, "not owner-local label"),
  );

  const continuityPacket = rustStringLiteralToJson('"packet installed"');
  const continuityFallback = rustStringLiteralToJson('"legacy summary fallback"');

  return `// Generated from crates/rocode-types/src/session.rs. Do not edit by hand.\n` +
    `\n` +
    `export const CONTEXT_CLOSURE_GOVERNANCE_LABELS = {\n` +
    `  ready: ${governanceReady},\n` +
    `  compacted: ${governanceCompacted},\n` +
    `  deferred: ${governanceDeferred},\n` +
    `  blocked: ${governanceBlocked},\n` +
    `} as const;\n` +
    `\n` +
    `export const CONTEXT_CLOSURE_SEVERITY_LABELS = {\n` +
    `  stable: ${severityStable},\n` +
    `  low_change: ${severityLow},\n` +
    `  medium_change: ${severityMedium},\n` +
    `  high_change: ${severityHigh},\n` +
    `} as const;\n` +
    `\n` +
    `export const CONTEXT_CLOSURE_SOURCE_LABELS = {\n` +
    `  none: ${sourceNone},\n` +
    `  cache_evidence: ${sourceCache},\n` +
    `  surface_evidence: ${sourceSurface},\n` +
    `  boundary_evidence: ${sourceBoundary},\n` +
    `} as const;\n` +
    `\n` +
    `export const CONTEXT_CLOSURE_STATUS_LABELS = {\n` +
    `  prefix_changed: ${prefixChanged},\n` +
    `  stable_prefix: ${stablePrefix},\n` +
    `  boundary_recorded: ${boundaryRecorded},\n` +
    `  boundary_clear: ${boundaryClear},\n` +
    `  cache_stable: ${cacheStable},\n` +
    `  cache_explained: ${cacheExplained},\n` +
    `  cache_unexplained: ${cacheUnexplained},\n` +
    `  leak_detected: ${leakDetected},\n` +
    `  isolated: ${isolated},\n` +
    `  not_owner_local: ${notOwnerLocal},\n` +
    `  continuity_packet: ${continuityPacket},\n` +
    `  raw_summary_fallback: ${continuityFallback},\n` +
    `} as const;\n` +
    `\n` +
    `export function contextClosureGovernanceStatusLabel(value?: string | null) {\n` +
    `  if (!value) return "--";\n` +
    `  return CONTEXT_CLOSURE_GOVERNANCE_LABELS[value as keyof typeof CONTEXT_CLOSURE_GOVERNANCE_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");\n` +
    `}\n` +
    `\n` +
    `export function contextClosureSeverityLabel(value?: string | null) {\n` +
    `  if (!value) return "--";\n` +
    `  return CONTEXT_CLOSURE_SEVERITY_LABELS[value as keyof typeof CONTEXT_CLOSURE_SEVERITY_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");\n` +
    `}\n` +
    `\n` +
    `export function contextClosureExplainabilitySourceLabel(value?: string | null) {\n` +
    `  if (!value) return "--";\n` +
    `  return CONTEXT_CLOSURE_SOURCE_LABELS[value as keyof typeof CONTEXT_CLOSURE_SOURCE_LABELS] ?? (value.replace(/[._-]+/g, " ").trim() || "--");\n` +
    `}\n` +
    `\n` +
    `export function contextClosurePrefixStatusLabel(prefix: { prefix_change_detected: boolean }) {\n` +
    `  return prefix.prefix_change_detected ? CONTEXT_CLOSURE_STATUS_LABELS.prefix_changed : CONTEXT_CLOSURE_STATUS_LABELS.stable_prefix;\n` +
    `}\n` +
    `\n` +
    `export function contextClosureBoundaryStatusLabel(boundary: { boundary_recorded: boolean }) {\n` +
    `  return boundary.boundary_recorded ? CONTEXT_CLOSURE_STATUS_LABELS.boundary_recorded : CONTEXT_CLOSURE_STATUS_LABELS.boundary_clear;\n` +
    `}\n` +
    `\n` +
    `export function contextClosureCacheStatusLabel(cache: { issue_present: boolean; explained: boolean }) {\n` +
    `  if (!cache.issue_present) return CONTEXT_CLOSURE_STATUS_LABELS.cache_stable;\n` +
    `  return cache.explained ? CONTEXT_CLOSURE_STATUS_LABELS.cache_explained : CONTEXT_CLOSURE_STATUS_LABELS.cache_unexplained;\n` +
    `}\n` +
    `\n` +
    `export function contextClosureIsolationStatusLabel(isolation: { child_history_in_live_prefix_detected: boolean; owner_local_live_prefix: boolean }) {\n` +
    `  if (isolation.child_history_in_live_prefix_detected) return CONTEXT_CLOSURE_STATUS_LABELS.leak_detected;\n` +
    `  return isolation.owner_local_live_prefix ? CONTEXT_CLOSURE_STATUS_LABELS.isolated : CONTEXT_CLOSURE_STATUS_LABELS.not_owner_local;\n` +
    `}\n` +
    `\n` +
    `export function compactionContinuitySourceLabel(continuity: { source: string }) {\n` +
    `  if (continuity.source === "continuity_packet") return CONTEXT_CLOSURE_STATUS_LABELS.continuity_packet;\n` +
    `  if (continuity.source === "raw_summary_fallback") return CONTEXT_CLOSURE_STATUS_LABELS.raw_summary_fallback;\n` +
    `  return continuity.source.replace(/[._-]+/g, " ").trim() || "--";\n` +
    `}\n` +
    `\n` +
    `export function contextClosureCoarseDiagnosticLabel(contract: {\n` +
    `  prefix_stability: { prefix_change_detected: boolean };\n` +
    `  compaction_boundary: { boundary_recorded: boolean };\n` +
    `  cache_explainability: { issue_present: boolean; explained: boolean };\n` +
    `} | null | undefined) {\n` +
    `  if (!contract) return null;\n` +
    `  const parts: string[] = [];\n` +
    `  if (contract.cache_explainability.issue_present) {\n` +
    `    parts.push(contextClosureCacheStatusLabel(contract.cache_explainability));\n` +
    `  }\n` +
    `  if (contract.prefix_stability.prefix_change_detected) {\n` +
    `    parts.push(contextClosurePrefixStatusLabel(contract.prefix_stability));\n` +
    `  } else if (contract.compaction_boundary.boundary_recorded) {\n` +
    `    parts.push(contextClosureBoundaryStatusLabel(contract.compaction_boundary));\n` +
    `  }\n` +
    `  return parts.length === 0 ? null : Array.from(new Set(parts)).join(" · ");\n` +
    `}\n`;
}

const check = process.argv.includes("--check");
const source = readFileSync(rustSourcePath, "utf8");
const next = generatedContent(source);

if (check) {
  const current = readFileSync(outputPath, "utf8");
  if (current !== next) {
    console.error("Generated context closure constants are out of date. Run `npm run generate:context-closure`.");
    process.exit(1);
  }
  process.exit(0);
}

writeFileSync(outputPath, next);
