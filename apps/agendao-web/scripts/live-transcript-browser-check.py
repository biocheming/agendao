#!/usr/bin/env python3

import asyncio
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

import websockets


BASE_URL = os.environ.get("AGENDAO_BASE_URL", "http://127.0.0.1:3000")
CHROME_BIN = os.environ.get("CHROME_BIN", "google-chrome")
CHROME_PORT = int(os.environ.get("AGENDAO_CHROME_PORT", "9224"))
TIMEOUT_MS = int(os.environ.get("AGENDAO_SMOKE_TIMEOUT_MS", "60000"))
WORKSPACE_DIR = os.environ.get(
    "AGENDAO_WORKSPACE_DIR", "/home/biocheming/tests/python/rust/test/life"
)
PROMPT = os.environ.get(
    "AGENDAO_BROWSER_PROMPT",
    "用文献检索的skill，检索中国海洋大学徐锡明2021-2026年发表的论文",
)


def sleep_ms(ms: int) -> None:
    time.sleep(ms / 1000)


def fetch_json(url: str, data: bytes | None = None, headers: dict[str, str] | None = None):
    request = urllib.request.Request(url, data=data, headers=headers or {})
    with urllib.request.urlopen(request, timeout=TIMEOUT_MS / 1000) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_for_http(url: str, timeout_ms: int = TIMEOUT_MS) -> None:
    deadline = time.time() + timeout_ms / 1000
    last_error = None
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if 200 <= response.status < 300:
                    return
                last_error = RuntimeError(f"HTTP {response.status}")
        except Exception as error:  # noqa: BLE001
            last_error = error
        sleep_ms(250)
    raise RuntimeError(f"Timed out waiting for {url}: {last_error}")


def launch_chrome():
    user_data_dir = tempfile.mkdtemp(prefix="agendao-live-check-chrome-")
    process = subprocess.Popen(
        [
            CHROME_BIN,
            f"--remote-debugging-port={CHROME_PORT}",
            "--headless=new",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--no-first-run",
            "--no-default-browser-check",
            "--no-sandbox",
            f"--user-data-dir={user_data_dir}",
            "about:blank",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_for_http(f"http://127.0.0.1:{CHROME_PORT}/json/version")
    return process, user_data_dir


def terminate_process(process: subprocess.Popen | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=2)


class CdpClient:
    def __init__(self, websocket):
        self.websocket = websocket
        self.next_id = 0
        self.pending: dict[int, asyncio.Future] = {}
        self.listeners: dict[str, list] = {}
        self.reader_task = asyncio.create_task(self._reader())

    async def _reader(self):
        async for raw_message in self.websocket:
            payload = json.loads(raw_message)
            if "id" in payload:
                future = self.pending.pop(payload["id"], None)
                if future is None:
                    continue
                if "error" in payload:
                    future.set_exception(RuntimeError(payload["error"]))
                else:
                    future.set_result(payload.get("result", {}))
                continue

            handlers = self.listeners.get(payload.get("method", ""), [])
            for handler in handlers:
                handler(payload.get("params", {}))

    async def send(self, method: str, params: dict | None = None):
        self.next_id += 1
        request_id = self.next_id
        future = asyncio.get_running_loop().create_future()
        self.pending[request_id] = future
        await self.websocket.send(
            json.dumps({"id": request_id, "method": method, "params": params or {}})
        )
        return await future

    def on(self, method: str, handler):
        handlers = self.listeners.setdefault(method, [])
        handlers.append(handler)

        def unsubscribe():
            current = self.listeners.get(method, [])
            if handler in current:
                current.remove(handler)
            if not current and method in self.listeners:
                del self.listeners[method]

        return unsubscribe

    async def close(self):
        self.reader_task.cancel()
        await self.websocket.close()


async def create_page_client() -> CdpClient:
    pages = fetch_json(f"http://127.0.0.1:{CHROME_PORT}/json/list")
    page = next((entry for entry in pages if entry.get("type") == "page"), None)
    if not page or not page.get("webSocketDebuggerUrl"):
        raise RuntimeError("Could not find a Chrome page target")

    websocket = await websockets.connect(page["webSocketDebuggerUrl"], max_size=10_000_000)
    client = CdpClient(websocket)
    await client.send("Page.enable")
    await client.send("Runtime.enable")
    await client.send("Network.enable")
    return client


async def evaluate(client: CdpClient, expression: str):
    result = await client.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": True,
        },
    )
    return result.get("result", {}).get("value")


