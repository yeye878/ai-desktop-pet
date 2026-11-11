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

const PRESETS = ['none', 'aurora', 'ink', 'cute', 'scifi', 'minimal', 'custom'];

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
    for (const id of ['aurora', 'ink', 'cute', 'scifi', 'minimal']) {
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

    // 3. 对话互动页：背景真的渲染在消息区背后，快捷条可切换且有可见反馈
    await page.getByText('对话互动', { exact: true }).first().click();
    await page.waitForSelector('.dash-chat-stage', { timeout: 15000 });
    const stage = page.locator('.dash-chat-stage');

    await page.locator('.dash-chat-bgbar-chip').filter({ hasText: '极光' }).click();
    await page.waitForTimeout(800);
    if ((await stage.getAttribute('data-chat-bg')) !== 'aurora') {
      throw new Error('quick chip did not switch the chat stage to aurora');
    }
    if ((await stage.locator('canvas').count()) !== 1) {
      throw new Error('chat stage canvas missing after switching to aurora');
    }
    if (!(await page.locator('.dash-chat-bg-hint').count())) {
      throw new Error('background switch hint did not appear');
    }
    await page.screenshot({ path: path.join(OUT, 'chat-aurora-full.png') });
    await stage.screenshot({ path: path.join(OUT, 'chat-aurora-stage.png') });

    for (const [label, file] of [
      ['水墨', 'chat-ink-stage.png'],
      ['科幻', 'chat-scifi-stage.png'],
      ['简洁', 'chat-minimal-stage.png'],
      ['可爱', 'chat-cute-stage.png'],
    ]) {
      await page.locator('.dash-chat-bgbar-chip').filter({ hasText: label }).click();
      await page.waitForTimeout(900);
      await stage.screenshot({ path: path.join(OUT, file) });
    }

    // 4. 关掉背景：canvas 消失，舞台去掉 has-bg
    await page.locator('.dash-chat-bgbar-chip').filter({ hasText: '无' }).click();
    await page.waitForTimeout(600);
    if ((await stage.locator('canvas').count()) !== 0) {
      throw new Error('canvas should be removed when the background is off');
    }
    if (await stage.evaluate((el) => el.classList.contains('has-bg'))) {
      throw new Error('has-bg class should be removed when the background is off');
    }

    // 5. 自定义图片：写入后刷新，快捷条应多出「自定义」并渲染图片
    await page.evaluate(() => {
      const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#223355"/></svg>';
      localStorage.setItem('ai-desktop-pet.chat-bg-custom', 'data:image/svg+xml;base64,' + btoa(svg));
      localStorage.setItem('ai-desktop-pet.chat-bg', 'custom');
    });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.sidebar-nav', { timeout: 20000 });
    await page.getByText('对话互动', { exact: true }).first().click();
    await page.waitForSelector('.dash-chat-stage', { timeout: 20000 });
    await page.waitForTimeout(700);
    if (!(await page.locator('.dash-chat-bgbar-chip').filter({ hasText: '自定义' }).count())) {
      throw new Error('custom background chip should appear once an image is stored');
    }
    await page.screenshot({ path: path.join(OUT, 'chat-custom-full.png') });

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
