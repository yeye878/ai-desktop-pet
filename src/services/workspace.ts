/**
 * 同屏多窗格工作区：布局预设 + 每个窗格绑定的会话 id。
 * 窗格身份（paneId）与会话身份（sessionId）分离：换绑会话不动窗格，
 * 关闭窗格只是移除视图，不停止也不删除会话。
 */
export type WorkspaceLayout = "single" | "two" | "three" | "master" | "four";

export const WORKSPACE_LAYOUTS: { id: WorkspaceLayout; label: string; panes: number }[] = [
  { id: "single", label: "单窗格", panes: 1 },
  { id: "two", label: "双窗格", panes: 2 },
  { id: "three", label: "三窗格", panes: 3 },
  { id: "master", label: "主从", panes: 3 },
  { id: "four", label: "四窗格", panes: 4 },
];

export const WORKSPACE_LAYOUT_KEY = "dash.workspace-layout";
export const WORKSPACE_PANES_KEY = "dash.workspace-panes";

export function layoutPaneCount(layout: WorkspaceLayout): number {
  return WORKSPACE_LAYOUTS.find((item) => item.id === layout)?.panes ?? 1;
}

export function readWorkspaceLayout(): WorkspaceLayout {
  try {
    const raw = localStorage.getItem(WORKSPACE_LAYOUT_KEY);
    if (WORKSPACE_LAYOUTS.some((item) => item.id === raw)) return raw as WorkspaceLayout;
  } catch { /* WebView 可能禁用 storage。 */ }
  return "single";
}

export function saveWorkspaceLayout(layout: WorkspaceLayout) {
  try { localStorage.setItem(WORKSPACE_LAYOUT_KEY, layout); } catch { /* ignore */ }
}

/** 返回长度等于窗格数的 sessionId 数组，空串 = 空窗格（待选择会话）。 */
export function readWorkspacePanes(layout: WorkspaceLayout): string[] {
  const count = layoutPaneCount(layout);
  let stored: string[] = [];
  try {
    const parsed = JSON.parse(localStorage.getItem(WORKSPACE_PANES_KEY) || "[]");
    if (Array.isArray(parsed)) stored = parsed.filter((item): item is string => typeof item === "string");
  } catch { /* 数据损坏时从空窗格开始。 */ }
  return Array.from({ length: count }, (_, index) => stored[index] || "");
}

export function saveWorkspacePanes(panes: string[]) {
  try { localStorage.setItem(WORKSPACE_PANES_KEY, JSON.stringify(panes)); } catch { /* ignore */ }
}
