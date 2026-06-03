/**
 * plugin-host.ts — JSON-RPC host for a single TypeScript plugin.
 *
 * Embedded into the Rust binary via include_str!() and written to
 * ~/.cache/opencode/plugin-host.ts at runtime.
 *
 * Protocol: Content-Length framed JSON-RPC 2.0 over stdin/stdout.
 * stderr is reserved for plugin log output.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

interface PluginContext {
  worktree: string;
  directory: string;
  serverUrl: string;
  internalToken?: string;
}

const LOCAL_NO_PROXY_ENTRIES = ["127.0.0.1", "localhost", "::1", "0.0.0.0"];
const LOCAL_SDK_REQUEST_TIMEOUT_MS = 8000;
const INTERNAL_PLUGIN_REQUEST_HEADER = "x-agendao-plugin-internal";
const INTERNAL_PLUGIN_ID_HEADER = "x-agendao-plugin-id";
const INTERNAL_TOKEN_HEADER = "x-agendao-internal-token";

type UnknownRecord = Record<string, unknown>;

interface ToolDefinition {
  description: string;
  args: unknown; // Zod schema or similar
  execute: (args: unknown, ctx: unknown) => Promise<unknown>;
}

interface ToolContext {
  sessionID: string;
  messageID: string;
  agent: string;
  directory: string;
  worktree: string;
  abort: AbortSignal;
  ask: () => Promise<never>;
  metadata: (input: unknown) => void;
}

interface Hooks {
  [key: string]: ((input: unknown, output: unknown) => Promise<unknown>) | unknown;
  tool?: Record<string, ToolDefinition>;
}

interface AuthMethod {
  type: string;
  label: string;
  inputs?: Record<string, { placeholder?: string; required?: boolean }>;
}

interface AuthorizeResult {
  url?: string;
  instructions?: string;
  method?: string;
  callback?: (code?: string) => Promise<unknown>;
}

interface AuthHook {
  provider: string;
  methods: AuthMethod[];
  authorize?: (method: AuthMethod, inputs?: Record<string, string>) => Promise<AuthorizeResult>;
  loader?: () => Promise<{
    apiKey?: string;
    fetch?: typeof globalThis.fetch;
    [key: string]: unknown;
  }>;
}

// ---------------------------------------------------------------------------
// Content-Length framing
// ---------------------------------------------------------------------------

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function encodeResponse(response: JsonRpcResponse): Uint8Array {
  const body = JSON.stringify(response);
  const header = `Content-Length: ${Buffer.byteLength(body)}\r\n\r\n`;
  return encoder.encode(header + body);
}

/**
 * Read exactly `n` bytes from stdin.
 */
async function readExact(n: number): Promise<Uint8Array> {
  const chunks: Uint8Array[] = [];
  let remaining = n;

  const reader = process.stdin as unknown as {
    read(size: number): Uint8Array | null;
    once(event: string, cb: () => void): void;
  };

  while (remaining > 0) {
    const chunk: Uint8Array | null = reader.read(remaining);
    if (chunk !== null) {
      chunks.push(chunk);
      remaining -= chunk.length;
    } else {
      await new Promise<void>((resolve) => reader.once("readable", resolve));
    }
  }

  if (chunks.length === 1) return chunks[0];
  const result = new Uint8Array(n);
  let offset = 0;
  for (const c of chunks) {
    result.set(c, offset);
    offset += c.length;
  }
  return result;
}

/**
 * Read one Content-Length framed JSON-RPC message from stdin.
 * Returns null on EOF.
 */
async function readMessage(): Promise<JsonRpcRequest | null> {
  // Read header lines until empty line
  let header = "";
  while (true) {
    const byte = await readExact(1);
    if (byte.length === 0) return null;
    header += decoder.decode(byte);
    if (header.endsWith("\r\n\r\n")) break;
  }

  const match = header.match(/Content-Length:\s*(\d+)/i);
  if (!match) {
    throw new Error(`Invalid header: ${header}`);
  }

  const contentLength = parseInt(match[1], 10);
  const body = await readExact(contentLength);
  return JSON.parse(decoder.decode(body));
}

