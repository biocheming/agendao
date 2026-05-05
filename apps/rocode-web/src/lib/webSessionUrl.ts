export interface WebSessionRouteState {
  sessionId: string | null;
  messageId: string | null;
  highlightIds: string[];
  externalProvisioning: WebExternalAdapterProvisioningRoute | null;
}

export interface WebExternalAdapterProvisioningRoute {
  adapterId: string;
  actorId: string;
  workspaceId: string | null;
  routePolicyId: string | null;
  schedulerProfile: string | null;
  directory: string | null;
  projectId: string | null;
  title: string | null;
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

function cleanText(value: string | null | undefined) {
  const trimmed = value?.trim() ?? "";
  return trimmed || null;
}

function readExternalProvisioning(
  params: URLSearchParams,
): WebExternalAdapterProvisioningRoute | null {
  const adapterId = cleanText(params.get("external_adapter"));
  const actorId = cleanText(params.get("external_actor_id"));
  if (!adapterId || !actorId) return null;

  return {
    adapterId,
    actorId,
    workspaceId: cleanText(params.get("external_workspace_id")),
    routePolicyId: cleanText(params.get("external_route_policy_id")),
    schedulerProfile: cleanText(params.get("external_scheduler_profile")),
    directory: cleanText(params.get("external_directory")),
    projectId: cleanText(params.get("external_project_id")),
    title: cleanText(params.get("external_title")),
  };
}

function writeExternalProvisioning(
  params: URLSearchParams,
  provisioning: WebExternalAdapterProvisioningRoute | null,
) {
  const keys = [
    "external_adapter",
    "external_actor_id",
    "external_workspace_id",
    "external_route_policy_id",
    "external_scheduler_profile",
    "external_directory",
    "external_project_id",
    "external_title",
  ];
  for (const key of keys) params.delete(key);
  if (!provisioning) return;

  params.set("external_adapter", provisioning.adapterId);
  params.set("external_actor_id", provisioning.actorId);
  if (provisioning.workspaceId) {
    params.set("external_workspace_id", provisioning.workspaceId);
  }
  if (provisioning.routePolicyId) {
    params.set("external_route_policy_id", provisioning.routePolicyId);
  }
  if (provisioning.schedulerProfile) {
    params.set("external_scheduler_profile", provisioning.schedulerProfile);
  }
  if (provisioning.directory) {
    params.set("external_directory", provisioning.directory);
  }
  if (provisioning.projectId) {
    params.set("external_project_id", provisioning.projectId);
  }
  if (provisioning.title) {
    params.set("external_title", provisioning.title);
  }
}

export function readWebSessionRoute(): WebSessionRouteState {
  if (typeof window === "undefined") {
    return {
      sessionId: null,
      messageId: null,
      highlightIds: [],
      externalProvisioning: null,
    };
  }

  const params = new URLSearchParams(window.location.search);
  return {
    sessionId: cleanId(params.get("session")),
    messageId: cleanId(params.get("message")),
    highlightIds: cleanIdList(params.get("highlight")),
    externalProvisioning: readExternalProvisioning(params),
  };
}

export function buildWebSessionUrl(state: Partial<WebSessionRouteState>) {
  const url = new URL(window.location.href);
  const sessionId = state.sessionId === undefined ? cleanId(url.searchParams.get("session")) : cleanId(state.sessionId);
  const messageId = state.messageId === undefined ? cleanId(url.searchParams.get("message")) : cleanId(state.messageId);
  const highlightIds = state.highlightIds === undefined ? cleanIdList(url.searchParams.get("highlight")) : state.highlightIds.filter(Boolean);
  const externalProvisioning =
    state.externalProvisioning === undefined
      ? readExternalProvisioning(url.searchParams)
      : state.externalProvisioning;

  if (sessionId) url.searchParams.set("session", sessionId);
  else url.searchParams.delete("session");

  if (messageId) url.searchParams.set("message", messageId);
  else url.searchParams.delete("message");

  if (highlightIds.length > 0) url.searchParams.set("highlight", highlightIds.join(","));
  else url.searchParams.delete("highlight");

  writeExternalProvisioning(url.searchParams, externalProvisioning ?? null);

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