async def wait_for_expression(client: CdpClient, expression: str, timeout_ms: int = TIMEOUT_MS):
    deadline = time.time() + timeout_ms / 1000
    while time.time() < deadline:
        value = await evaluate(client, expression)
        if value:
            return value
        await asyncio.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for expression: {expression}")


async def navigate(client: CdpClient, url: str):
    page_loaded = asyncio.get_running_loop().create_future()

    def on_load(_params):
        if not page_loaded.done():
            page_loaded.set_result(True)

    unsubscribe = client.on("Page.loadEventFired", on_load)
    try:
        await client.send("Page.navigate", {"url": url})
        await page_loaded
        await wait_for_expression(client, "document.readyState === 'complete'")
    finally:
        unsubscribe()


async def install_sse_probe(client: CdpClient):
    await client.send(
        "Page.addScriptToEvaluateOnNewDocument",
        {
            "source": """
(() => {
  const originalFetch = window.fetch.bind(window);
  window.__agendaoSseCapture = [];

  function pushCapture(entry) {
    try {
      window.__agendaoSseCapture.push({
        at: Date.now(),
        ...entry,
      });
      if (window.__agendaoSseCapture.length > 5000) {
        window.__agendaoSseCapture.splice(0, window.__agendaoSseCapture.length - 5000);
      }
    } catch {}
  }

  function parseSseStream(stream, url) {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    let eventName = null;
    let dataLines = [];

    const flush = () => {
      if (dataLines.length === 0) {
        eventName = null;
        return;
      }
      const raw = dataLines.join("\\n");
      dataLines = [];
      let payload = null;
      try {
        payload = JSON.parse(raw);
      } catch {
        payload = { raw };
      }
      const type = payload && typeof payload === "object" ? payload.type : null;
      if (type === "output_block" || type === "session.updated" || type === "session.status") {
        const block = payload.block && typeof payload.block === "object" ? payload.block : null;
        pushCapture({
          url,
          eventName,
          type,
          session_id: payload.session_id ?? payload.sessionID ?? null,
          live_identity: payload.live_identity ?? block?.live_identity ?? null,
          block: block
            ? {
                id: block.id ?? null,
                kind: block.kind ?? null,
                phase: block.phase ?? null,
                text: typeof block.text === "string" ? block.text : null,
                detail: typeof block.detail === "string" ? block.detail.slice(0, 160) : null,
                name: typeof block.name === "string" ? block.name : null,
                title: typeof block.title === "string" ? block.title : null,
                ts: block.ts ?? null,
              }
            : null,
        });
      }
      eventName = null;
    };

    (async () => {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\\n");
        buffer = lines.pop() ?? "";
        for (const rawLine of lines) {
          const line = rawLine.endsWith("\\r") ? rawLine.slice(0, -1) : rawLine;
          if (!line) {
            flush();
            continue;
          }
          if (line.startsWith("event:")) {
            eventName = line.slice(6).trim();
          } else if (line.startsWith("data:")) {
            dataLines.push(line.slice(5).trimStart());
          }
        }
      }
      flush();
    })().catch((error) => {
      pushCapture({ url, type: "probe.error", error: String(error) });
    });
  }

  window.fetch = async (...args) => {
    const response = await originalFetch(...args);
    try {
      const target = args[0];
      const url =
        typeof target === "string"
          ? target
          : target && typeof target === "object" && "url" in target
            ? String(target.url)
            : "";
      if (url.includes("/event?tier=web") && response.body) {
        parseSseStream(response.clone().body, url);
      }
    } catch {}
    return response;
  };
})();
""",
        },
    )


async def click(client: CdpClient, selector: str):
    clicked = await evaluate(
        client,
        f"""(() => {{
          const element = document.querySelector({json.dumps(selector)});
          if (!element) return false;
          element.click();
          return true;
        }})()""",
    )
    if not clicked:
        raise RuntimeError(f"Could not find clickable selector {selector}")


async def fill_input(client: CdpClient, selector: str, value: str):
    updated = await evaluate(
        client,
        f"""(() => {{
          const element = document.querySelector({json.dumps(selector)});
          if (!element) return false;
          const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')
            ?? Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value');
          descriptor?.set?.call(element, {json.dumps(value)});
          element.focus();
          element.dispatchEvent(new Event('input', {{ bubbles: true }}));
          element.dispatchEvent(new Event('change', {{ bubbles: true }}));
          return true;
        }})()""",
    )
    if not updated:
        raise RuntimeError(f"Could not find input selector {selector}")


