// Generated from crates/agendao-tui-revue/src/theme.rs. Do not edit by hand.
// 五行语义色（木=wood/用户，火=fire/工具，金=输出，土=系统，水=回流）+ 状态色，
// 与 TUI ds/color.rs 的 Semantic 枚举同一映射，色值唯一真源是 Rust Palette。

export type AgendaoThemeId = "tokyo-night" | "tokyo-night-light" | "tianqing" | "qianli";
export type SemanticToken = "wood" | "fire" | "earth" | "metal" | "water" | "ok" | "warn" | "error" | "info" | "muted" | "accent";

export const THEME_SEMANTIC_TOKENS: Record<AgendaoThemeId, Record<SemanticToken, string>> = {
  "tokyo-night": {
    wood: "#3cb8a2",
    fire: "#f0a852",
    earth: "#a9b1d6",
    metal: "#c0caf5",
    water: "#bb9af7",
    ok: "#9ece6a",
    warn: "#e0af68",
    error: "#f7768e",
    info: "#7dcfff",
    muted: "#565f89",
    accent: "#7dcfff",
  },
  "tokyo-night-light": {
    wood: "#007197",
    fire: "#b15c00",
    earth: "#565a6e",
    metal: "#343b58",
    water: "#7847bd",
    ok: "#587539",
    warn: "#8f5e15",
    error: "#f52a65",
    info: "#007197",
    muted: "#9699a3",
    accent: "#007197",
  },
  "tianqing": {
    wood: "#5f9184",
    fire: "#b5503c",
    earth: "#4d535b",
    metal: "#2e3238",
    water: "#7d6b8f",
    ok: "#54805e",
    warn: "#a8862a",
    error: "#9e3d3a",
    info: "#6b9e93",
    muted: "#8b8f88",
    accent: "#6b9e93",
  },
  "qianli": {
    wood: "#4cae8a",
    fire: "#c9a05a",
    earth: "#a9b8ac",
    metal: "#d8dfd3",
    water: "#9a8ab8",
    ok: "#6aae7c",
    warn: "#d3ae4e",
    error: "#cf6b55",
    info: "#6f9fd0",
    muted: "#5d6f6a",
    accent: "#6f9fd0",
  },
};

// Web 全局主题 → 五行 token 取色映射：cobalt 取 TUI 默认暗色，亮色主题取天青。
export const WEB_THEME_TOKEN_SOURCE: Record<string, AgendaoThemeId> = {
  daylight: "tianqing",
  sunset: "tianqing",
  cobalt: "tokyo-night",
};
