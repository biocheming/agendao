import { useCallback, useEffect, type MutableRefObject } from "react";
import type {
  SessionListResponseRecord,
  SessionRecord,
} from "../lib/session";
import type {
  ConfigProvidersResponseRecord,
  ConnectProtocolOption,
  KnownProviderEntry,
  ProviderConnectSchemaResponseRecord,
  ProviderRecord,
} from "../lib/provider";
import type { PathsResponseRecord, WorkspaceContextRecord } from "../lib/workspace";
import { workspaceRootFromContext } from "../lib/workspace";
import { readWebSessionRoute, type WebExternalAdapterProvisioningRoute } from "../lib/webSessionUrl";
import {
  applyPreferences,
  DEFAULT_WEB_MODE,
  THEMES,
  type ExecutionMode,
} from "../lib/webRuntime";
import { normalizeSessionRecords } from "../lib/sidebar";
import { useAgendaoStore } from "../store";

interface UseWebBootstrapOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  formatError: (error: unknown) => string;
  preferencesReadyRef: MutableRefObject<boolean>;
  provisionExternalAdapterSession: (
    route: WebExternalAdapterProvisioningRoute,
    options?: { replace?: boolean },
  ) => Promise<string>;
}

interface ConfigSurfaceData {
  providers: ProviderRecord[];
  knownProviders: KnownProviderEntry[];
  connectProtocols: ConnectProtocolOption[];
  modes: ExecutionMode[];
  workspaceContext: WorkspaceContextRecord | null;
}

interface CoreBootstrapData {
  sessionData: SessionRecord[];
  paths: PathsResponseRecord;
}

function isSessionWithinWorkspace(session: SessionRecord, workspaceRoot: string) {
  const directory = session.directory?.trim() ?? "";
  const root = workspaceRoot.trim();
  if (!directory || !root) return false;
  return directory === root || directory.startsWith(`${root}/`);
}

function visibleModes(modeData: ExecutionMode[] | null | undefined): ExecutionMode[] {
  return (modeData ?? [])
    .filter((mode) => mode.hidden !== true)
    .filter((mode) => mode.kind !== "agent" || mode.mode !== "subagent");
}

