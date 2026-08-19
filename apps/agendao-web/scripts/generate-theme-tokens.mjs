import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "../../..");
const rustSourcePath = resolve(repoRoot, "crates/agendao-tui-revue/src/theme.rs");
const tsOutputPath = resolve(repoRoot, "apps/agendao-web/src/generated/themeTokens.generated.ts");
const cssOutputPath = resolve(repoRoot, "apps/agendao-web/src/generated/themeTokens.generated.css");

// ── 解析 theme.rs：逐 Palette 构造器提取 `field: Color::rgb(r, g, b)` ──

const THEMES = ["tokyo_night", "tokyo_night_light", "tianqing", "qianli"];
const THEME_IDS = {
  tokyo_night: "tokyo-night",
  tokyo_night_light: "tokyo-night-light",
  tianqing: "tianqing",
  qianli: "qianli",
};
// 与 TUI `ds/color.rs` 同一五行+状态语义映射（单一契约，两处不得各自漂移）。
const SEMANTIC_FIELDS = {
  wood: "e_teal",
  fire: "e_amber",
  earth: "fg_secondary",
  metal: "fg_primary",
  water: "accent_purple",
  ok: "accent_green",
  warn: "accent_yellow",
  error: "accent_red",
  info: "accent_cyan",
  muted: "fg_muted",
  accent: "accent_cyan",
};
const BASE_FIELDS = ["bg_primary", "fg_primary", "fg_muted", "border"];

function parsePalettes(source) {
  const out = {};
  for (const name of THEMES) {
    const fnPattern = new RegExp(`pub const fn ${name}\\(\\) -> Self \\{([\\s\\S]*?)\\n    \\}`, "m");
    const fnMatch = source.match(fnPattern);
    if (!fnMatch) throw new Error(`Missing Palette constructor: ${name}`);
    const body = fnMatch[1];
    const fields = {};
    const colorPattern = /(\w+):\s*Color::rgb\((\d+),\s*(\d+),\s*(\d+)\)/g;
    let m;
    while ((m = colorPattern.exec(body)) !== null) {
      const [, field, r, g, b] = m;
      fields[field] = `#${[r, g, b].map((v) => Number(v).toString(16).padStart(2, "0")).join("")}`;
    }
    for (const required of [...Object.values(SEMANTIC_FIELDS), ...BASE_FIELDS]) {
      if (!fields[required]) throw new Error(`Palette ${name} missing field ${required}`);
    }
    out[THEME_IDS[name]] = fields;
  }
  return out;
}

function semanticOf(fields) {
  const sem = {};
  for (const [semantic, field] of Object.entries(SEMANTIC_FIELDS)) {
    sem[semantic] = fields[field];
  }
  return sem;
}

// ── TS 产物：四套主题完整语义 token（程序化消费：terminal / 图表 / shiki 等）──

function tsContent(palettes) {
  const themeIds = Object.values(THEME_IDS);
  const semanticKeys = Object.keys(SEMANTIC_FIELDS);
  const lines = [];
  lines.push("// Generated from crates/agendao-tui-revue/src/theme.rs. Do not edit by hand.");
  lines.push("// 五行语义色（木=wood/用户，火=fire/工具，金=输出，土=系统，水=回流）+ 状态色，");
  lines.push("// 与 TUI ds/color.rs 的 Semantic 枚举同一映射，色值唯一真源是 Rust Palette。");
  lines.push("");
  lines.push(`export type AgendaoThemeId = ${themeIds.map((t) => `"${t}"`).join(" | ")};`);
  lines.push(`export type SemanticToken = ${semanticKeys.map((k) => `"${k}"`).join(" | ")};`);
  lines.push("");
  lines.push("export const THEME_SEMANTIC_TOKENS: Record<AgendaoThemeId, Record<SemanticToken, string>> = {");
  for (const id of themeIds) {
    const sem = semanticOf(palettes[id]);
    lines.push(`  "${id}": {`);
    for (const [k, v] of Object.entries(sem)) lines.push(`    ${k}: "${v}",`);
    lines.push("  },");
  }
  lines.push("};");
  lines.push("");
  lines.push("// Web 全局主题 → 五行 token 取色映射：cobalt 取 TUI 默认暗色，亮色主题取天青。");
  lines.push("export const WEB_THEME_TOKEN_SOURCE: Record<string, AgendaoThemeId> = {");
  lines.push('  daylight: "tianqing",');
  lines.push('  sunset: "tianqing",');
  lines.push('  cobalt: "tokyo-night",');
  lines.push("};");
  lines.push("");
  return lines.join("\n");
}

// ── CSS 产物：--ds-* 变量（样式消费：各组件语义色统一挂点）──

function cssContent(palettes) {
  const lines = [];
  lines.push("/* Generated from crates/agendao-tui-revue/src/theme.rs. Do not edit by hand. */");
  lines.push("/* 五行语义色变量：与 TUI 同源，web 组件语义色统一挂点。 */");
  const light = semanticOf(palettes["tianqing"]);
  const dark = semanticOf(palettes["tokyo-night"]);
  lines.push(":root,");
  lines.push('[data-theme="daylight"],');
  lines.push('[data-theme="sunset"] {');
  for (const [k, v] of Object.entries(light)) lines.push(`  --ds-${k}: ${v};`);
  lines.push("}");
  lines.push("");
  lines.push('[data-theme="cobalt"] {');
  for (const [k, v] of Object.entries(dark)) lines.push(`  --ds-${k}: ${v};`);
  lines.push("}");
  lines.push("");
  return lines.join("\n");
}

// ── 生成 / --check 校验 ──

const check = process.argv.includes("--check");
const source = readFileSync(rustSourcePath, "utf8");
const palettes = parsePalettes(source);
const outputs = [
  [tsOutputPath, tsContent(palettes)],
  [cssOutputPath, cssContent(palettes)],
];

let stale = false;
for (const [path, next] of outputs) {
  let current = null;
  try {
    current = readFileSync(path, "utf8");
  } catch {
    current = null;
  }
  if (check) {
    if (current !== next) {
      console.error(`Stale generated artifact: ${path}`);
      stale = true;
    }
  } else {
    writeFileSync(path, next);
    console.log(`Generated ${path}`);
  }
}
if (stale) process.exit(1);
