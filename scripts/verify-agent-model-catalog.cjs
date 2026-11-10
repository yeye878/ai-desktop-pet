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

/** 从 .vue 的 <script setup> 里取出某个函数声明，单独求值（同 verify-multi-agent 的做法）。 */
function scriptFunction(file, name) {
  const vue = fs.readFileSync(path.join(root, file), "utf8");
  const script = vue.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const source = ts.createSourceFile(file + ".ts", script, ts.ScriptTarget.Latest, true);
  let found;
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) found = node;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(found, file + " 必须包含函数 " + name);
  return { node: found, source };
}

function componentText(file) {
  return fs.readFileSync(path.join(root, file), "utf8");
}

const EFFORTS_LOW_HIGH = [
  { id: "low", name: "Low" },
  { id: "high", name: "High" },
];
const EFFORTS_MAX_ONLY = [{ id: "max", name: "Max" }];

/**
 * 与契约第 2 节 + v2 provider 增量一致的目录夹具。
 * 注意同一个 model-flash 同时出现在 local-provider / remote-provider，用来验证 (provider,id) 唯一键。
 */
function makeCatalog(slot = "dsh") {
  return {
    slot,
    backend: slot === "dsh" ? "dsh" : slot,
    local_provider: "local-provider",
    local_model: "model-flash",
    local_model_label: "Local / model-flash",
    local_effort: "high",
    models: [
      { id: "model-flash", name: "Model Flash", provider: "local-provider", provider_label: "Local", default_effort: "high", efforts: EFFORTS_LOW_HIGH },
      { id: "model-pro", name: "Model Pro", provider: "local-provider", provider_label: "Local", default_effort: "high", efforts: EFFORTS_LOW_HIGH },
      { id: "model-flash", name: "Model Flash Remote", provider: "remote-provider", provider_label: "Remote", default_effort: "max", efforts: EFFORTS_MAX_ONLY },
      { id: "model-plain", name: "Model Plain", default_effort: "", efforts: [] },
    ],
    error: "",
    fetched_at: 1700000000000,
  };
}

// ---------- service 纯函数 ----------

test("effortLabel 覆盖已知 id 中文映射，未知 id 回退 fallback 或原 id", () => {
  const models = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke: async () => null } });
  const expected = {
    off: "关闭",
    minimal: "极低",
    low: "低",
    medium: "中",
    high: "高",
    xhigh: "极高",
    max: "最高",
    ultra: "极限",
  };
  for (const [id, label] of Object.entries(expected)) assert.equal(models.effortLabel(id), label, id);
  assert.equal(models.effortLabel("HIGH"), "高");
  assert.equal(models.effortLabel("future-effort"), "future-effort");
  assert.equal(models.effortLabel("future-effort", "未来档"), "未来档");
  assert.equal(models.effortLabel(""), "");
});

test("modelOptionFor / effortOptionsFor 支持 (provider, id) 精确匹配，空 id 用 local_provider+local_model", () => {
  const models = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke: async () => null } });
  const catalog = makeCatalog();
  // 不带 provider：优先解析本机渠道。
  assert.equal(models.modelOptionFor(catalog, "model-pro").name, "Model Pro");
  assert.equal(models.modelOptionFor(catalog, "").id, "model-flash");
  assert.equal(models.modelOptionFor(catalog, "").provider, "local-provider");
  // 同一 id 不同渠道：provider 精确匹配返回各自条目。
  assert.equal(models.modelOptionFor(catalog, "model-flash").name, "Model Flash");
  assert.equal(models.modelOptionFor(catalog, "model-flash", "local-provider").name, "Model Flash");
  assert.equal(models.modelOptionFor(catalog, "model-flash", "remote-provider").name, "Model Flash Remote");
  assert.equal(models.modelOptionFor(catalog, "model-flash", "remote-provider").provider_label, "Remote");
  // 未知 provider 退化为同 id 任一模型。
  assert.equal(models.modelOptionFor(catalog, "model-flash", "ghost-provider").name, "Model Flash");
  assert.equal(models.modelOptionFor(catalog, "missing"), undefined);
  assert.equal(models.modelOptionFor(null, "model-pro"), undefined);

  assert.deepEqual(plain(models.effortOptionsFor(catalog, "")).map((effort) => effort.id), ["low", "high"]);
  assert.deepEqual(plain(models.effortOptionsFor(catalog, "model-flash", "remote-provider")).map((effort) => effort.id), ["max"]);
  assert.deepEqual(plain(models.effortOptionsFor(catalog, "model-plain")), []);
  assert.deepEqual(plain(models.effortOptionsFor(catalog, "missing")), []);

  const noLocal = { ...catalog, local_model: "" };
  assert.equal(models.modelOptionFor(noLocal, ""), undefined);
});