function send(response: JsonRpcResponse): void {
  process.stdout.write(encodeResponse(response));
}

function sendNotification(method: string, params?: unknown): void {
  const body = JSON.stringify({
    jsonrpc: "2.0",
    method,
    params,
  } satisfies JsonRpcNotification);
  const header = `Content-Length: ${Buffer.byteLength(body)}\r\n\r\n`;
  process.stdout.write(encoder.encode(header + body));
}

function sendResult(id: number, result: unknown): void {
  send({ jsonrpc: "2.0", id, result });
}

function sendError(id: number, code: number, message: string): void {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

// ---------------------------------------------------------------------------
// Plugin state
// ---------------------------------------------------------------------------

let pluginHooks: Hooks = {};
let authHook: AuthHook | null = null;
let pendingAuthCallback: ((code?: string) => Promise<unknown>) | null = null;
let customFetch: typeof globalThis.fetch | null = null;
const activeToolInvocations = new Map<number, AbortController>();

// ---------------------------------------------------------------------------
// Plugin input compatibility helpers
// ---------------------------------------------------------------------------

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

function templateToShellCommand(parts: TemplateStringsArray, values: unknown[]): string {
  let command = parts[0] ?? "";
  for (let i = 0; i < values.length; i++) {
    const value = values[i];
    const serialized =
      typeof value === "string"
        ? value
        : typeof value === "number" || typeof value === "boolean"
          ? String(value)
          : JSON.stringify(value);
    command += shellQuote(serialized) + (parts[i + 1] ?? "");
  }
  return command;
}

async function runShellCommand(command: string): Promise<{
  stdout: string;
  stderr: string;
  exitCode: number;
}> {
  const childProcess = await import("node:child_process");
  return await new Promise((resolve, reject) => {
    const child = childProcess.spawn("bash", ["-lc", command], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk: Buffer | string) => {
      stdout += String(chunk);
    });
    child.stderr.on("data", (chunk: Buffer | string) => {
      stderr += String(chunk);
    });
    child.on("error", (err: unknown) => {
      reject(err);
    });
    child.on("close", (code: number | null) => {
      const exitCode = code ?? 1;
      const result = { stdout, stderr, exitCode };
      if (exitCode === 0) {
        resolve(result);
        return;
      }
      reject(new Error(stderr || `Command failed with exit code ${exitCode}`));
    });
  });
}

function createShell(): (parts: TemplateStringsArray, ...values: unknown[]) => Promise<unknown> {
  const maybeBun = (globalThis as UnknownRecord)["Bun"];
  if (maybeBun && typeof maybeBun === "object") {
    const dollar = (maybeBun as UnknownRecord)["$"];
    if (typeof dollar === "function") {
      return dollar as (parts: TemplateStringsArray, ...values: unknown[]) => Promise<unknown>;
    }
  }

  return async (parts: TemplateStringsArray, ...values: unknown[]): Promise<unknown> => {
    const command = templateToShellCommand(parts, values);
    return await runShellCommand(command);
  };
}

function createNoopClientProxy(path: string[] = []): unknown {
  const fn = async () => ({});
  return new Proxy(fn, {
    get(_target, prop: string | symbol) {
      if (typeof prop === "symbol") {
        if (prop === Symbol.toStringTag) return "PluginNoopClient";
        return undefined;
      }
      if (prop === "then") return undefined;
      if (prop === "toString") {
        return () => `[PluginNoopClient ${path.join(".")}]`;
      }
      return createNoopClientProxy([...path, prop]);
    },
    apply() {
      return Promise.resolve({});
    },
  });
}

function normalizeServerUrl(raw: string): string {
  try {
    const parsed = new URL(raw);
    if (parsed.hostname === "0.0.0.0") {
      parsed.hostname = "127.0.0.1";
    } else if (parsed.hostname === "::" || parsed.hostname === "[::]") {
      parsed.hostname = "localhost";
    }
    return parsed.toString().replace(/\/$/, "");
  } catch {
    return raw;
  }
}

