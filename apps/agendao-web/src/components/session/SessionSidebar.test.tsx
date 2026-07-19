import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n/I18nProvider";
import { resetAgendaoStore } from "../../test/store-test-utils";
import type { SessionTreeNode } from "../../lib/sidebar";
import { SessionSidebar } from "./SessionSidebar";

const sessionTree: SessionTreeNode[] = [
  { id: "s1", title: "Alpha session", children: [] },
];

function renderSidebar(overrides: Partial<Parameters<typeof SessionSidebar>[0]> = {}) {
  const props: Parameters<typeof SessionSidebar>[0] = {
    workspaces: [],
    currentWorkspacePath: "/tmp/workspace",
    currentWorkspaceLabel: "workspace",
    currentWorkspaceRootPath: "/tmp",
    currentWorkspaceMode: "shared",
    sessionTree,
    selectedSessionId: "s1",
    onCreateProject: vi.fn<(input: { path: string; title?: string }) => void>(),
    onCreateSession: vi.fn<() => void>(),
    onDeleteSessions: vi.fn<(sessionIds: string[]) => void>(),
    onExportSession: vi.fn<(sessionId: string) => void>(),
    onRenameSession: vi.fn<(sessionId: string, title: string) => void>(),
    onSelectWorkspace: vi.fn<(workspacePath: string) => void>(),
    onSelectSession: vi.fn<(sessionId: string) => void>(),
    onHideSidebar: vi.fn<() => void>(),
    ...overrides,
  };
  render(
    <I18nProvider>
      <SessionSidebar {...props} />
    </I18nProvider>,
  );
  return props;
}

describe("SessionSidebar inline rename", () => {
  beforeEach(() => {
    resetAgendaoStore();
  });

  it("prefills the current title and saves on Enter", () => {
    const props = renderSidebar();

    fireEvent.click(screen.getByTestId("session-rename-s1"));

    const input = screen.getByTestId("session-rename-input");
    expect(input).toHaveValue("Alpha session");

    fireEvent.change(input, { target: { value: "Beta session" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(props.onRenameSession).toHaveBeenCalledTimes(1);
    expect(props.onRenameSession).toHaveBeenCalledWith("s1", "Beta session");
    expect(screen.queryByTestId("session-rename-input")).toBeNull();
  });

  it("cancels on Escape without calling onRenameSession", () => {
    const props = renderSidebar();

    fireEvent.click(screen.getByTestId("session-rename-s1"));
    const input = screen.getByTestId("session-rename-input");
    fireEvent.change(input, { target: { value: "Discarded" } });
    fireEvent.keyDown(input, { key: "Escape" });

    expect(props.onRenameSession).not.toHaveBeenCalled();
    expect(screen.queryByTestId("session-rename-input")).toBeNull();
    expect(screen.getByTestId("session-item")).toHaveTextContent("Alpha session");
  });
});