test("providerLabel / isLocalProvider：空 provider 与 local_provider 都视为本机", () => {
  const models = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke: async () => null } });
  const catalog = makeCatalog();
  assert.equal(models.providerLabel(catalog, ""), "Local");
  assert.equal(models.providerLabel(catalog, "local-provider"), "Local");
  assert.equal(models.providerLabel(catalog, "remote-provider"), "Remote");
  assert.equal(models.providerLabel(catalog, "ghost-provider"), "ghost-provider");
  assert.equal(models.providerLabel(null, "remote-provider"), "remote-provider");
  assert.equal(models.providerLabel(null, ""), "");

  assert.equal(models.isLocalProvider(catalog, ""), true);
  assert.equal(models.isLocalProvider(catalog, "local-provider"), true);
  assert.equal(models.isLocalProvider(catalog, "remote-provider"), false);
  assert.equal(models.isLocalProvider(catalog, "ghost-provider"), false);
  assert.equal(models.isLocalProvider(null, ""), true);
  assert.equal(models.isLocalProvider(null, "remote-provider"), false);
});

// ---------- service 目录请求：缓存 / 并发去重 / force ----------

test("fetchAgentModelCatalog 按 slot 并发去重，命中缓存后不再 invoke，force 强制刷新", async () => {
  const calls = [];
  const resolvers = [];
  const invoke = (command, args) => {
    calls.push({ command, args });
    return new Promise((resolve) => resolvers.push(resolve));
  };
  const models = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke } });
  models.clearAgentModelCatalogCache();

  const first = models.fetchAgentModelCatalog("codex");
  const second = models.fetchAgentModelCatalog("codex");
  assert.equal(calls.length, 1, "同一 slot 的并发请求应合并为一次 invoke");
  assert.deepEqual(plain(calls[0]), { command: "get_agent_model_catalog", args: { slot: "codex", force: false } });
  resolvers.shift()(makeCatalog("codex"));
  const [a, b] = await Promise.all([first, second]);
  assert.equal(a.models.length, 4);
  assert.equal(a.local_provider, "local-provider");
  assert.equal(b.models.length, 4);

  const cached = await models.fetchAgentModelCatalog("codex");
  assert.equal(cached.models.length, 4);
  assert.equal(calls.length, 1, "60s 内应命中内存缓存");

  const forced = models.fetchAgentModelCatalog("codex", true);
  assert.equal(calls.length, 2, "force 刷新必须重新 invoke");
  assert.deepEqual(plain(calls[1].args), { slot: "codex", force: true });
  resolvers.shift()({ ...makeCatalog("codex"), fetched_at: 1700000009999 });
  assert.equal((await forced).fetched_at, 1700000009999);
});

