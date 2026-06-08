import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { WebSocket as UndiciWebSocket } from "undici";

const RuntimeWebSocket = globalThis.WebSocket ?? UndiciWebSocket;

const BASE_URL = process.env.AGENDAO_BASE_URL ?? "http://127.0.0.1:3000";
const WEB_URL = new URL("/web/", `${BASE_URL}/`).toString();
const CHROME_BIN = process.env.CHROME_BIN ?? "google-chrome";
const CHROME_PORT = Number.parseInt(process.env.AGENDAO_CHROME_PORT ?? "9230", 10);
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
  const userDataDir = await mkdtemp(path.join(tmpdir(), "agendao-settings-config-smoke-"));
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
  await client.send("Input.dispatchMouseEvent", {
    type: "mouseMoved",
    x,
    y,
    button: "none",
  });
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

async function pressKey(client, key, code, windowsVirtualKeyCode, modifiers = 0) {
  await client.send("Input.dispatchKeyEvent", {
    type: "keyDown",
    key,
    code,
    windowsVirtualKeyCode,
    nativeVirtualKeyCode: windowsVirtualKeyCode,
    modifiers,
  });
  await client.send("Input.dispatchKeyEvent", {
    type: "keyUp",
    key,
    code,
    windowsVirtualKeyCode,
    nativeVirtualKeyCode: windowsVirtualKeyCode,
    modifiers,
  });
}

async function fillInput(client, selector, value) {
  await click(client, selector);
  await pressKey(client, "a", "KeyA", 65, 2);
  await pressKey(client, "Backspace", "Backspace", 8);
  if (value) {
    await client.send("Input.insertText", { text: value });
  }
}

async function selectValue(client, selector, value) {
  const escapedSelector = JSON.stringify(selector);
  const state = await evaluate(
    client,
    `(() => {
      const element = document.querySelector(${escapedSelector});
      if (!(element instanceof HTMLSelectElement)) return null;
      return {
        currentIndex: element.selectedIndex,
        targetIndex: Array.from(element.options).findIndex((option) => option.value === ${JSON.stringify(value)}),
      };
    })()`,
  );
  if (!state || state.targetIndex < 0) {
    throw new Error(`Could not find select option ${value} for ${selector}`);
  }
  if (state.currentIndex === state.targetIndex) return;
  await click(client, selector);
  const direction = state.targetIndex > state.currentIndex ? 1 : -1;
  const steps = Math.abs(state.targetIndex - state.currentIndex);
  for (let index = 0; index < steps; index += 1) {
    if (direction > 0) {
      await pressKey(client, "ArrowDown", "ArrowDown", 40);
    } else {
      await pressKey(client, "ArrowUp", "ArrowUp", 38);
    }
  }
  await pressKey(client, "Tab", "Tab", 9);
}

async function checked(client, selector) {
  return evaluate(client, `Boolean(document.querySelector(${JSON.stringify(selector)})?.checked)`);
}

async function valueOf(client, selector) {
  return evaluate(client, `document.querySelector(${JSON.stringify(selector)})?.value ?? null`);
}

async function activeTheme(client) {
  return evaluate(
    client,
    `document.querySelector('[data-testid^="settings-theme-"][data-active="true"]')?.dataset.testid?.replace('settings-theme-', '') ?? null`,
  );
}

async function openSettings(client) {
  await click(client, "[data-testid='settings-open']");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-drawer\"]'))");
}

async function openSettingsTab(client, tabId) {
  await click(client, `[data-testid="settings-tab-${tabId}"]`);
  await waitForExpression(
    client,
    `document.querySelector('[data-testid="settings-tab-${tabId}"]')?.dataset.active === "true"`,
  );
}

async function waitForGeneralReady(client) {
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-general\"]'))");
  await waitForExpression(client, "Boolean(document.querySelector('#settings-mode-select'))");
  await waitForExpression(client, "Boolean(document.querySelector('#settings-model-select'))");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-show-thinking\"]'))");
}

