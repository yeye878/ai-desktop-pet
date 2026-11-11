// 对话背景服务的离线回归：预设清单、localStorage 读写、广播事件。
// 用 typescript 转译 src/services/chatBackgrounds.ts 后在 vm 里跑，不需要浏览器。
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");
const assert = require("node:assert/strict");
const ts = require("typescript");

const root = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(root, "src/services/chatBackgrounds.ts"), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
}).outputText;

/** 极简 localStorage + window.dispatchEvent 替身 */
function createHarness(initial = {}) {
  const store = new Map(Object.entries(initial));
  const events = [];
  const localStorage = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
  const window = {
    dispatchEvent(event) { events.push(event); return true; },
  };
  class CustomEvent {
    constructor(type, init) { this.type = type; this.detail = init?.detail; }
  }
  const moduleContext = { exports: {} };
  vm.runInNewContext(compiled, {
    module: moduleContext,
    exports: moduleContext.exports,
    localStorage,
    window,
    CustomEvent,
    Object,
    Boolean,
    String,
    console,
  });
  return { api: moduleContext.exports, store, events };
}

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

test("预设清单包含重设计后的全部模式且 id 唯一", () => {
  const { api } = createHarness();
  // vm 里的数组与宿主不同 realm，deepEqual 会因原型链不同而失败，这里比字符串
  const ids = Array.from(api.CHAT_BACKGROUNDS, (preset) => preset.id).sort().join(",");
  assert.equal(ids, "aurora,custom,cute,ink,minimal,none,scifi");
  assert.equal(new Set(Array.from(api.CHAT_BACKGROUNDS, (preset) => preset.id)).size, 7, "id 必须唯一");
  for (const preset of api.CHAT_BACKGROUNDS) {
    assert.ok(preset.name && preset.description, `${preset.id} 需要名称与说明`);
    assert.ok(preset.swatch.includes("gradient"), `${preset.id} 需要可渲染的缩略色板`);
  }
});

test("新增的 aurora / ink 是可选预设，自定义不混进预设网格", () => {
  const { api } = createHarness();
  const selectable = Array.from(api.SELECTABLE_CHAT_BACKGROUNDS, (preset) => preset.id);
  assert.ok(selectable.includes("aurora"), "极光应出现在预设里");
  assert.ok(selectable.includes("ink"), "水墨应出现在预设里");
  assert.ok(!selectable.includes("custom"), "自定义单独占一格");
});

test("未知取值回落到 none，避免旧数据把界面打空", () => {
  const { api } = createHarness();
  assert.equal(api.normalizeChatBackgroundId("aurora"), "aurora");
  assert.equal(api.normalizeChatBackgroundId("legacy-thing"), "none");
  assert.equal(api.normalizeChatBackgroundId(null), "none");
  assert.equal(api.chatBackgroundPreset("scifi").id, "scifi");
  assert.equal(api.chatBackgroundPreset("??? ").id, "none");
});

test("读不到存储时不抛错，默认 none", () => {
  const { api } = createHarness();
  assert.equal(api.readChatBackground(), "none");
  assert.equal(api.readCustomChatBackground(), "");
});

test("applyChatBackground 写入 localStorage 并广播同一事件名", () => {
  const { api, store, events } = createHarness();
  api.applyChatBackground("ink");
  assert.equal(store.get(api.CHAT_BG_STORAGE_KEY), "ink");
  assert.equal(events.length, 1, "应广播一次背景变更");
  assert.equal(events[0].type, api.CHAT_BG_CHANGED_EVENT);
  assert.equal(events[0].detail.id, "ink");
});

test("写入自定义图片会自动切到 custom，清除后回到 none", () => {
  const { api, store } = createHarness();
  api.applyCustomChatBackground("data:image/png;base64,AAAA");
  assert.equal(store.get(api.CHAT_BG_CUSTOM_STORAGE_KEY), "data:image/png;base64,AAAA");
  assert.equal(store.get(api.CHAT_BG_STORAGE_KEY), "custom");
  api.clearCustomChatBackground();
  assert.equal(store.has(api.CHAT_BG_CUSTOM_STORAGE_KEY), false);
  assert.equal(store.get(api.CHAT_BG_STORAGE_KEY), "none");
});

test("localStorage 配额抛错时界面状态仍然生效", () => {
  const { api } = createHarness();
  const events = [];
  const throwing = {
    getItem: () => { throw new Error("denied"); },
    setItem: () => { throw new Error("QuotaExceededError"); },
    removeItem: () => { throw new Error("denied"); },
  };
  assert.equal(api.readChatBackground.call(null), "none");
  // 用注入失败的 localStorage 再跑一次模块
  const moduleContext = { exports: {} };
  vm.runInNewContext(compiled, {
    module: moduleContext,
    exports: moduleContext.exports,
    localStorage: throwing,
    window: { dispatchEvent: (event) => events.push(event) },
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init?.detail; } },
    Object, Boolean, String, console,
  });
  const fallback = moduleContext.exports;
  assert.equal(fallback.readChatBackground(), "none");
  fallback.applyChatBackground("cute");
  assert.equal(events.length, 1, "存储失败也要广播，让本窗口即时切换");
  assert.equal(events[0].detail.id, "cute");
});

test("三个窗口共用同一组 storage key（兼容老版本设置）", () => {
  const { api } = createHarness();
  assert.equal(api.CHAT_BG_STORAGE_KEY, "ai-desktop-pet.chat-bg");
  assert.equal(api.CHAT_BG_CUSTOM_STORAGE_KEY, "ai-desktop-pet.chat-bg-custom");
});

let failed = 0;
for (const { name, fn } of tests) {
  try {
    fn();
    console.log(`✔ ${name}`);
  } catch (error) {
    failed += 1;
    console.error(`✖ ${name}\n  ${error.message}`);
  }
}

if (failed > 0) {
  console.error(`\n${failed} 个对话背景用例失败`);
  process.exitCode = 1;
} else {
  console.log(`\n${tests.length} 个对话背景用例全部通过`);
}