test("fetchAgentModelCatalog 归一化脏 JSON（含 local_provider/model provider），失败可重试", async () => {
  let attempt = 0;
  const invoke = async () => {
    attempt += 1;
    if (attempt === 1) throw new Error("探测超时");
    return {
      slot: "dsh",
      backend: 7,
      local_provider: 5,
      local_model: "model-flash",
      local_model_label: null,
      local_effort: "high",
      models: [
        null,
        { name: "缺少 id" },
        { id: "model-flash", name: "", provider: 3, provider_label: 4, default_effort: 3, efforts: [{ id: "high", name: "" }, null, { id: "" }] },
      ],
      error: 9,
      fetched_at: "not-a-number",
    };
  };
  const models = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke } });
  await assert.rejects(models.fetchAgentModelCatalog("dsh"), /探测超时/);
  const catalog = await models.fetchAgentModelCatalog("dsh");
  assert.equal(attempt, 2, "失败后不能留下卡死的 in-flight 请求");
  assert.equal(catalog.slot, "dsh");
  assert.equal(catalog.backend, "");
  assert.equal(catalog.local_provider, "");
  assert.equal(catalog.local_model_label, "");
  assert.equal(catalog.error, "");
  assert.equal(catalog.fetched_at, 0);
  assert.equal(catalog.models.length, 1);
  assert.equal(catalog.models[0].id, "model-flash");
  assert.equal(catalog.models[0].name, "model-flash");
  assert.equal(catalog.models[0].provider, undefined);
  assert.equal(catalog.models[0].default_effort, "");
  assert.deepEqual(plain(catalog.models[0].efforts).map((effort) => effort.id), ["high"]);
});

// ---------- AgentModelPicker：选中模型时校正 effort ----------

test("AgentModelPicker 选模型时按 (provider, model) 校验 effort，不支持的 effort 回退空串", () => {
  const { node, source } = scriptFunction("src/components/AgentModelPicker.vue", "compatibleEffort");
  const service = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke: async () => null } });
  const run = (provider, modelId, effort) => {
    const handler = evaluate("exports.run = (" + node.getText(source) + ");", {
      catalog: { value: makeCatalog() },
      effortOptionsFor: service.effortOptionsFor,
      modelOptionFor: service.modelOptionFor,
    }).run;
    return handler(provider, modelId, effort);
  };
  assert.equal(run("local-provider", "model-flash", "high"), "high");
  assert.equal(run("local-provider", "model-flash", "max"), "", "本机渠道不支持的 effort 必须回退空串");
  assert.equal(run("remote-provider", "model-flash", "max"), "max", "同 id 的远端渠道有自己的 efforts");
  assert.equal(run("remote-provider", "model-flash", "high"), "");
  assert.equal(run("", "", "high"), "", "跟随本机时 effort 也回到跟随默认");
  assert.equal(run("", "model-plain", "high"), "", "不支持选择的模型回退空串");
  assert.equal(run("", "custom-model", "high"), "high", "目录外模型保留用户选择");
  assert.equal(run("", "custom-model", ""), "");
});

test("AgentModelPicker 搜索按 name/id/provider 大小写不敏感过滤", () => {
  const { node, source } = scriptFunction("src/components/AgentModelPicker.vue", "matchesKeyword");
  const run = (keyword, model) => {
    // 组件里 keyword = computed(() => searchQuery.trim().toLowerCase())，这里对齐其语义。
    const handler = evaluate("exports.run = (" + node.getText(source) + ");", {
      keyword: { value: keyword.trim().toLowerCase() },
    }).run;
    return handler(model);
  };
  const model = { id: "claude-opus-5", name: "Claude Opus 5", provider: "opendesgin", provider_label: "opendesgin", default_effort: "high", efforts: [] };
  assert.equal(run("", model), true);
  assert.equal(run("opus", model), true);
  assert.equal(run("CLAUDE", model), true);
  assert.equal(run("opendesgin", model), true);
  assert.equal(run("claude-opus-5", model), true);
  assert.equal(run("qwen", model), false);
  assert.equal(run("", { id: "m", name: "M", default_effort: "", efforts: [] }), true);
});

