import mermaidScriptUrl from "mermaid/dist/mermaid.min.js?url";
import type { MermaidConfig } from "mermaid";

const DEFAULT_CONFIG = {
  startOnLoad: false,
  theme: "default",
  securityLevel: "strict",
  fontFamily: "monospace",
  suppressErrorRendering: true,
} as const satisfies MermaidConfig;

type MermaidRenderResult = {
  svg: string;
};

type MermaidBrowserRuntime = {
  initialize: (config: MermaidConfig) => void;
  render: (id: string, source: string) => Promise<MermaidRenderResult>;
};

type MermaidInstance = {
  initialize: (config: MermaidConfig) => void;
  render: (id: string, source: string) => Promise<MermaidRenderResult>;
};

type DiagramPlugin = {
  getMermaid: (config?: MermaidConfig) => MermaidInstance;
  language: string;
  name: "mermaid";
  type: "diagram";
};

type MermaidPluginOptions = {
  config?: MermaidConfig;
};

declare global {
  interface Window {
    mermaid?: MermaidBrowserRuntime;
  }
}

let mermaidRuntimePromise: Promise<MermaidBrowserRuntime> | null = null;

function getMermaidScriptElement(): HTMLScriptElement | null {
  return document.querySelector('script[data-rocode-mermaid-runtime="true"]');
}

function loadMermaidRuntime(): Promise<MermaidBrowserRuntime> {
  if (typeof window === "undefined") {
    return Promise.reject(new Error("Mermaid runtime is only available in the browser."));
  }

  if (window.mermaid) {
    return Promise.resolve(window.mermaid);
  }

  if (mermaidRuntimePromise) {
    return mermaidRuntimePromise;
  }

  mermaidRuntimePromise = new Promise<MermaidBrowserRuntime>((resolve, reject) => {
    const existingScript = getMermaidScriptElement();
    const handleReady = () => {
      if (!window.mermaid) {
        reject(new Error("Mermaid runtime loaded without exposing window.mermaid."));
        return;
      }
      resolve(window.mermaid);
    };

    if (existingScript) {
      existingScript.addEventListener("load", handleReady, { once: true });
      existingScript.addEventListener(
        "error",
        () => reject(new Error("Failed to load Mermaid runtime script.")),
        { once: true },
      );
      return;
    }

    const script = document.createElement("script");
    script.async = true;
    script.dataset.rocodeMermaidRuntime = "true";
    script.src = mermaidScriptUrl;
    script.addEventListener("load", handleReady, { once: true });
    script.addEventListener(
      "error",
      () => reject(new Error("Failed to load Mermaid runtime script.")),
      { once: true },
    );
    document.head.appendChild(script);
  }).catch((error) => {
    mermaidRuntimePromise = null;
    throw error;
  });

  return mermaidRuntimePromise;
}

export function createRocodeMermaidPlugin(options: MermaidPluginOptions = {}): DiagramPlugin {
  let initialized = false;
  let config: MermaidConfig = { ...DEFAULT_CONFIG, ...options.config };

  const runtime = {
    initialize(nextConfig: MermaidConfig) {
      config = { ...DEFAULT_CONFIG, ...options.config, ...nextConfig };
      initialized = false;
    },
    async render(id: string, source: string) {
      const mermaid = await loadMermaidRuntime();
      if (!initialized) {
        mermaid.initialize(config);
        initialized = true;
      }
      return mermaid.render(id, source);
    },
  } satisfies MermaidInstance;

  return {
    name: "mermaid",
    type: "diagram",
    language: "mermaid",
    getMermaid(nextConfig) {
      if (nextConfig) {
        runtime.initialize(nextConfig);
      }
      return runtime;
    },
  };
}

export const rocodeMermaidPlugin = createRocodeMermaidPlugin();
