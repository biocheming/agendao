import { webPluginRegistry } from "./web-plugin-registry";

interface WebPluginEntry {
  name: string;
  entry: string;
}

function webPluginModuleUrl(plugin: WebPluginEntry): string {
  const pluginName = encodeURIComponent(plugin.name);
  const entryPath = plugin.entry
    .split("/")
    .filter(Boolean)
    .map((segment) => encodeURIComponent(segment))
    .join("/");
  return `/web-plugin/serve/${pluginName}/${entryPath}`;
}

export async function loadWebPlugins(
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>,
): Promise<void> {
  let plugins: WebPluginEntry[];
  try {
    plugins = await apiJson<WebPluginEntry[]>("/web-plugin");
  } catch {
    return;
  }

  for (const plugin of plugins) {
    try {
      const mod = await import(
        /* @vite-ignore */
        webPluginModuleUrl(plugin)
      );
      const registerFn = mod.default ?? mod.register;
      if (typeof registerFn === "function") {
        registerFn(webPluginRegistry);
      }
    } catch (e) {
      console.warn(`[web-plugin] Failed to load: ${plugin.name}`, e);
    }
  }
}