test("AgentModelPicker 组件按渠道分组、>12 个模型显示搜索、(provider,id) 为键并透传 provider", () => {
  const source = componentText("src/components/AgentModelPicker.vue");
  assert.match(source, /fetchAgentModelCatalog/);
  assert.match(source, /providerLabel/);
  assert.match(source, /provider: string;/);
  assert.ok(source.includes('emit("select", {'), "必须 emit select");
  assert.ok(source.includes("model: model.id,") && source.includes("provider,"), "选模型时必须带 provider");
  assert.ok(source.includes("modelRowKey"), "(provider,id) 组合键");
  assert.ok(source.includes('(model.provider || "") + ":" + model.id'), "行 key = provider:id");
  assert.ok(source.includes("groups"), "必须按渠道分组");
  assert.ok(source.includes("group.models.length"), "组标题必须显示该渠道模型数");
  assert.ok(source.includes("totalModels") && source.includes("> 12"), "模型总数 > 12 时显示搜索框");
  assert.ok(source.includes("searchQuery") && source.includes("dash-agent-model-search"), "必须有搜索输入框");
  assert.ok(source.includes("matchesKeyword"), "搜索过滤函数");
  assert.ok(source.includes("dash-agent-model-channel"), "触发器必须显示渠道");
  assert.ok(source.includes("provider_label") && source.includes("provider_label || model.provider"));
  assert.ok(source.includes("model.provider_label || model.provider ||"), "组标题取 provider_label");
  assert.ok(source.includes("provider: currentProvider.value"), "effort 只更新 effort 时带上当前 provider");
  assert.ok(source.includes("dash-agent-model-effort-chips"));
  assert.ok(source.includes("跟随本机默认"));
  assert.ok(source.includes("自定义模型 ID"));
  assert.ok(source.includes("event.key === \"Escape\"") || source.includes('event.key === "Escape"'));
  assert.doesNotMatch(source, /datalist/, "选择器不应再依赖原生 datalist");
});

// ---------- Dashboard 接线 ----------

test("Dashboard 用 provider 透传：picker 传 provider、bind 只写变化字段、底部显示渠道", () => {
  const source = componentText("src/components/Dashboard.vue");
  assert.ok(!source.includes("datalist"), "Dashboard 不应再出现 datalist");
  assert.ok(!source.includes("AGENT_MODEL_PRESETS"), "Dashboard 不应再出现 AGENT_MODEL_PRESETS 硬编码预设");
  assert.ok(!source.includes("agentModelPresets"), "旧的 agentModelPresets computed 必须删除");
  assert.ok(!source.includes("agentModelDraft"), "旧的模型输入草稿必须删除");
  assert.ok(source.includes('from "./AgentModelPicker.vue"'));
  assert.match(source, /<AgentModelPicker/);
  assert.match(source, /:slot="agentSlot"/);
  assert.match(source, /:provider="currentAgentProviderValue"/);
  assert.match(source, /@select="onAgentModelSelect"/);
  assert.ok(source.includes("currentAgentProviderValue"), "必须有 provider 派生值");
  assert.ok(source.includes("provider: string"), "窗口偏好必须持久化 provider");
  assert.ok(source.includes("agentCatalogs"), "Dashboard 必须缓存目录用于展示名解析");
  assert.ok(source.includes("ensureAgentCatalog") && source.includes("fetchAgentModelCatalog"), "挂载/切 slot 预取目录");
  assert.ok(source.includes("currentSessionChannelLabel"), "底部信息条必须显示渠道");
  assert.ok(source.includes("currentSessionModelLabel"));
  assert.match(source, /思考：\{\{ currentSessionEffortLabel \}\}/);
  assert.ok(source.includes("patch.provider = provider"), "bind 只传变化的 provider");
  assert.ok(source.includes("patch.model = model"));
  assert.ok(source.includes("patch.reasoningEffort = effort"));
  assert.ok(source.includes("sessions.bind(session.id, patch)"));
  assert.ok(source.includes("provider: pref.provider"), "新建会话必须带 provider");
});

