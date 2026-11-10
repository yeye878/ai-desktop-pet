const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");
const ts = require("typescript");
const { createPinia } = require("pinia");

const root = path.resolve(__dirname, "..");
const plain = (value) => JSON.parse(JSON.stringify(value));
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function evaluate(source, globals = {}) {
  const exports = {};
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(compiled, { exports, require, console, setTimeout, clearTimeout, ...globals });
  return exports;
}

function loadModule(file, mocks = {}) {
  return evaluate(fs.readFileSync(path.join(root, file), "utf8"), {
    require: (name) => {
      if (mocks[name]) return mocks[name];
      if (name.startsWith(".")) {
        const dependency = path.resolve(root, path.dirname(file), name + ".ts");
        return loadModule(path.relative(root, dependency), mocks);
      }
      return require(name);
    },
  });
}

/** 从 .vue 的 <script setup> 里提取某个 listen(event, handler) 的 handler 源码。 */
function componentCallback(file, eventName) {
  const vue = fs.readFileSync(path.join(root, file), "utf8");
  const script = vue.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const source = ts.createSourceFile(file + ".ts", script, ts.ScriptTarget.Latest, true);
  let found;
  function visit(node) {
    if (
      ts.isCallExpression(node)
      && node.expression.getText() === "listen"
      && node.arguments[0]?.text === eventName
    ) {
      found = node;
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(found, `${file} 缺少 listen("${eventName}") 处理器`);
  return found.arguments[1].getText(source);
}

function makeSession(patch = {}) {
  return {
    id: "session-1",
    title: "默认对话",
    kind: "main",
    slot: "main",
    agent_id: "",
    agent_name: "",
    agent_avatar: "",
    model: "",
    created_at: 100,
    updated_at: 100,
    message_count: 2,
    preview: "你好",
    ...patch,
  };
}

/**
 * 内存版 chat_sessions 后端：只实现 sessions store 用到的命令，
 * 用来在无 Tauri / 无 SQLite 的环境里验证 store 行为。
 */
function createBackend(options = {}) {
  let sessions = structuredClone(options.sessions ?? []);
  let activeId = options.activeId ?? "";
  let created = 0;
  const calls = [];
  const listeners = new Map();

  async function invoke(command, args = {}) {
    calls.push({ command, args: structuredClone(args) });
    switch (command) {
      case "list_chat_sessions":
        return structuredClone(sessions.filter((session) => !args.slot || session.slot === args.slot));
      case "get_active_chat_session":
        return structuredClone(sessions.find((session) => session.id === activeId) ?? null);
      case "create_chat_session": {
        created += 1;
        const isMain = args.slot === "main";
        const session = makeSession({
          id: `session-new-${created}`,
          title: args.title || (isMain ? `新对话 ${created}` : "codex 对话"),
          kind: isMain ? "main" : "agent",
          slot: args.slot,
          agent_id: args.agentId || "",
          agent_name: isMain ? "" : "codex",
          agent_avatar: isMain ? "" : "⌘",
          model: args.model || "",
          created_at: 500 + created,
          updated_at: 500 + created,
          message_count: 0,
          preview: "",
        });
        sessions.unshift(session);
        if (isMain) activeId = session.id;
        return structuredClone(session);
      }
      case "set_active_chat_session": {
        const session = sessions.find((item) => item.id === args.id);
        if (!session) throw new Error("会话不存在");
        activeId = session.id;
        return structuredClone(session);
      }
      case "rename_chat_session": {
        const session = sessions.find((item) => item.id === args.id);
        if (!session) throw new Error("会话不存在");
        session.title = args.title;
        session.updated_at += 10;
        return structuredClone(session);
      }
      case "delete_chat_session": {
        sessions = sessions.filter((session) => session.id !== args.id);
        if (activeId === args.id) {
          activeId = sessions.filter((session) => session.slot === "main")
            .sort((left, right) => right.updated_at - left.updated_at)[0]?.id ?? "";
        }
        return { active_main_session_id: activeId };
      }
      case "set_chat_session_binding": {
        const session = sessions.find((item) => item.id === args.id);
        if (!session) throw new Error("会话不存在");
        if (args.agentId !== undefined) session.agent_id = args.agentId;
        if (args.agentName !== undefined) session.agent_name = args.agentName;
        if (args.agentAvatar !== undefined) session.agent_avatar = args.agentAvatar;
        if (args.model !== undefined) session.model = args.model;
        session.updated_at += 3;
        return structuredClone(session);
      }
      default:
        throw new Error(`unexpected command: ${command}`);
    }
  }

  async function listen(event, callback) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(callback);
    return () => listeners.get(event)?.delete(callback);
  }

  function emit(event, payload = {}) {
    for (const callback of listeners.get(event) ?? []) callback({ payload });
  }

  return {
    invoke,
    listen,
    emit,
    calls,
    listeners,
    get sessions() { return sessions; },
    get activeId() { return activeId; },
    pushSession(session) { sessions.unshift(session); },
    countCalls(command) { return calls.filter((call) => call.command === command).length; },
  };
}

function loadSessionsStore(backend) {
  const { useSessionsStore } = loadModule("src/stores/sessions.ts", {
    "@tauri-apps/api/core": { invoke: backend.invoke },
    "@tauri-apps/api/event": { listen: backend.listen },
  });
  return useSessionsStore(createPinia());
}

/**
 * 直接加载 src/dev-tauri-mock.ts 的内存后端：
 * 去掉浏览器专用 import，把 mockIPC/mockWindows/emit 换成可观测的桩，
 * 这样不用起 Vite 也能验证命令返回值与事件 payload 形状。
 */
function loadDevMock(options = {}) {
  const file = path.join(root, "src/dev-tauri-mock.ts");
  const captured = { handler: null, events: [] };
  let source = fs.readFileSync(file, "utf8");
  source = source.replace(/^import .*$/gm, "").replace(/!import\.meta\.env\.DEV/g, "!true");
  const globals = {
    mockIPC(handler) {
      captured.handler = handler;
    },
    mockWindows() {},
    emit: async (event, payload) => {
      captured.events.push({ event, payload: plain(payload) });
    },
    DEFAULT_VOICE_SETTINGS: {},
    DEFAULT_TTS_SETTINGS: {},
    URLSearchParams,
    window: { location: { search: options.search ?? "" }, __TAURI_INTERNALS__: undefined },
  };
  const { installDevTauriMock } = evaluate(source, globals);
  assert.equal(typeof installDevTauriMock, "function", "dev mock 应导出 installDevTauriMock");
  installDevTauriMock();
  assert.equal(typeof captured.handler, "function", "installDevTauriMock 应注册 mockIPC handler");
  return {
    // Tauri 的 invoke 是异步的：同步抛错也要变成 rejected promise。
    invoke: async (command, args) => captured.handler(command, args),
    events: captured.events,
    eventsOf: (event) => captured.events.filter((item) => item.event === event),
  };
}

test("load 拉取会话列表、排序并解析活跃主会话", async () => {
  const backend = createBackend({
    sessions: [
      makeSession({ id: "main-a", updated_at: 300 }),
      makeSession({ id: "agent-claude", kind: "agent", slot: "claude", updated_at: 500 }),
      makeSession({ id: "main-b", updated_at: 200 }),
    ],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.load();
  assert.equal(store.loading, false);
  assert.equal(store.activeMainId, "main-a");
  assert.deepEqual(plain(store.bySlot("main").map((session) => session.id)), ["main-a", "main-b"]);
  assert.deepEqual(plain(store.bySlot("claude").map((session) => session.id)), ["agent-claude"]);
  assert.deepEqual(plain(store.bySlot("codex")), []);
  assert.equal(store.activeMain()?.id, "main-a");
  assert.equal(store.find("main-b")?.title, "默认对话");
  assert.equal(store.find("missing"), undefined);
});

test("活跃主会话缺失时回退到 updated_at 最新的 main 会话", async () => {
  const backend = createBackend({
    sessions: [
      makeSession({ id: "main-old", updated_at: 10 }),
      makeSession({ id: "main-new", updated_at: 99 }),
      makeSession({ id: "agent-dsh", kind: "agent", slot: "dsh", updated_at: 999 }),
    ],
    activeId: "gone",
  });
  const store = loadSessionsStore(backend);
  await store.load();
  assert.equal(store.activeMainId, "main-new");
  assert.equal(store.activeMain()?.id, "main-new");
});

test("create(main) 更新活跃主会话，create(agent) 不影响活跃主会话", async () => {
  const backend = createBackend({
    sessions: [makeSession({ id: "main-a", updated_at: 300 })],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.load();

  const main = await store.create("main", { title: "" });
  assert.equal(main.slot, "main");
  assert.equal(store.activeMainId, main.id);
  assert.ok(store.find(main.id), "新建的主会话应直接 upsert 进列表");
  assert.equal(store.sessions.length, 2);
  assert.deepEqual(plain(backend.calls.at(-1)), {
    command: "create_chat_session",
    args: { slot: "main", agentId: "", model: "", provider: "", reasoningEffort: "", title: "" },
  });

  const activeBefore = store.activeMainId;
  const claude = await store.create("claude", { agentId: "builtin-claude", model: "sonnet" });
  assert.equal(claude.slot, "claude");
  assert.equal(store.activeMainId, activeBefore, "agent 窗口新建会话不能抢走主会话活跃 id");
  assert.equal(store.find(claude.id)?.agent_id, "builtin-claude");
  assert.equal(store.find(claude.id)?.model, "sonnet");
});

test("bySlot 始终按 updated_at 倒序返回各槽位会话", async () => {
  const backend = createBackend({
    sessions: [
      makeSession({ id: "main-a", updated_at: 100 }),
      makeSession({ id: "main-c", updated_at: 900 }),
      makeSession({ id: "main-b", updated_at: 500 }),
      makeSession({ id: "agent-codex-old", kind: "agent", slot: "codex", updated_at: 50 }),
      makeSession({ id: "agent-codex-new", kind: "agent", slot: "codex", updated_at: 800 }),
    ],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.load();
  assert.deepEqual(plain(store.bySlot("main").map((session) => session.id)), ["main-c", "main-b", "main-a"]);
  assert.deepEqual(plain(store.bySlot("codex").map((session) => session.id)), ["agent-codex-new", "agent-codex-old"]);

  store.sessions.push(makeSession({ id: "main-d", updated_at: 1000 }));
  assert.equal(store.bySlot("main")[0].id, "main-d", "直接 push 到列表的会话也要参与排序");
});

test("rename/remove/bind 以 upsert 语义更新列表，不产生重复项", async () => {
  const backend = createBackend({
    sessions: [
      makeSession({ id: "main-a", updated_at: 300 }),
      makeSession({ id: "main-b", updated_at: 200 }),
      makeSession({ id: "agent-1", kind: "agent", slot: "codex", agent_id: "builtin-codex" }),
    ],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.load();

  await store.rename("main-a", "周报整理");
  assert.equal(store.find("main-a")?.title, "周报整理");
  assert.equal(store.sessions.length, 3, "rename 应更新原会话而不是新增一条");

  await store.bind("agent-1", { agentId: "builtin-codex", agentName: "codex", agentAvatar: "⌘", model: "gpt-5-codex" });
  const bound = store.find("agent-1");
  assert.equal(bound?.agent_id, "builtin-codex");
  assert.equal(bound?.agent_name, "codex");
  assert.equal(bound?.agent_avatar, "⌘");
  assert.equal(bound?.model, "gpt-5-codex");
  assert.equal(store.sessions.length, 3, "bind 应 upsert 而不是新增");

  await store.remove("main-a");
  assert.equal(store.find("main-a"), undefined);
  assert.equal(store.sessions.length, 2);
  assert.equal(store.activeMainId, "main-b", "删除活跃主会话后应使用后端回传的活跃 id");
});

test("chat-sessions-changed 事件触发节流后的列表 reload", async () => {
  const backend = createBackend({
    sessions: [makeSession({ id: "main-a" })],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.startSync();
  const loadsBefore = backend.countCalls("list_chat_sessions");

  backend.pushSession(makeSession({ id: "main-new", title: "后端新增", updated_at: 999 }));
  backend.emit("chat-sessions-changed");
  await delay(320);
  assert.ok(backend.countCalls("list_chat_sessions") > loadsBefore, "收到事件后应重新 load");
  assert.ok(store.find("main-new"), "reload 后列表应包含后端新增会话");

  // 连续事件只会合并成一次 reload
  const loadsAfterFirst = backend.countCalls("list_chat_sessions");
  backend.emit("chat-sessions-changed");
  backend.emit("chat-sessions-changed");
  await delay(320);
  assert.equal(backend.countCalls("list_chat_sessions"), loadsAfterFirst + 1);
  store.stopSync();
});

test("chat-session-changed 更新活跃主会话、upsert 会话并同步刷新列表", async () => {
  const backend = createBackend({
    sessions: [makeSession({ id: "main-a" })],
    activeId: "main-a",
  });
  const store = loadSessionsStore(backend);
  await store.startSync();

  backend.emit("chat-session-changed", {
    session: makeSession({ id: "main-b", title: "新对话 2", updated_at: 777 }),
  });
  assert.equal(store.activeMainId, "main-b");
  assert.equal(store.find("main-b")?.title, "新对话 2");

  await delay(320);
  assert.ok(backend.countCalls("list_chat_sessions") >= 2, "活跃会话变化后也应刷新列表");
  store.stopSync();
});

test("startSync/stopSync 注册与释放事件监听，stop 后不再响应", async () => {
  const backend = createBackend({ sessions: [makeSession({ id: "main-a" })], activeId: "main-a" });
  const store = loadSessionsStore(backend);
  await store.startSync();
  assert.deepEqual(
    [...backend.listeners.keys()].sort(),
    ["chat-history-cleared", "chat-session-changed", "chat-sessions-changed"],
  );
  assert.equal(backend.listeners.get("chat-sessions-changed").size, 1);

  store.stopSync();
  assert.equal(backend.listeners.get("chat-sessions-changed").size, 0);
  assert.equal(backend.listeners.get("chat-session-changed").size, 0);

  const loadsBefore = backend.countCalls("list_chat_sessions");
  backend.emit("chat-sessions-changed");
  await delay(320);
  assert.equal(backend.countCalls("list_chat_sessions"), loadsBefore, "stopSync 后事件不应再触发 reload");
});

test("较早发出的 load 响应不会覆盖较新的 load 结果", async () => {
  const pendings = [];
  const invoke = (command) => new Promise((resolve) => pendings.push({ command, resolve }));
  const { useSessionsStore } = loadModule("src/stores/sessions.ts", {
    "@tauri-apps/api/core": { invoke },
    "@tauri-apps/api/event": { listen: async () => () => {} },
  });
  const store = useSessionsStore(createPinia());
  const first = store.load();
  const second = store.load();
  assert.equal(pendings.length, 4, "两次 load 各调用 list_chat_sessions 与 get_active_chat_session");
  const resolvePair = (listIndex, activeIndex, sessions, active) => {
    pendings[listIndex].resolve(sessions);
    pendings[activeIndex].resolve(active);
  };
  // 第二次 load 先返回，第一次 load 后返回
  resolvePair(2, 3, [makeSession({ id: "main-new", title: "新" })], makeSession({ id: "main-new", title: "新" }));
  await second;
  resolvePair(0, 1, [makeSession({ id: "main-old", title: "旧" })], makeSession({ id: "main-old", title: "旧" }));
  await first;
  assert.equal(store.sessions.length, 1);
  assert.equal(store.sessions[0].id, "main-new");
  assert.equal(store.activeMainId, "main-new");
});
test("dev mock：多会话命令、session 化事件与 agent 回显", async () => {
  const mock = loadDevMock();

  // 初次访问自动 seed 默认主会话 + 三个 agent 会话
  const list = await mock.invoke("list_chat_sessions", {});
  assert.ok(list.filter((session) => session.slot === "main").length >= 1);
  assert.deepEqual(
    ["claude", "codex", "dsh"].map((slot) => list.some((session) => session.slot === slot)),
    [true, true, true],
  );
  const active = await mock.invoke("get_active_chat_session", {});
  assert.equal(active.slot, "main");
  assert.ok(list.every((session) => typeof session.message_count === "number" && typeof session.preview === "string"));

  // create / bind 走通，bind 未传的字段保持原值
  const codex = await mock.invoke("create_chat_session", {
    slot: "codex", agentId: "builtin-codex", model: "gpt-5-codex", title: "",
  });
  assert.equal(codex.slot, "codex");
  assert.equal(codex.kind, "agent");
  assert.equal(codex.agent_id, "builtin-codex");
  assert.equal(codex.model, "gpt-5-codex");
  const bound = await mock.invoke("set_chat_session_binding", { id: codex.id, model: "gpt-5" });
  assert.equal(bound.model, "gpt-5");
  assert.equal(bound.agent_id, "builtin-codex", "bind 未传的字段应保持原值");

  const createdMain = await mock.invoke("create_chat_session", { slot: "main", title: "" });
  assert.equal(createdMain.kind, "main");
  assert.equal((await mock.invoke("get_active_chat_session", {})).id, createdMain.id);

  const startedNew = await mock.invoke("start_new_conversation", {});
  assert.equal(startedNew.slot, "main");
  assert.match(startedNew.title, /^新对话 \d+$/);
  assert.equal((await mock.invoke("get_active_chat_session", {})).id, startedNew.id);

  await mock.invoke("set_active_chat_session", { id: active.id });
  assert.equal((await mock.invoke("get_active_chat_session", {})).id, active.id);
  await assert.rejects(() => mock.invoke("set_active_chat_session", { id: codex.id }), /只有主对话会话/);

  // send_to_ai：用户消息落库、事件带 session_id、完成后历史含助手回复
  const started = await mock.invoke("send_to_ai", { message: "你好，多会话", sessionId: active.id });
  assert.equal(started.started, true);
  await assert.rejects(
    () => mock.invoke("send_to_ai", { message: "并发第二条", sessionId: active.id }),
    /AI 正在思考中/,
  );
  const runningChat = await mock.invoke("get_active_chat");
  assert.equal(runningChat.session_id, active.id);

  await delay(2200);
  const thinking = mock.eventsOf("ai-thinking")[0];
  assert.equal(thinking.payload.session_id, active.id);
  assert.equal(typeof thinking.payload.text, "string");
  const deltas = mock.eventsOf("ai-answer-delta");
  assert.ok(deltas.length >= 1);
  assert.ok(deltas.every((item) => item.payload.session_id === active.id));
  const finished = mock.eventsOf("ai-finished")[0];
  assert.equal(finished.payload.session_id, active.id);
  assert.equal(typeof finished.payload.text, "string");
  assert.equal(await mock.invoke("get_active_chat"), null);

  const history = await mock.invoke("get_chat_history", { sessionId: active.id });
  const userIndex = history.findIndex((message) => message.role === "user" && message.content === "你好，多会话");
  assert.ok(userIndex >= 0, "用户消息应落在 session 历史里");
  const replyIndex = history.findIndex(
    (message, index) => index > userIndex && message.role === "assistant" && message.content.includes("预览模式收到"),
  );
  assert.ok(replyIndex > userIndex, "助手回复应跟在本轮用户消息之后");
  assert.ok(history.every((message) => message.session_id === active.id));
  const sessionMessages = await mock.invoke("get_chat_session_messages", { sessionId: active.id });
  assert.deepEqual(plain(sessionMessages.map((message) => message.id)), plain(history.map((message) => message.id)));

  // agent 窗口回合 + 多智能体/多主对话并行：codex 在回答时 dsh 与新主对话可以同时发起回合
  const dshSession = (await mock.invoke("list_chat_sessions", { slot: "dsh" }))[0];
  const codexParallel = await mock.invoke("create_chat_session", {
    slot: "codex", agentId: "builtin-codex", model: "gpt-5-codex", title: "Codex并行",
  });
  const secondMain = await mock.invoke("create_chat_session", { slot: "main", title: "并行主对话" });

  await mock.invoke("send_to_ai", {
    message: "codex 任务", sessionId: codexParallel.id, targetAgentId: "builtin-codex", model: "gpt-5-codex",
  });
  // 同一会话重复发送仍被拦截
  await assert.rejects(
    () => mock.invoke("send_to_ai", { message: "codex 重复发送", sessionId: codexParallel.id, targetAgentId: "builtin-codex" }),
    /AI 正在思考中/,
  );
  // 但 dsh 智能体窗口与另一个主对话会话可以并行启动！
  const dshStarted = await mock.invoke("send_to_ai", {
    message: "agent 测试", sessionId: dshSession.id, targetAgentId: "builtin-dsh", model: "deepseek-chat",
  });
  assert.equal(dshStarted.started, true);
  const secondMainStarted = await mock.invoke("send_to_ai", {
    message: "并行主对话提问", sessionId: secondMain.id,
  });
  assert.equal(secondMainStarted.started, true);

  assert.equal((await mock.invoke("get_active_chat", { sessionId: codexParallel.id })).session_id, codexParallel.id);
  assert.equal((await mock.invoke("get_active_chat", { sessionId: dshSession.id })).session_id, dshSession.id);
  assert.equal((await mock.invoke("get_active_chat", { sessionId: secondMain.id })).session_id, secondMain.id);

  await delay(2200);
  const finishedEvents = mock.eventsOf("ai-finished");
  const dshFinished = finishedEvents.find((event) => event.payload.session_id === dshSession.id);
  assert.ok(dshFinished, "dsh 并行回合应正常收到 ai-finished");
  assert.equal(dshFinished.payload.agent_name, "dsh");
  assert.match(dshFinished.payload.text, /dsh/);
  const codexFinished = finishedEvents.find((event) => event.payload.session_id === codexParallel.id);
  assert.ok(codexFinished, "codex 并行回合应正常收到 ai-finished");
  assert.equal(codexFinished.payload.agent_name, "codex");
  const secondMainFinished = finishedEvents.find((event) => event.payload.session_id === secondMain.id);
  assert.ok(secondMainFinished, "并行主对话应正常收到 ai-finished");

  const agentHistory = await mock.invoke("get_chat_history", { sessionId: dshSession.id });
  assert.equal(agentHistory.at(-1).agent_name, "dsh");
  const mainHistory = await mock.invoke("get_chat_history", { sessionId: active.id });
  assert.ok(!mainHistory.some((message) => message.content === "agent 测试"), "agent 窗口消息不能混进主会话");
  const secondMainHistory = await mock.invoke("get_chat_history", { sessionId: secondMain.id });
  assert.ok(secondMainHistory.some((message) => message.role === "user" && message.content === "并行主对话提问"));
  assert.ok(secondMainHistory.some((message) => message.role === "assistant" && message.content.includes("预览模式收到")));

  // rename / delete / clear
  const renamed = await mock.invoke("rename_chat_session", { id: active.id, title: "主会话改名" });
  assert.equal(renamed.title, "主会话改名");
  await mock.invoke("delete_chat_session", { id: codex.id });
  assert.equal(await mock.invoke("get_chat_session", { id: codex.id }), null);
  const cleared = await mock.invoke("clear_chat_history");
  assert.ok(cleared.active_main_session_id);
  const afterClear = await mock.invoke("list_chat_sessions", {});
  assert.equal(afterClear.filter((session) => session.slot === "main").length, 1);
  assert.equal((await mock.invoke("get_chat_history", { sessionId: cleared.active_main_session_id })).length, 0);
});

test("ChatBubble ai-thinking 兼容对象 payload，并按 session_id 只处理活跃主会话", () => {
  const callback = componentCallback("src/components/ChatBubble.vue", "ai-thinking");
  const { useChatStore } = loadModule("src/stores/chat.ts");
  const chat = useChatStore(createPinia());
  const thinkingContent = { value: "" };
  const state = {
    chat,
    thinkingContent,
    sessions: { activeMainId: "session-a" },
    resetLoadingTimeout() {},
    scrollToBottomAfterRender() {},
    isActiveMainSessionEvent(payload) {
      const sessionId = payload?.session_id;
      if (!sessionId) return true;
      return !state.sessions.activeMainId || sessionId === state.sessions.activeMainId;
    },
  };
  const handler = evaluate(`exports.run = (${callback});`, state).run;

  handler({ payload: { session_id: "session-b", text: "别的会话" } });
  assert.equal(thinkingContent.value, "", "非活跃会话的 ai-thinking 必须忽略");
  assert.equal(chat.activity.items.length, 0);
  assert.equal(chat.isLoading, false);

  handler({ payload: { session_id: "session-a", text: "当前会话" } });
  assert.equal(thinkingContent.value, "当前会话");
  assert.equal(chat.isLoading, true);
  assert.equal(chat.activity.items.at(-1).text, "当前会话");

  // 兼容旧后端直接发字符串的 payload
  handler({ payload: "旧格式" });
  assert.equal(thinkingContent.value, "当前会话旧格式");
});

test("ChatBubble sync-chat-message 只接收当前活跃主会话的广播", () => {
  const callback = componentCallback("src/components/ChatBubble.vue", "sync-chat-message");
  const { useChatStore } = loadModule("src/stores/chat.ts");
  const chat = useChatStore(createPinia());
  const thinkingContent = { value: "残留思考" };
  const state = {
    chat,
    sessions: { activeMainId: "session-a" },
    followingActivity: { value: false },
    pendingConfirm: { value: null },
    thinkingContent,
    streamingAnswer: { value: "" },
    isThinkingCollapsed: { value: true },
    pet: { setState() {} },
    isActiveMainSessionEvent(payload) {
      const sessionId = payload?.session_id;
      if (!sessionId) return true;
      return !state.sessions.activeMainId || sessionId === state.sessions.activeMainId;
    },
  };
  const handler = evaluate(`exports.run = (${callback});`, state).run;

  handler({ payload: { role: "user", content: "别的会话消息", session_id: "session-b" } });
  assert.equal(chat.messages.length, 0, "非活跃会话的 sync-chat-message 必须忽略");
  assert.equal(chat.isLoading, false);

  handler({ payload: { role: "user", content: "当前会话消息", files: [{ name: "a.txt", isImage: false, extension: "txt" }], session_id: "session-a" } });
  assert.equal(chat.messages.length, 1);
  assert.equal(chat.messages[0].content, "当前会话消息");
  assert.equal(chat.messages[0].files[0].name, "a.txt");
  assert.equal(chat.isLoading, true);
  assert.equal(thinkingContent.value, "");
  assert.equal(chat.activity.items.length, 0);

  // 旧 payload 没有 session_id 时保持旧行为
  handler({ payload: { role: "assistant", content: "旧格式广播" } });
  assert.equal(chat.messages.length, 2);
});

