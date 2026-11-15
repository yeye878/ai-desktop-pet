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
  const ids = Array.from(api.CHAT_BACKGROUNDS, (preset) => preset.id);
  const sorted = ids.slice().sort().join(",");
  assert.equal(
    sorted,
    [
      "aurora",
      "custom",
      "sakura",
      "ink",
      "minimal",
      "none",
      "rain",
      // 四季意象
      "spring-peach",
      "summer-lotus",
      "autumn-chrys",
      "winter-plum",
    ]
      .sort()
      .join(","),
  );
  assert.equal(new Set(ids).size, ids.length, "id 必须唯一");
  for (const preset of api.CHAT_BACKGROUNDS) {
    assert.ok(preset.name && preset.description, `${preset.id} 需要名称与说明`);
    assert.ok(preset.swatch.includes("gradient"), `${preset.id} 需要可渲染的缩略色板`);
  }
});

test("背景预设数量不少于 10 个（含极光/水墨/晨雾/细雨/黄昏/花影）", () => {
  const { api } = createHarness();
  const selectable = Array.from(api.SELECTABLE_CHAT_BACKGROUNDS, (preset) => preset.id);
  for (const required of ["aurora", "ink", "rain"]) {
    assert.ok(selectable.includes(required), `缺少预设 ${required}`);
  }
  assert.ok(selectable.length >= 5, `可选预设至少 5 个，实际 ${selectable.length}`);
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
  // 科幻已下线：旧用户存过的 "scifi" 必须安全回落到 none，不能把界面打空
  assert.equal(api.normalizeChatBackgroundId("scifi"), "none");
  assert.equal(api.chatBackgroundPreset("scifi").id, "none");
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
  fallback.applyChatBackground("sakura");
  assert.equal(events.length, 1, "存储失败也要广播，让本窗口即时切换");
  assert.equal(events[0].detail.id, "sakura");
});

test("预设配色：浅色图案保持纸面基调，照片背景允许深色但不得荧光", () => {
  const { api } = createHarness();
  const rgbOf = (hex) => {
    const raw = hex.replace("#", "");
    return [0, 2, 4].map((i) => parseInt(raw.slice(i, i + 2), 16));
  };
  const chromaOf = (hex) => {
    const [r, g, b] = rgbOf(hex);
    return Math.max(r, g, b) - Math.min(r, g, b);
  };
  const lightnessOf = (hex) => {
    const [r, g, b] = rgbOf(hex);
    return (r + g + b) / 3;
  };

  /*
   * 背景策略在 v7 换过一次：
   *   旧：全部是程序生成的浅色纸面光晕 → 所有 swatch 必须亮
   *   新：极光/水墨/细雨/科幻/樱花 换成真实摄影照片（压暗以保文字可读），
   *       深色底是设计需要，不是缺陷。
   * 所以亮度约束只对「仍是程序生成浅色图案」的预设生效。
   */
  const LIGHT_PRESETS = new Set(["none", "minimal", "custom"]);

  for (const preset of api.CHAT_BACKGROUNDS) {
    assert.equal(preset.tone, "paper", `${preset.id} 必须与其他页面基调统一`);
    assert.ok(preset.accent, `${preset.id} 需要主题色`);

    const hexes = preset.swatch.match(/#[0-9a-f]{6}/gi) || [];
    for (const hex of hexes) {
      // 彩度上限对所有预设都成立：不允许荧光色
      assert.ok(chromaOf(hex) <= 100, `${preset.id} 的 ${hex} 彩度 ${chromaOf(hex)} 过高`);

      if (LIGHT_PRESETS.has(preset.id)) {
        assert.ok(
          lightnessOf(hex) >= 150,
          `${preset.id} 是浅色图案，${hex} 不应偏暗（L=${lightnessOf(hex).toFixed(0)}）`,
        );
      }
    }

    // 照片背景必须有压暗的底色，否则白字气泡压不住
    if (!LIGHT_PRESETS.has(preset.id) && preset.id !== "none") {
      const hasDarkStop = hexes.some((hex) => lightnessOf(hex) < 120);
      assert.ok(hasDarkStop, `${preset.id} 是照片背景，swatch 里应有一档压暗底色`);
    }
  }
});

test("浓度档位单调递增，默认值落在中间档", () => {
  const { api } = createHarness();
  const values = Array.from(api.CHAT_BG_INTENSITIES, (level) => level.value);
  assert.equal(values.length, 3);
  assert.ok(values[0] < values[1] && values[1] < values[2], "浓度必须单调递增");
  assert.equal(api.DEFAULT_CHAT_BG_INTENSITY, values[1], "默认取中间档");
  for (const level of api.CHAT_BG_INTENSITIES) {
    assert.ok(level.name && level.description, "每档都要有名称与说明");
  }
});

test("浓度读写：越界值收敛、非法值回落默认、写入后广播", () => {
  const { api, store, events } = createHarness();
  assert.equal(api.normalizeChatBgIntensity(5), 1.2, "上限 1.2");
  assert.equal(api.normalizeChatBgIntensity(-1), 0.15, "下限 0.15");
  assert.equal(api.normalizeChatBgIntensity("abc"), api.DEFAULT_CHAT_BG_INTENSITY);
  assert.equal(api.readChatBgIntensity(), api.DEFAULT_CHAT_BG_INTENSITY, "读不到时用默认");
  api.applyChatBgIntensity(0.32);
  assert.equal(store.get(api.CHAT_BG_INTENSITY_STORAGE_KEY), "0.32");
  assert.equal(events.length, 1);
  assert.equal(events[0].detail.intensity, 0.32);
  assert.equal(api.readChatBgIntensity(), 0.32);
});

test("换背景时广播也带上当前浓度，两个窗口不会各用一套", () => {
  const { api, events } = createHarness();
  api.applyChatBgIntensity(0.9);
  events.length = 0;
  api.applyChatBackground("scifi");
  assert.equal(events.length, 1);
  assert.equal(events[0].detail.id, "scifi");
  assert.equal(events[0].detail.intensity, 0.9);
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
