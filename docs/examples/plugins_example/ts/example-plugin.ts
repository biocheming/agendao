/**
 * Minimal TS plugin example for AgenDao plugin-host.
 *
 * Hook coverage:
 * - chat.headers: injects a request header
 * - tool.definition: rewrites a tool description
 */

export default async function ExamplePlugin() {
  return {
    async "chat.headers"(_input: any, output: any) {
      const next = output && typeof output === "object" ? { ...output } : {};
      const headers =
        next.headers && typeof next.headers === "object"
          ? { ...next.headers }
          : {};

      headers["x-agendao-plugin"] = "example-plugin";
      next.headers = headers;
      return next;
    },

    async "tool.definition"(input: any, output: any) {
      const next = output && typeof output === "object" ? { ...output } : {};
      const toolID = input?.toolID;
      if (toolID === "bash" && typeof next.description === "string") {
        next.description = `${next.description} (annotated by example-plugin)`;
      }
      return next;
    },
  };
}
