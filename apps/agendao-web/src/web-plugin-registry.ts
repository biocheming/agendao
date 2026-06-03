export type RendererComponent = (props: { filePath: string }) => HTMLElement | null;

function normalizeExtension(ext: string): string {
  return ext.trim().toLowerCase().replace(/^\.+/, "");
}

class WebPluginRegistry {
  private renderers = new Map<string, RendererComponent>();

  registerRenderer(extensions: string[], component: RendererComponent) {
    for (const ext of extensions) {
      const normalized = normalizeExtension(ext);
      if (!normalized) continue;
      this.renderers.set(normalized, component);
    }
  }

  getRenderer(ext: string): RendererComponent | undefined {
    return this.renderers.get(normalizeExtension(ext));
  }

  hasRenderer(ext: string): boolean {
    return this.renderers.has(normalizeExtension(ext));
  }

  clear() {
    this.renderers.clear();
  }
}

export const webPluginRegistry = new WebPluginRegistry();
