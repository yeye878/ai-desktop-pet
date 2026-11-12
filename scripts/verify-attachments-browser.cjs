const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const baseUrl = process.env.ATTACHMENTS_BASE_URL || 'http://127.0.0.1:1442';
const output = path.join(process.env.TEMP || process.cwd(), 'pet-attachments-qa');
fs.mkdirSync(output, { recursive: true });

async function nativeDrop(page, pane, paths, type = 'drop') {
  const box = await pane.boundingBox();
  await page.evaluate(async ({ box, paths, type }) => {
    const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
    await emit('tauri://drag-' + type, {
      paths,
      position: { x: (box.x + box.width / 2) * devicePixelRatio, y: (box.y + box.height / 2) * devicePixelRatio },
    });
  }, { box, paths, type });
}

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe' });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 820 }, deviceScaleFactor: 1.5 });
    page.setDefaultTimeout(15000);
    const errors = [];
    page.on('pageerror', error => { errors.push(error.message); console.error(error.message); });
    page.on('console', message => { if (message.type() === 'error' && message.text().includes('Vue Global Error')) errors.push(message.text()); });
    await page.addInitScript(() => {
      localStorage.setItem('dash.workspace-layout', 'four');
      localStorage.setItem('dash.workspace-panes', JSON.stringify(['mock-session-1', 'mock-session-2', 'mock-session-3', 'mock-session-4']));
      window.__attachmentQa = { calls: [], selected: ['C:/资料/新增文件.md'], delay: false };
    });
    await page.route('**/src/dev-tauri-mock.ts*', async route => {
      const response = await route.fetch();
      const body = await response.text();
      const marker = 'function handleMockCommand(cmd, args) {';
      assert.ok(body.includes(marker));
      await route.fulfill({ response, body: body.replace(marker, marker + `
        const qa = window.__attachmentQa;
        if (cmd === 'plugin:dialog|open') return qa.selected;
        if (cmd === 'get_file_metadata') {
          const metadata = args.paths.map(path => ({ path, name: path.split(/[\\\\/]/).pop(), size: path.includes('超大') ? 60 * 1024 * 1024 : 12345, extension: path.split('.').pop(), exists: true, is_file: true, is_dir: false }));
          if (qa.delay) return new Promise(resolve => { qa.complete = () => resolve(metadata); });
          return metadata;
        }
        if (cmd === 'send_to_ai') { qa.calls.push({ cmd, args }); return null; }
        if (cmd === 'read_file_as_data_url') throw new Error('fixture has no image bytes');
      `) });
    });
    await page.goto(baseUrl);
    await page.locator('.sidebar-item-main').click();
    await page.locator('.session-pane').nth(3).waitFor();
    const pane = index => page.locator('.session-pane').nth(index);

    await nativeDrop(page, pane(1), ['C:/资料/需求说明.pdf'], 'enter');
    assert.equal(await pane(1).evaluate(el => el.classList.contains('file-over')), true);
    assert.equal(await page.locator('.session-pane.file-over').count(), 1);
    await nativeDrop(page, pane(1), ['C:/资料/需求说明.pdf']);
    await pane(1).getByText('需求说明.pdf', { exact: true }).waitFor();
    assert.equal(await pane(0).locator('.attachment-card').count(), 0);
    assert.equal(await pane(2).locator('.attachment-card').count(), 0);
    assert.equal(await page.locator('.session-pane.file-over').count(), 0);
    await nativeDrop(page, pane(1), ['c:/资料/需求说明.pdf']);
    assert.equal(await pane(1).locator('.attachment-card').count(), 1);

    await pane(1).getByRole('button', { name: '添加附件', exact: true }).click();
    await pane(1).getByText('新增文件.md', { exact: true }).waitFor();
    await pane(1).getByRole('button', { name: '移除附件 新增文件.md', exact: true }).click();
    await pane(1).getByText('新增文件.md', { exact: true }).waitFor({ state: 'detached' });
    assert.equal(await pane(1).locator('.attachment-card').count(), 1);

    await nativeDrop(page, pane(2), ['C:/资料/代码.ts']);
    await pane(2).getByText('代码.ts', { exact: true }).waitFor();
    // 使用真正的指针拖动交换窗格，附件仍跟随会话。
    const from = await pane(1).locator('.session-pane-title').boundingBox();
    const to = await pane(2).locator('.session-pane-title').boundingBox();
    await page.mouse.move(from.x + 10, from.y + 10);
    await page.mouse.down();
    await page.mouse.move(to.x + 10, to.y + 10, { steps: 12 });
    await page.mouse.up();
    await pane(2).getByText('需求说明.pdf', { exact: true }).waitFor();
    await pane(1).getByText('代码.ts', { exact: true }).waitFor();

    await pane(2).getByRole('button', { name: '在单窗格中打开' }).click();
    await page.locator('.dash-chat-input-area').waitFor();
    await page.locator('.attachment-tray').getByText('需求说明.pdf', { exact: true }).waitFor();
    await page.getByRole('button', { name: '四窗格', exact: true }).click();
    await pane(2).getByText('需求说明.pdf', { exact: true }).waitFor();

    // 文件读取期间可移除，读取结束也不会复活；发送按钮在读取中不可用。
    await page.evaluate(() => { window.__attachmentQa.delay = true; });
    await nativeDrop(page, pane(0), ['C:/资料/慢文件.txt']);
    await pane(0).getByText('慢文件.txt', { exact: true }).waitFor();
    assert.equal(await pane(0).getByRole('button', { name: '发送', exact: true }).isDisabled(), true);
    await pane(0).getByRole('button', { name: '移除附件 慢文件.txt', exact: true }).click();
    await pane(0).getByText('慢文件.txt', { exact: true }).waitFor({ state: 'detached' });
    await page.evaluate(() => { window.__attachmentQa.complete(); window.__attachmentQa.delay = false; });
    assert.equal(await pane(0).locator('.attachment-card').count(), 0);

    await nativeDrop(page, pane(3), ['C:/资料/超大.zip']);
    await pane(3).getByRole('alert').waitFor();
    assert.match(await pane(3).getByRole('alert').innerText(), /文件过大/);
    await pane(3).getByRole('button', { name: '关闭附件提示' }).click();
    await nativeDrop(page, pane(3), ['C:/资料/' + '很长的文件名称'.repeat(12) + '.pdf']);
    await pane(3).locator('.attachment-card').waitFor();

    // 在常用与窄窗口中，卡片不会挤掉输入框与发送按钮。
    for (const width of [1280, 1024]) {
      await page.setViewportSize({ width, height: 820 });
      for (let index = 0; index < 4; index++) {
        assert.equal(await pane(index).evaluate(el => el.scrollWidth > el.clientWidth + 2), false);
        const sendBox = await pane(index).getByRole('button', { name: '发送', exact: true }).boundingBox();
        assert.ok(sendBox.y + sendBox.height <= 820);
        const inputBox = await pane(index).locator('.session-pane-composer input').boundingBox();
        assert.ok(inputBox.width >= 70);
      }
      await page.screenshot({ path: path.join(output, `four-panes-${width}.png`) });
    }

    // 仅发附件：发送到交换后的会话，消息气泡保留可见文件名。
    await pane(2).getByRole('button', { name: '发送', exact: true }).click();
    await pane(2).locator('.session-pane-sent-files').getByText(/需求说明.pdf/).waitFor();
    const call = await page.evaluate(() => window.__attachmentQa.calls.at(-1));
    assert.equal(call.args.sessionId, 'mock-session-2');
    assert.equal(call.args.attachments[0].path, 'C:/资料/需求说明.pdf');
    assert.equal(await pane(2).locator('.attachment-card').count(), 0);
    assert.equal(await pane(1).locator('.attachment-card').count(), 1);
    await pane(1).getByRole('button', { name: '全部移除', exact: true }).click();
    await pane(1).locator('.attachment-tray').waitFor({ state: 'detached' });
    assert.equal(await pane(1).locator('.attachment-tray').count(), 0);

    // 新建会话不继承旧会话附件。
    await pane(3).getByRole('button', { name: '新对话', exact: true }).click();
    await pane(3).locator('.attachment-tray').waitFor({ state: 'detached' });
    await nativeDrop(page, pane(3), ['C:/资料/新对话.txt']);
    await pane(3).getByText('新对话.txt', { exact: true }).waitFor();
    await pane(3).locator('.session-pane-switch').selectOption('mock-session-1');
    await pane(0).locator('.session-pane-empty').waitFor();
    await nativeDrop(page, pane(0), ['C:/资料/空窗格.txt'], 'enter');
    await pane(0).getByText('先选择会话，再添加文件', { exact: true }).waitFor();
    await nativeDrop(page, pane(0), ['C:/资料/空窗格.txt']);
    await page.locator('.dash-workspace-file-error').waitFor();
    assert.equal(await page.getByText('空窗格.txt', { exact: true }).count(), 0);
    assert.deepEqual(errors, []);
    console.log('Browser: native drop at 150% DPI, per-session isolation, picker/removal, pointer swap, single/multi switch, async removal, oversize feedback, file-only send, new conversation and 1024/1280 layouts passed');
    console.log('Screenshots: ' + output);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
