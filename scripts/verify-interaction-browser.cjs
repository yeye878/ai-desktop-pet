const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const baseUrl = process.env.ACTIVITY_BASE_URL || 'http://127.0.0.1:1441';
const output = path.join(process.env.TEMP || process.cwd(), 'pet-activity-browser-qa', 'interactions');
fs.mkdirSync(output, { recursive: true });
const question = (id) => ({ id, question: '你希望怎样处理这个项目？', options: [{ label: '保留当前配置', description: '继续验证现有功能' }, { label: '更新配置', description: '使用新的设置' }] });
const confirmation = (id) => ({ id, tool_name: 'run_command', summary: '运行中文项目的回归测试', command: 'npm run verify:agent-activity', path: 'D:/中文项目/src/main.ts', arguments: JSON.stringify({ command: 'npm run verify:agent-activity', note: '很长的参数'.repeat(300) }) });
const send = (page, event, payload) => page.evaluate(async ({ event, payload }) => {
  const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
  await emit(event, payload);
}, { event, payload });

async function fixture(page, pending = { questions: [], tool_confirms: [] }) {
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((pending) => { window.__interactionQa = { pending, calls: [], failNext: false, delay: false }; }, pending);
  await page.route('**/src/dev-tauri-mock.ts*', async (route) => {
    const response = await route.fetch();
    const body = await response.text();
    const marker = 'function handleMockCommand(cmd, args) {';
    assert.ok(body.includes(marker), 'mock command hook must exist');
    await route.fulfill({ response, body: body.replace(marker, marker + `
      const qa = window.__interactionQa;
      if (cmd === 'get_pending_interactions') return qa.pending;
      if (cmd === 'plugin:webview|get_all_webviews') return [{ windowLabel: 'main', label: 'main' }, { windowLabel: 'chat', label: 'chat' }];
      if (cmd === 'plugin:window|is_focused') return true;
      if (cmd.startsWith('plugin:window|') || cmd === 'plugin:webview|create_webview_window') qa.calls.push({ cmd, args });
      if (cmd === 'confirm_tool' || cmd === 'answer_question') {
        qa.calls.push({ cmd, args });
        if (qa.failNext) { qa.failNext = false; throw new Error('测试连接中断'); }
        const settle = async () => {
          const key = cmd === 'confirm_tool' ? 'tool_confirms' : 'questions';
          qa.pending[key] = qa.pending[key].filter(item => item.id !== args.id);
          await emit(cmd === 'confirm_tool' ? 'ai-tool-confirm-resolved' : 'ai-ask-user-resolved', { id: args.id, approved: args.approved, answered: true });
        };
        if (qa.delay) return new Promise(resolve => { qa.finish = async () => { await settle(); resolve(null); }; });
        return settle();
      }
    `) });
  });
  return errors;
}