// ---------- sessions store 透传 ----------

test("sessions store 的 create/bind 透传播 provider 与 reasoningEffort", async () => {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args });
    return { id: "sess-1", slot: "dsh", kind: "agent", title: "t", provider: "", reasoning_effort: "" };
  };
  const { useSessionsStore } = loadModule("src/stores/sessions.ts", {
    "@tauri-apps/api/core": { invoke },
    "@tauri-apps/api/event": { listen: async () => () => {} },
  });
  const store = useSessionsStore(createPinia());
  await store.create("dsh", { agentId: "builtin-dsh", model: "deepseek-v4.1-flash", provider: "opendesgin", reasoningEffort: "max" });
  assert.deepEqual(plain(calls[0]), {
    command: "create_chat_session",
    args: { slot: "dsh", agentId: "builtin-dsh", model: "deepseek-v4.1-flash", provider: "opendesgin", reasoningEffort: "max", title: "" },
  });
  await store.create("dsh", { model: "deepseek-v4-flash" });
  assert.equal(plain(calls[1].args).provider, "", "未指定 provider 时 create 必须传空串");
  assert.equal(plain(calls[1].args).reasoningEffort, "", "未指定 effort 时 create 必须传空串");

  await store.bind("sess-1", { model: "claude-opus-5", provider: "opendesgin", reasoningEffort: "" });
  assert.deepEqual(plain(calls[2]), {
    command: "set_chat_session_binding",
    args: { id: "sess-1", model: "claude-opus-5", provider: "opendesgin", reasoningEffort: "" },
  });
  await store.bind("sess-1", { model: "claude-opus-5" });
  assert.equal("provider" in plain(calls[3].args), false, "未指定 provider 时不能误清空会话 provider");
  assert.equal("reasoningEffort" in plain(calls[3].args), false, "未指定 effort 时不能误清空会话 effort");

  const storeSource = fs.readFileSync(path.join(root, "src/stores/sessions.ts"), "utf8");
  assert.match(storeSource, /provider: string/);
  assert.match(storeSource, /provider\?: string/);
  assert.match(storeSource, /reasoning_effort: string/);
  assert.match(storeSource, /reasoningEffort\?: string/);
});

// ---------- dev mock：三 slot 目录夹具 + v2 多渠道 ----------

function mockFixtureFunction() {
  const file = "src/dev-tauri-mock.ts";
  const source = ts.createSourceFile(file, fs.readFileSync(path.join(root, file), "utf8"), ts.ScriptTarget.Latest, true);
  let found;
  function visit(node) {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "mockAgentModelCatalog") found = node;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(found, "dev-tauri-mock 必须提供 mockAgentModelCatalog 夹具函数");
  const run = evaluate("exports.run = (" + found.getText(source) + ");").run;
  return (slot) => plain(run(slot));
}