async def collect_message_cards(client: CdpClient):
    return await evaluate(
        client,
        """(() => Array.from(document.querySelectorAll('[data-testid="message-card"], [data-testid="transcript-block"]')).map((node, index) => ({
          index,
          kind: node.getAttribute('data-kind'),
          blockId: node.getAttribute('data-block-id'),
          anchorId: node.getAttribute('data-message-anchor'),
          text: node.innerText,
          html: node.innerHTML,
        })))()""",
    )


async def collect_page_state(client: CdpClient):
    return await evaluate(
        client,
        """(() => {
          const textarea = document.querySelector('textarea[placeholder="Ask AgenDao"]');
          const submit = document.querySelector('[data-testid="composer-form"] button[type="submit"]');
          const sessions = Array.from(document.querySelectorAll('[data-testid="session-item"]')).map((node) => ({
            id: node.getAttribute('data-session-id'),
            active: node.getAttribute('data-active'),
            text: node.innerText,
          }));
          return {
            url: window.location.href,
            textareaDisabled: textarea ? textarea.disabled : null,
            textareaValue: textarea ? textarea.value : null,
            submitDisabled: submit ? submit.disabled : null,
            sessionCount: sessions.length,
            activeSessions: sessions.filter((node) => node.active === 'true'),
            firstSessions: sessions.slice(0, 5),
          };
        })()""",
    )


async def collect_sse_capture(client: CdpClient):
    return await evaluate(
        client,
        """(() => Array.isArray(window.__agendaoSseCapture) ? window.__agendaoSseCapture : [])()""",
    )


async def collect_debug_state(client: CdpClient):
    return await evaluate(
        client,
        """(() => window.__agendaoWebDebug ?? null)()""",
    )


async def main():
    seeded_session = fetch_json(
        f"{BASE_URL}/session",
        data=json.dumps(
            {
                "directory": WORKSPACE_DIR,
                "title": f"browser-check-{int(time.time() * 1000)}",
            }
        ).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )

    chrome = None
    user_data_dir = None
    client = None
    try:
        chrome, user_data_dir = launch_chrome()
        client = await create_page_client()
        await install_sse_probe(client)
        await navigate(client, f"{BASE_URL}/?session={seeded_session['id']}")
        await wait_for_expression(
            client,
            "Boolean(document.querySelector('textarea[placeholder=\"Ask AgenDao\"]'))",
        )
        await wait_for_expression(
            client,
            "document.querySelectorAll('[data-testid=\"session-item\"]').length > 0",
        )
        state_before = await collect_page_state(client)
        print("=== before submit ===", flush=True)
        print(json.dumps(state_before, ensure_ascii=False, indent=2), flush=True)
        await fill_input(client, "textarea[placeholder='Ask AgenDao']", PROMPT)
        await click(client, "[data-testid='composer-form'] button[type='submit']")
        await wait_for_expression(
            client,
            "document.querySelectorAll('[data-testid=\"message-card\"]').length > 0",
        )

        checkpoints_ms = [3000, 8000, 15000, 25000]
        previous = 0
        for checkpoint in checkpoints_ms:
            await asyncio.sleep((checkpoint - previous) / 1000)
            previous = checkpoint
            cards = await collect_message_cards(client)
            sse_events = await collect_sse_capture(client)
            debug_state = await collect_debug_state(client)
            print(f"\n=== {checkpoint}ms ===", flush=True)
            for card in cards:
                print(f"--- card {card['index']} ---", flush=True)
                print(
                    f"kind={card.get('kind')} blockId={card.get('blockId')} anchorId={card.get('anchorId')}",
                    flush=True,
                )
                print(card["text"], flush=True)
            relevant_events = [
                event
                for event in sse_events
                if event.get("session_id") == seeded_session["id"]
            ]
            print(f"--- sse events ({len(relevant_events)}) ---", flush=True)
            print("--- debug state ---", flush=True)
            print(json.dumps(debug_state, ensure_ascii=False, indent=2), flush=True)
            assistant_events = [
                event
                for event in relevant_events
                if (event.get("block") or {}).get("kind") in {"message", "reasoning"}
            ]
            print(f"--- assistant/reasoning sse ({len(assistant_events)}) ---", flush=True)
            for event in assistant_events[-20:]:
                print(json.dumps(event, ensure_ascii=False), flush=True)
            print(f"--- recent sse ({min(12, len(relevant_events))}) ---", flush=True)
            for event in relevant_events[-12:]:
                print(json.dumps(event, ensure_ascii=False), flush=True)
    finally:
        if client is not None:
            await client.close()
        terminate_process(chrome)
        if user_data_dir:
            shutil.rmtree(user_data_dir, ignore_errors=True)


if __name__ == "__main__":
    asyncio.run(main())
