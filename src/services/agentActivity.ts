export type AgentActivityTool = {
  id: string;
  tool_name: string;
  status: string;
  summary?: string | null;
  arguments?: string | null;
  command?: string | null;
  path?: string | null;
  output?: string | null;
  approved?: boolean | null;
};

export type AgentActivityItem =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool"; tool: AgentActivityTool };

const terminalStatuses = new Set(["completed", "denied", "skipped", "failed", "interrupted"]);
const statusLabels: Record<string, string> = {
  requested: "已请求",
  waiting: "等待确认",
  approved: "已授权",
  denied: "已拒绝",
  running: "执行中",
  completed: "已完成",
  skipped: "已跳过",
  failed: "失败",
  interrupted: "已中止",
};

export function toolStatusLabel(status: string): string {
  return statusLabels[status] ?? status;
}

export function formatArguments(value: string | null | undefined): string {
  if (!value) return "";
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

export function isNearBottom({ scrollHeight, scrollTop, clientHeight }: { scrollHeight: number; scrollTop: number; clientHeight: number }, threshold = 72) {
  return scrollHeight - (scrollTop + clientHeight) <= threshold;
}

function isToolLog(text: string): boolean {
  const marker = /^\s*\[(调用工具|执行结果|等待授权|用户已授权|用户拒绝执行此操作|授权通道失效|确认超时|等待用户回答|用户已回答|用户未在)/;
  return marker.test(text);
}

function cleanThinking(text: string): string {
  const marker = /(?:^|\n)\[(调用工具|执行结果|等待授权|用户已授权|用户拒绝执行此操作|授权通道失效|确认超时|等待用户回答|用户已回答|用户未在)[^\n]*/;
  const match = marker.exec(text);
  return match ? text.slice(0, match.index) : text;
}

function mergeTool(previous: AgentActivityTool, next: AgentActivityTool): AgentActivityTool {
  const previousTerminal = terminalStatuses.has(previous.status);
  const ranks: Record<string, number> = { requested: 0, waiting: 1, approved: 2, running: 3 };
  const regressed = previous.status in ranks && next.status in ranks && ranks[next.status] < ranks[previous.status];
  const nextStatus = previousTerminal || regressed ? previous.status : next.status;
  const result: AgentActivityTool = { ...previous, ...next, status: nextStatus };
  for (const key of ["summary", "arguments", "command", "path", "output", "approved"] as const) {
    const value = next[key];
    if (value === null || value === undefined || value === "") Object.assign(result, { [key]: previous[key] });
  }
  return result;
}

export function createAgentActivity(items: AgentActivityItem[] = []) {
  const append = (kind: "text" | "thinking", value: string) => {
    if (!value || (kind === "thinking" && isToolLog(value))) return;
    const text = kind === "thinking" ? cleanThinking(value) : value;
    if (!text) return;
    const previous = items[items.length - 1];
    if (previous?.kind === kind) previous.text += text;
    else items.push({ kind, text });
  };
  return {
    items,
    appendText(value: string) { append("text", value); },
    appendThinking(value: string) { append("thinking", value); },
    upsertTool(tool: AgentActivityTool) {
      const index = items.findIndex((item) => item.kind === "tool" && item.tool.id === tool.id);
      if (index >= 0) items[index] = { kind: "tool", tool: mergeTool(items[index].kind === "tool" ? items[index].tool : tool, tool) };
      else items.push({ kind: "tool", tool: { ...tool } });
    },
    finish(content: string, reason: "completed" | "failed" | "aborted" = "completed") {
      if (reason !== "completed") {
        for (const item of items) {
          if (item.kind === "tool" && !terminalStatuses.has(item.tool.status)) item.tool.status = "interrupted";
        }
      }
      const activity: AgentActivityItem[] = items.map((item) => item.kind === "tool" ? { kind: item.kind, tool: { ...item.tool } } : { ...item });
      let displayContent = content;
      const streamed = items.filter((item) => item.kind === "text").map((item) => item.text).join("");
      const last = activity[activity.length - 1];
      // Legacy backends settle with all intermediate text concatenated. Keep
      // the stored reply intact, but show its last segment once in the UI.
      if (reason === "completed" && last?.kind === "text" && (streamed === content || last.text.trim() === content.trim())) {
        displayContent = streamed === content ? last.text : content;
        activity.pop();
      }
      return { displayContent, activity };
    },
    reset() { items.splice(0, items.length); },
  };
}

export function legacyActivity(thinking: string | null | undefined): AgentActivityItem[] {
  if (!thinking) return [];
  const lines = thinking.split("\n");
  const result: AgentActivityItem[] = [];
  let pending: AgentActivityTool | null = null;
  let mode: "thinking" | "args" | "output" = "thinking";
  let buffer: string[] = [];
  const flushThinking = () => {
    const text = buffer.join("\n").trim();
    if (text) result.push({ kind: "thinking", text });
    buffer = [];
  };
  const flushTool = () => {
    if (pending) {
      try {
        const args = JSON.parse(pending.arguments || "{}");
        pending.path = typeof args?.path === "string" ? args.path : undefined;
        pending.command = typeof args?.command === "string" ? args.command : undefined;
      } catch { /* Historical providers can store plain-text arguments. */ }
      pending.status = pending.status === "denied" ? "denied" : pending.output ? "completed" : "interrupted";
      result.push({ kind: "tool", tool: pending });
    }
    pending = null;
  };
  for (const line of lines) {
    if (line.startsWith("[调用工具]")) {
      flushThinking(); flushTool();
      pending = { id: `legacy-${result.length}`, tool_name: line.slice(6).trim(), status: "completed", arguments: "" };
      mode = "args";
    } else if (line.startsWith("[执行结果]")) {
      mode = "output";
    } else if (/^\[(等待授权|用户已授权|用户拒绝执行此操作|授权通道失效|确认超时|等待用户回答|用户已回答|用户未在)/.test(line)) {
      if (pending && /^\[(用户拒绝|授权通道|确认超时)/.test(line)) pending.status = "denied";
    } else if (pending && mode === "args") {
      pending.arguments += (pending.arguments ? "\n" : "") + line;
    } else if (pending && mode === "output") {
      pending.output = (pending.output ? `${pending.output}\n` : "") + line;
    } else {
      buffer.push(line);
    }
  }
  flushThinking(); flushTool();
  return result;
}
