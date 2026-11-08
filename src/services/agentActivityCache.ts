import type { AgentActivityItem } from "./agentActivity";

const CACHE_KEY = "ai-desktop-pet.agent-activity.v1";
const MAX_CACHE_CHARS = 1_000_000;
type MessageIdentity = { content: string; thinking?: string; timestamp: number; agent?: { id?: string; name: string } | null };
type Presentation = { activity: AgentActivityItem[]; displayContent: string };
type CacheEntry = Presentation & { identity: string; timestamp: number };
type CacheStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

function identity(message: MessageIdentity): string {
  return JSON.stringify([message.content, message.thinking || "", message.agent?.id || message.agent?.name || ""]);
}
function read(storage: CacheStorage): CacheEntry[] {
  const data: unknown = JSON.parse(storage.getItem(CACHE_KEY) || "[]");
  return Array.isArray(data) ? data.filter((item) => item && typeof item.identity === "string" && typeof item.timestamp === "number" && typeof item.displayContent === "string" && Array.isArray(item.activity) && item.activity.every(validActivity)) : [];
}
function optionalString(value: unknown): boolean {
  return value === undefined || value === null || typeof value === "string";
}
function optionalNumber(value: unknown): boolean {
  return value === undefined || value === null || (typeof value === "number" && Number.isFinite(value));
}
function optionalChanges(value: unknown): boolean {
  return value === undefined || value === null
    || (Array.isArray(value) && value.every((change) => !!change && typeof change === "object" && typeof (change as { path?: unknown }).path === "string"));
}
function validActivity(value: unknown): value is AgentActivityItem {
  if (!value || typeof value !== "object") return false;
  const entry = value as Partial<AgentActivityItem>;
  if (entry.kind === "text" || entry.kind === "thinking") return typeof entry.text === "string";
  if (entry.kind !== "tool" || !entry.tool) return false;
  const tool = entry.tool as unknown as Record<string, unknown>;
  // Codex 轨迹字段（exit_code / duration_ms / changes）可选：旧缓存缺少它们时
  // 依然要能还原，字段类型不对则整条丢弃，避免把脏数据渲染到界面上。
  return typeof tool.id === "string" && typeof tool.tool_name === "string" && typeof tool.status === "string"
    && ["arguments", "summary", "command", "path", "output"].every((key) => optionalString(tool[key]))
    && optionalNumber(tool.exit_code) && optionalNumber(tool.duration_ms) && optionalChanges(tool.changes);
}

// Display-only, bounded cache. Database history remains the authority for
// content and model context; unavailable storage must never block a reply.
export function saveActivityPresentation(storage: CacheStorage | undefined, message: MessageIdentity & Presentation) {
  if (!storage || !message.activity.length) return;
  try {
    const key = identity(message);
    const entries = read(storage).filter((entry) => entry.identity !== key || Math.abs(entry.timestamp - message.timestamp) > 3000);
    entries.push({ identity: key, timestamp: message.timestamp, activity: message.activity, displayContent: message.displayContent });
    while (entries.length > 100) entries.shift();
    let serialized = JSON.stringify(entries);
    while (serialized.length > MAX_CACHE_CHARS && entries.length) {
      entries.shift(); serialized = JSON.stringify(entries);
    }
    storage.setItem(CACHE_KEY, serialized);
  } catch { /* Old or full browser storage falls back to database history. */ }
}

export function restoreActivityPresentation(storage: CacheStorage | undefined, message: MessageIdentity): Presentation | undefined {
  if (!storage) return;
  try {
    const key = identity(message);
    const entry = read(storage).reverse().find((item) => item.identity === key && Math.abs(item.timestamp - message.timestamp) <= 3000);
    return entry ? { activity: entry.activity, displayContent: entry.displayContent } : undefined;
  } catch { return; }
}

export function clearActivityPresentations(storage: CacheStorage | undefined) {
  try { storage?.removeItem(CACHE_KEY); } catch { /* Chat reset remains available without storage. */ }
}