function ensureLocalNoProxy(): void {
  const existing = process.env.NO_PROXY ?? process.env.no_proxy ?? "";
  const merged = new Set(
    existing
      .split(",")
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0),
  );
  for (const host of LOCAL_NO_PROXY_ENTRIES) {
    merged.add(host);
  }
  const value = Array.from(merged).join(",");
  process.env.NO_PROXY = value;
  process.env.no_proxy = value;
}

function createSdkFetch(pluginPath: string, internalToken?: string) {
  return async (request: Request): Promise<Response> => {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), LOCAL_SDK_REQUEST_TIMEOUT_MS);
    try {
      const headers = new Headers(request.headers);
      headers.set(INTERNAL_PLUGIN_REQUEST_HEADER, "1");
      headers.set(INTERNAL_PLUGIN_ID_HEADER, pluginPath);
      const token = internalToken?.trim();
      if (token) {
        headers.set(INTERNAL_TOKEN_HEADER, token);
      }
      const reqWithSignal = new Request(request, {
        signal: controller.signal,
        headers,
      });
      return await fetch(reqWithSignal);
    } finally {
      clearTimeout(timeout);
    }
  };
}

async function createPluginClient(
  context: PluginContext,
  pluginPath: string,
): Promise<unknown> {
  ensureLocalNoProxy();
  const normalizedServerUrl = normalizeServerUrl(context.serverUrl);
  const sdkFetch = createSdkFetch(pluginPath, context.internalToken);
  const candidateUrls = new Set<string>();
  const addCandidate = (url: string) => {
    candidateUrls.add(url);
  };

  try {
    const sdk = (await import("@opencode-ai/sdk")) as UnknownRecord;
    const createOpencodeClient = sdk["createOpencodeClient"];
    if (typeof createOpencodeClient === "function") {
      return (createOpencodeClient as (config: UnknownRecord) => unknown)({
        baseUrl: normalizedServerUrl,
        directory: context.directory,
        fetch: sdkFetch,
      });
    }
  } catch {
    // Fallback below.
  }

  try {
    const pathMod = await import("node:path");
    const urlMod = await import("node:url");

    const addCandidatePath = (path: string) => {
      addCandidate(urlMod.pathToFileURL(path).href);
    };

    let pluginFsPath: string | null = null;
    if (pluginPath.startsWith("file://")) {
      pluginFsPath = urlMod.fileURLToPath(pluginPath);
    } else if (pluginPath.startsWith("/")) {
      pluginFsPath = pluginPath;
    }

    addCandidatePath(
      pathMod.join(
        process.cwd(),
        "node_modules",
        "@opencode-ai",
        "sdk",
        "dist",
        "index.js",
      ),
    );

    if (pluginFsPath) {
      let cursor = pathMod.dirname(pluginFsPath);
      while (true) {
        addCandidatePath(
          pathMod.join(
            cursor,
            "node_modules",
            "@opencode-ai",
            "sdk",
            "dist",
            "index.js",
          ),
        );
        const parent = pathMod.dirname(cursor);
        if (parent === cursor) break;
        cursor = parent;
      }
    }

    try {
      const moduleMod = await import("node:module");
      const addFromRequire = (basePath: string) => {
        try {
          const req = moduleMod.createRequire(basePath);
          const resolved = req.resolve("@opencode-ai/sdk");
          addCandidate(urlMod.pathToFileURL(resolved).href);
        } catch {
          // Try next base path.
        }
      };
      addFromRequire(pathMod.join(process.cwd(), "package.json"));
      if (pluginFsPath) {
        addFromRequire(pluginFsPath);
        addFromRequire(pathMod.join(pathMod.dirname(pluginFsPath), "package.json"));
      }
    } catch {
      // createRequire path resolution is optional.
    }
  } catch {
    // If node:module/node:url is unavailable, keep noop fallback.
  }

  for (const url of candidateUrls) {
    try {
      const sdk = (await import(url)) as UnknownRecord;
      const createOpencodeClient = sdk["createOpencodeClient"];
      if (typeof createOpencodeClient === "function") {
        return (createOpencodeClient as (config: UnknownRecord) => unknown)({
          baseUrl: normalizedServerUrl,
          directory: context.directory,
          fetch: sdkFetch,
        });
      }
    } catch {
      // Try next candidate.
    }
  }

  return createNoopClientProxy(["client"]);
}

