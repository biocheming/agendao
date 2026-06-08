import { useAgendaoStore } from "../store";

const storeInitialState = useAgendaoStore.getInitialState();

function cloneSessionBreadcrumbs() {
  return storeInitialState.sessionBreadcrumbs.map((crumb) => ({ ...crumb }));
}

function cloneRuntimeSurfaceBySession() {
  return Object.fromEntries(
    Object.entries(storeInitialState.runtimeSurfaceBySession).map(([key, value]) => [
      key,
      {
        ...value,
        sessionEvents: [...value.sessionEvents],
        inspectItems: [...value.inspectItems],
        queueItems: [...value.queueItems],
      },
    ]),
  );
}

function cloneWorkspaceNodeLoading() {
  return { ...storeInitialState.workspaceNodeLoading };
}

export function resetAgendaoStore() {
  useAgendaoStore.setState(
    {
      ...storeInitialState,
      selectedMessageIds: new Set<string>(),
      sessionBreadcrumbs: cloneSessionBreadcrumbs(),
      runtimeSurfaceBySession: cloneRuntimeSurfaceBySession(),
      workspaceNodeLoading: cloneWorkspaceNodeLoading(),
    },
    true,
  );
}

export function resetBrowserRoute(url = "/web/") {
  window.history.replaceState(null, "", url);
}
