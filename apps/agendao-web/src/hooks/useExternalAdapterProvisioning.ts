import { useCallback } from "react";
import {
  type ProvisionExternalAdapterSessionRequestRecord,
  type ProvisionExternalAdapterSessionResponseRecord,
} from "../lib/session";
import type { SessionRecord } from "../lib/session";
import { normalizeSessionRecord } from "../lib/sidebar";
import type { WebExternalAdapterProvisioningRoute } from "../lib/webSessionUrl";

export interface ProvisioningCallbacks {
  apiJson: <T>(url: string, init?: RequestInit) => Promise<T>;
  onSessionReady: (session: SessionRecord, directory: string, replace: boolean) => void;
}

export function useExternalAdapterProvisioning(
  callbacks: ProvisioningCallbacks,
): (route: WebExternalAdapterProvisioningRoute, options?: { replace?: boolean }) => Promise<string> {
  const { apiJson, onSessionReady } = callbacks;

  return useCallback(
    async (
      route: WebExternalAdapterProvisioningRoute,
      options: { replace?: boolean } = {},
    ): Promise<string> => {
      const request: ProvisionExternalAdapterSessionRequestRecord = {
        adapter_id: route.adapterId,
        actor_id: route.actorId,
        workspace_id: route.workspaceId,
        route_policy_id: route.routePolicyId,
        scheduler_profile: route.schedulerProfile,
        directory: route.directory,
        project_id: route.projectId,
        title: route.title,
      };
      const provisioned =
        await apiJson<ProvisionExternalAdapterSessionResponseRecord>(
          "/external-adapter/session/provision",
          { method: "POST", body: JSON.stringify(request) },
        );
      const normalized = normalizeSessionRecord(provisioned.session);
      onSessionReady(
        normalized,
        normalized.directory?.trim() || request.directory?.trim() || "",
        options.replace ?? true,
      );
      return normalized.id;
    },
    [apiJson, onSessionReady],
  );
}
