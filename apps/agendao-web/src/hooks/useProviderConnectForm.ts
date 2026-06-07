import { type Dispatch, type SetStateAction, useEffect, useRef, useState } from "react";
import type { ConnectProtocolOption, ResolveProviderConnectResponseRecord } from "../lib/provider";

export interface ConnectFormState {
  query: string;
  providerId: string;
  protocol: string;
  apiKey: string;
  baseUrl: string;
  resolution: ResolveProviderConnectResponseRecord | null;
  resolveBusy: boolean;
  resolveError: string | null;
  busy: boolean;
}

export interface ConnectFormActions {
  setQuery: Dispatch<SetStateAction<string>>;
  setProviderId: Dispatch<SetStateAction<string>>;
  setProtocol: Dispatch<SetStateAction<string>>;
  setApiKey: Dispatch<SetStateAction<string>>;
  setBaseUrl: Dispatch<SetStateAction<string>>;
  setBusy: Dispatch<SetStateAction<boolean>>;
}

export function useProviderConnectForm(
  connectProtocols: ConnectProtocolOption[],
  apiJson: <T>(url: string, init?: RequestInit) => Promise<T>,
  formatError: (error: unknown) => string,
): [ConnectFormState, ConnectFormActions] {
  const [query, setQuery] = useState("");
  const [providerId, setProviderId] = useState("");
  const [protocol, setProtocol] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [resolution, setResolution] =
    useState<ResolveProviderConnectResponseRecord | null>(null);
  const [resolveBusy, setResolveBusy] = useState(false);
  const [resolveError, setResolveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const resolveRequestRef = useRef(0);

  // Debounced provider connect resolution.
  useEffect(() => {
    const trimmed = query.trim();
    if (!trimmed) {
      resolveRequestRef.current += 1;
      setResolveBusy(false);
      setResolveError(null);
      setResolution(null);
      return;
    }

    const requestId = resolveRequestRef.current + 1;
    resolveRequestRef.current = requestId;
    const timer = window.setTimeout(() => {
      setResolveBusy(true);
      setResolveError(null);
      void (async () => {
        try {
          const response = await apiJson<ResolveProviderConnectResponseRecord>(
            "/provider/connect/resolve",
            { method: "POST", body: JSON.stringify({ query: trimmed }) },
          );
          if (resolveRequestRef.current !== requestId) return;
          setResolution(response);
          setProviderId(response.draft.provider_id);
          setBaseUrl(response.draft.base_url ?? "");
          setProtocol(
            response.draft.protocol ?? connectProtocols[0]?.id ?? "openai",
          );
        } catch (error) {
          if (resolveRequestRef.current !== requestId) return;
          setResolution(null);
          setResolveError(formatError(error));
        } finally {
          if (resolveRequestRef.current === requestId) {
            setResolveBusy(false);
          }
        }
      })();
    }, 120);

    return () => window.clearTimeout(timer);
  }, [apiJson, connectProtocols, query, formatError]);

  return [
    { query, providerId, protocol, apiKey, baseUrl, resolution, resolveBusy, resolveError, busy },
    { setQuery, setProviderId, setProtocol, setApiKey, setBaseUrl, setBusy } as ConnectFormActions,
  ];
}
