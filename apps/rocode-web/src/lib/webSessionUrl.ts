export interface WebSessionRouteState {
  sessionId: string | null;
  messageId: string | null;
  highlightIds: string[];
}

function cleanId(value: string | null | undefined) {
  const trimmed = value?.trim() ?? "";
  return trimmed || null;
}

function cleanIdList(value: string | null | undefined) {
  const trimmed = value?.trim() ?? "";
  if (!trimmed) return [];
  return trimmed
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

export function readWebSessionRoute(): WebSessionRouteState {
  if (typeof window === "undefined") {
    return { sessionId: null, messageId: null, highlightIds: [] };
  }

  const params = new URLSearchParams(window.location.search);
  return {
    sessionId: cleanId(params.get("session")),
    messageId: cleanId(params.get("message")),
    highlightIds: cleanIdList(params.get("highlight")),
  };
}

export function buildWebSessionUrl(state: Partial<WebSessionRouteState>) {
  const url = new URL(window.location.href);
  const sessionId = state.sessionId === undefined ? cleanId(url.searchParams.get("session")) : cleanId(state.sessionId);
  const messageId = state.messageId === undefined ? cleanId(url.searchParams.get("message")) : cleanId(state.messageId);
  const highlightIds = state.highlightIds === undefined ? cleanIdList(url.searchParams.get("highlight")) : state.highlightIds.filter(Boolean);

  if (sessionId) url.searchParams.set("session", sessionId);
  else url.searchParams.delete("session");

  if (messageId) url.searchParams.set("message", messageId);
  else url.searchParams.delete("message");

  if (highlightIds.length > 0) url.searchParams.set("highlight", highlightIds.join(","));
  else url.searchParams.delete("highlight");

  return `${url.pathname}${url.search}${url.hash}`;
}

export function writeWebSessionRoute(
  state: Partial<WebSessionRouteState>,
  options: { replace?: boolean } = {},
) {
  const next = buildWebSessionUrl(state);
  const current = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (next === current) return;

  if (options.replace) {
    window.history.replaceState(null, "", next);
  } else {
    window.history.pushState(null, "", next);
  }
}
