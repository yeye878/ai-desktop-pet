// Run against a Vite dev server with local Chrome and Node >= 22; no extra npm dependencies.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');
const baseUrl = process.env.RESIZE_BASE_URL || 'http://127.0.0.1:1420';
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 0;
    this.pending = new Map();
    this.errors = [];
    ws.addEventListener('message', ({ data }) => {
      const message = JSON.parse(data);
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        clearTimeout(pending.timer);
        if (message.error) pending.reject(new Error(JSON.stringify(message.error)));
        else pending.resolve(message.result);
      } else if (message.method === 'Runtime.exceptionThrown') {
        this.errors.push(message.params.exceptionDetails);
      } else if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
        const text = message.params.args.map(arg => arg.value || arg.description || '').join(' ');
        if (text.includes('Vue Global Error')) this.errors.push(text);
      }
    });
  }
  send(method, params = {}, sessionId) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error('CDP timed out: ' + method));
      }, 15000);
      this.pending.set(id, { resolve, reject, timer });
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }
  close() {
    for (const pending of this.pending.values()) clearTimeout(pending.timer);
    this.ws.close();
  }
}

(async () => {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'pet-resize-qa-'));
  const chrome = spawn(process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe', [
    '--headless=new', '--remote-debugging-port=0', '--user-data-dir=' + profile,
    '--no-first-run', '--no-default-browser-check', 'about:blank',
  ], { windowsHide: true, stdio: 'ignore' });
  let cdp;
  let sessionId;
  let launchError;
  chrome.on('error', error => { launchError = error; });
  async function evaluate(expression) {
    const result = await cdp.send('Runtime.evaluate', {
      expression, awaitPromise: true, returnByValue: true, userGesture: true,
    }, sessionId);
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result?.value;
  }
  async function waitFor(expression) {
    for (let i = 0; i < 120; i++) {
      if (await evaluate(expression)) return;
      await pause(100);
    }
    throw new Error('Timed out: ' + expression);
  }
  async function resize(width, height) {
    await cdp.send('Emulation.setDeviceMetricsOverride', {
      width, height, deviceScaleFactor: 1.5, mobile: false,
    }, sessionId);
    await evaluate('new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))');
  }
  async function emitDrag(type, paths = ['C:/qa/resize.txt']) {
    await evaluate(`(async () => {
      const rect = document.querySelector('.session-pane').getBoundingClientRect();
      const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
      await emit('tauri://drag-${type}', {
        paths: ${JSON.stringify(paths)},
        position: { x: (rect.x + rect.width / 2) * devicePixelRatio, y: (rect.y + rect.height / 2) * devicePixelRatio },
      });
    })()`);
  }
  async function mouse(type, point, buttons = 0) {
    await cdp.send('Input.dispatchMouseEvent', {
      type, ...point, button: type === 'mouseMoved' && !buttons ? 'none' : 'left',
      buttons, clickCount: type === 'mouseMoved' ? 0 : 1,
    }, sessionId);
  }
  async function center(selector) {
    return evaluate(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  }
  async function click(selector) {
    const point = await center(selector);
    await mouse('mousePressed', point, 1);
    await mouse('mouseReleased', point);
  }
  async function beginPaneDrag() {
    const from = await center('[data-pane-id="pane-0"] .session-pane-title');
    const to = await center('[data-pane-id="pane-1"] .session-pane-title');
    await mouse('mouseMoved', from);
    await mouse('mousePressed', from, 1);
    await mouse('mouseMoved', to, 1);
    await waitFor('document.querySelectorAll(".session-pane.dragging").length === 1');
    return to;
  }
  const noDrag = '!document.querySelector(".session-pane.dragging, .session-pane.drag-over, .session-pane.file-over, .dash-drop-overlay") && document.body.style.cursor !== "grabbing"';
  try {
    const activePort = path.join(profile, 'DevToolsActivePort');
    for (let i = 0; i < 120 && !fs.existsSync(activePort); i++) {
      if (launchError) throw launchError;
      await pause(100);
    }
    const [port, endpoint] = fs.readFileSync(activePort, 'utf8').split(/\r?\n/);
    const ws = new WebSocket('ws://127.0.0.1:' + port + endpoint);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Chrome connection timed out')), 10000);
      ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      ws.addEventListener('error', () => { clearTimeout(timer); reject(new Error('Chrome connection failed')); }, { once: true });
    });
    cdp = new Cdp(ws);
    const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
    sessionId = (await cdp.send('Target.attachToTarget', { targetId, flatten: true })).sessionId;
    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);
    await cdp.send('Page.bringToFront', {}, sessionId);
    await resize(1280, 900);
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: `
      localStorage.setItem('dash.workspace-layout', 'four');
      localStorage.setItem('dash.workspace-panes', JSON.stringify(['mock-session-1', 'mock-session-2', 'mock-session-3', 'mock-session-4']));
    ` }, sessionId);
    await cdp.send('Page.navigate', { url: baseUrl }, sessionId);
    await waitFor('!!document.querySelector(".sidebar-item-main")');
    await click('.sidebar-item-main');
    await waitFor('document.querySelectorAll(".session-pane .md-body").length === 4');
    await evaluate(`(() => {
      const original = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = (cmd, args, options) => cmd === 'get_file_metadata'
        ? Promise.resolve(args.paths.map(path => ({
            path, name: path.split('/').pop(), size: 123, extension: 'txt',
            exists: true, is_file: true, is_dir: false,
          })))
        : original(cmd, args, options);
    })()`);

    await emitDrag('over');
    assert.equal(await evaluate('!!document.querySelector(".session-pane.file-over")'), false, 'stray over cannot show a file overlay');
    await emitDrag('enter', []);
    assert.equal(await evaluate('!!document.querySelector(".session-pane.file-over")'), false, 'text drag cannot arm the file overlay');
    for (const event of ['resize', 'blur', 'pointerup', 'pointercancel', 'dragend', 'escape', 'leave', 'drop']) {
      await emitDrag('enter');
      await waitFor('!!document.querySelector(".session-pane.file-over")');
      if (event === 'leave' || event === 'drop') await emitDrag(event, []);
      else await evaluate(event === 'escape'
        ? 'window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))'
        : `window.dispatchEvent(new Event(${JSON.stringify(event)}))`);
      await waitFor(noDrag);
      await emitDrag('over');
      assert.equal(await evaluate('!!document.querySelector(".session-pane.file-over")'), false, event + ' disarms late native over events');
    }
    await emitDrag('enter');
    await emitDrag('drop');
    await waitFor('!!document.querySelector(".attachment-card")');
    console.log('PASS: real file drop at 150% DPI; interrupted drags and stray native events leave no overlay.');

    const initialPanes = await evaluate('localStorage.getItem("dash.workspace-panes")');
    for (const event of ['resize', 'blur', 'pointercancel', 'escape', 'released-outside']) {
      const to = await beginPaneDrag();
      if (event === 'resize') await resize(1279, 900);
      else if (event === 'released-outside') await mouse('mouseMoved', to);
      else await evaluate(event === 'escape'
        ? 'window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }))'
        : event === 'pointercancel'
          ? 'window.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 1 }))'
          : 'window.dispatchEvent(new Event("blur"))');
      await mouse('mouseReleased', to);
      await waitFor(noDrag);
      assert.equal(await evaluate('localStorage.getItem("dash.workspace-panes")'), initialPanes, event + ' cancels without swapping sessions');
      await resize(1280, 900);
    }
    const target = await beginPaneDrag();
    await mouse('mouseReleased', target);
    await waitFor('JSON.parse(localStorage.getItem("dash.workspace-panes"))[1] === "mock-session-1"');
    await waitFor(noDrag);
    console.log('PASS: resize, focus loss, cancellation, Escape and lost mouse release clean up pane drags; normal swaps work.');

    for (const [width, height] of [[1024, 680], [1600, 1000], [1280, 800], [1920, 1080]]) {
      await resize(width, height);
      const bounds = await evaluate('(() => { const r = document.querySelector(".dashboard-root").getBoundingClientRect(); return { width: r.width, height: r.height, viewportWidth: innerWidth, viewportHeight: innerHeight, overflow: document.documentElement.scrollWidth > innerWidth }; })()');
      assert.equal(bounds.width, bounds.viewportWidth);
      assert.equal(bounds.height, bounds.viewportHeight);
      assert.equal(bounds.overflow, false);
      await click('.session-picker-trigger');
      await waitFor('!!document.querySelector(".session-picker-panel")');
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 }, sessionId);
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 }, sessionId);
      await waitFor('!document.querySelector(".session-picker-panel")');
    }
    for (let i = 0; i < 40; i++) await resize(1100 + i % 5 * 60, 720 + i % 3 * 40);
    await resize(1280, 900);
    await click('.sidebar-nav > .sidebar-item');
    await waitFor('!!document.querySelector(".home-page")');
    await click('.sidebar-item-main');
    await waitFor('document.querySelectorAll(".session-pane").length === 4');
    await waitFor(noDrag);
    assert.deepEqual(cdp.errors, []);
    const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId);
    const imagePath = path.join(os.tmpdir(), 'pet-window-resize-qa.png');
    fs.writeFileSync(imagePath, Buffer.from(screenshot.data, 'base64'));
    console.log('PASS: 1024–1920 layouts, repeated viewport resize and real clicks on session pickers and page navigation.');
    console.log('Screenshot: ' + imagePath);
    console.log('Native WebView2 resize / GDI leak needs desktop validation; browser viewport tests cover UI recovery only.');
  } finally {
    cdp?.close();
    if (chrome.pid) spawnSync('taskkill', ['/PID', String(chrome.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
