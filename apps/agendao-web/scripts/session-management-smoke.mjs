import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { WebSocket as UndiciWebSocket } from "undici";

const RuntimeWebSocket = globalThis.WebSocket ?? UndiciWebSocket;

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:3000";
const WEB_URL = new URL("/web/", `${BASE_URL}/`).toString();
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9226", 10);
const TIMEOUT_MS = Number.parseInt(process.env.AGENDAO_SMOKE_TIMEOUT_MS ?? "30000", 10);

const trackerInitScript = `
(() => {
  const state = { fetches: [] };
  window.__agendaoTracker = state;
  const originalFetch = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const input = args[0];
    const init = args[1];
    const url =
      typeof input === "string"
        ? input
        : input instanceof Request
          ? input.url
          : String(input);
    const method =
      init?.method ??
      (input instanceof Request && input.method ? input.method : "GET");
    state.fetches.push({ url, method: String(method).toUpperCase() });
    return originalFetch(...args);
  };
})();
`;

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

async function serviceWorkspacePath() {
  const context = await fetchJson(`${BASE_URL}/workspace/context`);
  return (
    context?.identity?.workspace_root?.trim() ||
    context?.identity?.requested_dir?.trim() ||
    (await fetchJson(`${BASE_URL}/path`)).cwd ||
    ""
  );
}

async function createSeedSession(title) {
  const directory = await serviceWorkspacePath();
  return fetchJson(`${BASE_URL}/session`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ directory, title }),
  });
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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-session-smoke-"));
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
  await client.send("Network.enable");
  await client.send("Page.addScriptToEvaluateOnNewDocument", { source: trackerInitScript });
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

async function ensureWorkspaceSelected(client) {
  const hasWorkspace = await evaluate(
    client,
    "document.querySelector('[data-testid=\"session-new\"]')?.disabled === false",
  );
  if (hasWorkspace) return;

  const projectsVisible = await evaluate(
    client,
    "Boolean(document.querySelector('[data-testid=\"workspace-project-item\"]'))",
  );
  if (!projectsVisible) {
    await click(client, "[data-testid='projects-toggle']");
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"workspace-project-item\"]'))",
    );
  }

  await click(client, "[data-testid='workspace-project-item']");
  await waitForExpression(
    client,
    "document.querySelector('[data-testid=\"session-new\"]')?.disabled === false",
  );
}

async function ensureSessionRoute(client, title) {
  await navigate(client, WEB_URL);
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"session-sidebar\"]'))");
  const hasWorkspace = await evaluate(
    client,
    "document.querySelector('[data-testid=\"session-new\"]')?.disabled === false",
  );
  if (hasWorkspace) {
    return;
  }
  const hasWorkspaceList = await evaluate(
    client,
    "Boolean(document.querySelector('[data-testid=\"workspace-project-item\"]'))",
  );
  if (hasWorkspaceList) {
    await ensureWorkspaceSelected(client);
    return;
  }
  const seeded = await createSeedSession(title);
  await navigate(client, `${WEB_URL}?session=${encodeURIComponent(seeded.id)}`);
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"session-sidebar\"]'))");
}

async function activeSessionId(client) {
  return evaluate(
    client,
    "document.querySelector('[data-testid=\"session-item\"][data-active=\"true\"]')?.dataset.sessionId ?? null",
  );
}

async function sessionIds(client) {
  return evaluate(
    client,
    "Array.from(document.querySelectorAll('[data-testid=\"session-item\"]')).map((item) => item.dataset.sessionId).filter(Boolean)",
  );
}

async function waitForFetchCount(client, predicateExpression, expectedCount) {
  await waitForExpression(
    client,
    `(window.__agendaoTracker?.fetches ?? []).filter((entry) => ${predicateExpression}).length >= ${expectedCount}`,
  );
}

async function fetchCount(client, predicateExpression) {
  return evaluate(
    client,
    `(window.__agendaoTracker?.fetches ?? []).filter((entry) => ${predicateExpression}).length`,
  );
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;

  try {
    client = await createPageClient();
    await ensureSessionRoute(client, `session-management-smoke-${Date.now()}`);

    let rootSessionId = await activeSessionId(client);
    if (!rootSessionId) {
      await click(client, "[data-testid='session-new']");
      await waitForFetchCount(client, "entry.url.endsWith('/session') && entry.method === 'POST'", 1);
      await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"session-item\"][data-active=\"true\"]'))");
      rootSessionId = await activeSessionId(client);
    }
    assert(rootSessionId, "failed to resolve or create initial active session");

    const beforeIds = await sessionIds(client);
    const sessionCreateCountBefore = await fetchCount(
      client,
      "entry.url.endsWith('/session') && entry.method === 'POST'",
    );

    await click(client, "[data-testid='session-new']");
    await waitForFetchCount(
      client,
      "entry.url.endsWith('/session') && entry.method === 'POST'",
      sessionCreateCountBefore + 1,
    );
    await waitForExpression(
      client,
      `(() => {
        const active = document.querySelector('[data-testid="session-item"][data-active="true"]');
        return Boolean(active && active.dataset.sessionId && active.dataset.sessionId !== ${JSON.stringify(rootSessionId)});
      })()`,
    );
    const createdSessionId = await activeSessionId(client);
    assert(createdSessionId && createdSessionId !== rootSessionId, "new session did not become active");

    await click(client, `[data-testid="session-item"][data-session-id="${rootSessionId}"]`);
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="session-item"][data-session-id="${rootSessionId}"]')?.dataset.active === "true"`,
    );

    await click(client, "[title='Select sessions']");
    await waitForExpression(client, "Boolean(document.querySelector('[title=\"Cancel session selection\"]'))");
    await click(client, `[data-testid="session-item"][data-session-id="${createdSessionId}"]`);
    await click(client, "[title^='Delete']");
    await waitForExpression(client, "Boolean(document.querySelector('[role=\"dialog\"]'))");
    await click(client, "[data-slot='button'][data-variant='destructive']");
    await waitForFetchCount(client, `entry.url.includes('/session/${createdSessionId}') && entry.method === 'DELETE'`, 1);
    await waitForExpression(
      client,
      `!document.querySelector('[data-testid="session-item"][data-session-id="${createdSessionId}"]')`,
    );
    const afterIds = await sessionIds(client);
    const fallbackSessionId = await activeSessionId(client);
    assert(fallbackSessionId, "no fallback session became active after deleting the created session");
    assert(fallbackSessionId !== createdSessionId, "deleted session remained active after delete");
    assert(afterIds.includes(fallbackSessionId), "active fallback session is missing from the session list");
    assert(
      fallbackSessionId === rootSessionId || afterIds.includes(rootSessionId),
      "expected to return to the previously selected session or another surviving fallback session",
    );
    assert(!afterIds.includes(createdSessionId), "deleted session still present in session list");
    assert(afterIds.length >= Math.max(beforeIds.length - 1, 1), "session list unexpectedly collapsed after delete");

    console.log("session management smoke passed");
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
  console.error(`Session management smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
