import type {
  CodeHighlighterPlugin,
  HighlightOptions,
  HighlightResult,
  ThemeInput,
} from "@streamdown/code";
import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import githubDark from "@shikijs/themes/github-dark";
import githubLight from "@shikijs/themes/github-light";

const DEFAULT_THEMES = [githubLight, githubDark] as const satisfies [ThemeInput, ThemeInput];

type LanguageModule = {
  default: unknown;
};

const LANGUAGE_LOADERS = {
  bash: () => import("@shikijs/langs/bash"),
  diff: () => import("@shikijs/langs/diff"),
  dockerfile: () => import("@shikijs/langs/dockerfile"),
  go: () => import("@shikijs/langs/go"),
  html: () => import("@shikijs/langs/html"),
  ini: () => import("@shikijs/langs/ini"),
  javascript: () => import("@shikijs/langs/javascript"),
  json: () => import("@shikijs/langs/json"),
  jsx: () => import("@shikijs/langs/jsx"),
  markdown: () => import("@shikijs/langs/markdown"),
  python: () => import("@shikijs/langs/python"),
  rust: () => import("@shikijs/langs/rust"),
  scss: () => import("@shikijs/langs/scss"),
  sql: () => import("@shikijs/langs/sql"),
  toml: () => import("@shikijs/langs/toml"),
  tsx: () => import("@shikijs/langs/tsx"),
  typescript: () => import("@shikijs/langs/typescript"),
  xml: () => import("@shikijs/langs/xml"),
  yaml: () => import("@shikijs/langs/yaml"),
  css: () => import("@shikijs/langs/css"),
} satisfies Record<string, () => Promise<LanguageModule>>;

type SupportedLanguage = keyof typeof LANGUAGE_LOADERS;

const LANGUAGE_ALIASES: Record<string, SupportedLanguage | "text"> = {
  bash: "bash",
  console: "bash",
  css: "css",
  diff: "diff",
  docker: "dockerfile",
  dockerfile: "dockerfile",
  go: "go",
  golang: "go",
  htm: "html",
  html: "html",
  ini: "ini",
  javascript: "javascript",
  js: "javascript",
  json: "json",
  json5: "json",
  jsonc: "json",
  jsonl: "json",
  jsx: "jsx",
  markdown: "markdown",
  md: "markdown",
  patch: "diff",
  plaintext: "text",
  plain: "text",
  py: "python",
  python: "python",
  rs: "rust",
  rust: "rust",
  scss: "scss",
  sh: "bash",
  shell: "bash",
  shellscript: "bash",
  sql: "sql",
  text: "text",
  toml: "toml",
  ts: "typescript",
  tsx: "tsx",
  txt: "text",
  typescript: "typescript",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zsh: "bash",
};

const SUPPORTED_LANGUAGES = Object.keys(LANGUAGE_LOADERS) as SupportedLanguage[];

const highlighterPromise = createHighlighterCore({
  engine: createJavaScriptRegexEngine({ forgiving: true }),
  langs: [],
  themes: [...DEFAULT_THEMES],
});

const loadedLanguages = new Set<SupportedLanguage>();
const highlightCache = new Map<string, HighlightResult>();
const subscribers = new Map<string, Set<(result: HighlightResult) => void>>();

function normalizeLanguage(language: string): SupportedLanguage | "text" {
  const normalized = language.trim().toLowerCase();
  return LANGUAGE_ALIASES[normalized] ?? "text";
}

function themeName(theme: ThemeInput | undefined, fallback: string): string {
  if (!theme) {
    return fallback;
  }
  return typeof theme === "string" ? theme : theme.name ?? fallback;
}

function cacheKey(
  code: string,
  language: string,
  themes: readonly [ThemeInput, ThemeInput],
): string {
  const start = code.slice(0, 100);
  const end = code.length > 100 ? code.slice(-100) : "";
  return [
    language,
    themeName(themes[0], "github-light"),
    themeName(themes[1], "github-dark"),
    code.length,
    start,
    end,
  ].join(":");
}

async function ensureLanguage(language: SupportedLanguage | "text"): Promise<SupportedLanguage | "text"> {
  if (language === "text" || loadedLanguages.has(language)) {
    return language;
  }

  const module = await LANGUAGE_LOADERS[language]();
  const highlighter = await highlighterPromise;
  await highlighter.loadLanguage(module.default);
  loadedLanguages.add(language);
  return language;
}

function notifySubscribers(key: string, result: HighlightResult) {
  const pending = subscribers.get(key);
  if (!pending) {
    return;
  }
  for (const callback of pending) {
    callback(result);
  }
  subscribers.delete(key);
}

function resolvedThemeNames(themes: readonly [ThemeInput, ThemeInput]): [string, string] {
  return [
    themeName(themes[0], "github-light"),
    themeName(themes[1], "github-dark"),
  ];
}

export function createAgendaoCodePlugin(
  themes: [ThemeInput, ThemeInput] = [...DEFAULT_THEMES],
): CodeHighlighterPlugin {
  return {
    name: "shiki",
    type: "code-highlighter",
    getSupportedLanguages: () => [...SUPPORTED_LANGUAGES] as HighlightOptions["language"][],
    getThemes: () => themes,
    supportsLanguage: (language) => normalizeLanguage(language) !== "text",
    highlight: (options, callback) => {
      const canonicalLanguage = normalizeLanguage(options.language);
      const activeThemes = options.themes ?? themes;
      const key = cacheKey(options.code, canonicalLanguage, activeThemes);

      const cached = highlightCache.get(key);
      if (cached) {
        return cached;
      }

      if (callback) {
        if (!subscribers.has(key)) {
          subscribers.set(key, new Set());
        }
        subscribers.get(key)?.add(callback);
      }

      void ensureLanguage(canonicalLanguage)
        .then(async (resolvedLanguage) => {
          const highlighter = await highlighterPromise;
          const [lightTheme, darkTheme] = resolvedThemeNames(activeThemes);
          const result = highlighter.codeToTokens(options.code, {
            lang: resolvedLanguage,
            themes: {
              dark: darkTheme,
              light: lightTheme,
            },
          });
          highlightCache.set(key, result);
          notifySubscribers(key, result);
        })
        .catch((error) => {
          console.error("[AgenDao Web] Failed to highlight code:", error);
          subscribers.delete(key);
        });

      return null;
    },
  };
}

export const agendaoCodePlugin = createAgendaoCodePlugin();
