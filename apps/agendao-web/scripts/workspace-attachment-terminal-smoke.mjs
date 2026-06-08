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
    let body = null;
    if (typeof init?.body === "string") {
      body = init.body;
    } else if (input instanceof Request && typeof input.bodyUsed === "boolean") {
      body = null;
    }
    state.fetches.push({ url, method: String(method).toUpperCase(), body });
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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-workspace-terminal-smoke-"));
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

async function waitForOptionalExpression(client, expression, timeoutMs = TIMEOUT_MS) {
  try {
    await waitForExpression(client, expression, timeoutMs);
    return true;
  } catch (error) {
    if (error instanceof Error && error.message.includes("Timed out waiting for expression:")) {
      return false;
    }
    throw error;
  }
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

async function clickExpression(client, expression) {
  const clicked = await evaluate(
    client,
    `(() => {
      const element = (${expression});
      if (!element) return false;
      element.click();
      return true;
    })()`,
  );
  if (!clicked) {
    throw new Error(`Could not click expression: ${expression}`);
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
  await waitForRootShell(client);
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
  await waitForRootShell(client);
}

async function activeSessionId(client) {
  return evaluate(
    client,
    "document.querySelector('[data-testid=\"session-item\"][data-active=\"true\"]')?.dataset.sessionId ?? null",
  );
}

async function ensureActiveSession(client) {
  let sessionId = await activeSessionId(client);
  if (sessionId) return sessionId;
  await click(client, "[data-testid='session-new']");
  await waitForExpression(
    client,
    "(window.__agendaoTracker?.fetches ?? []).some((entry) => entry.url.endsWith('/session') && entry.method === 'POST')",
  );
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"session-item\"][data-active=\"true\"]'))");
  sessionId = await activeSessionId(client);
  assert(sessionId, "failed to activate session");
  return sessionId;
}

async function waitForRootShell(client) {
  await waitForExpression(
    client,
    "Boolean(document.querySelector('[data-testid=\"session-sidebar\"]') && document.querySelector('[data-testid=\"composer-input\"]') && document.querySelector('[data-testid=\"workspace-panel\"]'))",
  );
}

async function ensureFilesTab(client) {
  await clickExpression(
    client,
    "document.querySelector('[data-testid=\"workspace-panel-tabs\"] button:first-child')",
  );
}

async function workspaceRootPath(client) {
  return evaluate(
    client,
    "document.querySelector('[data-testid=\"workspace-node\"][data-node-type=\"directory\"]')?.dataset.path ?? null",
  );
}

function workspaceNodeSelector(pathname) {
  return `[data-testid="workspace-node"][data-path="${pathname}"]`;
}

async function trackerFetchCount(client, predicateExpression) {
  return evaluate(
    client,
    `(window.__agendaoTracker?.fetches ?? []).filter((entry) => ${predicateExpression}).length`,
  );
}

async function maybeAllowTerminalPermission(client, expectedReplyCount) {
  const permissionVisible = await waitForOptionalExpression(
    client,
    "Boolean(document.querySelector('[data-testid=\"permission-overlay\"]'))",
    2500,
  );
  if (!permissionVisible) {
    return false;
  }

  await click(client, "[data-testid='permission-once']");
  await waitForExpression(
    client,
    `(window.__agendaoTracker?.fetches ?? []).filter((entry) =>
      /\\/permission\\/.+\\/reply$/.test(entry.url) && entry.method === 'POST'
    ).length >= ${expectedReplyCount}`,
  );
  await waitForExpression(
    client,
    "!document.querySelector('[data-testid=\"permission-overlay\"]') || Boolean(document.querySelector('[data-testid=\"permission-submit-completed\"]'))",
  );
  return true;
}

async function ptyCount() {
  const sessions = await fetchJson(`${BASE_URL}/pty`);
  return Array.isArray(sessions) ? sessions.length : 0;
}

async function deleteWorkspacePath(pathname) {
  await fetchJson(`${BASE_URL}/file`, {
    method: "DELETE",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      path: pathname,
      recursive: true,
    }),
  });
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;
  let cleanupPath = null;

  try {
    client = await createPageClient();
    await ensureSessionRoute(client, `workspace-terminal-smoke-${Date.now()}`);
    await ensureActiveSession(client);
    await ensureFilesTab(client);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"workspace-node\"]'))");

    const rootPath = await workspaceRootPath(client);
    assert(rootPath, "failed to resolve workspace root path");

    const stamp = Date.now();
    const topDir = `${rootPath}/agendao-web-phase14-${stamp}`;
    const nestedDir = `${topDir}/nested`;
    const filePath = `${nestedDir}/lazy-file.txt`;
    cleanupPath = topDir;
    const referencePath = filePath.startsWith(`${rootPath}/`)
      ? filePath.slice(rootPath.length + 1)
      : filePath;
    const topDirSelector = workspaceNodeSelector(topDir);
    const nestedDirSelector = workspaceNodeSelector(nestedDir);
    const fileSelector = workspaceNodeSelector(filePath);
    const encodedTopDir = encodeURIComponent(topDir);
    const encodedNestedDir = encodeURIComponent(nestedDir);

    await fetchJson(`${BASE_URL}/file/content`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        path: filePath,
        content: "phase 1.4 workspace smoke\nlazy expand\nattachment context\n",
        create_parents: true,
      }),
    });

    await navigate(client, WEB_URL);
    await waitForRootShell(client);
    await ensureActiveSession(client);
    await ensureFilesTab(client);
    await waitForExpression(client, `Boolean(document.querySelector(${JSON.stringify(topDirSelector)}))`);
    await waitForExpression(client, `!document.querySelector(${JSON.stringify(nestedDirSelector)})`);
    await waitForExpression(client, `!document.querySelector(${JSON.stringify(fileSelector)})`);

    const topDirFetchCountBefore = await trackerFetchCount(
      client,
      `entry.method === 'GET' && entry.url.includes('/file/tree') && entry.url.includes(${JSON.stringify(`path=${encodedTopDir}`)}) && entry.url.includes('depth=1')`,
    );
    assert(topDirFetchCountBefore === 0, "top directory should not be lazily fetched before expansion");

    await click(client, topDirSelector);
    await waitForExpression(
      client,
      `(window.__agendaoTracker?.fetches ?? []).filter((entry) =>
        entry.method === 'GET' &&
        entry.url.includes('/file/tree') &&
        entry.url.includes(${JSON.stringify(`path=${encodedTopDir}`)}) &&
        entry.url.includes('depth=1')
      ).length >= 1`,
    );
    await waitForExpression(client, `Boolean(document.querySelector(${JSON.stringify(nestedDirSelector)}))`);
    await waitForExpression(client, `!document.querySelector(${JSON.stringify(fileSelector)})`);

    const nestedDirFetchCountBefore = await trackerFetchCount(
      client,
      `entry.method === 'GET' && entry.url.includes('/file/tree') && entry.url.includes(${JSON.stringify(`path=${encodedNestedDir}`)}) && entry.url.includes('depth=1')`,
    );
    assert(nestedDirFetchCountBefore === 0, "nested directory should not be fetched before nested expansion");

    await click(client, nestedDirSelector);
    await waitForExpression(
      client,
      `(window.__agendaoTracker?.fetches ?? []).filter((entry) =>
        entry.method === 'GET' &&
        entry.url.includes('/file/tree') &&
        entry.url.includes(${JSON.stringify(`path=${encodedNestedDir}`)}) &&
        entry.url.includes('depth=1')
      ).length >= 1`,
    );
    await waitForExpression(client, `Boolean(document.querySelector(${JSON.stringify(fileSelector)}))`);

    await click(client, fileSelector);
    await click(client, "[data-testid='workspace-insert-reference']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="composer-input"]')?.value.includes(${JSON.stringify(`@${referencePath}`)}) === true`,
    );

    await click(client, "[data-testid='workspace-attach']");
    await waitForExpression(
      client,
      `Boolean(document.querySelector('[data-testid="context-attachment-chip"][data-workspace-path=${JSON.stringify(filePath)}]'))`,
    );
    await clickExpression(
      client,
      `document.querySelector('[data-testid="context-attachment-chip"][data-workspace-path=${JSON.stringify(filePath)}] [data-testid="context-attachment-main"]')`,
    );
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"attachment-details\"]'))");
    await click(client, "[data-testid='attachment-locate']");
    await waitForExpression(
      client,
      `document.querySelector(${JSON.stringify(fileSelector)})?.dataset.selected === "true"`,
    );

    const ptyCountBeforeOpen = await ptyCount();
    const ptyPostCountBeforeOpen = await trackerFetchCount(
      client,
      "entry.method === 'POST' && /\\/pty$/.test(new URL(entry.url, location.origin).pathname)",
    );
    assert(
      (await trackerFetchCount(client, "entry.url.includes('/pty')")) === 0,
      "terminal should not prefetch PTY routes before opening",
    );

    await click(client, "[data-testid='terminal-toggle']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-panel\"]'))");
    await maybeAllowTerminalPermission(client, 1);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-viewport\"]'))");
    await waitForExpression(
      client,
      "(window.__agendaoTracker?.fetches ?? []).some((entry) => entry.url.includes('/pty'))",
    );
    await sleep(800);
    const ptyCountAfterFirstOpen = await ptyCount();
    assert(ptyCountAfterFirstOpen >= ptyCountBeforeOpen, "terminal open unexpectedly reduced PTY session count");
    const ptyPostCountAfterFirstOpen = await trackerFetchCount(
      client,
      "entry.method === 'POST' && /\\/pty$/.test(new URL(entry.url, location.origin).pathname)",
    );
    assert(ptyPostCountAfterFirstOpen >= ptyPostCountBeforeOpen, "terminal first open regressed PTY creation tracking");

    await click(client, "[data-testid='terminal-toggle']");
    await waitForExpression(client, "!document.querySelector('[data-testid=\"terminal-panel\"]')");
    await click(client, "[data-testid='terminal-toggle']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-panel\"]'))");
    await maybeAllowTerminalPermission(client, 2);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-viewport\"]'))");
    await sleep(800);
    const ptyCountAfterReopen = await ptyCount();
    const ptyPostCountAfterReopen = await trackerFetchCount(
      client,
      "entry.method === 'POST' && /\\/pty$/.test(new URL(entry.url, location.origin).pathname)",
    );
    if (ptyCountAfterFirstOpen > 0) {
      assert(
        ptyCountAfterReopen === ptyCountAfterFirstOpen,
        "terminal reopen should reuse existing PTY sessions instead of creating new ones",
      );
    }
    if (ptyPostCountAfterFirstOpen > ptyPostCountBeforeOpen) {
      assert(
        ptyPostCountAfterReopen === ptyPostCountAfterFirstOpen,
        "terminal reopen emitted an unexpected extra PTY create request",
      );
    }

    await navigate(client, WEB_URL);
    await waitForRootShell(client);
    await ensureActiveSession(client);
    assert(
      (await trackerFetchCount(client, "entry.url.includes('/pty')")) === 0,
      "fresh page load should not prefetch PTY routes before terminal reopen",
    );
    await click(client, "[data-testid='terminal-toggle']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-panel\"]'))");
    await maybeAllowTerminalPermission(client, 3);
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-viewport\"]'))");
    await sleep(800);
    const ptyCountAfterReloadReopen = await ptyCount();
    const ptyPostCountAfterReloadReopen = await trackerFetchCount(
      client,
      "entry.method === 'POST' && /\\/pty$/.test(new URL(entry.url, location.origin).pathname)",
    );
    assert(
      ptyCountAfterReloadReopen === ptyCountAfterFirstOpen,
      "terminal reopen after full page reload should reuse existing PTY sessions",
    );
    assert(
      ptyPostCountAfterReloadReopen === 0,
      "terminal reopen after page reload should not create a duplicate PTY session",
    );

    console.log("workspace attachment terminal smoke passed");
  } finally {
    if (cleanupPath) {
      try {
        await deleteWorkspacePath(cleanupPath);
      } catch {
        // Best-effort cleanup for the synthetic smoke directory.
      }
    }
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
  console.error(`Workspace attachment terminal smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
