import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const BASE_URL = process.env.ROCODE_BASE_URL ?? "http://127.0.0.1:4096";
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.ROCODE_CHROME_PORT ?? "9222", 10);
const TIMEOUT_MS = Number.parseInt(process.env.ROCODE_SMOKE_TIMEOUT_MS ?? "30000", 10);
const PROVIDER_SOAK_ENABLED = process.env.ROCODE_PROVIDER_SOAK === "1";
const PROVIDER_SOAK_TIMEOUT_MS = Number.parseInt(
  process.env.ROCODE_PROVIDER_SOAK_TIMEOUT_MS ?? "90000",
  10,
);
const PROVIDER_SOAK_PROMPT =
  process.env.ROCODE_PROVIDER_SOAK_PROMPT ??
  "Provide a detailed, structured analysis of the current repository migration status, with multiple sections and enough depth to exercise a longer streaming response.";

const trackerInitScript = `
(() => {
  const state = {
    fetches: [],
    sockets: [],
  };
  window.__rocodeTracker = state;

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

  const OriginalWebSocket = window.WebSocket;
  function TrackingWebSocket(url, protocols) {
    state.sockets.push(String(url));
    return new OriginalWebSocket(url, protocols);
  }
  TrackingWebSocket.prototype = OriginalWebSocket.prototype;
  Object.setPrototypeOf(TrackingWebSocket, OriginalWebSocket);
  window.WebSocket = TrackingWebSocket;
})();
`;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }
  return response.json();
}

async function postJson(url, payload) {
  return fetchJson(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "rocode-phase4-chrome-"));
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
    {
      stdio: ["ignore", "pipe", "pipe"],
    },
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

  const socket = new WebSocket(page.webSocketDebuggerUrl);
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
        if (nextHandlers.length) {
          listeners.set(method, nextHandlers);
        } else {
          listeners.delete(method);
        }
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

async function clickLast(client, selector) {
  const escaped = JSON.stringify(selector);
  const clicked = await evaluate(
    client,
    `(() => {
      const elements = Array.from(document.querySelectorAll(${escaped}));
      const element = elements.at(-1);
      if (!element) return false;
      element.click();
      return true;
    })()`,
  );
  if (!clicked) {
    throw new Error(`Could not find clickable selector ${selector}`);
  }
}

async function fillInput(client, selector, value) {
  const escapedSelector = JSON.stringify(selector);
  const escapedValue = JSON.stringify(value);
  const updated = await evaluate(
    client,
    `(() => {
      const element = document.querySelector(${escapedSelector});
      if (!element) return false;
      const prototype = element.tagName === 'TEXTAREA'
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
      const descriptor = Object.getOwnPropertyDescriptor(prototype, 'value');
      descriptor?.set?.call(element, ${escapedValue});
      element.focus();
      element.dispatchEvent(new Event('input', { bubbles: true }));
      element.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    })()`,
  );
  if (!updated) {
    throw new Error(`Could not find input selector ${selector}`);
  }
}

async function activeSessionId(client) {
  return evaluate(
    client,
    "document.querySelector('[data-testid=\"session-item\"].active')?.dataset.sessionId ?? null",
  );
}