export function useWebBootstrap({
  apiJson,
  formatError,
  preferencesReadyRef,
  provisionExternalAdapterSession,
}: UseWebBootstrapOptions) {
  const loadCoreBootstrapData = useCallback(async (): Promise<CoreBootstrapData> => {
    const [sessionData, paths] = await Promise.all([
      apiJson<SessionListResponseRecord>("/session?limit=500").then((response) =>
        normalizeSessionRecords(response?.items ?? []),
      ),
      apiJson<PathsResponseRecord>("/path"),
    ]);
    return { sessionData, paths };
  }, [apiJson]);

  const loadConfigSurface = useCallback(
    async (includeWorkspaceContext: boolean): Promise<ConfigSurfaceData> => {
      const [providersData, modeData, connectSchema, context] = await Promise.all([
        apiJson<ConfigProvidersResponseRecord>("/config/providers"),
        apiJson<ExecutionMode[]>("/mode"),
        apiJson<ProviderConnectSchemaResponseRecord>("/provider/connect/schema"),
        includeWorkspaceContext
          ? apiJson<WorkspaceContextRecord>("/workspace/context")
          : Promise.resolve(null),
      ]);
      return {
        providers: providersData.providers ?? providersData.all ?? [],
        knownProviders: connectSchema.providers ?? [],
        connectProtocols: connectSchema.protocols ?? [],
        modes: visibleModes(modeData),
        workspaceContext: context,
      };
    },
    [apiJson],
  );

  const applyConfigSurface = useCallback(
    (data: ConfigSurfaceData, options: { includePreferences: boolean }) => {
      const store = useAgendaoStore.getState();
      store.setProviders(data.providers);
      store.setKnownProviders(data.knownProviders);
      store.setConnectProtocols(data.connectProtocols);
      store.setModes(data.modes);
      if (data.workspaceContext) {
        store.setWorkspaceContext(data.workspaceContext);
        const currentRoot = store.serviceRootPath;
        store.setServiceRootPath(workspaceRootFromContext(data.workspaceContext) || currentRoot);
      }
      if (!options.includePreferences || !data.workspaceContext) {
        return;
      }
      const prefs = applyPreferences(data.workspaceContext.config ?? {});
      store.setTheme(THEMES.some((item) => item.id === prefs.theme) ? prefs.theme : "daylight");
      store.setSelectedMode(prefs.mode || DEFAULT_WEB_MODE);
      store.setSelectedModel(prefs.model);
      store.setShowThinking(prefs.showThinking);
      const currentProtocol = store.connectProtocols[0]?.id || "";
      if (currentProtocol && !store.connectProtocols.some((p) => p.id === currentProtocol)) {
        // keep current if already set, otherwise default to first
      }
    },
    [],
  );

  const reloadCoreSettingsData = useCallback(async () => {
    try {
      const data = await loadConfigSurface(true);
      applyConfigSurface(data, { includePreferences: true });
    } catch (error) {
      useAgendaoStore.getState().setBanner(
        `Failed to refresh config data: ${formatError(error)}`,
      );
    }
  }, [applyConfigSurface, formatError, loadConfigSurface]);

  const reloadProvidersAndModes = useCallback(() => {
    void (async () => {
      try {
        const data = await loadConfigSurface(false);
        applyConfigSurface(data, { includePreferences: false });
      } catch (error) {
        useAgendaoStore.getState().setBanner(
          `Failed to refresh config data: ${formatError(error)}`,
        );
      }
    })();
  }, [applyConfigSurface, formatError, loadConfigSurface]);

  useEffect(() => {
    let cancelled = false;

    const loadBootstrap = async () => {
      try {
        const initialRoute = readWebSessionRoute();
        let routeSessionId = initialRoute.sessionId;
        let routeSessionProvisioned = false;
        if (!routeSessionId && initialRoute.externalProvisioning) {
          try {
            routeSessionId = await provisionExternalAdapterSession(
              initialRoute.externalProvisioning,
              { replace: true },
            );
            routeSessionProvisioned = true;
          } catch (error) {
            if (!cancelled) {
              useAgendaoStore.getState().setBanner(
                `Failed to provision external adapter session: ${formatError(error)}`,
              );
            }
          }
        }

        const [coreBootstrapResult, configSurfaceResult] = await Promise.allSettled([
          loadCoreBootstrapData(),
          loadConfigSurface(true),
        ]);

        if (cancelled) return;
        if (coreBootstrapResult.status !== "fulfilled") {
          throw coreBootstrapResult.reason;
        }

        const store = useAgendaoStore.getState();
        const configData =
          configSurfaceResult.status === "fulfilled" ? configSurfaceResult.value : null;
        const serviceRootPath =
          workspaceRootFromContext(configData?.workspaceContext ?? null) ||
          coreBootstrapResult.value.paths.cwd ||
          "";
        store.setServiceRootPath(serviceRootPath);
        store.setSessions(coreBootstrapResult.value.sessionData);
        if (configData) {
          applyConfigSurface(configData, { includePreferences: true });
        } else if (configSurfaceResult.status === "rejected") {
          store.setBanner(`Config surface degraded: ${formatError(configSurfaceResult.reason)}`);
        }

        const workspaceSessions = coreBootstrapResult.value.sessionData.filter((session: SessionRecord) =>
          isSessionWithinWorkspace(session, serviceRootPath),
        );

        const routeSessionExists = Boolean(
          routeSessionId &&
            coreBootstrapResult.value.sessionData.some(
              (session: SessionRecord) => session.id === routeSessionId,
            ),
        );
        const currentId = store.selectedSessionId;
        const currentSessionExists = Boolean(
          currentId &&
            coreBootstrapResult.value.sessionData.some(
              (session: SessionRecord) => session.id === currentId,
            ),
        );
        const currentSessionWithinWorkspace = Boolean(
          currentId &&
            coreBootstrapResult.value.sessionData.some(
              (session: SessionRecord) =>
                session.id === currentId &&
                isSessionWithinWorkspace(session, serviceRootPath),
            ),
        );
        const defaultWorkspaceSessionId = workspaceSessions[0]?.id ?? null;
        store.setSelectedSessionId(
          routeSessionProvisioned || routeSessionExists
            ? routeSessionId
            : currentSessionExists && currentSessionWithinWorkspace
              ? currentId
              : defaultWorkspaceSessionId,
        );
        preferencesReadyRef.current = true;
      } catch (error) {
        if (!cancelled) {
          useAgendaoStore.getState().setBanner(
            `Bootstrap failed: ${formatError(error)}`,
          );
        }
      }
    };

    void loadBootstrap();
    return () => {
      cancelled = true;
    };
  }, [
    apiJson,
    applyConfigSurface,
    formatError,
    loadCoreBootstrapData,
    loadConfigSurface,
    preferencesReadyRef,
    provisionExternalAdapterSession,
  ]);

  return {
    reloadCoreSettingsData,
    reloadProvidersAndModes,
  };
}
