const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");
const ts = require("typescript");

const root = path.resolve(__dirname, "..");
const plain = (value) => JSON.parse(JSON.stringify(value));

/** 用 typescript 转译 + vm 加载纯 TS 模块（无 Vite / Tauri / pinia 依赖），模式与 verify-chat-sessions.cjs 一致。 */
function loadModule(file) {
  const exports = {};
  const compiled = ts.transpileModule(fs.readFileSync(path.join(root, file), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(compiled, { exports, require, console });
  return exports;
}

const labels = loadModule("src/services/sessionLabels.ts");
const {
  SLOT_ORDER,
  SLOT_LABELS,
  slotLabel,
  displayTitle,
  relativeTime,
  sessionMeta,
  groupSessions,
  duplicateSuffixes,
  matchesQuery,
  filterSessions,
} = labels;

/** 构造字段齐全的 ChatSession；测试只关心展示/搜索相关字段。 */
function makeSession(patch = {}) {
  return {
    id: "session-1",
    title: "",
    kind: "main",
    slot: "main",
    agent_id: "",
    agent_name: "",
    agent_avatar: "",
    model: "",
    provider: "",
    reasoning_effort: "",
    permission: "",
    created_at: 0,
    updated_at: 0,
    message_count: 0,
    preview: "",
    ...patch,
  };
}

test("导出 API 与契约一致", () => {
  for (const name of [
    "slotLabel",
    "displayTitle",
    "relativeTime",
    "sessionMeta",
    "groupSessions",
    "duplicateSuffixes",
    "matchesQuery",
    "filterSessions",
  ]) {
    assert.equal(typeof labels[name], "function", name + " 应导出为函数");
  }
  assert.equal(SLOT_ORDER.length, 4);
});

test("槽位顺序与展示名固定", () => {
  assert.deepEqual([...SLOT_ORDER], ["main", "claude", "codex", "dsh"]);
  assert.deepEqual({ ...SLOT_LABELS }, {
    main: "主对话",
    claude: "Claude Code",
    codex: "Codex",
    dsh: "DeepSeek Harness",
  });
  assert.equal(slotLabel("main"), "主对话");
  assert.equal(slotLabel("claude"), "Claude Code");
  assert.equal(slotLabel("codex"), "Codex");
  assert.equal(slotLabel("dsh"), "DeepSeek Harness");
});

test("displayTitle：空白标题回退为未命名会话", () => {
  assert.equal(displayTitle(makeSession({ title: "" })), "未命名会话");
  assert.equal(displayTitle(makeSession({ title: "   " })), "未命名会话");
  assert.equal(displayTitle(makeSession({ title: "\n\t" })), "未命名会话");
  assert.equal(displayTitle(makeSession({ title: "  周报整理  " })), "周报整理");
  assert.equal(displayTitle(makeSession({ title: "未命名会话" })), "未命名会话");
});

test("relativeTime：0 与非法时间戳返回空串", () => {
  for (const value of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(relativeTime(value), "", String(value) + " 应返回空串");
  }
});

test("relativeTime：分钟/小时/天分档边界", () => {
  const now = 1700000000000;
  assert.equal(relativeTime(now, now), "刚刚");
  assert.equal(relativeTime(now - 59999, now), "刚刚");
  assert.equal(relativeTime(now + 5000, now), "刚刚", "未来时间戳按刚刚展示");
  assert.equal(relativeTime(now - 60000, now), "1 分钟前");
  assert.equal(relativeTime(now - (59 * 60000 + 59999), now), "59 分钟前");
  assert.equal(relativeTime(now - 3600000, now), "1 小时前");
  assert.equal(relativeTime(now - (23 * 3600000 + 59 * 60000), now), "23 小时前");
  assert.equal(relativeTime(now - 86400000, now), "1 天前");
  assert.equal(relativeTime(now - (7 * 86400000 - 1000), now), "6 天前");
  assert.match(relativeTime(now - 7 * 86400000, now), /^\d+月\d+日$/, "满 7 天改显示日期");
});

test("relativeTime：秒级时间戳自动换算为毫秒", () => {
  const now = 1700000000000;
  const secondsNow = Math.floor(now / 1000);
  assert.ok(secondsNow < 1e12, "测试用秒级时间戳必须小于 1e12");
  assert.equal(relativeTime(secondsNow, now), "刚刚");
  assert.equal(relativeTime(secondsNow - 90, now), "1 分钟前");
  assert.equal(relativeTime(secondsNow - 3 * 3600, now), "3 小时前");
  assert.equal(relativeTime(secondsNow - 2 * 86400, now), "2 天前");
  assert.equal(relativeTime(now - 90000, now), relativeTime(secondsNow - 90, now));
});

test("relativeTime：同年显示月日，跨年显示年月日", () => {
  const now = new Date(2026, 10, 20, 12, 0, 0).getTime();
  assert.equal(relativeTime(new Date(2026, 10, 12, 12, 0, 0).getTime(), now), "11月12日");
  assert.equal(relativeTime(new Date(2026, 0, 5, 12, 0, 0).getTime(), now), "1月5日");
  assert.equal(relativeTime(new Date(2026, 0, 1, 12, 0, 0).getTime(), now), "1月1日");
  assert.equal(relativeTime(new Date(2025, 11, 31, 12, 0, 0).getTime(), now), "2025年12月31日");
});

test("sessionMeta：槽位、消息量、相对时间与可选标记用分隔符连接", () => {
  const now = 1700000000000;
  const session = makeSession({ slot: "codex", title: " 修复登录 ", message_count: 12, updated_at: now - 5 * 60000 });
  assert.equal(sessionMeta(session, { now }), "Codex · 12 条消息 · 5 分钟前");
  assert.equal(sessionMeta(makeSession({ slot: "main", message_count: 0, updated_at: now }), { now }), "主对话 · 空会话 · 刚刚");
  assert.equal(sessionMeta(makeSession({ slot: "dsh", message_count: 3 }), { now }), "DeepSeek Harness · 3 条消息");
  assert.equal(sessionMeta(session, { now, busy: true }), "Codex · 12 条消息 · 5 分钟前 · 运行中");
  assert.equal(sessionMeta(session, { now, suffix: "#a1b2" }), "Codex · 12 条消息 · 5 分钟前 · #a1b2");
  assert.equal(
    sessionMeta(session, { now, busy: true, suffix: "#a1b2" }),
    "Codex · 12 条消息 · 5 分钟前 · 运行中 · #a1b2",
  );
  assert.ok(sessionMeta(makeSession({ slot: "main" })).startsWith("主对话 · 空会话"));
});

test("groupSessions：固定槽位顺序、空组省略、组内时间倒序且同时间按 id 升序", () => {
  const sessions = [
    makeSession({ id: "codex-old", slot: "codex", updated_at: 10 }),
    makeSession({ id: "main-a", slot: "main", updated_at: 50 }),
    makeSession({ id: "codex-new", slot: "codex", updated_at: 90 }),
    makeSession({ id: "main-b", slot: "main", updated_at: 50 }),
    makeSession({ id: "claude-old", slot: "claude", updated_at: 30 }),
    makeSession({ id: "dsh-1", slot: "dsh", updated_at: 5 }),
  ];
  const groups = plain(groupSessions(sessions));
  assert.deepEqual(groups.map((group) => group.slot), ["main", "claude", "codex", "dsh"]);
  assert.deepEqual(groups.map((group) => group.label), ["主对话", "Claude Code", "Codex", "DeepSeek Harness"]);
  assert.deepEqual(groups[0].sessions.map((session) => session.id), ["main-a", "main-b"]);
  assert.deepEqual(groups[1].sessions.map((session) => session.id), ["claude-old"]);
  assert.deepEqual(groups[2].sessions.map((session) => session.id), ["codex-new", "codex-old"]);
  assert.deepEqual(groups[3].sessions.map((session) => session.id), ["dsh-1"]);
  assert.deepEqual(plain(groupSessions([])), []);
  assert.deepEqual(sessions.map((session) => session.id), ["codex-old", "main-a", "codex-new", "main-b", "claude-old", "dsh-1"]);
});

test("duplicateSuffixes：只有归一化后重名的会话带 id 后缀", () => {
  const sessions = [
    makeSession({ id: "session-1a2b", title: "周报整理" }),
    makeSession({ id: "agent-x-yz9k", title: "  周报整理  " }),
    makeSession({ id: "session-7f3c", title: "周报整理" }),
    makeSession({ id: "unique-id-55", title: "唯一会话" }),
    makeSession({ id: "session-0000", title: "" }),
    makeSession({ id: "session-9999", title: "   " }),
  ];
  assert.deepEqual(plain(duplicateSuffixes(sessions)), {
    "session-1a2b": "#1a2b",
    "agent-x-yz9k": "#yz9k",
    "session-7f3c": "#7f3c",
    "session-0000": "#0000",
    "session-9999": "#9999",
  });
  assert.deepEqual(plain(duplicateSuffixes([makeSession({ id: "unique-id-55", title: "唯一会话" })])), {});
});

test("duplicateSuffixes：大小写与首尾空白不同的标题视为重名", () => {
  const sessions = [
    makeSession({ id: "sess-aaaa", title: "Claude" }),
    makeSession({ id: "sess-bbbb", title: "  claude  " }),
    makeSession({ id: "sess-cccc", title: "codex" }),
  ];
  assert.deepEqual(plain(duplicateSuffixes(sessions)), {
    "sess-aaaa": "#aaaa",
    "sess-bbbb": "#bbbb",
  });
});

test("matchesQuery：空搜索命中全部，中英文/预览/槽位/id 不区分大小写", () => {
  const session = makeSession({
    id: "session-abc123",
    title: "修复登录页",
    preview: "调整 OAuth 回调与错误提示",
    agent_name: "Claude",
    slot: "claude",
    message_count: 2,
  });
  assert.equal(matchesQuery(session, ""), true);
  assert.equal(matchesQuery(session, "   "), true);
  assert.equal(matchesQuery(session, "登录"), true);
  assert.equal(matchesQuery(session, "oauth"), true);
  assert.equal(matchesQuery(session, "  OAuth  "), true);
  assert.equal(matchesQuery(session, "CLAUDE"), true);
  assert.equal(matchesQuery(session, "CLAUDE CODE"), true, "应命中槽位展示名");
  assert.equal(matchesQuery(session, "abc123"), true);
  assert.equal(matchesQuery(session, "deepseek"), false);
  assert.equal(matchesQuery(session, "不存在的词"), false);

  const agentSession = makeSession({ id: "agent-dsh-1", title: "", slot: "dsh", agent_name: "dsh" });
  assert.equal(matchesQuery(agentSession, "DeepSeek Harness"), true);
  assert.equal(matchesQuery(agentSession, "dsh"), true);
  assert.equal(matchesQuery(agentSession, "空会话"), false, "搜索不命中拼装出来的展示文案");
});

test("filterSessions：保持输入顺序并原样过滤", () => {
  const sessions = [
    makeSession({ id: "s1", title: "周报整理", updated_at: 100 }),
    makeSession({ id: "s2", title: "Claude 调试", agent_name: "claude", slot: "claude", updated_at: 900 }),
    makeSession({ id: "s3", title: "", preview: "整理 OAuth 回调", updated_at: 500 }),
  ];
  assert.deepEqual(plain(filterSessions(sessions, "整理")).map((session) => session.id), ["s1", "s3"]);
  assert.deepEqual(plain(filterSessions(sessions, " ")).map((session) => session.id), ["s1", "s2", "s3"]);
  assert.deepEqual(plain(filterSessions(sessions, "codex")), []);
  assert.deepEqual(sessions.map((session) => session.id), ["s1", "s2", "s3"], "过滤不应改动输入数组");
});
