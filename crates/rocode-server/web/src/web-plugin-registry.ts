export type RendererComponent = (props: { filePath: string }) => HTMLElement | null;

class WebPluginRegistry {
  private renderers = new Map<string, RendererComponent>();

  registerRenderer(extensions: string[], component: RendererComponent) {
    for (const ext of extensions) {
      this.renderers.set(ext.toLowerCase(), component);
    }
  }

  getRenderer(ext: string): RendererComponent | undefined {
    return this.renderers.get(ext.toLowerCase());
  }

  hasRenderer(ext: string): boolean {
    return this.renderers.has(ext.toLowerCase());
  }
}

export const webPluginRegistry = new WebPluginRegistry();
