import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { WebSocket as UndiciWebSocket } from "undici";

const RuntimeWebSocket = globalThis.WebSocket ?? UndiciWebSocket;

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:3000";
const WEB_URL = new URL("/web/", `${BASE_URL}/`).toString();
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9231", 10);
const TIMEOUT_MS = Number.parseInt(process.env.AGENDAO_SMOKE_TIMEOUT_MS ?? "30000", 10);

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }
  return response.json();
}

async function waitForHttp(url, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError}`);
}

async function launchChrome() {
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-settings-readonly-smoke-"));
  const chrome = spawn(
    CHROME_BIN,
    [
      `--remote-debugging-port=${CHROME_PORT}`,
      "--headless=new",
      "--disable-gpu",
      "--disable-dev-shm-usage",
      "--no-first-run",
      "--no-default-browser-check",
      "--no-sandbox",
      `--user-data-dir=${userDataDir}`,
      "about:blank",
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );

  let stderr = "";
  chrome.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });

  await waitForHttp(`http://127.0.0.1:${CHROME_PORT}/json/version`);
  return { chrome, userDataDir, stderr: () => stderr };
}

async function terminateProcess(child) {
  if (!child || child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(2000),
  ]);
  if (child.exitCode === null) {
    child.kill("SIGKILL");
    await new Promise((resolve) => child.once("exit", resolve));
  }
}

async function createPageClient() {
  const pages = await fetchJson(`http://127.0.0.1:${CHROME_PORT}/json/list`);
  const page = pages.find((entry) => entry.type === "page");
  if (!page?.webSocketDebuggerUrl) {
    throw new Error("Could not find a Chrome page target");
  }

  const socket = new RuntimeWebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve, { once: true });
    socket.addEventListener("error", reject, { once: true });
  });

  let nextId = 0;
  const pending = new Map();
  const listeners = new Map();
  const pageErrors = [];
  const consoleErrors = [];

  socket.addEventListener("message", (event) => {
    const payload = JSON.parse(event.data);
    if (typeof payload.id === "number") {
      const resolver = pending.get(payload.id);
      if (!resolver) return;
      pending.delete(payload.id);
      if (payload.error) {
        resolver.reject(new Error(payload.error.message ?? JSON.stringify(payload.error)));
      } else {
        resolver.resolve(payload.result ?? {});
      }
      return;
    }

    const handlers = listeners.get(payload.method);
    if (!handlers) return;
    handlers.forEach((handler) => handler(payload.params ?? {}));
  });

  const client = {
    async send(method, params = {}) {
      const id = ++nextId;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });
    },
    on(method, handler) {
      const handlers = listeners.get(method) ?? [];
      handlers.push(handler);
      listeners.set(method, handlers);
      return () => {
        const nextHandlers = (listeners.get(method) ?? []).filter((item) => item !== handler);
        if (nextHandlers.length) listeners.set(method, nextHandlers);
        else listeners.delete(method);
      };
    },
    close() {
      socket.close();
    },
    getPageErrors() {
      return [...pageErrors, ...consoleErrors];
    },
  };

  await client.send("Page.enable");
  await client.send("DOM.enable");
  await client.send("Runtime.enable");
  await client.send("Network.enable");
  client.on("Runtime.exceptionThrown", (params) => {
    const detail = params.exceptionDetails ?? {};
    const description =
      params.exceptionDetails?.exception?.description ??
      params.exceptionDetails?.exception?.value ??
      "";
    const stack = detail.stackTrace?.callFrames
      ?.map((frame) => `${frame.functionName || "<anon>"}@${frame.url}:${frame.lineNumber}:${frame.columnNumber}`)
      .join(" <- ");
    pageErrors.push(
      [detail.text ?? "Runtime.exceptionThrown", description, stack]
        .filter(Boolean)
        .join(" :: "),
    );
  });
  client.on("Runtime.consoleAPICalled", (params) => {
    if (params.type !== "error") return;
    const text = (params.args ?? [])
      .map((arg) => arg.value ?? arg.description ?? "")
      .filter(Boolean)
      .join(" ");
    if (text) {
      consoleErrors.push(text);
    }
  });
  return client;
}

async function evaluate(client, expression) {
  const result = await client.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails) {
    const detail = result.exceptionDetails;
    const description =
      detail.exception?.description ??
      detail.exception?.value ??
      "";
    const stack = detail.stackTrace?.callFrames
      ?.map((frame) => `${frame.functionName || "<anon>"}@${frame.url}:${frame.lineNumber}:${frame.columnNumber}`)
      .join(" <- ");
    throw new Error(
      [detail.text ?? "Runtime evaluation failed", description, stack]
        .filter(Boolean)
        .join(" :: "),
    );
  }
  return result.result?.value;
}

async function waitForExpression(client, expression, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await evaluate(client, expression);
    if (value) return value;
    await sleep(200);
  }
  throw new Error(`Timed out waiting for expression: ${expression}`);
}

async function navigate(client, url) {
  const loadEvent = new Promise((resolve) => {
    const unsubscribe = client.on("Page.loadEventFired", () => {
      unsubscribe();
      resolve();
    });
  });
  await client.send("Page.navigate", { url });
  await loadEvent;
  await waitForExpression(client, "document.readyState === 'complete'");
}

