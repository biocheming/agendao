import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:4100";
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

async function fetchJson(url, init) {
  const response = await fetch(url, init);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}: ${await response.text()}`);
  }
  return response.json();
}

async function waitForHttp(url, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // retry
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for ${url}`);
}

async function waitFor(check, message, timeoutMs = TIMEOUT_MS) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await check()) {
      return;
    }
    await sleep(200);
  }
  throw new Error(message);
}

async function launchChrome() {
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-autoresearch-chrome-"));
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
  await waitForHttp(`http://127.0.0.1:${CHROME_PORT}/json/version`);
  return { chrome, userDataDir };
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
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
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

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function run() {
  const { chrome, userDataDir } = await launchChrome();
  let client = null;
  const consoleMessages = [];
  const runtimeExceptions = [];
  let failed = false;
  try {
    const seededSession = await fetchJson(`${BASE_URL}/session`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
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
    await navigate(client, `${BASE_URL}/`);
    await waitForExpression(
      client,
      "Boolean(document.querySelector('textarea[placeholder*=\"Send a prompt\"]'))",
    );
    const sessionId = seededSession.id;
    assert(sessionId, "could not resolve active session id");

    await fillInput(client, "textarea[placeholder*='Send a prompt']", "/autoresearch");
    await click(client, "[data-testid='composer-form'] button[type='submit']");
    await waitForExpression(
      client,
      `(window.__agendaoTracker?.fetches ?? []).filter((entry) =>
        entry.url.includes('/session/${sessionId}/prompt') && entry.method === 'POST'
      ).length >= 1`,
    );
    await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"question-overlay\"]'))");

    await fillInput(client, "[data-testid='question-input'][data-question-index='0']", "Exercise the browser command preflight and answer flow.");
    await fillInput(client, "[data-testid='question-input'][data-question-index='1']", "crates/agendao-server/**\napps/agendao-web/**");
    await fillInput(client, "[data-testid='question-input'][data-question-index='2']", "The frontend replies once and resubmits the command.");
    await fillInput(client, "[data-testid='question-input'][data-question-index='3']", "cargo build -p agendao-server");
    await click(client, "[data-testid='question-submit']");

    await waitForExpression(
      client,
      "(window.__agendaoTracker?.fetches ?? []).some((entry) => /\\/question\\/.+\\/reply$/.test(entry.url) && entry.method === 'POST')",
    );
    await waitForExpression(
      client,
      `(window.__agendaoTracker?.fetches ?? []).filter((entry) =>
        entry.url.includes('/session/${sessionId}/prompt') && entry.method === 'POST'
      ).length >= 2`,
    );
    await waitForExpression(client, "!document.querySelector('[data-testid=\"question-overlay\"]')");

    await waitFor(async () => {
      const session = await fetchJson(`${BASE_URL}/session/${sessionId}`);
      return !session?.metadata?.pending_command_invocation;
    }, "pending_command_invocation should be cleared after browser resubmit");

    console.log(`browser session ${sessionId}`);
  } catch (error) {
    failed = true;
    throw error;
  } finally {
    if (client && failed) {
      try {
        const snapshot = {
          href: await evaluate(client, "location.href"),
          title: await evaluate(client, "document.title"),
          readyState: await evaluate(client, "document.readyState"),
          body: await evaluate(
            client,
            "document.body ? document.body.innerHTML.slice(0, 1200) : '(no body)'",
          ),
          fetches: await evaluate(client, "window.__agendaoTracker?.fetches ?? []"),
        };
        console.error("Web smoke debug snapshot:", JSON.stringify(snapshot, null, 2));
      } catch {
        // best effort
      }
      if (consoleMessages.length) {
        console.error("Console messages:");
        consoleMessages.forEach((message) => console.error(`  ${message}`));
      }
      if (runtimeExceptions.length) {
        console.error("Runtime exceptions:");
        runtimeExceptions.forEach((message) => console.error(`  ${message}`));
      }
    }
    if (client) {
      client.close();
    }
    await terminateProcess(chrome);
    await rm(userDataDir, { recursive: true, force: true });
  }
}

run().catch((error) => {
  console.error(`Autoresearch web smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
