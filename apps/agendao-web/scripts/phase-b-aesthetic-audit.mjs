import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { WebSocket as UndiciWebSocket } from "undici";

const RuntimeWebSocket = globalThis.WebSocket ?? UndiciWebSocket;

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:4096";
const WEB_URL = new URL("/web/", `${BASE_URL}/`).toString();
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9223", 10);
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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-aesthetic-audit-"));
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

async function removeDirectoryWithRetry(pathname, attempts = 8, delayMs = 250) {
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      await rm(pathname, { recursive: true, force: true });
      return;
    } catch (error) {
      lastError = error;
      if (!error || typeof error !== "object" || error.code !== "ENOTEMPTY") {
        throw error;
      }
      await sleep(delayMs);
    }
  }
  if (lastError) throw lastError;
}

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }
  return response.json();
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

async function waitForRootShell(client) {
  await waitForExpression(
    client,
    "Boolean(document.querySelector('[data-testid=\"session-sidebar\"]') && document.querySelector('[data-testid=\"composer-input\"]') && (document.querySelector('[data-testid=\"workspace-panel\"]') || document.querySelector('[data-testid=\"workspace-inspector\"]')))",
  );
}

async function ensureActiveSession(client) {
  const activeSession = await evaluate(
    client,
    "document.querySelector('[data-testid=\"session-item\"][data-active=\"true\"]')?.dataset.sessionId ?? null",
  );
  if (activeSession && !String(activeSession).startsWith("optimistic:")) return activeSession;
  await click(client, "[data-testid='session-new']");
  return waitForExpression(
    client,
    `(() => {
      const sessionId = document.querySelector('[data-testid="session-item"][data-active="true"]')?.dataset.sessionId ?? null;
      return sessionId && !String(sessionId).startsWith('optimistic:') ? sessionId : null;
    })()`,
  );
}

