import { webPluginRegistry } from "./web-plugin-registry";

interface WebPluginEntry {
  name: string;
  entry: string;
}

function workspaceQueryValue(workspacePath?: string | null): string {
  return workspacePath?.trim() ? workspacePath.trim() : "";
}

function webPluginModuleUrl(plugin: WebPluginEntry, workspacePath?: string | null): string {
  const pluginName = encodeURIComponent(plugin.name);
  const entryPath = plugin.entry
    .split("/")
    .filter(Boolean)
    .map((segment) => encodeURIComponent(segment))
    .join("/");
  const workspace = workspaceQueryValue(workspacePath);
  return workspace
    ? `/web-plugin/serve/${pluginName}/${entryPath}?workspace=${encodeURIComponent(workspace)}`
    : `/web-plugin/serve/${pluginName}/${entryPath}`;
}

function webPluginIndexUrl(workspacePath?: string | null): string {
  const workspace = workspaceQueryValue(workspacePath);
  return workspace ? `/web-plugin?workspace=${encodeURIComponent(workspace)}` : "/web-plugin";
}

export async function loadWebPlugins(
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>,
  options?: { workspacePath?: string | null },
): Promise<void> {
  webPluginRegistry.clear();
  let plugins: WebPluginEntry[];
  try {
    plugins = await apiJson<WebPluginEntry[]>(webPluginIndexUrl(options?.workspacePath));
  } catch {
    return;
  }

  for (const plugin of plugins) {
    try {
      const mod = await import(
        /* @vite-ignore */
        webPluginModuleUrl(plugin, options?.workspacePath)
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