async function waitForProvidersReady(client) {
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-providers\"]'))");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-provider-id\"]'))");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-provider-api-key\"]'))");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-provider-submit\"]'))");
}

async function waitForSchedulerReady(client) {
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-panel-scheduler\"]'))");
  await waitForExpression(client, "Boolean(document.querySelector('#settings-scheduler-content'))");
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-scheduler-save\"]'))");
}

async function reloadWorkbench(client) {
  await navigate(client, WEB_URL);
  await waitForExpression(client, "Boolean(document.querySelector('[data-testid=\"settings-open\"]'))");
}

async function run() {
  const { chrome, userDataDir, stderr } = await launchChrome();
  let client = null;
  let step = "boot";

  try {
    client = await createPageClient();
    step = "reload workbench";
    await reloadWorkbench(client);

    step = "open settings general";
    await openSettings(client);
    await openSettingsTab(client, "general");
    await waitForGeneralReady(client);

    const initialTheme = await activeTheme(client);
    const initialMode = (await valueOf(client, "#settings-mode-select")) ?? "";
    const initialModel = (await valueOf(client, "#settings-model-select")) ?? "";
    const initialThinking = await checked(client, "[data-testid='settings-show-thinking']");

    const themeOptions = await evaluate(
      client,
      `Array.from(document.querySelectorAll('[data-testid^="settings-theme-"]')).map((element) =>
        element.dataset.testid?.replace('settings-theme-', '')
      ).filter(Boolean)`,
    );
    const nextTheme = themeOptions.find((value) => value !== initialTheme) ?? initialTheme;

    const modeOptions = await evaluate(
      client,
      `Array.from(document.querySelector('#settings-mode-select')?.options ?? []).map((option) => option.value)`,
    );
    const nextMode =
      modeOptions.find((value) => value && value !== initialMode) ??
      initialMode;

    const modelOptions = await evaluate(
      client,
      `Array.from(document.querySelector('#settings-model-select')?.options ?? []).map((option) => option.value).filter(Boolean)`,
    );
    const nextModel =
      modelOptions.find((value) => value !== initialModel) ?? initialModel;

    step = "mutate general preferences";
    if (nextTheme !== initialTheme) {
      await click(client, `[data-testid="settings-theme-${nextTheme}"]`);
    }
    if (nextMode !== initialMode) {
      await selectValue(client, "#settings-mode-select", nextMode);
    }
    if (nextModel && nextModel !== initialModel) {
      await selectValue(client, "#settings-model-select", nextModel);
    }
    await click(client, "[data-testid='settings-show-thinking']");
    if (nextTheme !== initialTheme) {
      await waitForExpression(
        client,
        `document.querySelector('[data-testid="settings-theme-${nextTheme}"]')?.dataset.active === "true"`,
      );
    }
    await waitForExpression(
      client,
      `document.querySelector('#settings-mode-select')?.value === ${JSON.stringify(nextMode)}`,
    );
    if (nextModel && nextModel !== initialModel) {
      await waitForExpression(
        client,
        `document.querySelector('#settings-model-select')?.value === ${JSON.stringify(nextModel)}`,
      );
    }
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-show-thinking"]')?.checked === ${String(!initialThinking)}`,
    );
    await sleep(900);

    step = "reload and verify general persistence";
    await reloadWorkbench(client);
    await openSettings(client);
    await openSettingsTab(client, "general");
    await waitForGeneralReady(client);

    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-theme-${nextTheme}"]')?.dataset.active === "true"`,
    );
    await waitForExpression(
      client,
      `document.querySelector('#settings-mode-select')?.value === ${JSON.stringify(nextMode)}`,
      10000,
    );
    if (nextModel && nextModel !== initialModel) {
      await waitForExpression(
        client,
        `document.querySelector('#settings-model-select')?.value === ${JSON.stringify(nextModel)}`,
        10000,
      );
    }
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-show-thinking"]')?.checked === ${String(!initialThinking)}`,
      10000,
    );

    step = "restore general preferences";
    if (initialTheme !== nextTheme) {
      await click(client, `[data-testid="settings-theme-${initialTheme}"]`);
    }
    if (initialMode !== nextMode) {
      await selectValue(client, "#settings-mode-select", initialMode);
    }
    if (nextModel && nextModel !== initialModel) {
      await selectValue(client, "#settings-model-select", initialModel);
    }
    await click(client, "[data-testid='settings-show-thinking']");
    await sleep(900);

    step = "providers reopen workbench";
    await reloadWorkbench(client);
    await openSettings(client);

    step = "providers tab click";
    await click(client, `[data-testid="settings-tab-providers"]`);
    step = "providers tab active";
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-tab-providers"]')?.dataset.active === "true"`,
    );
    step = "providers ready";
    await waitForProvidersReady(client);
    const providerId = `phase22-smoke-${Date.now()}`;
    await fillInput(client, "[data-testid='settings-provider-id']", providerId);
    await fillInput(client, "[data-testid='settings-provider-api-key']", "phase22-smoke-key");
    await fillInput(client, "[data-testid='settings-provider-base-url']", "https://example.invalid/v1");
    const currentProtocol = (await valueOf(client, "[data-testid='settings-provider-protocol']")) ?? "";
    if (!currentProtocol) {
      throw new Error("provider protocol selector had no current value");
    }
    step = "providers connect";
    await click(client, "[data-testid='settings-provider-submit']");
    await waitForExpression(client, `Boolean(document.querySelector('[data-testid="settings-provider-row-${providerId}"]'))`, 45000);
    step = "providers refresh";
    await click(client, "[data-testid='settings-provider-refresh']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-provider-refresh"]')?.textContent?.includes('Refreshing') ?? false`,
      10000,
    ).catch(() => null);
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-provider-refresh"]')?.textContent?.includes('Refresh Catalogue') ?? false`,
      20000,
    );
    step = "providers remove";
    await click(client, `[data-testid="settings-provider-remove-${providerId}"]`);
    await waitForExpression(client, `!document.querySelector('[data-testid="settings-provider-row-${providerId}"]')`, 45000);

    step = "scheduler tab click";
    await click(client, `[data-testid="settings-tab-scheduler"]`);
    step = "scheduler tab active";
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-tab-scheduler"]')?.dataset.active === "true"`,
    );
    step = "scheduler ready";
    await waitForSchedulerReady(client);
    const originalSchedulerContent = String((await valueOf(client, "#settings-scheduler-content")) ?? "");
    const marker = `# phase22-smoke-${Date.now()}`;
    const mutatedSchedulerContent = `${originalSchedulerContent.trimEnd()}\n${marker}\n`;
    await fillInput(client, "#settings-scheduler-content", mutatedSchedulerContent);
    await click(client, "[data-testid='settings-scheduler-save']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-scheduler-save"]')?.textContent?.includes('Saving') ?? false`,
      10000,
    ).catch(() => null);
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-scheduler-save"]')?.textContent?.includes('Save Scheduler Config') ?? false`,
      20000,
    );
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-feedback"]')?.textContent?.includes('Scheduler config saved.') ?? false`,
    );

    step = "scheduler persistence verify";
    await reloadWorkbench(client);
    await openSettings(client);
    await openSettingsTab(client, "scheduler");
    await waitForSchedulerReady(client);
    const persistedSchedulerContent = String((await valueOf(client, "#settings-scheduler-content")) ?? "");
    assert(persistedSchedulerContent.includes(marker), "scheduler content did not persist after reload");

    await fillInput(client, "#settings-scheduler-content", originalSchedulerContent);
    await click(client, "[data-testid='settings-scheduler-save']");
    await waitForExpression(
      client,
      `document.querySelector('[data-testid="settings-scheduler-save"]')?.textContent?.includes('Save Scheduler Config') ?? false`,
      20000,
    );

    console.log("settings general/providers/scheduler smoke passed");
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
  console.error(`Settings general/providers/scheduler smoke failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
