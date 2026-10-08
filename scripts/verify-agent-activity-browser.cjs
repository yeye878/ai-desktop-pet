const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const baseUrl = process.env.ACTIVITY_BASE_URL || 'http://127.0.0.1:1441';
const output = path.join(process.env.TEMP || process.cwd(), 'pet-activity-browser-qa', 'screenshots');
fs.mkdirSync(output, { recursive: true });

(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'chromium' });
  try {
    for (const [name, viewport, suffix] of [
      ['desktop', { width: 1280, height: 820 }, ''],
      ['small-window', { width: 340, height: 400 }, '?window=chat'],
    ]) {
      const page = await browser.newPage({ viewport });
      const errors = [];
      page.on('pageerror', (error) => errors.push(error.message));
      await page.goto(baseUrl + suffix);
      if (!suffix) await page.getByText('对话互动', { exact: true }).click();
      await page.waitForTimeout(800);
      await page.clock.install();
      const send = (name, payload) => page.evaluate(async ({ name, payload }) => {
        const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
        await emit(name, payload);
      }, { name, payload });
      await send('ai-ask-user', { id: 'qa-question', question: '确认要继续检查吗？', options: [{ label: '继续', description: '继续执行剩余检查' }, { label: '停止', description: '停止本轮任务' }] });
      await page.locator('.user-question').waitFor();
      await page.locator('.question-option').first().click();
      await page.getByRole('button', { name: '提交回答' }).click();
      await page.locator('.user-question').waitFor({ state: 'detached' });
      await send('sync-chat-message', { role: 'user', content: '检查项目配置并运行测试' });
      await send('ai-answer-delta', { text: '先检查入口和测试配置。' });
      await page.getByText('先检查入口和测试配置。', { exact: true }).waitFor();
      await send('ai-thinking', '从入口确认调用关系');
      const event = { id: 'qa-read', tool_name: 'read_file', summary: '读取 src/main.ts', status: 'running', path: 'src/main.ts', arguments: '{"path":"src/main.ts"}' };
      await send('ai-tool-event', event);
      await page.locator('.activity-tool.running').waitFor();
      await page.locator('.activity-tool > summary').click();
      await send('ai-tool-event', { ...event, status: 'completed', output: '<img src=x onerror="window.__activityInjected=true">' });
      assert.equal(await page.locator('.activity-output img').count(), 0, 'tool output must render as text');
      assert.equal(await page.evaluate(() => window.__activityInjected), undefined);
      await send('ai-tool-event', { ...event, status: 'completed', output: '入口配置正常。' });
      assert.equal(await page.locator('.activity-tool[open]').count(), 1, 'status update must retain expanded details');
      await send('ai-thinking', '[执行结果]\n入口配置正常。\n');
      await send('ai-answer-delta', { text: '测试正在运行。' });
      await send('ai-tool-event', { id: 'qa-test', tool_name: 'run_command', summary: '运行测试', status: 'waiting', command: 'npm run verify:agent-activity', arguments: '{"command":"npm run verify:agent-activity"}' });
      await page.locator('.activity-tool.waiting').waitFor();
      await send('ai-tool-confirm', { id: 'qa-confirm', tool_name: 'run_command', summary: '运行测试', command: 'npm run verify:agent-activity', arguments: '{"command":"npm run verify:agent-activity"}' });
      await page.clock.fastForward(95_000);
      assert.equal(await page.locator('.activity-status').count(), 1, 'authorization wait must survive the 90-second UI timeout');
      await send('ai-tool-confirm-resolved', { id: 'qa-confirm', approved: true });
      await page.screenshot({ path: path.join(output, name + '-running.png') });
      await send('ai-tool-event', { id: 'qa-test', tool_name: 'run_command', status: 'completed', summary: '运行测试', output: '11 tests passed' });
      await send('ai-answer-delta', { text: '检查完成，测试通过。' });
      await send('ai-finished', { text: '先检查入口和测试配置。测试正在运行。检查完成，测试通过。', thinking: '从入口确认调用关系' });
      await page.getByText('检查完成，测试通过。', { exact: true }).waitFor();
      assert.equal(await page.locator('.activity-status').count(), 0);
      assert.equal(await page.locator('.activity-tool.completed').count(), 2);
      assert.equal(await page.getByText('先检查入口和测试配置。', { exact: true }).count(), 1);
      assert.equal(await page.locator('.activity-reasoning').count(), 1);
      await page.clock.resume();
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(output, name + '-completed.png') });

      await send('sync-chat-message', { role: 'user', content: '继续检查' });
      await send('ai-answer-delta', { text: '仍在检查，已保留进度。' });
      await send('ai-tool-event', { ...event, id: 'qa-interrupt' });
      await page.getByRole('button', { name: '停止生成' }).click();
      await page.locator(suffix ? '.bubble' : '.dash-msg-text').filter({ hasText: /^已中止$/ }).waitFor();
      assert.equal(await page.locator('.activity-tool.interrupted').count(), 1);
      assert.equal(await page.getByText('仍在检查，已保留进度。', { exact: true }).count(), 1);

      const container = page.locator(suffix ? '.chat-messages' : '.dash-chat-messages');
      await send('sync-chat-message', { role: 'user', content: '压力测试' });
      await send('ai-answer-delta', { text: Array.from({ length: 150 }, (_, index) => `过程 ${index}\n`).join('') });
      await page.waitForTimeout(550);
      await container.evaluate((element) => { element.scrollTop = 0; element.dispatchEvent(new Event('scroll')); });
      await page.waitForTimeout(50);
      await send('ai-answer-delta', { text: '追加输出' });
      await page.waitForTimeout(550);
      assert.ok(await container.evaluate((element) => element.scrollTop < 30), 'new output must not pull an inspecting user to the bottom');
      await page.getByRole('button', { name: '回到最新消息' }).click();
      await page.waitForTimeout(550);
      assert.ok(await container.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight < 80));
      const overflow = await page.locator('.agent-activity').evaluateAll((elements) => elements.some((element) => element.scrollWidth > element.clientWidth + 2));
      assert.equal(overflow, false, 'activity must fit its container');
      await send('ai-error', { message: 'fixture interruption', thinking: null, aborted: true });
      await page.evaluate(async () => {
        const { useChatStore } = await import('/src/stores/chat.ts');
        const store = useChatStore();
        const completed = store.messages.find((message) => message.displayContent === '检查完成，测试通过。');
        store.setMessages([{ role: 'assistant', content: completed.content, thinking: completed.thinking, timestamp: Math.floor(completed.timestamp / 1000), agent: completed.agent }]);
      });
      assert.equal(await page.locator('.activity-tool.completed').count(), 2, 'history reload must restore structured tools');
      assert.equal(await page.getByText('检查完成，测试通过。', { exact: true }).count(), 1);
      await page.evaluate(async () => {
        const { useChatStore } = await import('/src/stores/chat.ts');
        useChatStore().setMessages([{ role: 'assistant', content: '旧会话回答', thinking: '旧版推理\n[调用工具] 读取配置\n{"path":"src/main.ts"}\n[执行结果]\n正常', timestamp: Date.now() }]);
      });
      assert.equal(await page.locator('.activity-tool.completed').count(), 1, 'legacy history must still render tools');
      assert.equal(await page.getByText('旧会话回答', { exact: true }).count(), 1);
      await send('collaboration-state', { run_id: 'qa-team', status: 'running', active_agent: { id: 'qa-agent', name: 'claude' }, queued: [], turn: 1, max_turns: 8, message: '' });
      await send('ai-answer-delta', { text: '协同检查中' });
      await send('ai-tool-event', { ...event, id: 'team-read', status: 'completed', output: '协同结果' });
      await send('collaboration-reply', { run_id: 'qa-team', turn_id: 'team-1', text: '协同检查中', thinking: null, agent_name: 'claude', agent_id: 'qa-agent', failed: false });
      assert.equal(await page.locator('.activity-tool.completed').count(), 2, 'collaboration reply must retain its activity snapshot');
      await send('collaboration-state', { run_id: 'qa-team', status: 'running', active_agent: { id: 'qa-agent2', name: 'codex' }, queued: [], turn: 2, max_turns: 8, message: '' });
      assert.equal(await page.locator('.activity-status .activity-count').count(), 0, 'handoff starts with a clean activity stream');
      await send('collaboration-state', { run_id: 'qa-team', status: 'completed', active_agent: null, queued: [], turn: 2, max_turns: 8, message: '' });
      assert.deepEqual(errors, []);
      console.log(`${name}: live output, completion, expansion, 90s confirmation wait, abort, scrolling, overflow, history, and handoff passed`);
      await page.close();
    }
    const confirmPage = await browser.newPage({ viewport: { width: 390, height: 350 } });
    await confirmPage.goto(baseUrl + '/?window=tool-confirm&request=qa-panel');
    await confirmPage.waitForTimeout(700);
    await confirmPage.evaluate(async () => {
      const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
      await emit('ai-tool-confirm', { id: 'qa-panel', tool_name: 'run_command', summary: '运行回归测试', command: 'npm run verify:agent-activity', arguments: '{"command":"npm run verify:agent-activity"}' });
    });
    await confirmPage.getByText('运行回归测试', { exact: true }).waitFor();
    assert.equal(await confirmPage.locator('[data-tauri-drag-region]').count(), 1, 'only the titlebar should be draggable');
    await confirmPage.getByRole('button', { name: '拒绝并关闭' }).click();
    await confirmPage.screenshot({ path: path.join(output, 'tool-confirm-panel.png') });
    await confirmPage.close();
    const demo = await browser.newPage({ viewport: { width: 1280, height: 820 } });
    await demo.goto(baseUrl + '/?activity=demo');
    await demo.getByText('检查完成，活动流和历史消息均正常。', { exact: true }).waitFor();
    assert.equal(await demo.locator('.activity-tool.completed').count(), 2);
    await demo.screenshot({ path: path.join(output, 'demo.png') });
    await demo.close();
    console.log(`Screenshots: ${output}`);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