test("dev mock 的三 slot 目录夹具结构与 v2 多渠道一致", () => {
  const catalogFor = mockFixtureFunction();
  for (const slot of ["claude", "codex", "dsh"]) {
    const catalog = catalogFor(slot);
    assert.equal(catalog.slot, slot);
    assert.equal(typeof catalog.backend, "string");
    assert.equal(typeof catalog.local_provider, "string");
    assert.equal(typeof catalog.local_model, "string");
    assert.equal(typeof catalog.local_model_label, "string");
    assert.equal(typeof catalog.local_effort, "string");
    assert.equal(catalog.error, "");
    assert.equal(typeof catalog.fetched_at, "number");
    assert.ok(Array.isArray(catalog.models) && catalog.models.length > 0, slot + " 必须有模型");
    for (const model of catalog.models) {
      assert.ok(model.id, slot + " 模型 id 不能为空");
      assert.equal(typeof model.name, "string");
      assert.equal(typeof model.default_effort, "string");
      assert.ok(Array.isArray(model.efforts) && model.efforts.length > 0, model.id + " 必须有 efforts");
      for (const effort of model.efforts) {
        assert.ok(effort.id, model.id + " effort id 不能为空");
        assert.equal(typeof effort.name, "string");
      }
    }
  }

  const dsh = catalogFor("dsh");
  assert.equal(dsh.backend, "dsh");
  assert.equal(dsh.local_provider, "opendesgin");
  assert.equal(dsh.local_model, "deepseek-v4.1-flash");
  assert.equal(dsh.local_model_label, "opendesgin / deepseek-v4.1-flash");
  assert.ok(dsh.models.length > 12, "dsh 夹具模型数必须 > 12 以便浏览器验收搜索框");
  for (const model of dsh.models) {
    assert.ok(model.provider, model.id + " 的 provider 不能为空");
    assert.ok(model.provider_label, model.id + " 的 provider_label 不能为空");
  }
  const byProvider = {};
  for (const model of dsh.models) (byProvider[model.provider] = byProvider[model.provider] || []).push(model.id);
  assert.ok((byProvider.opendesgin || []).length >= 5, "opendesgin 至少 5 个模型");
  for (const required of ["claude-opus-5", "claude-sonnet-5.5", "deepseek-v4.1-flash", "gemini-3.7-flash", "glm-5.2"]) {
    assert.ok((byProvider.opendesgin || []).includes(required), "opendesgin 必须包含 " + required);
  }
  assert.deepEqual(byProvider["deepseek-official"], ["deepseek-v4-flash", "deepseek-v4-pro", "deepseek-v4-flash-vision-exp"]);
  assert.deepEqual(byProvider.together, ["deepseek-v4.1-flash"]);
  assert.ok((byProvider.qwen || []).length >= 1, "qwen 渠道至少 1 个模型");
  // 同一 model id 出现在多个渠道：(provider,id) 才是唯一键。
  const flashProviders = dsh.models.filter((model) => model.id === "deepseek-v4.1-flash").map((model) => model.provider);
  assert.deepEqual(flashProviders.sort(), ["opendesgin", "qwen", "together"]);

  const opendesginModel = dsh.models.find((model) => model.provider === "opendesgin" && model.id === "claude-opus-5");
  assert.deepEqual(opendesginModel.efforts.map((effort) => effort.id), ["off", "low", "medium", "high", "xhigh", "max"]);
  const deepseekModel = dsh.models.find((model) => model.provider === "deepseek-official" && model.id === "deepseek-v4-flash");
  assert.deepEqual(deepseekModel.efforts.map((effort) => effort.id), ["off", "low", "high", "max"]);
  const togetherModel = dsh.models.find((model) => model.provider === "together");
  assert.deepEqual(togetherModel.efforts.map((effort) => effort.id), ["low", "medium", "high", "xhigh", "max"]);

  const codex = catalogFor("codex");
  assert.equal(codex.backend, "codex");
  assert.equal(codex.local_provider, "");
  assert.equal(codex.models.length, 6);
  for (const model of codex.models) {
    assert.equal(model.provider, undefined);
    assert.deepEqual(model.efforts.map((effort) => effort.id), ["low", "medium", "high", "xhigh", "max", "ultra"]);
  }

  const claude = catalogFor("claude");
  assert.equal(claude.backend, "claude_code");
  assert.equal(claude.local_provider, "");
  assert.deepEqual(claude.models.map((model) => model.id), ["opus", "sonnet", "haiku"]);
  for (const model of claude.models) {
    assert.deepEqual(model.efforts.map((effort) => effort.id), ["low", "medium", "high", "xhigh", "max"]);
  }
});

