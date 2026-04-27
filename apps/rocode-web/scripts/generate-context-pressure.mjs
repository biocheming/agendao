import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "../../..");
const rustSourcePath = resolve(repoRoot, "crates/rocode-types/src/context_pressure.rs");
const outputPath = resolve(repoRoot, "apps/rocode-web/src/generated/contextPressure.generated.ts");

function readRustConst(source, name) {
  const pattern = new RegExp(`pub const ${name}: [^=]+ = ([^;]+);`);
  const match = source.match(pattern);
  if (!match) throw new Error(`Missing Rust context pressure const: ${name}`);
  return match[1].trim();
}

function rustStringLiteralToJson(value) {
  if (!value.startsWith('"') || !value.endsWith('"')) {
    throw new Error(`Expected Rust string literal, got ${value}`);
  }
  return JSON.stringify(JSON.parse(value));
}

function generatedContent(source) {
  const warning = readRustConst(source, "CONTEXT_PRESSURE_WARNING_PERCENT");
  const autoCompactSoon = readRustConst(source, "CONTEXT_PRESSURE_AUTO_COMPACT_SOON_PERCENT");
  const critical = readRustConst(source, "CONTEXT_PRESSURE_CRITICAL_PERCENT");
  const warningLabel = rustStringLiteralToJson(readRustConst(source, "CONTEXT_PRESSURE_WARNING_LABEL"));
  const autoCompactSoonLabel = rustStringLiteralToJson(readRustConst(source, "CONTEXT_PRESSURE_AUTO_COMPACT_SOON_LABEL"));
  const criticalLabel = rustStringLiteralToJson(readRustConst(source, "CONTEXT_PRESSURE_CRITICAL_LABEL"));

  return `// Generated from crates/rocode-types/src/context_pressure.rs. Do not edit by hand.\n` +
    `\n` +
    `export const CONTEXT_PRESSURE_THRESHOLDS = {\n` +
    `  warning: ${warning},\n` +
    `  autoCompactSoon: ${autoCompactSoon},\n` +
    `  critical: ${critical},\n` +
    `} as const;\n` +
    `\n` +
    `export const CONTEXT_PRESSURE_LABELS = {\n` +
    `  warning: ${warningLabel},\n` +
    `  autoCompactSoon: ${autoCompactSoonLabel},\n` +
    `  critical: ${criticalLabel},\n` +
    `} as const;\n`;
}

const check = process.argv.includes("--check");
const source = readFileSync(rustSourcePath, "utf8");
const next = generatedContent(source);

if (check) {
  const current = readFileSync(outputPath, "utf8");
  if (current !== next) {
    console.error("Generated context pressure constants are out of date. Run `npm run generate:context-pressure`.");
    process.exit(1);
  }
  process.exit(0);
}

writeFileSync(outputPath, next);