async function seedRuntimeSurface(client) {
  return evaluate(
    client,
    `(() => {
      const injected = window.__agendaoWebDebug?.injectRuntimeSurface?.({
        banner: null,
        queueItems: [
          {
            kind: "queue_item",
            role: "assistant",
            id: "phase-b-runtime-audit-queue",
            title: "Audit queue item",
            phase: "running",
            summary: "Synthetic runtime item for Phase B audit",
            text: "runtime audit queue block",
            ts: Date.now(),
          },
        ],
      });
      return injected === true;
    })()`,
  );
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function measureLayout(client) {
  return evaluate(
    client,
    `(() => {
      const runtimeSurface = document.querySelector('[data-testid="runtime-surface"]');
      const runtimeExpanded = document.querySelector('[data-testid="runtime-surface-expanded"]');
      const sessionHeader = document.querySelector('[data-testid="session-header"]');
      const workspaceHeader = document.querySelector('[data-testid="workspace-panel-header"]');
      const composerForm = document.querySelector('[data-testid="composer-form"]');
      const conversation = document.querySelector('[data-testid="session-header"]')
        ? document.querySelector('[data-testid="session-header"]')?.parentElement?.nextElementSibling
        : null;
      const terminalPanel = document.querySelector('[data-testid="terminal-panel"]');

      const centralCandidates = [
        runtimeSurface ? { key: 'runtime-surface', rect: runtimeSurface.getBoundingClientRect() } : null,
        sessionHeader ? { key: 'session-header', rect: sessionHeader.getBoundingClientRect() } : null,
        conversation ? { key: 'conversation-feed', rect: conversation.getBoundingClientRect() } : null,
        composerForm ? { key: 'composer-form', rect: composerForm.getBoundingClientRect() } : null,
        terminalPanel ? { key: 'terminal-panel', rect: terminalPanel.getBoundingClientRect() } : null,
      ].filter(Boolean).filter((entry) => entry.rect.height >= 40);

      return {
        runtimeSurfaceHeight: runtimeSurface ? runtimeSurface.getBoundingClientRect().height : null,
        runtimeSurfaceExpandedContentHeight: runtimeExpanded ? runtimeExpanded.getBoundingClientRect().height : null,
        sessionHeaderHeight: sessionHeader ? sessionHeader.getBoundingClientRect().height : null,
        workspaceHeaderHeight: workspaceHeader ? workspaceHeader.getBoundingClientRect().height : null,
        centralBlockCount: centralCandidates.length,
        centralBlocks: centralCandidates.map((entry) => ({
          key: entry.key,
          height: Math.round(entry.rect.height),
        })),
      };
    })()`,
  );
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;
  const consoleMessages = [];
  const runtimeExceptions = [];
  const loadingFailures = [];
  let failed = false;

  try {
    client = await createPageClient();
    client.on("Runtime.consoleAPICalled", (params) => {
      const rendered = (params.args ?? [])
        .map((arg) => arg.value ?? arg.description ?? arg.type ?? "")
        .join(" ");
      consoleMessages.push(`${params.type}: ${rendered}`.trim());
    });
    client.on("Runtime.exceptionThrown", (params) => {
      const detail = params.exceptionDetails;
      runtimeExceptions.push(
        JSON.stringify(
          {
            text: detail.text,
            url: detail.url,
            lineNumber: detail.lineNumber,
            columnNumber: detail.columnNumber,
            exception: detail.exception?.description ?? detail.exception?.value,
          },
          null,
          2,
        ),
      );
    });
    client.on("Network.loadingFailed", (params) => {
      loadingFailures.push(
        JSON.stringify(
          {
            type: params.type,
            errorText: params.errorText,
            blockedReason: params.blockedReason,
            canceled: params.canceled,
          },
          null,
          2,
        ),
      );
    });
    await navigate(client, WEB_URL);
    await waitForRootShell(client);
    await ensureActiveSession(client);
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"session-header\"]'))",
    );
    await waitForExpression(client, "Boolean(window.__agendaoWebDebug?.selectedSessionId)");
    const seeded = await seedRuntimeSurface(client);
    assert(seeded, "failed to inject runtime surface debug payload");
    await waitForExpression(
      client,
      "Boolean(document.querySelector('[data-testid=\"runtime-surface\"]'))",
    );

    const collapsed = await measureLayout(client);
    await click(client, "[data-testid='runtime-surface-toggle']");
    await waitForExpression(
      client,
      "document.querySelector('[data-testid=\"runtime-surface\"]')?.dataset.expanded === 'true'",
    );

    const expanded = await measureLayout(client);

    console.log("Phase B aesthetic audit summary");
    console.log(`1. runtime surface collapsed height: ${Math.round(collapsed.runtimeSurfaceHeight)}px`);
    console.log(`2. runtime surface expanded height: ${Math.round(expanded.runtimeSurfaceHeight)}px`);
    console.log(`   expanded content height: ${Math.round(expanded.runtimeSurfaceExpandedContentHeight ?? 0)}px`);
    console.log(`3. session header height: ${Math.round(collapsed.sessionHeaderHeight)}px`);
    console.log(`4. workspace header height: ${Math.round(collapsed.workspaceHeaderHeight)}px`);
    console.log(`5. central block count: ${collapsed.centralBlockCount}`);
    console.log(`   blocks: ${collapsed.centralBlocks.map((item) => `${item.key}:${item.height}px`).join(", ")}`);

    const failures = [];
    if (!(typeof collapsed.runtimeSurfaceHeight === "number" && collapsed.runtimeSurfaceHeight <= 44)) {
      failures.push(`runtime surface collapsed height > 44px (${collapsed.runtimeSurfaceHeight})`);
    }
    if (!(typeof expanded.runtimeSurfaceHeight === "number" && expanded.runtimeSurfaceHeight <= 240)) {
      failures.push(`runtime surface expanded height > 240px (${expanded.runtimeSurfaceHeight})`);
    }
    if (!(typeof collapsed.sessionHeaderHeight === "number" && collapsed.sessionHeaderHeight <= 128)) {
      failures.push(`session header height > 128px (${collapsed.sessionHeaderHeight})`);
    }
    if (!(typeof collapsed.workspaceHeaderHeight === "number" && collapsed.workspaceHeaderHeight <= 56)) {
      failures.push(`workspace header height > 56px (${collapsed.workspaceHeaderHeight})`);
    }
    if (!(typeof collapsed.centralBlockCount === "number" && collapsed.centralBlockCount <= 4)) {
      failures.push(`central block count too high (${collapsed.centralBlockCount})`);
    }
    if (failures.length > 0) {
      throw new Error(failures.join("; "));
    }
  } catch (error) {
    failed = true;
    throw error;
  } finally {
    if (failed && client) {
      try {
        const debugSnapshot = {
          href: await evaluate(client, "location.href"),
          title: await evaluate(client, "document.title"),
          readyState: await evaluate(client, "document.readyState"),
          bodySnippet: await evaluate(
            client,
            "document.body ? document.body.innerHTML.slice(0, 2000) : '(no body)'",
          ),
          rootShellState: await evaluate(
            client,
            `(() => ({
              viewport: { width: window.innerWidth, height: window.innerHeight },
              sessionSidebar: Boolean(document.querySelector('[data-testid="session-sidebar"]')),
              composerInput: Boolean(document.querySelector('[data-testid="composer-input"]')),
              workspacePanel: Boolean(document.querySelector('[data-testid="workspace-panel"]')),
              workspaceInspector: Boolean(document.querySelector('[data-testid="workspace-inspector"]')),
              sessionHeader: Boolean(document.querySelector('[data-testid="session-header"]')),
              sessionItems: document.querySelectorAll('[data-testid="session-item"]').length,
            }))()`,
          ),
          sessionHeaderBreakdown: await evaluate(
            client,
            `(() => {
              const header = document.querySelector('[data-testid="session-header"]');
              if (!header) return null;
              const children = Array.from(header.children).map((element, index) => ({
                index,
                className: element.className,
                text: element.textContent?.trim().slice(0, 240) ?? '',
                height: Math.round(element.getBoundingClientRect().height),
              }));
              return {
                className: header.className,
                height: Math.round(header.getBoundingClientRect().height),
                children,
              };
            })()`,
          ),
          webDebug: await evaluate(client, "window.__agendaoWebDebug ?? null"),
          tracker: await evaluate(client, "window.__agendaoTracker ?? null"),
          resources: await evaluate(
            client,
            "performance.getEntriesByType('resource').map((entry) => entry.name).slice(-40)",
          ),
        };
        console.error("Debug snapshot:", JSON.stringify(debugSnapshot, null, 2));
      } catch (error) {
        console.error(
          `Failed to capture debug snapshot: ${error instanceof Error ? error.message : String(error)}`,
        );
      }
      if (consoleMessages.length) {
        console.error("Console messages:");
        consoleMessages.forEach((message) => console.error(`  ${message}`));
      }
      if (runtimeExceptions.length) {
        console.error("Runtime exceptions:");
        runtimeExceptions.forEach((message) => console.error(`  ${message}`));
      }
      if (loadingFailures.length) {
        console.error("Loading failures:");
        loadingFailures.forEach((message) => console.error(`  ${message}`));
      }
    }
    if (client) client.close();
    await terminateProcess(chrome);
    await removeDirectoryWithRetry(userDataDir);
    const chromeStderr = stderr().trim();
    if (chrome.exitCode && chromeStderr) {
      console.error(chromeStderr);
    }
  }
}

run().catch((error) => {
  console.error(`Debug base url: ${BASE_URL}`);
  console.error(`Phase B aesthetic audit failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
