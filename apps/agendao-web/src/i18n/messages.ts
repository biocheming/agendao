export type Locale = "en" | "zh";

type MessageParams = Record<string, string | number>;
type MessageValue = string | ((params: MessageParams) => string);

type MessageCatalog = Record<string, MessageValue>;

function resolveLocaleMessage(
  catalog: MessageCatalog,
  fallbackCatalog: MessageCatalog,
  key: string,
  params?: MessageParams,
) {
  const value = catalog[key] ?? fallbackCatalog[key] ?? key;
  if (typeof value === "function") {
    return value(params ?? {});
  }
  if (!params) return value;
  return value.replaceAll(/\{(\w+)\}/g, (match, name) => {
    const replacement = params[name];
    return replacement === undefined ? match : String(replacement);
  });
}

export const messages: Record<Locale, MessageCatalog> = {
  en: {
    "app.attention": "Attention",
    "app.copyMarkdown": "Copy Markdown",
    "app.copySelectedLink": "Copy selected link",
    "app.dismissStatusMessage": "Dismiss status message",
    "app.forkSession": "Fork session",
    "app.hideTerminal": "Hide terminal",
    "app.hideWorkspace": "Hide workspace",
    "app.loadingSessions": "Loading sessions...",
    "app.loadingSettings": "Loading settings...",
    "app.loadingWorkspace": "Loading workspace...",
    "app.messageSelected": ({ count }) => `${count} message${count === 1 ? "" : "s"} selected`,
    "app.noEventsYet": "No events yet.",
    "app.pleaseWait": "Please wait",
    "app.runtimeSurface": "Runtime Surface",
    "app.runtimeSurfaceInspect": "Inspect",
    "app.runtimeSurfaceQueue": "Queue",
    "app.runtimeSurfaceSessionEvents": "Session Events",
    "app.runtimeSurfaceSummary": "Session-scoped runtime events that do not belong in the conversation transcript.",
    "app.settings": "Settings",
    "app.showSidebar": "Show sidebar",
    "app.showTerminal": "Show terminal",
    "app.showWorkspace": "Show workspace",
    "app.clear": "Clear",
    "app.resizeTerminal": "Resize terminal",
    "sidebar.branchCount": ({ count }) => `${count} branch${count === 1 ? "" : "es"}`,
    "sidebar.cancel": "Cancel",
    "sidebar.cancelSessionSelection": "Cancel session selection",
    "sidebar.chooseWorkspace": "Choose a workspace",
    "sidebar.createProject": "Create Project",
    "sidebar.createProjectDescription": "Create a new workspace folder and open its root session in the left sidebar.",
    "sidebar.createProjectPrimary": "Create Project",
    "sidebar.deleteAction": ({ count }) => `Delete ${count}`,
    "sidebar.deleteSelectedSessions": "Delete Selected Sessions",
    "sidebar.deleteSelectedSessionsHint": "Select sessions to delete",
    "sidebar.deleteSessionCountTitle": ({ count }) => `Delete ${count} selected session${count === 1 ? "" : "s"}`,
    "sidebar.deleteSelectionDescription.one": "The selected session will be deleted permanently.",
    "sidebar.deleteSelectionDescription.many": ({ count }) => `The ${count} selected sessions will be deleted permanently.`,
    "sidebar.deleteSelectionFollowup": "If a parent session is included, its follow-up threads are removed with it.",
    "sidebar.deleting": "Deleting…",
    "sidebar.deselectSession": "Deselect session",
    "sidebar.emptyMatchingWorkspaces": "No matching workspaces.",
    "sidebar.emptySessions": "No sessions in this workspace yet.",
    "sidebar.emptyWorkspaces": "No workspaces yet.",
    "sidebar.expandSession": "Expand session",
    "sidebar.hideSidebar": "Hide Sidebar",
    "sidebar.hideSidebarTitle": "Hide sidebar",
    "sidebar.newProject": "New Project",
    "sidebar.newSession": "New Session",
    "sidebar.noTitle": "(untitled)",
    "sidebar.projectPathPlaceholder": "projects/new-project",
    "sidebar.projectTitlePlaceholder": "Natural Products Workspace",
    "sidebar.projects": "Projects",
    "sidebar.root": "root",
    "sidebar.rootSessionTitle": "Root Session Title",
    "sidebar.searchProjects": "Search projects",
    "sidebar.selectSession": "Select session",
    "sidebar.selectSessions": "Select sessions",
    "sidebar.selectedCount": ({ count }) => `${count} selected`,
    "sidebar.selectionDescription": "This action cannot be undone. The current selection mode will close after deletion.",
    "sidebar.sessionCount": ({ count }) => `${count} session${count === 1 ? "" : "s"} selected`,
    "sidebar.sessions": "Sessions",
    "sidebar.shared": "SHARED",
    "sidebar.isolated": "ISOLATED",
    "sidebar.threadDepth": ({ depth }) => `thread ${depth}`,
    "sidebar.workspace": "Workspace",
    "sidebar.workspaceFolder": "Workspace Folder",
    "sidebar.collapseSession": "Collapse session",
  },
  zh: {
    "app.attention": "注意",
    "app.copyMarkdown": "复制 Markdown",
    "app.copySelectedLink": "复制所选链接",
    "app.dismissStatusMessage": "关闭状态消息",
    "app.forkSession": "分叉会话",
    "app.hideTerminal": "隐藏终端",
    "app.hideWorkspace": "隐藏工作区",
    "app.loadingSessions": "正在加载会话...",
    "app.loadingSettings": "正在加载设置...",
    "app.loadingWorkspace": "正在加载工作区...",
    "app.messageSelected": ({ count }) => `已选择 ${count} 条消息`,
    "app.noEventsYet": "还没有事件。",
    "app.pleaseWait": "请稍候",
    "app.runtimeSurface": "运行时侧面板",
    "app.runtimeSurfaceInspect": "检查",
    "app.runtimeSurfaceQueue": "队列",
    "app.runtimeSurfaceSessionEvents": "会话事件",
    "app.runtimeSurfaceSummary": "这些是会话级运行时事件，不适合放进对话 transcript。",
    "app.settings": "设置",
    "app.showSidebar": "显示侧栏",
    "app.showTerminal": "显示终端",
    "app.showWorkspace": "显示工作区",
    "app.clear": "清空",
    "app.resizeTerminal": "调整终端大小",
    "sidebar.branchCount": ({ count }) => `${count} 个分支`,
    "sidebar.cancel": "取消",
    "sidebar.cancelSessionSelection": "取消会话选择",
    "sidebar.chooseWorkspace": "选择一个工作区",
    "sidebar.createProject": "创建项目",
    "sidebar.createProjectDescription": "创建新的工作区目录，并在左侧栏打开它的根会话。",
    "sidebar.createProjectPrimary": "创建项目",
    "sidebar.deleteAction": ({ count }) => `删除 ${count} 个`,
    "sidebar.deleteSelectedSessions": "删除所选会话",
    "sidebar.deleteSelectedSessionsHint": "先选择要删除的会话",
    "sidebar.deleteSessionCountTitle": ({ count }) => `删除已选中的 ${count} 个会话`,
    "sidebar.deleteSelectionDescription.one": "所选会话将被永久删除。",
    "sidebar.deleteSelectionDescription.many": ({ count }) => `所选的 ${count} 个会话将被永久删除。`,
    "sidebar.deleteSelectionFollowup": "如果包含父会话，它的后续线程也会一并删除。",
    "sidebar.deleting": "删除中…",
    "sidebar.deselectSession": "取消选择会话",
    "sidebar.emptyMatchingWorkspaces": "没有匹配的工作区。",
    "sidebar.emptySessions": "这个工作区里还没有会话。",
    "sidebar.emptyWorkspaces": "还没有工作区。",
    "sidebar.expandSession": "展开会话",
    "sidebar.hideSidebar": "隐藏侧栏",
    "sidebar.hideSidebarTitle": "隐藏侧栏",
    "sidebar.newProject": "新建项目",
    "sidebar.newSession": "新建会话",
    "sidebar.noTitle": "（未命名）",
    "sidebar.projectPathPlaceholder": "projects/new-project",
    "sidebar.projectTitlePlaceholder": "天然产物工作区",
    "sidebar.projects": "项目",
    "sidebar.root": "根会话",
    "sidebar.rootSessionTitle": "根会话标题",
    "sidebar.searchProjects": "搜索项目",
    "sidebar.selectSession": "选择会话",
    "sidebar.selectSessions": "选择会话",
    "sidebar.selectedCount": ({ count }) => `已选 ${count} 个`,
    "sidebar.selectionDescription": "此操作无法撤销。删除后会自动退出当前选择模式。",
    "sidebar.sessionCount": ({ count }) => `已选择 ${count} 个会话`,
    "sidebar.sessions": "会话",
    "sidebar.shared": "共享",
    "sidebar.isolated": "隔离",
    "sidebar.threadDepth": ({ depth }) => `线程 ${depth}`,
    "sidebar.workspace": "工作区",
    "sidebar.workspaceFolder": "工作区目录",
    "sidebar.collapseSession": "折叠会话",
  },
};

export function detectLocale(): Locale {
  if (typeof navigator === "undefined") return "en";
  const normalized = navigator.language.toLowerCase();
  if (normalized.startsWith("zh")) return "zh";
  return "en";
}

export function translate(locale: Locale, key: string, params?: MessageParams) {
  return resolveLocaleMessage(messages[locale], messages.en, key, params);
}