function buildPluginInput(context: PluginContext, client: unknown): UnknownRecord {
  let serverUrl: string | URL = context.serverUrl;
  try {
    serverUrl = new URL(context.serverUrl);
  } catch {
    // Keep as string if URL parsing fails.
  }

  return {
    // Legacy Rust host shape (already used by some plugins)
    context,
    // TS plugin ecosystem shape
    client,
    directory: context.directory,
    worktree: context.worktree,
    serverUrl,
    project: {
      directory: context.directory,
      worktree: context.worktree,
    },
    $: createShell(),
  };
}

// ---------------------------------------------------------------------------
// Minimal Zod-to-JSON-Schema converter (no external deps)
// ---------------------------------------------------------------------------

function normalizeZodTypeTag(tag: unknown): string | undefined {
  if (typeof tag !== "string" || tag.length === 0) {
    return undefined;
  }
  if (tag.startsWith("Zod")) {
    return tag.slice(3).toLowerCase();
  }
  return tag.toLowerCase();
}

function zodTypeTagFromDef(def: UnknownRecord | undefined): string | undefined {
  if (!def) {
    return undefined;
  }
  // Zod v3 uses _def.typeName ("ZodString"), Zod v4 uses _def.type ("string").
  return normalizeZodTypeTag(def["typeName"]) ?? normalizeZodTypeTag(def["type"]);
}

function isOptionalLikeZodField(value: unknown): boolean {
  if (!value || typeof value !== "object") {
    return false;
  }
  const fieldDef = (value as UnknownRecord)["_def"] as UnknownRecord | undefined;
  const fieldTypeTag = zodTypeTagFromDef(fieldDef);
  return fieldTypeTag === "optional" || fieldTypeTag === "nullable" || fieldTypeTag === "default";
}

