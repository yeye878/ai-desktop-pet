const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");
const ts = require("typescript");
const { createPinia } = require("pinia");

const root = path.resolve(__dirname, "..");
const plain = (value) => JSON.parse(JSON.stringify(value));
function evaluate(source, globals = {}) {
  const exports = {};
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(compiled, { exports, require, console, ...globals });
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
function componentNode(file, predicate) {
  const vue = fs.readFileSync(path.join(root, file), "utf8");
  const script = vue.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const source = ts.createSourceFile(file + ".ts", script, ts.ScriptTarget.Latest, true);
  let result;
  function visit(node) {
    if (predicate(node, source)) result = node;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(result, `Production handler must exist in ${file}`);
  return { node: result, source };
}

const mentions = loadModule("src/services/mentions.ts");
test("@ suggestions stop at whitespace and ignore embedded email addresses", () => {
  assert.deepEqual(plain(mentions.mentionQueryFromInput("请找 @designer")), { query: "designer", start: 3 });
  assert.deepEqual(plain(mentions.mentionQueryFromInput("@")), { query: "", start: 0 });
  for (const input of ["@周报助手 ", "@周报助手\n", "me@designer", "me@周报助手"]) {
    assert.equal(mentions.mentionQueryFromInput(input), null, input);
  }
});

test("single/multiple mentions preserve order, accept s, and deduplicate names", () => {
  const inputs = [
    ["@周报助手 帮我写周报", ["周报助手"]],
    ["@周报助手 @代码助手 一起分析 @周报助手", ["周报助手", "代码助手"]],
    ["请找\n@designer\t@助手　分析", ["designer", "助手"]],
    ["me@designer and example@助手", []],
  ];
  for (const [input, expected] of inputs) assert.deepEqual(plain(mentions.extractMentionNames(input)), expected);
});

test("selecting an @ suggestion produces a routable message", () => {
  const input = "请找 @周";
  const selected = mentions.insertMentionAt(input, mentions.mentionQueryFromInput(input), "周报助手");
  assert.equal(selected, "请找 @周报助手 ");
  assert.deepEqual(plain(mentions.extractMentionNames(selected + "帮我写周报")), ["周报助手"]);
  assert.equal(mentions.mentionQueryFromInput(selected), null);
});

test("mention highlighting preserves text and whitespace without swallowing the task", () => {
  const input = "请 @周报助手 @designer 帮忙，邮箱 me@designer";
  const parts = plain(mentions.splitMentionSegments(input));
  assert.deepEqual(parts.filter((part) => part.type === "mention").map((part) => part.content), ["周报助手", "designer"]);
  assert.equal(parts.map((part) => (part.type === "mention" ? "@" : "") + part.content).join(""), input);
});

test("agent editor accepts designer and rejects whitespace or @ in names", async () => {
  const { node, source } = componentNode("src/components/Dashboard.vue", (node) =>
    ts.isFunctionDeclaration(node) && node.name?.text === "saveAgentDraft");
  for (const [name, valid] of [["designer", true], ["周报助手", true], ["分析 助手", false], ["分析\t助手", false], ["@助手", false]]) {
    const saved = [];
    const state = {
      agentSaveError: { value: "" }, agentEditorOpen: { value: true }, agentSaved: { value: false },
      agentDraft: { value: { id: "test", name, avatar: "🤖", description: "", system_prompt: "", model: "", allowed_tools: [] } },
      agentsStore: { upsert: async (agent) => saved.push(agent) }, setTimeout: () => 0,
    };
    const handler = evaluate(`exports.run = (${node.getText(source)});`, state).run;
    await handler();
    assert.equal(saved.length, valid ? 1 : 0, name);
    assert.equal(Boolean(state.agentSaveError.value), !valid, name);
  }
});

for (const component of ["Dashboard", "ChatBubble"]) {
  test(`${component} live completion preserves backend identity and clears loading`, () => {
    const { node, source } = componentNode(`src/components/${component}.vue`, (node) =>
      ts.isCallExpression(node) && node.expression.getText() === "listen" && node.arguments[0]?.text === "ai-finished");
    const callback = node.arguments[1].getText(source);
    const chatModule = loadModule("src/stores/chat.ts");
    const chat = chatModule.useChatStore(createPinia());
    const state = {
      chat, clearLoadingTimeout() {}, setTimeout() {},
      messageAgentFromPayload: chatModule.messageAgentFromPayload,
      toolEvents: { value: [] }, thinkingContent: { value: "" }, streamingAnswer: { value: "" },
      pendingConfirm: { value: null }, isThinkingCollapsed: { value: false },
      pet: { setState() {}, updateMood() {} }, speakDashboardReply() {}, stripQuoteMarkers: (text) => text, speakableText: (text) => text,
      appendAssistantOnce(text, thinking, agent) { chat.addMessage("assistant", text, thinking, undefined, undefined, undefined, agent); },
    };
    const handler = evaluate(`exports.run = (${callback});`, state).run;
    chat.isLoading = true;
    handler({ payload: { text: "本周工作已整理。", thinking: null, agent_id: "report", agent_name: "周报助手", agent_avatar: "📝" } });
    assert.deepEqual(plain(chat.messages[0].agent), { id: "report", name: "周报助手", avatar: "📝" });
    assert.equal(chat.isLoading, false);
    handler({ payload: { text: "普通桌宠回复", thinking: null } });
    assert.equal(chat.messages[1].agent, null);
  });
}

function agentStores() {
  let agents = [];
  const listeners = new Set();
  const events = {
    async listen(name, callback) {
      assert.equal(name, "agents-changed");
      listeners.add(callback);
      return () => listeners.delete(callback);
    },
  };
  const emit = () => listeners.forEach((callback) => callback({ payload: {} }));
  const core = {
    async invoke(command, args) {
      if (command === "list_agents") return structuredClone(agents);
      if (command === "save_agent") {
        agents = [...agents.filter((agent) => agent.id !== args.agent.id), structuredClone(args.agent)];
      } else if (command === "delete_agent") {
        agents = agents.filter((agent) => agent.id !== args.id);
      } else { throw new Error(command); }
      // The backend emits only after successfully persisting the change.
      emit();
    },
  };
  const { useAgentsStore } = loadModule("src/stores/agents.ts", {
    "@tauri-apps/api/core": core, "@tauri-apps/api/event": events,
  });
  return { dashboard: useAgentsStore(createPinia()), chat: useAgentsStore(createPinia()), listeners };
}

test("open windows synchronize agent create, rename and delete; unmount removes listeners", async () => {
  const { dashboard, chat, listeners } = agentStores();
  assert.equal(typeof dashboard.startSync, "function");
  await dashboard.startSync();
  await chat.startSync();
  const draft = { id: "report", name: "周报助手", avatar: "📝", description: "", system_prompt: "", model: "", allowed_tools: [], created_at: 1, updated_at: 1 };
  await dashboard.upsert(draft);
  await new Promise(setImmediate);
  assert.equal(chat.findByName("周报助手")?.id, "report");
  await dashboard.upsert({ ...draft, name: "designer" });
  await new Promise(setImmediate);
  assert.equal(chat.findByName("周报助手"), undefined);
  assert.equal(chat.findByName("designer")?.id, "report");
  await dashboard.remove("report");
  await new Promise(setImmediate);
  assert.equal(chat.agents.length, 0);
  dashboard.stopSync(); chat.stopSync();
  assert.equal(listeners.size, 0);
});

test("an older agents response cannot overwrite a newer refresh", async () => {
  const pending = [];
  const { useAgentsStore } = loadModule("src/stores/agents.ts", {
    "@tauri-apps/api/core": { invoke: () => new Promise((resolve) => pending.push(resolve)) },
    "@tauri-apps/api/event": { listen: async () => () => {} },
  });
  const store = useAgentsStore(createPinia());
  const first = store.load(); const second = store.load();
  pending[1]([{ id: "new", name: "新名字" }]); await second;
  pending[0]([{ id: "old", name: "旧名字" }]); await first;
  assert.equal(store.agents[0].name, "新名字");
});

test("closing a window during listener registration leaves no active subscription", async () => {
  let finishRegistration;
  let removed = false;
  let loads = 0;
  const { useAgentsStore } = loadModule("src/stores/agents.ts", {
    "@tauri-apps/api/core": { invoke: async () => { ++loads; return []; } },
    "@tauri-apps/api/event": { listen: () => new Promise((resolve) => { finishRegistration = resolve; }) },
  });
  const store = useAgentsStore(createPinia());
  const starting = store.startSync();
  store.stopSync();
  finishRegistration(() => { removed = true; });
  await starting;
  assert.equal(removed, true);
  assert.equal(loads, 0);
});

test("collaboration keeps distinct agent replies, deduplicates events, and stays busy through handoffs", () => {
  const { useChatStore } = loadModule("src/stores/chat.ts");
  const chat = useChatStore(createPinia());
  const running = { run_id: "team-1", status: "running", active_agent: { id: "a", name: "claude" }, queued: [{ id: "b", name: "codex" }], turn: 1, max_turns: 12, message: "工作中" };
  chat.receiveCollaborationState(running);
  const first = { run_id: "team-1", turn_id: "team-1:1", text: "已完成", agent_id: "a", agent_name: "claude", failed: false };
  assert.equal(chat.receiveCollaborationReply(first), true);
  assert.equal(chat.receiveCollaborationReply(first), false);
  chat.receiveCollaborationState({ ...running, active_agent: { id: "b", name: "codex" }, turn: 2, queued: [] });
  assert.equal(chat.receiveCollaborationReply({ ...first, turn_id: "team-1:2", agent_id: "b", agent_name: "codex" }), true);
  assert.equal(chat.isLoading, true);
  assert.deepEqual(plain(chat.messages.map(message => message.agent.name)), ["claude", "codex"]);
  chat.receiveCollaborationState({ ...running, status: "completed", active_agent: null, queued: [] });
  assert.equal(chat.isLoading, false);
  assert.equal(chat.receiveCollaborationReply({ ...first, turn_id: "team-1:late" }), false);
});

test("an old team's completion and reply cannot interrupt the next collaboration", () => {
  const { useChatStore } = loadModule("src/stores/chat.ts");
  const chat = useChatStore(createPinia());
  const current = { run_id: "new", status: "running", active_agent: null, queued: [], turn: 0, max_turns: 12, message: "新任务" };
  chat.receiveCollaborationState(current);
  assert.equal(chat.receiveCollaborationState({ ...current, run_id: "old", status: "aborted" }), false);
  assert.equal(chat.receiveCollaborationReply({ run_id: "old", turn_id: "old:1", text: "迟到的结果", agent_name: "codex" }), false);
  assert.equal(chat.isLoading, true);
  assert.equal(chat.messages.length, 0);
  chat.receiveCollaborationState({ ...current, status: "aborted" });
  assert.equal(chat.isLoading, false);
});

test("built-in CLI agents resolve both lowercase and display-case mentions", async () => {
  const { useAgentsStore } = loadModule("src/stores/agents.ts", {
    "@tauri-apps/api/core": { invoke: async () => [{ id: "builtin-claude", name: "claude", is_builtin: true }, { id: "builtin-codex", name: "codex", is_builtin: true }, { id: "builtin-dsh", name: "dsh", is_builtin: true }] },
    "@tauri-apps/api/event": { listen: async () => () => {} },
  });
  const store = useAgentsStore(createPinia());
  await store.load();
  assert.equal(store.findByName("Claude").id, "builtin-claude");
  assert.equal(store.findByName("Codex").id, "builtin-codex");
  assert.equal(store.findByName("codex").id, "builtin-codex");
  // DeepSeek Harness 的内置角色：@dsh 必须解析到内置卡，并且不被当成旧式同名自定义角色。
  assert.equal(store.findByName("DSH").id, "builtin-dsh");
  assert.equal(store.findByName("dsh").id, "builtin-dsh");
  assert.equal(store.mentionName(store.findByName("dsh")), "dsh");
});
