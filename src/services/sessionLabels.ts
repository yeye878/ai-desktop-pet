import type { ChatSession, ChatSlot } from "../stores/sessions";

/** 槽位展示顺序：主对话在最前，其次是三个内置 CLI 窗口。 */
export const SLOT_ORDER: readonly ChatSlot[] = ["main", "claude", "codex", "dsh"];

/** 槽位的中文展示名；未命名会话至少还能靠它区分窗口来源。 */
export const SLOT_LABELS: Record<ChatSlot, string> = {
  main: "主对话",
  claude: "Claude Code",
  codex: "Codex",
  dsh: "DeepSeek Harness",
};

/** 槽位展示名。 */
export function slotLabel(slot: ChatSlot): string {
  return SLOT_LABELS[slot];
}

/** 会话标题展示：空白标题统一回退为“未命名会话”。 */
export function displayTitle(session: Pick<ChatSession, "title">): string {
  const title = (session.title || "").trim();
  return title || "未命名会话";
}

/** 小于该值的 updated_at 按秒解释，否则按毫秒（兼容后端早期写入的秒级时间戳）。 */
const SECOND_TIMESTAMP_LIMIT = 1e12;
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
const WEEK_MS = 7 * DAY_MS;

/** 秒/毫秒统一换算成毫秒。 */
function toMilliseconds(timestamp: number): number {
  return timestamp < SECOND_TIMESTAMP_LIMIT ? timestamp * 1000 : timestamp;
}

/**
 * 相对时间展示：60 秒内“刚刚”，随后依次是分钟/小时/天，
 * 超过 7 天显示日期（跨年带年份）。0 或非法时间戳返回空串。
 */
export function relativeTime(timestamp: number, now: number = Date.now()): string {
  if (!Number.isFinite(timestamp) || timestamp <= 0) return "";
  const time = toMilliseconds(timestamp);
  const diff = Math.max(0, now - time);
  if (diff < MINUTE_MS) return "刚刚";
  if (diff < HOUR_MS) return Math.floor(diff / MINUTE_MS) + " 分钟前";
  if (diff < DAY_MS) return Math.floor(diff / HOUR_MS) + " 小时前";
  if (diff < WEEK_MS) return Math.floor(diff / DAY_MS) + " 天前";
  const then = new Date(time);
  const month = then.getMonth() + 1;
  const day = then.getDate();
  if (then.getFullYear() === new Date(now).getFullYear()) return month + "月" + day + "日";
  return then.getFullYear() + "年" + month + "月" + day + "日";
}

/**
 * 会话副标题：槽位 · 消息量 · 更新时间（可选“运行中”与重名后缀），
 * 统一用 " · " 连接，空片段自动跳过。
 */
export function sessionMeta(
  session: ChatSession,
  options: { now?: number; busy?: boolean; suffix?: string } = {},
): string {
  const parts: string[] = [
    slotLabel(session.slot),
    session.message_count > 0 ? session.message_count + " 条消息" : "空会话",
    relativeTime(session.updated_at, options.now),
  ];
  if (options.busy) parts.push("运行中");
  const suffix = (options.suffix || "").trim();
  if (suffix) parts.push(suffix);
  return parts.filter((part) => part !== "").join(" · ");
}

/** 组内排序：updated_at 降序，同一时间戳按 id 升序，保证顺序稳定。 */
function compareSessions(left: ChatSession, right: ChatSession): number {
  const diff = right.updated_at - left.updated_at;
  if (diff !== 0) return diff;
  if (left.id === right.id) return 0;
  return left.id < right.id ? -1 : 1;
}

/**
 * 按槽位分组：固定按 SLOT_ORDER 输出，空组不返回；
 * 组内 updated_at 降序、同一时间戳按 id 升序。
 */
export function groupSessions(
  sessions: ChatSession[],
): Array<{ slot: ChatSlot; label: string; sessions: ChatSession[] }> {
  const groups = SLOT_ORDER.map((slot) => ({
    slot,
    label: slotLabel(slot),
    sessions: sessions.filter((session) => session.slot === slot).sort(compareSessions),
  }));
  return groups.filter((group) => group.sessions.length > 0);
}

/** id 去掉非字母数字字符后取后 4 位，作为重名会话的短后缀。 */
function idSuffix(id: string): string {
  return "#" + id.replace(/[^0-9A-Za-z]/g, "").slice(-4);
}

/**
 * 只给“displayTitle 归一化（trim + 小写）后重名”的会话返回短后缀，
 * 让同名/未命名会话在多窗格选择器里可以区分。
 */
export function duplicateSuffixes(sessions: ChatSession[]): Record<string, string> {
  const groups = new Map<string, ChatSession[]>();
  for (const session of sessions) {
    const key = displayTitle(session).trim().toLowerCase();
    const group = groups.get(key);
    if (group) group.push(session);
    else groups.set(key, [session]);
  }
  const suffixes: Record<string, string> = {};
  for (const group of groups.values()) {
    if (group.length < 2) continue;
    for (const session of group) suffixes[session.id] = idSuffix(session.id);
  }
  return suffixes;
}

/** 搜索命中：标题、预览、agent 名、槽位名/值、id 拼接文本的大小写无关包含匹配。 */
export function matchesQuery(session: ChatSession, query: string): boolean {
  const needle = (query || "").trim().toLowerCase();
  if (!needle) return true;
  const haystack = [
    session.title,
    session.preview,
    session.agent_name,
    slotLabel(session.slot),
    session.slot,
    session.id,
  ].join(" ").toLowerCase();
  return haystack.includes(needle);
}

/** 按搜索词过滤会话，保持传入列表的原始顺序，不改动输入数组。 */
export function filterSessions(sessions: ChatSession[], query: string): ChatSession[] {
  return sessions.filter((session) => matchesQuery(session, query));
}