async function waitForRootShell(client) {
  await waitForExpression(
    client,
    "Boolean(document.querySelector('[data-testid=\"session-sidebar\"]') && document.querySelector('[data-testid=\"composer-input\"]') && document.querySelector('[data-testid=\"workspace-inspector\"]'))",
  );
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

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function selectValue(client, selector, value) {
  const escapedSelector = JSON.stringify(selector);
  const escapedValue = JSON.stringify(value);
  const updated = await evaluate(
    client,
    `(() => {
      const element = document.querySelector(${escapedSelector});
      if (!element) return false;
      element.value = ${escapedValue};
      element.dispatchEvent(new Event('input', { bubbles: true }));
      element.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    })()`,
  );
  if (!updated) {
    throw new Error(`Could not find select selector ${selector}`);
  }
}

async function maybeRunProviderSoak(client, record) {
  if (!PROVIDER_SOAK_ENABLED) {
    return;
  }

  const providersData = await fetchJson(`${BASE_URL}/config/providers`);
  const providers = providersData.providers ?? providersData.all ?? [];
  const firstProvider = providers.find((provider) => Array.isArray(provider.models) && provider.models.length > 0);
  if (!firstProvider) {
    console.log("SKIP provider-soak: no configured provider models reported by /config/providers");
    return;
  }

  const modelId =
    process.env.ROCODE_PROVIDER_SOAK_MODEL ??
    `${firstProvider.id}/${firstProvider.models[0].id}`;

  await click(client, "[data-testid='settings-open']");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-drawer\"]'))");
  await click(client, "[data-testid='settings-tab-general']");
  await waitForExpression(
    client,
    "document.querySelector('[data-testid=\"settings-tab-general\"]')?.classList.contains('active') === true",
  );
  const hasModelSelector = await waitForOptionalExpression(
    client,
    "Boolean(document.querySelector('#settings-model-select'))",
    10000,
  );
  if (!hasModelSelector) {
    await click(client, "[data-testid='settings-close']");
    await waitForExpression(client, "!document.querySelector('[data-testid=\"settings-drawer\"]')");
    console.log("SKIP provider-soak: settings model selector is not available in the current environment");
    return;
  }

  const availableModelIds = await evaluate(
    client,
    `(() => {
      const select = document.querySelector('#settings-model-select');
      if (!select) return [];
      return Array.from(select.options).map((option) => option.value).filter(Boolean);
    })()`,
  );
  if (!Array.isArray(availableModelIds) || !availableModelIds.includes(modelId)) {
    await click(client, "[data-testid='settings-close']");
    await waitForExpression(client, "!document.querySelector('[data-testid=\"settings-drawer\"]')");
    console.log(`SKIP provider-soak: model option ${modelId} is not present in the settings selector`);
    return;
  }

  await selectValue(client, "#settings-model-select", modelId);
  await click(client, "[data-testid='settings-close']");
  await waitForExpression(client, "!document.querySelector('[data-testid=\"settings-drawer\"]')");

  const sessionId = await activeSessionId(client);
  assert(sessionId, "no active session for provider soak");
  const initialMessageCount = await evaluate(
    client,
    "document.querySelectorAll('[data-testid=\"message-card\"]').length",
  );

  await fillInput(client, "[data-testid='composer-input']", PROVIDER_SOAK_PROMPT);
  await click(client, "[data-testid='composer-send']");
  await waitForExpression(
    client,
    "document.querySelector('[data-testid=\"composer-send\"]')?.textContent?.includes('Streaming') === true",
    5000,
  );
  await waitForExpression(
    client,
    `(window.__rocodeTracker?.fetches ?? []).some((entry) =>
      entry.url.includes('/session/${sessionId}/stream') && entry.method === 'POST'
    )`,
    5000,
  );
  await waitForExpression(
    client,
    `document.querySelectorAll('[data-testid="message-card"]').length > ${initialMessageCount}`,
    PROVIDER_SOAK_TIMEOUT_MS,
  );
  await waitForExpression(
    client,
    "document.querySelector('[data-testid=\"composer-send\"]')?.textContent?.includes('Send') === true",
    PROVIDER_SOAK_TIMEOUT_MS,
  );
  record("provider-soak", `real provider stream completed with model ${modelId}`);
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;
  const checks = [];
  const consoleMessages = [];
  const runtimeExceptions = [];
  const loadingFailures = [];
  let failed = false;

  const record = (label, detail) => {
    checks.push({ label, detail });
    console.log(`PASS ${label}: ${detail}`);
  };

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
    await navigate(client, `${BASE_URL}/`);
    await waitForRootShell(client);
    record("root-shell", "new frontend rendered on /");

    const initialSessionCount = await evaluate(
      client,
      "document.querySelectorAll('[data-testid=\"session-item\"]').length",
    );
    assert(Number.isInteger(initialSessionCount), "session count did not resolve");
    record("session-list", `${initialSessionCount} session items rendered`);

    const noInitialPtyFetch = await evaluate(
      client,
      "!(window.__rocodeTracker?.fetches ?? []).some((entry) => entry.url.includes('/pty'))",
    );
    assert(noInitialPtyFetch, "terminal fetches started before expansion");
    record("terminal-deferred-fetch", "no /pty fetch before terminal expansion");

    const noInitialTerminalChunk = await evaluate(
      client,
      "!performance.getEntriesByType('resource').some((entry) => entry.name.includes('terminal-') || entry.name.includes('TerminalPanel-'))",
    );
    assert(noInitialTerminalChunk, "terminal chunk loaded before expansion");
    record("terminal-deferred-chunk", "no terminal chunk before expansion");

    await click(client, "[data-testid='settings-open']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-drawer\"]'))");
    await click(client, "[data-testid='settings-tab-providers']");
    await waitForExpression(
      client,
      "document.querySelector('[data-testid=\"settings-tab-providers\"]')?.classList.contains('active') === true",
    );
    record("settings-drawer", "settings drawer opened and providers tab activated");

    await click(client, "[data-testid='settings-close']");
    await waitForExpression(client, "!document.querySelector('[data-testid=\"settings-drawer\"]')");
    record("settings-close", "settings drawer closed cleanly");

    const previousActiveSessionId = await activeSessionId(client);
    await click(client, "[data-testid='session-new']");
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.fetches ?? []).some((entry) => entry.url.endsWith('/session') && entry.method === 'POST')",
    );
    record("session-create", "new session POST emitted from the new frontend");
    await waitForExpression(
      client,
      previousActiveSessionId
        ? `(() => {
            const active = document.querySelector('[data-testid="session-item"].active');
            return Boolean(active && active.dataset.sessionId && active.dataset.sessionId !== ${JSON.stringify(previousActiveSessionId)});
          })()`
        : "Boolean(document.querySelector('[data-testid=\"session-item\"].active'))",
    );
    const sessionId = await activeSessionId(client);
    assert(sessionId, "failed to resolve the newly created active session");

    await click(client, "[data-testid='terminal-open']");
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"terminal-panel\"]'))");
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.fetches ?? []).some((entry) => entry.url.includes('/pty'))",
    );
    record("terminal-expand", "terminal panel expanded and PTY fetch started");

    await click(client, "[data-testid='terminal-create']");
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.fetches ?? []).some((entry) => /\\/pty(?:\\?|$)/.test(entry.url) && entry.method === 'POST')",
    );
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.sockets ?? []).some((url) => url.includes('/pty/'))",
    );
    record("terminal-create", "PTY creation emitted POST /pty and opened PTY websocket");

    await postJson(`${BASE_URL}/experimental/frontend-smoke/question`, {
      session_id: sessionId,
      questions: [
        {
          question: "What should the smoke harness answer?",
        },
      ],
    });
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"question-overlay\"]'))");
    await fillInput(client, "[data-testid='question-input']", "smoke-answer");
    await click(client, "[data-testid='question-submit']");
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.fetches ?? []).some((entry) => /\\/question\\/.+\\/reply$/.test(entry.url) && entry.method === 'POST')",
    );
    await waitForExpression(client, "!document.querySelector('[data-testid=\"question-overlay\"]')");
    record("question-loop", "question overlay opened and submitted through the real reply route");

    await postJson(`${BASE_URL}/experimental/frontend-smoke/permission`, {
      session_id: sessionId,
      permission: "workspace.write",
      patterns: ["**/*.md"],
      description: "Smoke test permission request",
      command: "write_file",
      filepath: "smoke.md",
    });
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"permission-overlay\"]'))");
    await click(client, "[data-testid='permission-once']");
    await waitForExpression(
      client,
      "(window.__rocodeTracker?.fetches ?? []).some((entry) => /\\/permission\\/.+\\/reply$/.test(entry.url) && entry.method === 'POST')",
    );
    await waitForExpression(client, "!document.querySelector('[data-testid=\"permission-overlay\"]')");
    record("permission-loop", "permission overlay opened and replied through the real permission route");

    await fillInput(client, "[data-testid='composer-input']", "Smoke prompt from browser automation");
    await click(client, "[data-testid='composer-send']");
    await waitForExpression(
      client,
      `(window.__rocodeTracker?.fetches ?? []).some((entry) =>
        entry.url.includes('/session/${sessionId}/stream') && entry.method === 'POST'
      )`,
    );
    await waitForExpression(
      client,
      "document.querySelectorAll('[data-testid=\"message-card\"]').length >= 1",
    );
    record("prompt-submit", "composer submit issued POST /session/{id}/stream and rendered feed output");

    const sessionInfo = await fetchJson(`${BASE_URL}/session/${sessionId}`);
    const attachmentPath = `${sessionInfo.directory.replace(/\/+$/, "")}/smoke-attachment.txt`;
    await fetchJson(`${BASE_URL}/file/content`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        path: attachmentPath,
        content: "smoke attachment payload\nwith multiple lines",
      }),
    });

    await navigate(client, `${BASE_URL}/`);
    await waitForRootShell(client);
    await waitForExpression(
      client,
      `Boolean(document.querySelector('[data-testid="workspace-node"][data-path="${attachmentPath.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"]'))`,
    );
    await click(client, `[data-testid="workspace-node"][data-path="${attachmentPath}"]`);
    await click(client, "[data-testid='workspace-insert-reference']");
    await waitForExpression(
      client,
      "document.querySelector('[data-testid=\"composer-input\"]')?.value.includes('@smoke-attachment.txt') === true",
    );
    await click(client, "[data-testid='workspace-attach']");
    await waitForExpression(
      client,
      `Boolean(document.querySelector('[data-testid="context-attachment-chip"][data-workspace-path="${attachmentPath.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"]'))`,
    );
    await click(
      client,
      `[data-testid="context-attachment-main"]`,
    );
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"attachment-details\"]'))");
    await click(client, "[data-testid='attachment-locate']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="workspace-node"][data-path="${attachmentPath.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"")}"]')?.classList.contains('active') === true`,
    );
    record("attachment-workspace", "workspace file was inserted as @reference, attached as chip, and located back in the tree");

    const activeSessionAfterReload = await activeSessionId(client);
    assert(activeSessionAfterReload, "no active session after workspace reload");
    const childSession = await postJson(`${BASE_URL}/session`, {
      parent_id: activeSessionAfterReload,
    });
    await postJson(`${BASE_URL}/experimental/frontend-smoke/output-block`, {
      session_id: activeSessionAfterReload,
      id: "smoke-stage-block",
      block: {
        kind: "scheduler_stage",
        role: "assistant",
        id: "smoke-stage-block",
        stage_id: "stage-smoke",
        tool_call_id: "call_smoke_tool",
        title: "Smoke Child Session Stage",
        status: "running",
        profile: "smoke",
        child_session_id: childSession.id,
        focus: "Validate child-session jump from the message feed",
        text: "synthetic scheduler stage for frontend smoke",
      },
    });
    await postJson(`${BASE_URL}/experimental/frontend-smoke/output-block`, {
      session_id: activeSessionAfterReload,
      id: "call_smoke_tool",
      block: {
        kind: "tool",
        role: "assistant",
        id: "call_smoke_tool",
        stage_id: "stage-smoke",
        title: "Smoke Tool Call",
        text: "synthetic tool block for provenance jump coverage",
      },
    });
    await waitForExpression(
      client,
      `Boolean(document.querySelector('[data-testid="scheduler-stage-card"][data-child-session-id="${childSession.id}"]'))`,
    );
    await clickLast(client, "[data-testid='scheduler-stage-open-child']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="session-item"][data-session-id="${childSession.id}"]')?.classList.contains('active') === true`,
    );
    await waitForExpression(
      client,
      "document.querySelectorAll('[data-testid=\"session-breadcrumb\"]').length >= 2",
    );
    record("child-session-jump", "scheduler stage card opened the real child session and preserved breadcrumbs");

    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"provenance-tool\"]'))");
    await click(client, "[data-testid='provenance-tool']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="session-item"][data-session-id="${activeSessionAfterReload}"]')?.classList.contains('active') === true`,
    );
    await waitForExpression(
      client,
      "document.querySelector('[data-testid=\"message-card\"][data-block-id=\"call_smoke_tool\"]')?.classList.contains('focused') === true",
    );
    record("provenance-tool-jump", "child-session provenance tool link returned to the parent tool block and highlighted it");

    await maybeRunProviderSoak(client, record);

    console.log("");
    console.log("Phase 4 smoke summary");
    checks.forEach((check, index) => {
      console.log(`${index + 1}. ${check.label}: ${check.detail}`);
    });
  } catch (error) {
    failed = true;
    throw error;
  } finally {
    if (client && failed) {
      try {
        const debugSnapshot = {
          href: await evaluate(client, "location.href"),
          title: await evaluate(client, "document.title"),
          readyState: await evaluate(client, "document.readyState"),
          body: await evaluate(
            client,
            "document.body ? document.body.innerHTML.slice(0, 1200) : '(no body)'",
          ),
          fetches: await evaluate(client, "window.__rocodeTracker?.fetches ?? []"),
          resources: await evaluate(
            client,
            "performance.getEntriesByType('resource').map((entry) => entry.name)",
          ),
        };
        console.error("Debug snapshot:", JSON.stringify(debugSnapshot, null, 2));
      } catch (error) {
        console.error(`Failed to capture debug snapshot: ${error instanceof Error ? error.message : String(error)}`);
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
    if (client) {
      client.close();
    }
    await terminateProcess(chrome);
    await rm(userDataDir, { recursive: true, force: true });
    const chromeStderr = stderr().trim();
    if (chrome.exitCode && chromeStderr) {
      console.error(chromeStderr);
    }
  }
}

run().catch((error) => {
  console.error(`Phase 4 smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
