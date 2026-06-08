import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { WebSocket as UndiciWebSocket } from "undici";

const RuntimeWebSocket = globalThis.WebSocket ?? UndiciWebSocket;

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:3000";
const WEB_URL = new URL("/web/", `${BASE_URL}/`).toString();
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9229", 10);
const TIMEOUT_MS = Number.parseInt(process.env.AGENDAO_SMOKE_TIMEOUT_MS ?? "30000", 10);

const SETTINGS_TABS = [
  { id: "general", panel: "[data-testid='settings-panel-general']" },
  { id: "memory", panel: "[data-testid='settings-panel-memory'], [data-testid='settings-panel-memory-loading']" },
  { id: "providers", panel: "[data-testid='settings-panel-providers']" },
  { id: "scheduler", panel: "[data-testid='settings-panel-scheduler']" },
  { id: "validation", panel: "[data-testid='settings-panel-validation']" },
  { id: "skills", panel: "[data-testid='settings-panel-skills'], [data-testid='settings-panel-skills-loading']" },
  { id: "mcp", panel: "[data-testid='settings-panel-mcp']" },
  { id: "plugins", panel: "[data-testid='settings-panel-plugins']" },
  { id: "lsp", panel: "[data-testid='settings-panel-lsp']" },
];

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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-settings-smoke-"));
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
  };

  await client.send("Page.enable");
  await client.send("Runtime.enable");
  return client;
}

async function evaluate(client, expression) {
  const result = await client.send("Runtime.evaluate", {
    expression,
    returnByValue: true,
    awaitPromise: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text ?? "Runtime evaluation failed");
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
  const escaped = JSON.stringify(selector);
  const clicked = await evaluate(
    client,
    `(() => {
      const element = document.querySelector(${escaped});
      if (!element) return false;
      element.click();
      return true;
    })()`,
  );
  if (!clicked) {
    throw new Error(`Could not find clickable selector ${selector}`);
  }
}

async function waitForNoFatalRenderError(client) {
  const fatal = await evaluate(
    client,
    `(() => {
      const text = document.body?.innerText ?? "";
      return text.includes("Minified React error") || text.includes("Something went wrong");
    })()`,
  );
  assert(!fatal, "settings shell hit a fatal render error");
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;

  try {
    client = await createPageClient();
    await navigate(client, WEB_URL);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-open\"]'))");

    await click(client, "[data-testid='settings-open']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-page\"]'))");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-drawer\"]'))");
    await waitForNoFatalRenderError(client);

    for (const tab of SETTINGS_TABS) {
      await click(client, `[data-testid="settings-tab-${tab.id}"]`);
      await waitForExpression(
        client,
        `document.querySelector('[data-testid="settings-tab-${tab.id}"]')?.dataset.active === "true"`,
      );
      await waitForExpression(
        client,
        `Boolean(document.querySelector(${JSON.stringify(tab.panel)}))`,
      );
      await waitForNoFatalRenderError(client);
    }

    const activeCount = await evaluate(
      client,
      "document.querySelectorAll('[data-testid^=\"settings-tab-\"][data-active=\"true\"]').length",
    );
    assert(activeCount === 1, `expected exactly one active settings tab, got ${activeCount}`);

    await click(client, "[data-testid='settings-close']");
    await waitForExpression(client, "!document.querySelector('[data-testid=\"settings-page\"]')");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"composer-form\"]'))");

    console.log("settings shell smoke passed");
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
  console.error(`Settings shell smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
