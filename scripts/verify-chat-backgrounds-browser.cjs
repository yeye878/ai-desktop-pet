// 对话背景改版视觉验证：个性外观页的实时预览与卡片，对话互动页的背景舞台与快捷切换条。
// 需要先起 dev server（dev 模式的 dev-tauri-mock 会接管全部 invoke）：
//   npx vite --port 1441 --strictPort --host 127.0.0.1
// 再跑：PLAYWRIGHT_MODULE=<playwright 路径> node scripts/verify-chat-backgrounds-browser.cjs
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');

const BASE = process.env.BG_BASE_URL || 'http://127.0.0.1:1441';
const OUT = path.join(process.env.TEMP || process.cwd(), 'pet-bg-qa');
fs.mkdirSync(OUT, { recursive: true });

// 顺序必须与 chatBackgrounds.ts 的 CHAT_BACKGROUNDS 完全一致 ——
// 测试用 PRESETS.indexOf(id) 去点第 n 张卡片，顺序错位就会点错。
const PRESETS = [
  'none', 'aurora', 'ink', 'rain', 'sakura',
  'spring-willow', 'summer-lotus', 'autumn-chrys', 'winter-plum',
  'minimal', 'custom',
];

(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chromium' });
  const errors = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1320, height: 860 } });
    page.on('pageerror', (error) => errors.push('pageerror: ' + error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push('console: ' + message.text());
    });

    // 1. 个性外观页：预设卡片与实时预览
    await page.goto(BASE + '/?window=main', { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.sidebar-nav', { timeout: 20000 });
    await page.getByText('个性外观', { exact: true }).click();
    await page.waitForSelector('.bg-panel', { timeout: 15000 });

    const cardCount = await page.locator('.dash-bg-card').count();
    if (cardCount !== PRESETS.length) {
      throw new Error(`expected ${PRESETS.length} preset cards, got ${cardCount}`);
    }
    if (!(await page.locator('.dash-bg-stage-frame').count())) {
      throw new Error('appearance live preview frame missing');
    }

    // 2. 逐个点预设：卡片高亮、预览标记一致、canvas 生效、localStorage 落盘
    for (const id of ['aurora', 'ink', 'rain', 'sakura', 'spring-willow', 'summer-lotus', 'autumn-chrys', 'winter-plum', 'minimal']) {
      await page.locator('.dash-bg-card').nth(PRESETS.indexOf(id)).click();
      await page.waitForTimeout(420);
      const stored = await page.evaluate(() => localStorage.getItem('ai-desktop-pet.chat-bg'));
      if (stored !== id) throw new Error(`card ${id} did not persist, got ${stored}`);
      const activeName = await page.locator('.dash-bg-card.active .dash-bg-card-name').innerText();
      const tagText = await page.locator('.dash-bg-stage-tag').innerText();
      if (!tagText.includes(activeName)) {
        throw new Error(`preview tag "${tagText}" does not match active card "${activeName}"`);
      }
      const canvasCount = await page.locator('.dash-bg-stage-frame canvas').count();
      if (canvasCount !== 1) throw new Error(`preview canvas missing for ${id}`);
      await page.locator('.dash-bg-stage-frame').screenshot({ path: path.join(OUT, `appearance-${id}.png`) });
    }
    await page.locator('.bg-panel').screenshot({ path: path.join(OUT, 'appearance-panel.png') });

    // 3. 背景现在只能在「个性外观」里调：在那边选，切到对话页验证生效
    //    （对话页顶部的快捷条已按要求移除）
    await page.getByText('对话互动', { exact: true }).first().click();
    await page.waitForSelector('.dash-chat-stage', { timeout: 15000 });
    const stage = page.locator('.dash-chat-stage');

    // 确认快捷条确实不存在了
    if (await page.locator('.dash-chat-bgbar').count()) {
      throw new Error('对话页不应再有背景快捷条（只能在个性外观里调）');
    }

    // 回外观页，逐个选背景，再回对话页核对
    const CASES = [
      ['极光', 'aurora', 'chat-aurora-stage.png'],
      ['水墨', 'ink', 'chat-ink-stage.png'],
      ['细雨', 'rain', 'chat-rain-stage.png'],
      ['简洁', 'minimal', 'chat-minimal-stage.png'],
      ['樱花', 'sakura', 'chat-sakura-stage.png'],
      ['春·柳', 'spring-willow', 'chat-spring-stage.png'],
      ['夏·荷', 'summer-lotus', 'chat-summer-stage.png'],
      ['秋·菊', 'autumn-chrys', 'chat-autumn-stage.png'],
      ['冬·梅', 'winter-plum', 'chat-winter-stage.png'],
    ];

    for (const [label, id, file] of CASES) {
      await page.getByText('个性外观', { exact: true }).first().click();
      await page.waitForSelector('.bg-panel', { timeout: 15000 });
      await page.locator('.dash-bg-card').filter({ hasText: label }).first().click();
      await page.waitForTimeout(420);

      const stored = await page.evaluate(() => localStorage.getItem('ai-desktop-pet.chat-bg'));
      if (stored !== id) throw new Error(`${label} 没写进 localStorage，实际 ${stored}`);

      await page.getByText('对话互动', { exact: true }).first().click();
      await page.waitForSelector('.dash-chat-stage', { timeout: 15000 });
      await page.waitForTimeout(700);

      const applied = await stage.getAttribute('data-chat-bg');
      if (applied !== id) throw new Error(`${label} 没应用到对话页，实际 ${applied}`);
      if ((await stage.locator('canvas').count()) !== 1) {
        throw new Error(`${label} 的 canvas 缺失`);
      }
      await stage.screenshot({ path: path.join(OUT, file) });
    }

    // 4. 关掉背景：canvas 消失，舞台去掉 has-bg
    await page.getByText('个性外观', { exact: true }).first().click();
    await page.waitForSelector('.bg-panel', { timeout: 15000 });
    await page.locator('.dash-bg-card').filter({ hasText: '无' }).first().click();
    await page.waitForTimeout(420);
    await page.getByText('对话互动', { exact: true }).first().click();
    await page.waitForSelector('.dash-chat-stage', { timeout: 15000 });
    await page.waitForTimeout(600);
    if ((await stage.locator('canvas').count()) !== 0) {
      throw new Error('canvas should be removed when the background is off');
    }
    if (await stage.evaluate((el) => el.classList.contains('has-bg'))) {
      throw new Error('has-bg class should be removed when the background is off');
    }


    // 5. 自定义图片：写入后刷新，外观页的「自定义」卡片可选中，对话页渲染图片
    await page.evaluate(() => {
      const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#2b6cb0"/></svg>';
      localStorage.setItem('ai-desktop-pet.chat-bg-custom', 'data:image/svg+xml;base64,' + btoa(svg));
      localStorage.setItem('ai-desktop-pet.chat-bg', 'custom');
    });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.sidebar-nav', { timeout: 20000 });
    await page.getByText('对话互动', { exact: true }).first().click();
    await page.waitForSelector('.dash-chat-stage', { timeout: 20000 });
    await page.waitForTimeout(700);

    const customApplied = await page.locator('.dash-chat-stage').getAttribute('data-chat-bg');
    if (customApplied !== 'custom') {
      throw new Error(`自定义背景没应用到对话页，实际 ${customApplied}`);
    }

    await page.getByText('个性外观', { exact: true }).first().click();
    await page.waitForSelector('.bg-panel', { timeout: 15000 });
    const customActive = await page.locator('.dash-bg-card.active').filter({ hasText: '自定义' }).count();
    if (!customActive) {
      throw new Error('外观页的「自定义」卡片应为选中态');
    }
    await page.screenshot({ path: path.join(OUT, 'appearance-custom.png') });


    // 清理，别污染用户真实设置
    await page.evaluate(() => {
      localStorage.removeItem('ai-desktop-pet.chat-bg-custom');
      localStorage.setItem('ai-desktop-pet.chat-bg', 'none');
    });

    const meaningful = errors.filter((entry) => !entry.includes('404'));
    if (meaningful.length) {
      console.log('runtime errors:', meaningful);
      throw new Error(`unexpected runtime errors: ${meaningful.length}`);
    }
    console.log('screenshots:', OUT);
    console.log('chat background browser checks passed');
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error('FAIL', error);
  process.exitCode = 1;
});
