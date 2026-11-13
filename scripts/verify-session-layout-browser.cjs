const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const baseUrl = process.env.SESSION_BASE_URL || 'http://127.0.0.1:1443';
const output = path.join(process.env.TEMP || process.cwd(), 'pet-session-layout-qa');
fs.mkdirSync(output, { recursive: true });
const send = (page, event, payload) => page.evaluate(async ({ event, payload }) => {
  const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
  await emit(event, payload);
}, { event, payload });

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe' });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12000);
    const errors = [];
    page.on('pageerror', err => { errors.push(err.message); console.error(err.message); });
    page.on('console', msg => { if (msg.type() === 'error' && msg.text().includes('Vue Global Error')) errors.push(msg.text()); });
    await page.addInitScript(() => {
      localStorage.setItem('dash.workspace-layout', 'two');
      localStorage.setItem('dash.workspace-panes', JSON.stringify(['mock-session-1', 'mock-session-2']));
      window.__layoutQa = { calls: [] };
    });
    await page.route('**/src/dev-tauri-mock.ts*', async route => {
      const response = await route.fetch();
      const body = await response.text();
      const marker = 'function handleMockCommand(cmd, args) {';
      await route.fulfill({ response, body: body.replace(marker, marker + `
        if (cmd === 'send_to_ai') { window.__layoutQa.calls.push(args); return null; }
      `) });
    });
    await page.goto(baseUrl);
    await page.locator('.sidebar-item-main').click();
    const pane = page.locator('.session-pane').nth(1);
    await pane.locator('.session-pane-composer input').fill('研究生成智能体功能');
    await pane.getByRole('button', { name: '发送', exact: true }).click();
    await send(page, 'ai-thinking', { session_id: 'mock-session-2', text: '先定位智能体生成入口并检查实现。' });
    await send(page, 'ai-answer-delta', { session_id: 'mock-session-2', text: '我继续把生成智能体的链路检查到底。' });
    await send(page, 'ai-tool-event', { session_id: 'mock-session-2', id: 'qa-read', tool_name: 'read_file', status: 'completed', summary: '生成入口', path: 'D:/ai-desktop-pet/src/components/AgentPicker.vue', output: '检查结果' });
    await page.screenshot({ path: path.join(output, 'multi-running.png') });

    await pane.getByRole('button', { name: '在单窗格中打开' }).click();
    const single = page.locator('.dash-chat-messages');
    await single.getByText('我继续把生成智能体的链路检查到底。', { exact: true }).waitFor();
    assert.equal(await single.locator('.activity-reasoning').count() > 0, true);
    assert.equal(await single.locator('.activity-tool').count() > 0, true);
    assert.equal(await page.locator('.dash-chat-send-btn').isDisabled(), true);
    await page.screenshot({ path: path.join(output, 'single-running.png') });

    // 模拟截图里的中断：后端没有保存该 assistant 消息，但两种布局都必须保留它。
    await send(page, 'ai-error', { session_id: 'mock-session-2', aborted: true, message: '已中断' });
    await single.getByText('已中止', { exact: true }).waitFor();
    for (let i = 0; i < 3; i++) {
      await page.getByRole('button', { name: '双窗格', exact: true }).click();
      await pane.getByText('已中止', { exact: true }).waitFor();
      await pane.getByText('我继续把生成智能体的链路检查到底。', { exact: true }).waitFor();
      if (i === 1) await page.getByRole('button', { name: '单窗格', exact: true }).click();
      else await pane.getByRole('button', { name: '在单窗格中打开' }).click();
      await single.getByText('已中止', { exact: true }).waitFor();
      assert.equal(await single.locator('.activity-tool').count(), 1);
      assert.equal(await single.locator('.activity-reasoning').count(), 1);
    }
    await page.locator('.dash-chat-input').fill('继续检查');
    await page.locator('.dash-chat-send-btn').click();
    await send(page, 'ai-thinking', { session_id: 'mock-session-2', text: '继续检查保存流程。' });
    await send(page, 'ai-answer-delta', { session_id: 'mock-session-2', text: '保存流程已确认。' });
    await send(page, 'ai-finished', { session_id: 'mock-session-2', text: '模型回复：已修复问题。', thinking: '继续检查保存流程。', agent_id: 'builtin-claude', agent_name: 'claude' });
    await single.getByText('模型回复：已修复问题。', { exact: true }).waitFor();
    await page.getByRole('button', { name: '双窗格', exact: true }).click();
    await pane.getByText('模型回复：已修复问题。', { exact: true }).waitFor();
    await pane.getByRole('button', { name: '在单窗格中打开' }).click();
    await single.getByText('模型回复：已修复问题。', { exact: true }).waitFor();
    assert.equal(await single.getByText('已中止', { exact: true }).count(), 1);
    assert.equal(await single.getByText('模型回复：已修复问题。', { exact: true }).count(), 1);
    await page.screenshot({ path: path.join(output, 'single-completed.png') });

    await page.locator('.sidebar-item').filter({ hasText: '首页' }).first().click();
    await page.getByRole('button', { name: 'Claude Code', exact: true }).click();
    await single.getByText('模型回复：已修复问题。', { exact: true }).waitFor();

    // 主对话同样保留过程与完成回复，并且不会混入 agent 会话。
    await page.getByRole('button', { name: '双窗格', exact: true }).click();
    const main = page.locator('.session-pane').first();
    await main.locator('.session-pane-composer input').fill('主会话任务');
    await main.getByRole('button', { name: '发送', exact: true }).click();
    await send(page, 'ai-thinking', { session_id: 'mock-session-1', text: '主会话思考' });
    await send(page, 'ai-answer-delta', { session_id: 'mock-session-1', text: '主会话运行中回复' });
    await main.getByRole('button', { name: '在单窗格中打开' }).click();
    await single.getByText('主会话运行中回复', { exact: true }).waitFor();
    await send(page, 'ai-finished', { session_id: 'mock-session-1', text: '主会话已完成', thinking: '主会话思考' });
    await single.getByText('主会话已完成', { exact: true }).waitFor();
    assert.equal(await single.getByText('模型回复：已修复问题。', { exact: true }).count(), 0);
    await page.getByRole('button', { name: '双窗格', exact: true }).click();
    await main.getByText('主会话已完成', { exact: true }).waitFor();
    await pane.getByText('模型回复：已修复问题。', { exact: true }).waitFor();
    assert.deepEqual(errors, []);
    console.log('PASS: running switch, abort without saved history, completed reply, repeated toolbar/expand switches, page navigation, main/agent isolation and intact thinking/tool records.');
    console.log('Screenshots: ' + output);
  } finally { await browser.close(); }
})().catch(err => { console.error(err); process.exitCode = 1; });
