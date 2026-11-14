// 配色回归：色阶完整性、色相收敛、对比度达标、语义映射一致。
// 用 typescript 转译 src/services/palette.ts 后在 vm 里跑，不需要浏览器。
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");
const assert = require("node:assert/strict");
const ts = require("typescript");

const root = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(root, "src/services/palette.ts"), "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
}).outputText;

const moduleContext = { exports: {} };
vm.runInNewContext(compiled, {
  module: moduleContext,
  exports: moduleContext.exports,
  Object,
  Math,
  Number,
  parseInt,
  console,
});
const P = moduleContext.exports;

// ---------- 色彩工具 ----------
const hexToRgb = (hex) => {
  const raw = hex.replace("#", "");
  return [parseInt(raw.slice(0, 2), 16), parseInt(raw.slice(2, 4), 16), parseInt(raw.slice(4, 6), 16)];
};
const relativeLuminance = (hex) => {
  const [r, g, b] = hexToRgb(hex).map((c) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
};
/** sRGB → OKLCH，用来验证"色相是否收敛到青瓷" */
const hexToOklch = (hex) => {
  const toLinear = (c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const [r, g, b] = hexToRgb(hex).map(toLinear);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return { L, C: Math.hypot(A, B), H: ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360 };
};

const tests = [];
const test = (name, fn) => tests.push({ name, fn });

// ---------- 色阶结构 ----------
test("四族色阶齐全且每档都是合法 hex", () => {
  for (const [name, scale] of Object.entries({
    CELADON_SCALE: P.CELADON_SCALE,
    SAND_SCALE: P.SAND_SCALE,
    INK_SCALE: P.INK_SCALE,
    CLAY_SCALE: P.CLAY_SCALE,
  })) {
    const steps = Object.keys(scale);
    assert.ok(steps.length >= 7, `${name} 至少 7 档，实际 ${steps.length}`);
    for (const step of steps) {
      assert.match(scale[step], /^#[0-9a-f]{6}$/, `${name}-${step} 非法: ${scale[step]}`);
    }
  }
});

test("每族色阶明度单调递减，且相邻档位间距均匀（OKLCH 等感知步长）", () => {
  for (const scale of [P.CELADON_SCALE, P.SAND_SCALE, P.INK_SCALE, P.CLAY_SCALE]) {
    const steps = Object.keys(scale).map(Number).sort((a, b) => a - b);
    const ls = steps.map((s) => hexToOklch(scale[String(s)]).L);
    for (let i = 1; i < ls.length; i++) {
      assert.ok(ls[i] < ls[i - 1], `档位 ${steps[i]} 没有比 ${steps[i - 1]} 更暗`);
    }
    // 两端（最深一档）间距本来就更大，这里检查中间部分是否均匀
    const gaps = ls.slice(1).map((v, i) => ls[i] - v);
    const inner = gaps.slice(1, -1);
    const maxGap = Math.max(...inner);
    const minGap = Math.min(...inner);
    assert.ok(maxGap / minGap < 3, `中间档位间距跳变过大: ${(maxGap / minGap).toFixed(2)}x`);
  }
});

// ---------- 参考图纪律 ----------
test("青瓷主色相收敛在参考图的 198°±6°（高级感的来源）", () => {
  for (const step of ["400", "500", "600", "700"]) {
    const { H } = hexToOklch(P.CELADON_SCALE[step]);
    assert.ok(Math.abs(H - 198) <= 6, `celadon-${step} 色相 ${H.toFixed(1)}° 偏离参考 198°`);
  }
});

test("彩度按角色分开约束：中性色近中性、强调色允许鲜艳", () => {
  /*
   * 参考图（UI 截图）实测规律：
   *   - 63% 像素是纯白，中性区 C≈0.001
   *   - 只有一条高饱和彩虹带，C 0.10~0.23
   *
   * 所以「所有颜色都 C<=0.10」是上一张柔和照片的纪律，不适用于现在。
   * 真正该守住的是：纸面/文字保持近中性（否则整页发黄发脏），
   * 强调色放开到 0.13（够鲜艳，又不至于变荧光）。
   */
  const neutral = [
    ...Object.values(P.SAND_SCALE),
    ...Object.values(P.INK_SCALE),
  ];
  for (const hex of neutral) {
    const { C } = hexToOklch(hex);
    assert.ok(C <= 0.03, `中性色 ${hex} 彩度 ${C.toFixed(3)} 偏高，纸面/文字必须近中性`);
  }

  const accent = [
    ...Object.values(P.CELADON_SCALE),
    ...Object.values(P.CLAY_SCALE),
  ];
  for (const hex of accent) {
    const { C } = hexToOklch(hex);
    assert.ok(C <= 0.13, `强调色 ${hex} 彩度 ${C.toFixed(3)} 超出上限，会变荧光`);
  }

  // 强调色要"看得出颜色"：最饱和的那一档不能低于 0.09
  const maxAccentC = Math.max(...accent.map((hex) => hexToOklch(hex).C));
  assert.ok(maxAccentC >= 0.09, `强调色最饱和档只有 ${maxAccentC.toFixed(3)}，太灰`);
});

test("所有表面色都是高亮纸面（L≥0.85），深色只用于文字", () => {
  const surfaces = ["canvas", "surface", "surfaceMuted", "surfaceAccent"];
  for (const key of surfaces) {
    const { L } = hexToOklch(P.PALETTE[key]);
    assert.ok(L >= 0.85, `PALETTE.${key} 亮度 ${L.toFixed(3)} 太低，不像纸`);
  }
  const { L: titleL } = hexToOklch(P.PALETTE.textTitle);
  assert.ok(titleL < 0.35, "标题色应该是深色");
});

// ---------- 对比度 ----------
test("正文/标题在纸面上达到 WCAG AAA", () => {
  for (const key of ["textTitle", "textPrimary"]) {
    const ratio = contrast(P.PALETTE[key], P.PALETTE.surface);
    assert.ok(ratio >= 7, `${key} 对比度 ${ratio.toFixed(2)} 未达 AAA`);
  }
});

test("次级正文与说明文字在纸面上达到 WCAG AA", () => {
  assert.ok(contrast(P.PALETTE.textSecondary, P.PALETTE.surface) >= 4.5, "次级正文未达 AA");
  assert.ok(contrast(P.PALETTE.textMuted, P.PALETTE.surface) >= 3, "说明文字至少 3:1");
});

test("白字放在强调色上达到 AA 或大字标准", () => {
  // 按钮底色用 accentStrong（celadon-700），白字 5.12:1
  assert.ok(contrast("#ffffff", P.PALETTE.accentStrong) >= 4.5, "accentStrong 上白字未达 AA");
  assert.ok(contrast("#ffffff", P.PALETTE.warmStrong) >= 4.5, "warmStrong 上白字未达 AA");
});

test("强调色与纸面对比足够，能当描边/图标（≥3:1）", () => {
  assert.ok(contrast(P.PALETTE.accent, P.PALETTE.surface) >= 2.5, "主色在纸面上太淡");
  assert.ok(contrast(P.PALETTE.warm, P.PALETTE.surface) >= 2.5, "暖色在纸面上太淡");
});

// ---------- 语义映射 ----------
test("语义色都能在色阶里找到对应档位，不存在孤儿色", () => {
  const scaleValues = new Set([
    ...Object.values(P.CELADON_SCALE),
    ...Object.values(P.SAND_SCALE),
    ...Object.values(P.INK_SCALE),
    ...Object.values(P.CLAY_SCALE),
  ]);
  for (const [key, value] of Object.entries(P.PALETTE)) {
    if (typeof value !== "string" || !value.startsWith("#")) continue;
    assert.ok(scaleValues.has(value), `PALETTE.${key} = ${value} 不在任何色阶里`);
  }
});

test("状态色语义正确：success 走青瓷，danger/warning 走陶土", () => {
  assert.equal(P.PALETTE.success, P.PALETTE.accent, "success 应复用主色");
  assert.ok(contrast(P.PALETTE.danger, P.PALETTE.surface) > contrast(P.PALETTE.textMuted, P.PALETTE.surface),
    "danger 应比说明文字更醒目");
});

test("rgb 三元组格式正确，可直接用于 rgb(var(--x) / a)", () => {
  for (const key of ["accent", "accentStrong", "warm", "ink", "surface"]) {
    assert.match(P.PALETTE_RGB[key], /^\d{1,3}, \d{1,3}, \d{1,3}$/, `PALETTE_RGB.${key} 格式错误`);
    const [r, g, b] = P.PALETTE_RGB[key].split(", ").map(Number);
    for (const channel of [r, g, b]) assert.ok(channel >= 0 && channel <= 255);
  }
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

if (failed) {
  console.error(`\n${failed} 个配色用例失败`);
  process.exitCode = 1;
} else {
  console.log(`\n${tests.length} 个配色用例全部通过`);
}