(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chromium' });
  try {
    for (const [name, viewport, suffix] of [
      ['dashboard', { width: 1280, height: 820 }, ''],
      ['pet-chat', { width: 340, height: 400 }, '?window=chat'],
    ]) {
      const page = await browser.newPage({ viewport });
      const errors = await fixture(page, { questions: [question('initial')], tool_confirms: [] });
      await page.goto(baseUrl + suffix);
      await page.locator('.user-question').waitFor();
      assert.equal(await page.getByRole('button', { name: '提交回答' }).isDisabled(), true, 'no answer is chosen automatically');
      await page.clock.install();
      await page.clock.fastForward(95_000);
      assert.equal(await page.locator('.user-question').count(), 1);
      assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'answer_question').length), 0);
      await page.locator('.question-option').first().click();
      await page.getByRole('button', { name: '提交回答' }).click();
      await page.locator('.user-question').waitFor({ state: 'detached' });
      assert.equal(await page.evaluate(() => window.__interactionQa.calls.find(item => item.cmd === 'answer_question').args.answer), '保留当前配置');
      await page.clock.resume();

      await send(page, 'ai-ask-user', question('freeform'));
      await page.getByPlaceholder('填写答案').fill('保留中文路径\n只验证测试');
      await page.evaluate(() => { window.__interactionQa.failNext = true; });
      await page.getByRole('button', { name: '提交回答' }).click();
      await page.getByRole('alert').waitFor();
      assert.equal(await page.locator('.user-question').count(), 1, 'failed answers stay visible');
      await page.screenshot({ path: path.join(output, `${name}-question.png`) });
      await page.getByRole('button', { name: '提交回答' }).click();
      await page.locator('.user-question').waitFor({ state: 'detached' });
      assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'answer_question').at(-1).args.answer), '保留中文路径\n只验证测试');

      await send(page, 'ai-ask-user', question('skip'));
      await page.getByRole('button', { name: '跳过', exact: true }).click();
      assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'answer_question').at(-1).args.answer), '');
      await page.locator('.user-question').waitFor({ state: 'detached' });

      const many = { ...question('many'), options: Array.from({ length: 25 }, (_, index) => ({ label: `方案${index}`, description: '很长的中文说明'.repeat(20) })) };
      await send(page, 'ai-ask-user', many);
      await page.locator('.user-question').waitFor();
      assert.ok(await page.locator('.user-question').evaluate(element => element.scrollWidth <= element.clientWidth + 2));
      const bounds = await page.getByRole('button', { name: '提交回答' }).boundingBox();
      assert.ok(bounds.y + bounds.height <= viewport.height, 'footer stays in viewport with many options');
      await send(page, 'ai-ask-user-resolved', { id: 'many', answered: false });
      await page.locator('.user-question').waitFor({ state: 'detached' });

      if (!suffix) {
        await send(page, 'ai-tool-confirm', confirmation('inline-confirm'));
        await page.getByText('运行中文项目的回归测试', { exact: true }).waitFor();
        assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'plugin:webview|create_webview_window' && item.args?.options?.label === 'tool-confirm').length), 0, 'focused chat uses its inline confirmation');
        await send(page, 'ai-tool-confirm-resolved', { id: 'inline-confirm', approved: false });
      }
      assert.deepEqual(errors, []);
      console.log(`${name}: pending recovery, real selection, Chinese answers, failure retry, explicit skip and bounded layout passed`);
      await page.close();
    }

    const page = await browser.newPage({ viewport: { width: 390, height: 350 } });
    const errors = await fixture(page, { questions: [], tool_confirms: [confirmation('panel-initial')] });
    await page.goto(baseUrl + '/?window=tool-confirm');
    await page.getByText('运行中文项目的回归测试', { exact: true }).waitFor();
    assert.equal(await page.locator('[data-tauri-drag-region]').count(), 1);
    assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'plugin:window|set_focus' || item.cmd === 'plugin:window|set_always_on_top').length), 0, 'panel does not override background focus policy');
    const button = page.getByRole('button', { name: '允许', exact: true });
    const bounds = await button.boundingBox();
    assert.ok(bounds.y + bounds.height <= 350);
    assert.equal(await page.locator('.tool-confirm-window').evaluate(element => element.scrollWidth > element.clientWidth + 2), false);
    await page.evaluate(() => { window.__interactionQa.failNext = true; });
    await button.click(); await page.getByRole('alert').waitFor();
    await page.screenshot({ path: path.join(output, 'standalone-confirm.png') });
    await page.evaluate(() => { window.__interactionQa.delay = true; });
    await button.click();
    assert.equal(await button.isDisabled(), true);
    await send(page, 'ai-tool-confirm', confirmation('panel-next'));
    await page.evaluate(async () => { await window.__interactionQa.finish(); window.__interactionQa.delay = false; });
    assert.equal(await page.locator('.tool-confirm-body').count(), 1);
    assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'plugin:window|hide').length), 0, 'old submission does not hide the next request');
    await page.getByRole('button', { name: '拒绝并关闭' }).click();
    assert.equal(await page.evaluate(() => window.__interactionQa.calls.filter(item => item.cmd === 'confirm_tool').at(-1).args.approved), false);
    assert.deepEqual(errors, []);
    console.log('standalone confirm: backend recovery, long parameters, interactive buttons, failure retry, new-request race and focus policy passed');
    console.log(`Screenshots: ${output}`);
    await page.close();
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