async function click(client, selector) {
  const { root } = await client.send("DOM.getDocument", { depth: -1, pierce: true });
  const { nodeId } = await client.send("DOM.querySelector", {
    nodeId: root.nodeId,
    selector,
  });
  if (!nodeId) {
    throw new Error(`Could not find clickable selector ${selector}`);
  }
  await client.send("DOM.scrollIntoViewIfNeeded", { nodeId }).catch(() => null);
  const { model } = await client.send("DOM.getBoxModel", { nodeId });
  const content = model?.content ?? [];
  if (content.length < 8) {
    throw new Error(`Could not resolve clickable box for ${selector}`);
  }
  const x = (content[0] + content[2] + content[4] + content[6]) / 4;
  const y = (content[1] + content[3] + content[5] + content[7]) / 4;
  await client.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none" });
  await client.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
  await client.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
}

async function openSettings(client) {
  await click(client, "[data-testid='settings-open']");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-drawer\"]'))");
}

async function openTab(client, tabId) {
  await click(client, `[data-testid="settings-tab-${tabId}"]`);
  await waitForExpression(
    client,
    `document.querySelector('[data-testid="settings-tab-${tabId}"]')?.dataset.active === "true"`,
  );
}

async function assertNoFatalRender(client) {
  const fatal = await evaluate(
    client,
    `(() => {
      const text = document.body?.innerText ?? "";
      return text.includes("Minified React error") || text.includes("Something went wrong");
    })()`,
  );
  assert(!fatal, "settings readonly smoke hit a fatal render error");
}

async function installFetchFailure(client, pathname) {
  await evaluate(
    client,
    `(() => {
      const state = window.__agendaoReadonlySmoke ?? {};
      if (!state.originalFetch) {
        state.originalFetch = window.fetch.bind(window);
      }
      state.failPath = ${JSON.stringify(pathname)};
      window.fetch = async (...args) => {
        const input = args[0];
        const url =
          typeof input === "string"
            ? input
            : input instanceof Request
              ? input.url
              : String(input);
        if (url.includes(state.failPath)) {
          return new Response(JSON.stringify({ error: { message: "phase23 synthetic failure" } }), {
            status: 503,
            headers: { "Content-Type": "application/json" },
          });
        }
        return state.originalFetch(...args);
      };
      window.__agendaoReadonlySmoke = state;
      return true;
    })()`,
  );
}

async function clearFetchFailure(client) {
  await evaluate(
    client,
    `(() => {
      const state = window.__agendaoReadonlySmoke;
      if (state?.originalFetch) {
        window.fetch = state.originalFetch;
      }
      delete window.__agendaoReadonlySmoke;
      return true;
    })()`,
  );
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;
  let step = "boot";

  try {
    client = await createPageClient();
    step = "navigate";
    await navigate(client, WEB_URL);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-open\"]'))");

    step = "open settings";
    await openSettings(client);
    await assertNoFatalRender(client);

    step = "validation ready";
    await openTab(client, "validation");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-validation\"]'))");
    await waitForExpression(
      client,
      "document.querySelector('[data-testid=\"settings-panel-validation\"]')?.textContent?.includes('Config Revision') ?? false",
    );
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"settings-validation-group\"], [data-testid=\"settings-validation-empty\"], [data-testid=\"settings-validation-unavailable\"]'))",
    );
    await assertNoFatalRender(client);

    step = "mcp ready";
    await openTab(client, "mcp");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-mcp\"]'))");
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"settings-mcp-server\"], [data-testid=\"settings-mcp-empty\"]'))",
    );
    await assertNoFatalRender(client);

    step = "plugins ready";
    await openTab(client, "plugins");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-plugins\"]'))");
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"settings-plugin-provider\"], [data-testid=\"settings-plugins-empty\"]'))",
    );
    await assertNoFatalRender(client);

    step = "lsp ready";
    await openTab(client, "lsp");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-lsp\"]'))");
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"settings-lsp-server\"], [data-testid=\"settings-lsp-empty\"]'))",
    );
    await assertNoFatalRender(client);

    step = "settings reload failure feedback";
    await installFetchFailure(client, "/mcp");
    await click(client, "[data-testid='settings-refresh']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-feedback"]')?.textContent?.includes('Failed to load settings data:') ?? false`,
    );
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-lsp\"]'))");
    await assertNoFatalRender(client);
    await clearFetchFailure(client);

    console.log("settings readonly surfaces smoke passed");
  } catch (error) {
    const suffix =
      client
        ? (() => {
            const runtimeErrors = client.getPageErrors().filter(Boolean);
            return runtimeErrors.length > 0
              ? ` | step=${step} | page_errors=${runtimeErrors.join(" || ")}`
              : ` | step=${step}`;
          })()
        : ` | step=${step}`;
    const baseMessage = error instanceof Error ? error.message : String(error);
    throw new Error(`${baseMessage}${suffix}`);
  } finally {
    if (client) {
      client.close();
    }
    await terminateProcess(chrome);
    await rm(userDataDir, { recursive: true, force: true });
    if (stderr()) {
      // no-op on success
    }
  }
}

run().catch((error) => {
  console.error(`Settings readonly surfaces smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