function zodToJsonSchema(schema: unknown): Record<string, unknown> {
  if (schema == null || typeof schema !== "object") {
    return { type: "object", properties: {} };
  }

  const s = schema as UnknownRecord;

  // Check for _def (Zod internals)
  const def = s["_def"] as UnknownRecord | undefined;
  if (!def) {
    // Not a Zod schema — return as-is if it looks like JSON Schema already
    if (s["type"] || s["properties"]) {
      return s as Record<string, unknown>;
    }
    // Plain object whose values may be Zod schemas (e.g. tool args: { name: z.string(), ... }).
    // Detect by checking if any value has _def.
    const entries = Object.entries(s);
    if (entries.length > 0 && entries.some(([, v]) => v && typeof v === "object" && (v as UnknownRecord)["_def"])) {
      const properties: Record<string, unknown> = {};
      const required: string[] = [];
      for (const [key, value] of entries) {
        properties[key] = zodToJsonSchema(value);
        if (!isOptionalLikeZodField(value)) {
          required.push(key);
        }
      }
      const result: Record<string, unknown> = { type: "object", properties };
      if (required.length > 0) {
        result.required = required;
      }
      return result;
    }
    return { type: "object", properties: {} };
  }

  const typeTag = zodTypeTagFromDef(def);
  const description = def["description"] as string | undefined;

  let result: Record<string, unknown>;
  switch (typeTag) {
    case "string":
      result = { type: "string" };
      break;
    case "number":
      result = { type: "number" };
      break;
    case "boolean":
      result = { type: "boolean" };
      break;
    case "array": {
      // Zod v4 uses "element"; some shapes may still expose "type".
      const itemType = def["element"] ?? def["type"];
      result = { type: "array", items: itemType ? zodToJsonSchema(itemType) : {} };
      break;
    }
    case "enum": {
      // Zod v3: _def.values = string[]
      // Zod v4: _def.entries = { A: "A", B: "B" }, schema.options = string[]
      const values = def["values"];
      if (Array.isArray(values) && values.length > 0) {
        result = { type: "string", enum: values };
      } else if (Array.isArray((s as UnknownRecord)["options"])) {
        result = { type: "string", enum: (s as UnknownRecord)["options"] as unknown[] };
      } else if (def["entries"] && typeof def["entries"] === "object") {
        const enumValues = Object.values(def["entries"] as UnknownRecord);
        result = { type: "string", enum: enumValues };
      } else {
        result = { type: "string" };
      }
      break;
    }
    case "optional":
    case "nullable": {
      const inner = def["innerType"];
      result = inner ? zodToJsonSchema(inner) : { type: "object", properties: {} };
      break;
    }
    case "default": {
      const inner = def["innerType"];
      result = inner ? zodToJsonSchema(inner) : { type: "object", properties: {} };
      break;
    }
    case "object": {
      // Zod stores shape in different ways across versions:
      // - s.shape may be a plain object (getter) or undefined
      // - def.shape may be a function (lazy getter) or a plain object
      let shape: UnknownRecord | undefined;
      const sShape = (s as UnknownRecord)["shape"];
      if (sShape && typeof sShape === "object" && !Array.isArray(sShape)) {
        shape = sShape as UnknownRecord;
      } else {
        const defShape = def["shape"];
        if (typeof defShape === "function") {
          try {
            shape = defShape() as UnknownRecord;
          } catch {
            shape = undefined;
          }
        } else if (defShape && typeof defShape === "object" && !Array.isArray(defShape)) {
          shape = defShape as UnknownRecord;
        }
      }
      if (!shape) {
        result = { type: "object", properties: {} };
        break;
      }
      const properties: Record<string, unknown> = {};
      const required: string[] = [];
      for (const [key, value] of Object.entries(shape)) {
        properties[key] = zodToJsonSchema(value);
        // Check if field is required (not optional/nullable/default)
        if (!isOptionalLikeZodField(value)) {
          required.push(key);
        }
      }
      result = { type: "object", properties };
      if (required.length > 0) {
        result.required = required;
      }
      break;
    }
    default:
      // Unsupported Zod type — fallback
      result = { type: "object", properties: {} };
  }

  // Propagate Zod .describe() to JSON Schema description
  if (description && !result["description"]) {
    result.description = description;
  }
  return result;
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async function handleInitialize(
  id: number,
  params: { pluginPath: string; context: PluginContext },
): Promise<void> {
  try {
    const mod = await import(params.pluginPath);
    const pluginFn = mod.default ?? mod;

    if (typeof pluginFn !== "function") {
      sendError(id, -32600, "Plugin module does not export a function");
      return;
    }

    // Build a PluginInput compatible with upstream @opencode-ai/plugin shape,
    // while keeping `context` for backward compatibility.
    const client = await createPluginClient(params.context, params.pluginPath);
    const pluginInput = buildPluginInput(params.context, client);

    const hooks: Hooks = await pluginFn(pluginInput);
    pluginHooks = hooks;

    // Collect hook names
    const hookNames: string[] = [];
    for (const key of Object.keys(hooks)) {
      if (key === "auth" || key === "event" || key === "config") continue;
      if (typeof hooks[key] === "function") {
        hookNames.push(key);
      }
    }

    // Extract auth metadata if present
    let authMeta: { provider: string; methods: AuthMethod[] } | undefined;
    if (hooks.auth && typeof hooks.auth === "object") {
      authHook = hooks.auth as AuthHook;
      authMeta = {
        provider: authHook.provider,
        methods: authHook.methods.map((m) => ({
          type: m.type,
          label: m.label,
        })),
      };
    }

    // Resolve canonical plugin ID to avoid duplicate registrations from
    // symlinks or relative paths. For file:// and filesystem paths, use
    // realpathSync; for npm bare specifiers or other non-path strings,
    // fall back to path.resolve (which normalizes . and ..) or the raw string.
    let pluginID: string;
    try {
      const fs = await import("node:fs");
      const path = await import("node:path");
      // Only attempt realpathSync on strings that look like filesystem paths
      if (params.pluginPath.startsWith("/") || params.pluginPath.startsWith("./") || params.pluginPath.startsWith("../") || params.pluginPath.startsWith("file://")) {
        const fsPath = params.pluginPath.startsWith("file://")
          ? new URL(params.pluginPath).pathname
          : params.pluginPath;
        pluginID = fs.realpathSync(fsPath);
      } else {
        // npm bare specifier — normalize but don't resolve against filesystem
        pluginID = path.resolve(params.pluginPath);
      }
    } catch {
      pluginID = params.pluginPath;
    }

    // Collect plugin-registered custom tool definitions
    let toolDefs: Record<string, { description: string; parameters: Record<string, unknown> }> | undefined;
    if (hooks.tool && typeof hooks.tool === "object") {
      toolDefs = {};
      for (const [toolId, def] of Object.entries(hooks.tool)) {
        try {
          toolDefs[toolId] = {
            description: def.description,
            parameters: zodToJsonSchema(def.args),
          };
        } catch (err) {
          toolDefs[toolId] = { description: def.description, parameters: { type: "object", properties: {} } };
          process.stderr.write(`[plugin-host] zodToJsonSchema fallback for tool "${toolId}": ${err}\n`);
        }
      }
    }

    sendResult(id, {
      name: params.pluginPath.split("/").pop()?.replace(/\.[tj]s$/, "") ?? "unknown",
      hooks: hookNames,
      auth: authMeta,
      pluginID,
      tools: toolDefs,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Failed to initialize plugin: ${msg}`);
  }
}

async function handleHookInvoke(
  id: number,
  params: { hook: string; input: unknown; output: unknown },
): Promise<void> {
  const handler = pluginHooks[params.hook];
  if (typeof handler !== "function") {
    sendError(id, -32601, `Hook not found: ${params.hook}`);
    return;
  }

  try {
    // TS parity: `config` hooks mutate the first argument in-place.
    // Use one shared object for both input/output so in-place edits are preserved.
    if (params.hook === "config") {
      const seed =
        (params.output as UnknownRecord | null) ??
        (params.input as UnknownRecord | null) ??
        ({} as UnknownRecord);
      const result = await handler(seed, seed);
      sendResult(id, { output: result ?? seed });
      return;
    }

    const result = await handler(params.input, params.output);
    sendResult(id, { output: result ?? params.output });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Hook ${params.hook} failed: ${msg}`);
  }
}

async function handleHookInvokeFile(
  id: number,
  params: { file: string; token: string },
): Promise<void> {
  try {
    const fs = await import("node:fs");
    const path = await import("node:path");
    const os = await import("node:os");

    // Validate: file must be in controlled directory and filename must contain token
    const expectedDir = path.join(os.tmpdir(), "agendao-plugin-ipc");
    const resolvedPath = path.resolve(params.file);
    if (
      !resolvedPath.startsWith(expectedDir + path.sep) ||
      !path.basename(resolvedPath).includes(params.token)
    ) {
      sendError(id, -32602, "Invalid file path or token mismatch");
      return;
    }

    const content = fs.readFileSync(resolvedPath, "utf-8");
    const hookParams = JSON.parse(content) as {
      hook: string;
      input: unknown;
      output: unknown;
    };

    // Delegate to existing hook invoke logic
    await handleHookInvoke(id, hookParams);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `hook.invoke.file failed: ${msg}`);
  }
}

async function handleToolInvoke(
  id: number,
  params: { toolID: string; args: unknown; context: UnknownRecord },
): Promise<void> {
  const toolMap = pluginHooks?.tool;
  if (!toolMap || typeof toolMap !== "object") {
    sendError(id, -32602, `No plugin tools registered`);
    return;
  }
  const tool = (toolMap as Record<string, ToolDefinition>)[params.toolID];
  if (!tool || typeof tool.execute !== "function") {
    sendError(id, -32602, `Unknown or invalid tool: ${params.toolID}`);
    return;
  }
  const abortController = new AbortController();
  activeToolInvocations.set(id, abortController);

  try {
    const ctx: ToolContext = {
      sessionID: (params.context?.sessionID as string) ?? "",
      messageID: (params.context?.messageID as string) ?? "",
      agent: (params.context?.agent as string) ?? "",
      directory: (params.context?.directory as string) ?? "",
      worktree: (params.context?.worktree as string) ?? "",
      abort: abortController.signal,
      // fail-closed: ask throws to refuse, not silent no-op
      ask: async () => { throw new Error("Permission ask not supported in plugin tool bridge"); },
      metadata: (input) => {
        if (process.env.PLUGIN_DEBUG) {
          try {
            process.stderr.write(`[plugin-host] tool.invoke metadata: ${JSON.stringify(input)}\n`);
          } catch {
            process.stderr.write(`[plugin-host] tool.invoke metadata: [unserializable]\n`);
          }
        }
      },
    };

    // Start progress heartbeat to prevent timeout
    const progressInterval = setInterval(() => {
      sendNotification("notifications/progress", {
        message: `executing tool: ${params.toolID}`,
      });
    }, 5000); // Send heartbeat every 5 seconds

    try {
      const result = await tool.execute(params.args, ctx);
      sendResult(id, { output: result });
    } finally {
      clearInterval(progressInterval);
      activeToolInvocations.delete(id);
    }
  } catch (err: unknown) {
    activeToolInvocations.delete(id);
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `Tool execution failed: ${msg}`);
  }
}

