const API_BASE_QUERY_PARAM = "api_base_url";
const API_BASE_STORAGE_KEY = "agendao.api-base-url";
const SERVER_PASSWORD_QUERY_PARAM = "server_password";
const SERVER_PASSWORD_STORAGE_KEY = "agendao.server-password";

function hasWindow(): boolean {
  return typeof window !== "undefined" && typeof window.location !== "undefined";
}

function normalizeBaseUrl(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) return null;

  try {
    if (hasWindow()) {
      return new URL(trimmed, window.location.origin).toString().replace(/\/+$/, "");
    }
    return new URL(trimmed).toString().replace(/\/+$/, "");
  } catch {
    return null;
  }
}

function readQueryApiBaseUrl(): string | null {
  if (!hasWindow()) return null;
  const search = new URLSearchParams(window.location.search);
  return normalizeBaseUrl(search.get(API_BASE_QUERY_PARAM));
}

function readEnvApiBaseUrl(): string | null {
  return normalizeBaseUrl(import.meta.env.VITE_AGENDAO_API_BASE_URL);
}

function readStoredApiBaseUrl(): string | null {
  if (!hasWindow()) return null;
  try {
    return normalizeBaseUrl(window.localStorage.getItem(API_BASE_STORAGE_KEY));
  } catch {
    return null;
  }
}

function persistApiBaseUrl(value: string | null): void {
  if (!hasWindow()) return;
  try {
    if (value) {
      window.localStorage.setItem(API_BASE_STORAGE_KEY, value);
    } else {
      window.localStorage.removeItem(API_BASE_STORAGE_KEY);
    }
  } catch {
    // Ignore localStorage failures; the app can still fall back to same-origin requests.
  }
}

function normalizeServerPassword(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}

function readQueryServerPassword(): string | null {
  if (!hasWindow()) return null;
  const search = new URLSearchParams(window.location.search);
  return normalizeServerPassword(search.get(SERVER_PASSWORD_QUERY_PARAM));
}

function readEnvServerPassword(): string | null {
  return normalizeServerPassword(import.meta.env.VITE_AGENDAO_SERVER_PASSWORD);
}

function readStoredServerPassword(): string | null {
  if (!hasWindow()) return null;
  try {
    return normalizeServerPassword(window.localStorage.getItem(SERVER_PASSWORD_STORAGE_KEY));
  } catch {
    return null;
  }
}

function persistServerPassword(value: string | null): void {
  if (!hasWindow()) return;
  try {
    if (value) {
      window.localStorage.setItem(SERVER_PASSWORD_STORAGE_KEY, value);
    } else {
      window.localStorage.removeItem(SERVER_PASSWORD_STORAGE_KEY);
    }
  } catch {
    // Ignore localStorage failures; callers can still pass the password in the URL.
  }
}

export function currentServerPassword(): string | null {
  const queryValue = readQueryServerPassword();
  if (queryValue) {
    persistServerPassword(queryValue);
    return queryValue;
  }

  const envValue = readEnvServerPassword();
  if (envValue) return envValue;

  const storedValue = readStoredServerPassword();
  if (storedValue) return storedValue;

  return null;
}

export function currentApiBaseUrl(): string | null {
  const queryValue = readQueryApiBaseUrl();
  if (queryValue) {
    persistApiBaseUrl(queryValue);
    return queryValue;
  }

  const envValue = readEnvApiBaseUrl();
  if (envValue) return envValue;

  const storedValue = readStoredApiBaseUrl();
  if (storedValue) return storedValue;

  return null;
}

export function apiUrl(path: string): string {
  const serverPassword = currentServerPassword();
  if (/^[a-z]+:\/\//i.test(path)) {
    if (!serverPassword) return path;
    const url = new URL(path);
    if (!url.searchParams.has(SERVER_PASSWORD_QUERY_PARAM)) {
      url.searchParams.set(SERVER_PASSWORD_QUERY_PARAM, serverPassword);
    }
    return url.toString();
  }

  const baseUrl = currentApiBaseUrl();
  if (!baseUrl) {
    if (!serverPassword) return path;
    const url = new URL(path, hasWindow() ? window.location.origin : "http://127.0.0.1");
    url.searchParams.set(SERVER_PASSWORD_QUERY_PARAM, serverPassword);
    return `${url.pathname}${url.search}${url.hash}`;
  }

  const url = new URL(path, `${baseUrl}/`);
  if (serverPassword && !url.searchParams.has(SERVER_PASSWORD_QUERY_PARAM)) {
    url.searchParams.set(SERVER_PASSWORD_QUERY_PARAM, serverPassword);
  }
  return url.toString();
}

export function webSocketUrl(path: string): string {
  const resolved = apiUrl(path);
  const url = hasWindow()
    ? new URL(resolved, window.location.origin)
    : new URL(resolved);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

export async function api(path: string, options: RequestInit = {}): Promise<Response> {
  const headers = new Headers(options.headers);
  if (!headers.has("Content-Type") && options.body) {
    headers.set("Content-Type", "application/json");
  }
  const serverPassword = currentServerPassword();
  if (serverPassword && !headers.has("Authorization") && !headers.has("X-AGENDAO-Server-Password")) {
    headers.set("Authorization", `Bearer ${serverPassword}`);
  }
  const response = await fetch(apiUrl(path), { ...options, headers });
  if (!response.ok) {
    throw new Error(await response.text());
  }
  return response;
}

export async function apiJson<T>(path: string, options: RequestInit = {}): Promise<T> {
  const response = await api(path, options);
  return response.json() as Promise<T>;
}

export async function parseSSE(
  response: Response,
  onEvent: (eventName: string, data: unknown) => void,
): Promise<void> {
  if (!response.body) return;
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let eventName: string | null = null;
  let dataLines: string[] = [];

  const flush = () => {
    if (dataLines.length === 0) {
      eventName = null;
      return;
    }
    const data = dataLines.join("\n");
    dataLines = [];
    let parsed: unknown;
    try {
      parsed = JSON.parse(data);
    } catch {
      parsed = { raw: data };
    }
    onEvent(eventName ?? "message", parsed);
    eventName = null;
  };

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    for (const rawLine of lines) {
      const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
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
}

export function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Unknown error";
}
