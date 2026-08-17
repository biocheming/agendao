import {
  resolveSetState,
  type AgendaoState,
  type AppNotice,
  type NoticeTone,
  type StoreGet,
  type StoreSet,
} from "./types";

const MAX_NOTICES = 5;

function inferNoticeTone(message: string): NoticeTone {
  if (/\b(fail(?:ed)?|error|invalid|cannot|unable|timed out)\b/i.test(message)) {
    return "error";
  }
  if (
    /^(copied|deleted|renamed|forked|exported|saved|connected|stopped|created|applied|added|removed)\b/i.test(
      message,
    )
  ) {
    return "success";
  }
  return "info";
}

let nextNoticeId = 1;

export function createUiSlice(
  set: StoreSet,
  get: StoreGet,
): Pick<
  AgendaoState,
  | "route"
  | "leftSidebarOpen"
  | "rightSidebarOpen"
  | "terminalOpen"
  | "banner"
  | "notices"
  | "setRoute"
  | "setLeftSidebarOpen"
  | "setRightSidebarOpen"
  | "setTerminalOpen"
  | "setBanner"
  | "pushNotice"
  | "dismissNotice"
  | "clearBanner"
> {
  const pushNotice = (message: string, tone?: NoticeTone) => {
    const notice: AppNotice = {
      id: nextNoticeId++,
      tone: tone ?? inferNoticeTone(message),
      message,
      at: Date.now(),
    };
    const notices = [...get().notices, notice].slice(-MAX_NOTICES);
    set({ notices, banner: notice.message });
  };

  return {
    route: "workbench",
    leftSidebarOpen: true,
    rightSidebarOpen: true,
    terminalOpen: false,
    banner: null,
    notices: [],

    setRoute: (route) =>
      set({ route: resolveSetState(route, get().route) }),
    setLeftSidebarOpen: (leftSidebarOpen) =>
      set({ leftSidebarOpen: resolveSetState(leftSidebarOpen, get().leftSidebarOpen) }),
    setRightSidebarOpen: (rightSidebarOpen) =>
      set({ rightSidebarOpen: resolveSetState(rightSidebarOpen, get().rightSidebarOpen) }),
    setTerminalOpen: (terminalOpen) =>
      set({ terminalOpen: resolveSetState(terminalOpen, get().terminalOpen) }),
    // Legacy single-banner surface: null clears everything, a message pushes
    // a notice so history is never silently overwritten.
    setBanner: (message, tone) => {
      const resolved = resolveSetState(message, get().banner);
      if (!resolved) {
        set({ notices: [], banner: null });
        return;
      }
      pushNotice(resolved, tone);
    },
    pushNotice: (message, tone) => pushNotice(message, tone),
    dismissNotice: (id) => {
      const notices = get().notices.filter((notice) => notice.id !== id);
      set({ notices, banner: notices.at(-1)?.message ?? null });
    },
    clearBanner: () => set({ notices: [], banner: null }),
  };
}
