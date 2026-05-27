import { useCallback, useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type {
  ProvisionExternalAdapterSessionResponseRecord,
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
  type ThemeId,
} from "../lib/webRuntime";

interface UseWebBootstrapOptions {
  apiJson: <T>(path: string, options?: RequestInit) => Promise<T>;
  fetchSessions: () => Promise<SessionRecord[]>;
  formatError: (error: unknown) => string;
  preferencesReadyRef: MutableRefObject<boolean>;
  provisionExternalAdapterSession: (
    route: WebExternalAdapterProvisioningRoute,
    options?: { replace?: boolean },
  ) => Promise<string>;
  setBanner: Dispatch<SetStateAction<string | null>>;
  setConnectProtocol: Dispatch<SetStateAction<string>>;
  setConnectProtocols: Dispatch<SetStateAction<ConnectProtocolOption[]>>;
  setKnownProviders: Dispatch<SetStateAction<KnownProviderEntry[]>>;
  setModes: Dispatch<SetStateAction<ExecutionMode[]>>;
  setProviders: Dispatch<SetStateAction<ProviderRecord[]>>;
  setSelectedMode: Dispatch<SetStateAction<string>>;
  setSelectedModel: Dispatch<SetStateAction<string>>;
  setSelectedSessionId: Dispatch<SetStateAction<string | null>>;
  setServiceRootPath: Dispatch<SetStateAction<string>>;
  setSessions: Dispatch<SetStateAction<SessionRecord[]>>;
  setShowThinking: Dispatch<SetStateAction<boolean>>;
  setTheme: Dispatch<SetStateAction<ThemeId>>;
  setWorkspaceContext: Dispatch<SetStateAction<WorkspaceContextRecord | null>>;
}

interface ConfigSurfaceData {
  providers: ProviderRecord[];
  knownProviders: KnownProviderEntry[];
  connectProtocols: ConnectProtocolOption[];
  modes: ExecutionMode[];
  workspaceContext: WorkspaceContextRecord | null;
}

function visibleModes(modeData: ExecutionMode[] | null | undefined): ExecutionMode[] {
  return (modeData ?? [])
    .filter((mode) => mode.hidden !== true)
    .filter((mode) => mode.kind !== "agent" || mode.mode !== "subagent");
}

export function useWebBootstrap({
  apiJson,
  fetchSessions,
  formatError,
  preferencesReadyRef,
  provisionExternalAdapterSession,
  setBanner,
  setConnectProtocol,
  setConnectProtocols,
  setKnownProviders,
  setModes,
  setProviders,
  setSelectedMode,
  setSelectedModel,
  setSelectedSessionId,
  setServiceRootPath,
  setSessions,
  setShowThinking,
  setTheme,
  setWorkspaceContext,
}: UseWebBootstrapOptions) {
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
      setProviders(data.providers);
      setKnownProviders(data.knownProviders);
      setConnectProtocols(data.connectProtocols);
      setModes(data.modes);
      if (data.workspaceContext) {
        setWorkspaceContext(data.workspaceContext);
        setServiceRootPath((current) => workspaceRootFromContext(data.workspaceContext) || current);
      }
      if (!options.includePreferences || !data.workspaceContext) {
        return;
      }
      const prefs = applyPreferences(data.workspaceContext.config ?? {});
      setTheme(THEMES.some((item) => item.id === prefs.theme) ? prefs.theme : "daylight");
      setSelectedMode(prefs.mode || DEFAULT_WEB_MODE);
      setSelectedModel(prefs.model);
      setShowThinking(prefs.showThinking);
      setConnectProtocol((current) => current || data.connectProtocols[0]?.id || "");
    },
    [
      setConnectProtocol,
      setConnectProtocols,
      setKnownProviders,
      setModes,
      setProviders,
      setSelectedMode,
      setSelectedModel,
      setServiceRootPath,
      setShowThinking,
      setTheme,
      setWorkspaceContext,
    ],
  );

  const reloadCoreSettingsData = useCallback(async () => {
    try {
      const data = await loadConfigSurface(true);
      applyConfigSurface(data, { includePreferences: true });
    } catch (error) {
      setBanner(`Failed to refresh config data: ${formatError(error)}`);
    }
  }, [applyConfigSurface, formatError, loadConfigSurface, setBanner]);

  const reloadProvidersAndModes = useCallback(() => {
    void (async () => {
      try {
        const data = await loadConfigSurface(false);
        applyConfigSurface(data, { includePreferences: false });
      } catch (error) {
        setBanner(`Failed to refresh config data: ${formatError(error)}`);
      }
    })();
  }, [applyConfigSurface, formatError, loadConfigSurface, setBanner]);

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
              setBanner(`Failed to provision external adapter session: ${formatError(error)}`);
            }
          }
        }

        const [sessionData, configData, paths] = await Promise.all([
          fetchSessions(),
          loadConfigSurface(true),
          apiJson<PathsResponseRecord>("/path"),
        ]);

        if (cancelled) return;

        setServiceRootPath(workspaceRootFromContext(configData.workspaceContext) || paths.cwd || "");
        setSessions(sessionData);
        applyConfigSurface(configData, { includePreferences: true });

        const routeSessionExists = Boolean(
          routeSessionId && sessionData.some((session) => session.id === routeSessionId),
        );
        setSelectedSessionId(
          (current) =>
            current
            ?? (routeSessionProvisioned || routeSessionExists
              ? routeSessionId
              : sessionData[0]?.id ?? null),
        );
        preferencesReadyRef.current = true;
      } catch (error) {
        if (!cancelled) {
          setBanner(`Bootstrap failed: ${formatError(error)}`);
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
    fetchSessions,
    formatError,
    loadConfigSurface,
    preferencesReadyRef,
    provisionExternalAdapterSession,
    setBanner,
    setSelectedSessionId,
    setServiceRootPath,
    setSessions,
  ]);

  return {
    reloadCoreSettingsData,
    reloadProvidersAndModes,
  };
}