async function handleAuthAuthorize(
  id: number,
  params: { methodIndex: number; inputs?: Record<string, string> },
): Promise<void> {
  if (!authHook?.authorize) {
    sendError(id, -32601, "No auth.authorize handler");
    return;
  }

  try {
    const method = authHook.methods[params.methodIndex];
    if (!method) {
      sendError(id, -32602, `Invalid method index: ${params.methodIndex}`);
      return;
    }

    const result = await authHook.authorize(method, params.inputs);
    // Stash callback for later auth.callback call
    if (result.callback) {
      pendingAuthCallback = result.callback;
    }

    sendResult(id, {
      url: result.url,
      instructions: result.instructions,
      method: result.method,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.authorize failed: ${msg}`);
  }
}

async function handleAuthCallback(
  id: number,
  params: { code?: string },
): Promise<void> {
  if (!pendingAuthCallback) {
    sendError(id, -32601, "No pending auth callback");
    return;
  }

  try {
    const result = await pendingAuthCallback(params.code);
    pendingAuthCallback = null;
    sendResult(id, result);
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.callback failed: ${msg}`);
  }
}

async function handleAuthLoad(id: number): Promise<void> {
  if (!authHook?.loader) {
    sendError(id, -32601, "No auth.loader handler");
    return;
  }

  try {
    const loaded = await authHook.loader();
    const hasCustomFetch = typeof loaded.fetch === "function";
    if (hasCustomFetch) {
      customFetch = loaded.fetch!;
    }

    sendResult(id, {
      apiKey: loaded.apiKey,
      hasCustomFetch,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.load failed: ${msg}`);
  }
}

async function handleAuthFetch(
  id: number,
  params: { url: string; method: string; headers: Record<string, string>; body?: string },
): Promise<void> {
  if (!customFetch) {
    sendError(id, -32601, "No custom fetch available");
    return;
  }

  try {
    const resp = await customFetch(params.url, {
      method: params.method,
      headers: params.headers,
      body: params.body,
    });

    const respHeaders: Record<string, string> = {};
    resp.headers.forEach((v: string, k: string) => {
      respHeaders[k] = v;
    });

    const body = await resp.text();
    sendResult(id, {
      status: resp.status,
      headers: respHeaders,
      body,
    });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendError(id, -32603, `auth.fetch failed: ${msg}`);
  }
}

async function handleAuthFetchStream(
  id: number,
  params: { url: string; method: string; headers: Record<string, string>; body?: string },
): Promise<void> {
  if (!customFetch) {
    sendError(id, -32601, "No custom fetch available");
    return;
  }

  try {
    const resp = await customFetch(params.url, {
      method: params.method,
      headers: params.headers,
      body: params.body,
    });

    const respHeaders: Record<string, string> = {};
    resp.headers.forEach((v: string, k: string) => {
      respHeaders[k] = v;
    });

    // First response carries status/headers so Rust can begin the stream pipeline.
    sendResult(id, {
      status: resp.status,
      headers: respHeaders,
    });

    if (!resp.body) {
      sendNotification("auth.fetch.stream.end", { requestId: id });
      return;
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.length === 0) continue;
      const chunk = decoder.decode(value, { stream: true });
      if (chunk.length === 0) continue;
      sendNotification("auth.fetch.stream.chunk", {
        requestId: id,
        chunk,
      });
    }

    const rest = decoder.decode();
    if (rest.length > 0) {
      sendNotification("auth.fetch.stream.chunk", {
        requestId: id,
        chunk: rest,
      });
    }
    sendNotification("auth.fetch.stream.end", { requestId: id });
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    sendNotification("auth.fetch.stream.error", {
      requestId: id,
      message: msg,
    });
    sendNotification("auth.fetch.stream.end", { requestId: id });
  }
}

function handleCancelRequest(params: { id: number }): void {
  const controller = activeToolInvocations.get(params.id);
  if (controller) {
    controller.abort();
    activeToolInvocations.delete(params.id);
  }
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  // Set stdin to raw binary mode
  if (typeof process.stdin.setEncoding === "function") {
    // Don't call setEncoding — we want raw bytes
  }
  process.stdin.resume();

  while (true) {
    let msg: JsonRpcRequest | null;
    try {
      msg = await readMessage();
    } catch {
      break; // stdin closed or parse error
    }

    if (msg === null) break; // EOF

    const { id, method, params } = msg;

    switch (method) {
      case "$/cancelRequest":
        handleCancelRequest(params as { id: number });
        break;
      case "initialize":
        await handleInitialize(id, params as Parameters<typeof handleInitialize>[1]);
        break;
      case "hook.invoke":
        await handleHookInvoke(id, params as Parameters<typeof handleHookInvoke>[1]);
        break;
      case "hook.invoke.file":
        await handleHookInvokeFile(id, params as { file: string; token: string });
        break;
      case "tool.invoke":
        await handleToolInvoke(id, params as { toolID: string; args: unknown; context: UnknownRecord });
        break;
      case "auth.authorize":
        await handleAuthAuthorize(id, params as Parameters<typeof handleAuthAuthorize>[1]);
        break;
      case "auth.callback":
        await handleAuthCallback(id, params as Parameters<typeof handleAuthCallback>[1]);
        break;
      case "auth.load":
        await handleAuthLoad(id);
        break;
      case "auth.fetch":
        await handleAuthFetch(id, params as Parameters<typeof handleAuthFetch>[1]);
        break;
      case "auth.fetch.stream":
        await handleAuthFetchStream(id, params as Parameters<typeof handleAuthFetchStream>[1]);
        break;
      case "shutdown":
        sendResult(id, {});
        process.exit(0);
      default:
        sendError(id, -32601, `Unknown method: ${method}`);
    }
  }
}

main().catch((err) => {
  process.stderr.write(`plugin-host fatal: ${err}\n`);
  process.exit(1);
});