test("三个 slot 的 mock 目录经 service 解析后都能展示模型名、渠道与中文思考强度", () => {
  const catalogFor = mockFixtureFunction();
  const service = loadModule("src/services/agentModels.ts", { "@tauri-apps/api/core": { invoke: async () => null } });
  for (const slot of ["claude", "codex", "dsh"]) {
    const catalog = catalogFor(slot);
    assert.ok(service.modelOptionFor(catalog, ""), slot + " 的 local_model 必须能在 models 里找到");
    assert.ok(catalog.local_model_label.length > 0, slot + " 必须给出 local_model_label");
    const seen = new Set();
    for (const model of catalog.models) {
      const key = (model.provider || "") + ":" + model.id;
      assert.ok(!seen.has(key), slot + " 的 (provider,id) 必须唯一");
      seen.add(key);
      const option = service.modelOptionFor(catalog, model.id, model.provider);
      assert.equal(option.id, model.id, slot + " 的模型必须能按 (provider,id) 解析");
      assert.equal(option.provider, model.provider);
      const efforts = service.effortOptionsFor(catalog, model.id, model.provider);
      assert.ok(efforts.length > 0, model.provider + ":" + model.id + " 必须有可选思考强度");
      for (const effort of efforts) assert.ok(service.effortLabel(effort.id, effort.name).length > 0);
    }
  }
  const dsh = catalogFor("dsh");
  // 同一 id 不同渠道经 service 解析到对应渠道。
  assert.equal(service.modelOptionFor(dsh, "deepseek-v4.1-flash").provider, "opendesgin");
  assert.equal(service.modelOptionFor(dsh, "deepseek-v4.1-flash", "together").provider, "together");
  assert.equal(service.modelOptionFor(dsh, "deepseek-v4.1-flash", "qwen").provider, "qwen");
  assert.equal(service.providerLabel(dsh, "together"), "Together");
  assert.equal(service.providerLabel(dsh, ""), "opendesgin");
  assert.equal(service.isLocalProvider(dsh, "opendesgin"), true);
  assert.equal(service.isLocalProvider(dsh, "together"), false);
  assert.deepEqual(
    plain(service.effortOptionsFor(dsh, "claude-opus-5", "opendesgin")).map((effort) => service.effortLabel(effort.id, effort.name)),
    ["关闭", "低", "中", "高", "极高", "最高"],
  );
  assert.deepEqual(
    plain(service.effortOptionsFor(dsh, "deepseek-v4-flash", "deepseek-official")).map((effort) => service.effortLabel(effort.id, effort.name)),
    ["关闭", "低", "高", "最高"],
  );
  const claude = catalogFor("claude");
  assert.deepEqual(
    plain(service.effortOptionsFor(claude, "opus")).map((effort) => service.effortLabel(effort.id, effort.name)),
    ["低", "中", "高", "极高", "最高"],
  );
  const codex = catalogFor("codex");
  assert.deepEqual(
    plain(service.effortOptionsFor(codex, "gpt-6.1-terra")).map((effort) => effort.id),
    ["low", "medium", "high", "xhigh", "max", "ultra"],
  );
});

test("dev mock 会话 create/bind 落库 provider/reasoning_effort，并注册 get_agent_model_catalog 命令", () => {
  const source = fs.readFileSync(path.join(root, "src/dev-tauri-mock.ts"), "utf8");
  assert.match(source, /provider: string;/);
  assert.match(source, /provider: options\.provider \|\| ""/);
  assert.match(source, /provider: readArg\(args, "provider", "model_provider", ""\)/);
  assert.match(source, /provider !== undefined\) session\.provider = String\(provider\)/);
  assert.match(source, /reasoning_effort: string;/);
  assert.match(source, /reasoning_effort: options\.reasoningEffort \|\| ""/);
  assert.match(source, /reasoningEffort !== undefined\) session\.reasoning_effort = String\(reasoningEffort\)/);
  assert.match(source, /case "get_agent_model_catalog"/);
});

// ---------- package.json ----------

test("package.json 注册 verify:agent-model-catalog 脚本", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  assert.equal(pkg.scripts["verify:agent-model-catalog"], "node --test scripts/verify-agent-model-catalog.cjs");
});
