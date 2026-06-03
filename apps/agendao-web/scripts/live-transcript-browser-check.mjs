import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:3000";
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9224", 10);
const TIMEOUT_MS = Number.parseInt(process.env.AGENDAO_SMOKE_TIMEOUT_MS ?? "30000", 10);
const WORKSPACE_DIR =
  process.env.AGENDAO_WORKSPACE_DIR ?? "/home/biocheming/tests/python/rust/test/life";
const PROMPT =
  process.env.AGENDAO_BROWSER_PROMPT ??
  "用文献检索的skill，检索中国海洋大学徐锡明2021-2026年发表的论文";

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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-live-check-chrome-"));
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

async function fillInput(client, selector, value) {
  const escapedSelector = JSON.stringify(selector);
  const escapedValue = JSON.stringify(value);
  const updated = await evaluate(
    client,
    `(() => {
      const element = document.querySelector(${escapedSelector});
      if (!element) return false;
      const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')
        ?? Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
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

async function collectMessageCards(client) {
  return evaluate(
    client,
    `(() => Array.from(document.querySelectorAll('[data-testid="message-card"]')).map((node, index) => ({
      index,
      text: node.innerText,
    })))()`,
  );
}

async function run() {
  const seededSession = await fetchJson(`${BASE_URL}/session`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      directory: WORKSPACE_DIR,
      title: `browser-check-${Date.now()}`,
    }),
  });

  const { chrome, userDataDir } = await launchChrome();
  let client = null;

  try {
    client = await createPageClient();
    await navigate(client, `${BASE_URL}/`);
    await waitForExpression(
      client,
      "Boolean(document.querySelector('textarea[placeholder*=\"Send a prompt\"]'))",
    );

    await click(client, `[data-testid="session-item"][data-session-id="${seededSession.id}"]`);
    await fillInput(client, "textarea[placeholder*='Send a prompt']", PROMPT);
    await click(client, "[data-testid='composer-form'] button[type='submit']");

    const checkpointsMs = [3000, 8000, 15000, 25000];
    let previous = 0;
    for (const checkpoint of checkpointsMs) {
      await sleep(checkpoint - previous);
      previous = checkpoint;
      const cards = await collectMessageCards(client);
      console.log(`\n=== ${checkpoint}ms ===`);
      for (const card of cards) {
        console.log(`--- card ${card.index} ---`);
        console.log(card.text);
      }
    }
  } finally {
    if (client) client.close();
    await terminateProcess(chrome);
    await rm(userDataDir, { recursive: true, force: true });
  }
}

run().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : String(error));
  process.exitCode = 1;
});
