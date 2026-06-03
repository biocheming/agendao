import type { SessionRecord } from "./session";

export interface WorkspaceSummary {
  path: string;
  label: string;
  sessionCount: number;
  rootCount: number;
}

export interface SessionTreeNode {
  id: string;
  title?: string;
  directory?: string;
  updated?: number;
  children: SessionTreeNode[];
}

export function normalizeSessionRecord(session: SessionRecord): SessionRecord {
  return {
    ...session,
    title: session.title || "(untitled)",
    updated: session.time?.updated ?? session.updated ?? Date.now(),
  };
}

export function normalizeSessionRecords(sessions: SessionRecord[]): SessionRecord[] {
  return (sessions ?? [])
    .map(normalizeSessionRecord)
    .sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));
}

export function basenamePath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function buildWorkspaceSummaries(
  sessions: SessionRecord[],
  serviceRootPath: string,
): WorkspaceSummary[] {
  const workspaces = new Map<
    string,
    {
      path: string;
      label: string;
      sessionCount: number;
      rootCount: number;
      lastUpdated: number;
    }
  >();

  for (const session of sessions) {
    const path = session.directory?.trim();
    if (!path) continue;
    if (serviceRootPath && !path.startsWith(serviceRootPath)) continue;
    const current = workspaces.get(path);
    if (current) {
      current.sessionCount += 1;
      if (!session.parent_id) current.rootCount += 1;
      current.lastUpdated = Math.max(current.lastUpdated, session.updated ?? 0);
      continue;
    }
    workspaces.set(path, {
      path,
      label: basenamePath(path),
      sessionCount: 1,
      rootCount: session.parent_id ? 0 : 1,
      lastUpdated: session.updated ?? 0,
    });
  }

  return Array.from(workspaces.values())
    .sort((left, right) => right.lastUpdated - left.lastUpdated)
    .map(({ lastUpdated: _lastUpdated, ...workspace }) => workspace);
}

export function buildSessionTree(
  sessions: SessionRecord[],
  workspacePath: string | null,
): SessionTreeNode[] {
  if (!workspacePath) return [];
  const workspaceSessions = sessions.filter(
    (session) => session.directory?.trim() === workspacePath,
  );
  if (workspaceSessions.length === 0) return [];

  const sessionMap = new Map(workspaceSessions.map((session) => [session.id, session]));
  const childMap = new Map<string, SessionRecord[]>();

  for (const session of workspaceSessions) {
    if (!session.parent_id || !sessionMap.has(session.parent_id)) continue;
    const children = childMap.get(session.parent_id) ?? [];
    children.push(session);
    childMap.set(session.parent_id, children);
  }

  const sortByUpdated = (items: SessionRecord[]) =>
    items.slice().sort((left, right) => (right.updated ?? 0) - (left.updated ?? 0));

  const roots = sortByUpdated(
    workspaceSessions.filter((session) => !session.parent_id || !sessionMap.has(session.parent_id)),
  );

  const visit = (session: SessionRecord): SessionTreeNode => ({
    id: session.id,
    title: session.title,
    directory: session.directory,
    updated: session.updated,
    children: sortByUpdated(childMap.get(session.id) ?? []).map(visit),
  });

  return roots.map(visit);
}
