#!/usr/bin/env node
/**
 * 浏览器端到端验收（v2 多渠道）：DSH 窗口按 provider 分组的模型目录 + 渠道（provider）切换。
 *
 * 运行方式（本机 Chrome headless + CDP，不新增 npm 依赖，Node >= 22 自带 WebSocket）：
 *   node scripts/verify-agent-model-catalog-browser.cjs
 *
 * 可选环境变量：
 *   CATALOG_BASE_URL    默认 http://127.0.0.1:1420
 *   CATALOG_CDP_PORT    默认 9223（避免占用日常调试的 9222）
 *   CHROME_PATH         默认 C:/Program Files/Google/Chrome/Application/chrome.exe
 *
 * 行为：
 *   1. 若 1420 已有 dev server 则复用（不负责 kill）；否则自己 spawn vite --port 1420 --strictPort，结束时 kill；
 *   2. 用本机 Chrome 无头模式 + CDP 打开应用（main.ts 在 DEV 下自动装 Tauri mock，无需真实后端）；
 *   3. dsh（v2 重点）：
 *      - 弹层出现 opendesgin / DeepSeek / Together / Qwen 四个渠道分组与组标题数量；
 *      - 模型总数 14 > 12，出现搜索框；输入 opus 只留下 claude-opus-5；
 *      - 点选 claude-opus-5（opendesgin）后触发器显示「Claude Opus 5 · opendesgin」、
 *        底部信息条显示「模型：Claude Opus 5（opendesgin）· 思考：高」；
 *      - 同一 id deepseek-v4.1-flash 在 together / opendesgin / qwen 三个渠道可区分、选择正确，
 *        且渠道切换后思考强度 chips 按 (provider, model) 更新；
 *      - 只改 effort 时 set_chat_session_binding 只传 reasoningEffort（探针断言）；
 *      - localStorage dash.agent-window.dsh 持久化 provider，reload 后新建会话恢复；
 *   4. codex / claude 回归：单组无渠道后缀、无分组标题、模型数 <= 12 无搜索框、选择/思考强度/自定义模型 ID 不变；
 *   5. 小窗口 340x400 弹层不溢出（记录，不作硬性失败）；
 *   6. 截图写入 docs/verification/screenshots-v2/；结束 kill 自己启动的 chrome / vite 进程并删除临时 user-data-dir；
 *   7. 汇总断言 pageerror = 0 且无致命 console error。
 *
 * 说明：mock 目录夹具来自 src/dev-tauri-mock.ts（dsh 14 个模型: opendesgin 8 / deepseek-official 3 /
 * together 1 / qwen 2；local_provider=opendesgin，local_model=deepseek-v4.1-flash）。
 */
'use strict';

const assert = require('node:assert/strict');
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const BASE_URL = (process.env.CATALOG_BASE_URL || 'http://127.0.0.1:1420').replace(/\/$/, '');
const CDP_PORT = Number(process.env.CATALOG_CDP_PORT || 9223);
const CHROME_PATH = process.env.CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const SHOT_DIR = path.join(ROOT, 'docs', 'verification', 'screenshots-v2');
const TEMP_PROFILE = fs.mkdtempSync(path.join(os.tmpdir(), 'pet-model-catalog-v2-chrome-'));

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** 与 src/dev-tauri-mock.ts 的 v2 夹具一致：dsh 四个渠道分组与模型数。 */
const DSH_GROUPS = [
  { label: 'opendesgin', count: 8 },
  { label: 'DeepSeek', count: 3 },
  { label: 'Together', count: 1 },
  { label: 'Qwen', count: 2 },
];
const DSH_TOTAL_MODELS = DSH_GROUPS.reduce((sum, group) => sum + group.count, 0);

const SLOTS = {
  dsh: {
    navLabel: 'DeepSeek Harness',
    multiChannel: true,
    totalModels: DSH_TOTAL_MODELS,
    groups: DSH_GROUPS,
    localHint: '本机默认：opendesgin / deepseek-v4.1-flash',
    localEfforts: ['关闭', '低', '中', '高', '极高', '最高'],
    opus: {
      provider: 'opendesgin',
      modelId: 'claude-opus-5',
      modelName: 'Claude Opus 5',
      triggerValue: 'Claude Opus 5 · opendesgin',
      effortLabel: '高',
      effortId: 'high',
    },
    sameIdModel: 'deepseek-v4.1-flash',
    sameIdName: 'DeepSeek V4.1 Flash',
    together: { provider: 'Together', triggerValue: 'DeepSeek V4.1 Flash · Together' },
    opendesginFlash: { provider: 'opendesgin', triggerValue: 'DeepSeek V4.1 Flash · opendesgin' },
    finalEffortLabel: '最高',
    finalEffortId: 'max',
  },
  codex: {
    navLabel: 'Codex',
    multiChannel: false,
    totalModels: 6,
    groups: [],
    localHint: '本机默认：GPT-6.1 Terra',
    localEfforts: ['低', '中', '高', '极高', '最高', '极限'],
    pickModelId: 'gpt-6.1-pro',
    pickModelName: 'GPT-6.1 Pro',
    pickEffortLabel: '极高',
    pickEffortId: 'xhigh',
  },
  claude: {
    navLabel: 'Claude Code',
    multiChannel: false,
    totalModels: 3,
    groups: [],
    localHint: '本机默认：Sonnet',
    localEfforts: ['低', '中', '高', '极高', '最高'],
    pickModelId: 'opus',
    pickModelName: 'Opus',
    pickEffortLabel: '高',
    pickEffortId: 'high',
    customModelId: 'claude-opus-5-5',
  },
};
const SLOT_ORDER = ['dsh', 'codex', 'claude'];

// ---------- 进程 / HTTP 工具 ----------

function httpGet(url, timeoutMs = 2500) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout: timeoutMs }, (response) => {
      let body = '';
      response.setEncoding('utf8');
      response.on('data', (chunk) => { body += chunk; });
      response.on('end', () => resolve({ status: response.statusCode || 0, body }));
    });
    request.on('timeout', () => request.destroy(new Error('请求 ' + url + ' 超时')));
    request.on('error', reject);
  });
}

async function waitForHttpReady(url, timeoutMs) {
  const startedAt = Date.now();
  let lastError;
  while (Date.now() - startedAt < timeoutMs) {
    try {
      const response = await httpGet(url);
      if (response.status >= 200 && response.status < 500) return response;
    } catch (error) {
      lastError = error;
    }
    await sleep(200);
  }
  throw new Error('等待 ' + url + ' 就绪超时' + (lastError ? '：' + lastError.message : ''));
}

async function isHttpUp(url) {
  try {
    await httpGet(url, 1200);
    return true;
  } catch {
    return false;
  }
}

function isPortFree(port) {
  return new Promise((resolve) => {
    const server = net.createServer();
    server.once('error', () => resolve(false));
    server.once('listening', () => server.close(() => resolve(true)));
    server.listen(port, '127.0.0.1');
  });
}

function killTree(pid) {
  if (!pid) return false;
  try {
    const result = spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
    return result.status === 0;
  } catch {
    return false;
  }
}

function removeDirWithRetry(dir) {
  for (let attempt = 0; attempt < 10; attempt += 1) {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
      return true;
    } catch {
      spawnSync('cmd', ['/c', 'ping', '-n', '1', '-w', '250', '127.0.0.1'], { stdio: 'ignore', windowsHide: true });
    }
  }
  return false;
}

// ---------- CDP 客户端（Node 内置 WebSocket） ----------

class CdpConnection {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.pageErrors = [];
    this.consoleErrors = [];
    this.phase = '启动';
    ws.addEventListener('message', (event) => this.handleMessage(event.data));
  }

  static async connect(webSocketUrl) {
    const ws = new WebSocket(webSocketUrl);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('CDP WebSocket 连接超时')), 15000);
      ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      ws.addEventListener('error', () => { clearTimeout(timer); reject(new Error('CDP WebSocket 连接失败')); }, { once: true });
    });
    return new CdpConnection(ws);
  }

  handleMessage(raw) {
    let message;
    try {
      message = JSON.parse(raw);
    } catch {
      return;
    }
    if (message.id) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(pending.method + ' 失败：' + JSON.stringify(message.error)));
      else pending.resolve(message.result);
      return;
    }
    const params = message.params || {};
    if (message.method === 'Runtime.exceptionThrown') {
      const details = params.exceptionDetails || {};
      const exception = details.exception || {};
      const text = exception.description || (exception.value !== undefined ? String(exception.value) : '') || details.text || 'unknown exception';
      this.pageErrors.push({ phase: this.phase, text: String(text) });
    } else if (message.method === 'Runtime.consoleAPICalled' && params.type === 'error') {
      const args = Array.isArray(params.args) ? params.args : [];
      const text = args
        .map((arg) => (arg && (arg.value !== undefined ? arg.value : arg.description)) || '')
        .join(' ')
        .trim();
      this.consoleErrors.push({ phase: this.phase, text });
    }
  }

  send(method, params = {}, sessionId) {
    const id = this.nextId += 1;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method });
      const payload = { id, method, params };
      if (sessionId) payload.sessionId = sessionId;
      try {
        this.ws.send(JSON.stringify(payload));
      } catch (error) {
        this.pending.delete(id);
        reject(error);
      }
    });
  }

  close() {
    try { this.ws.close(); } catch { /* ignore */ }
  }
}

// ---------- 主流程 ----------

async function main() {
  fs.mkdirSync(SHOT_DIR, { recursive: true });
  let viteProcess = null;
  let chromeProcess = null;
  let cdp = null;
  let sessionId = '';
  let serverOwned = false;
  const screenshots = [];

  async function evaluate(expression, label = '') {
    const result = await cdp.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    }, sessionId);
    if (result.exceptionDetails) {
      const details = result.exceptionDetails;
      const description = (details.exception && details.exception.description) || details.text || 'unknown';
      throw new Error('页面脚本执行失败' + (label ? '（' + label + '）' : '') + '：' + description);
    }
    return result.result ? result.result.value : undefined;
  }

  async function waitFor(expression, description, timeoutMs = 10000) {
    const startedAt = Date.now();
    let lastError;
    while (Date.now() - startedAt < timeoutMs) {
      try {
        const value = await evaluate(expression, description);
        if (value) return value;
      } catch (error) {
        lastError = error;
      }
      await sleep(100);
    }
    throw new Error('等待超时：' + description + (lastError ? '（最后一次错误：' + lastError.message + '）' : ''));
  }

  async function screenshot(name) {
    const result = await cdp.send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId);
    const filePath = path.join(SHOT_DIR, name);
    fs.writeFileSync(filePath, Buffer.from(result.data, 'base64'));
    screenshots.push(filePath);
    return filePath;
  }

  const APP_READY = 'document.readyState === "complete" && !!document.querySelector(".sidebar-item-main")';

  async function reloadApp(label) {
    await evaluate('location.reload(); true', label || 'reload');
    await waitFor(APP_READY, '页面重新加载', 25000);
  }

  /** localStorage 复位该 slot 的窗口偏好与会话，并写入目标 slot。 */
  async function resetSlot(slot) {
    await evaluate(
      '(() => {' +
        'localStorage.removeItem(' + JSON.stringify('dash.agent-window.' + slot) + ');' +
        'localStorage.removeItem(' + JSON.stringify('dash.agent-session.' + slot) + ');' +
        'localStorage.setItem("dash.chat-slot", ' + JSON.stringify(slot) + ');' +
        'return true;' +
      '})()',
      '重置 localStorage',
    );
    await reloadApp('重置后 reload');
  }

  /** 目录请求计数 + bind 入参记录探针；reload 之后需要重装。 */
  async function installProbe() {
    const installed = await evaluate(
      '(() => {' +
        'const internals = window.__TAURI_INTERNALS__;' +
        'if (!internals || typeof internals.invoke !== "function") return false;' +
        'if (!internals.__catalogProbe) {' +
          'const original = internals.invoke.bind(internals);' +
          'internals.__catalogProbe = { count: 0, binds: [] };' +
          'internals.invoke = (cmd, args, options) => {' +
            'if (cmd === "get_agent_model_catalog") internals.__catalogProbe.count += 1;' +
            'if (cmd === "set_chat_session_binding") {' +
              'const raw = args || {};' +
              'internals.__catalogProbe.binds.push({' +
                'model: raw.model === undefined ? null : raw.model,' +
                'provider: raw.provider === undefined ? null : raw.provider,' +
                'reasoningEffort: raw.reasoningEffort === undefined ? null : raw.reasoningEffort' +
              '});' +
            '}' +
            'return original(cmd, args, options);' +
          '};' +
        '}' +
        'return true;' +
      '})()',
      '安装 invoke 探针',
    );
    assert.equal(installed, true, 'Tauri mock 未安装，浏览器验收需要 npm run dev 的 DEV 模式');
  }

  async function clickSidebarChat() {
    await evaluate('document.querySelector(".sidebar-item-main").click(); true', '点击对话互动');
    await waitFor('!!document.querySelector(".dash-chat-page")', '进入对话互动页', 12000);
  }

  async function clickAgentSubItem(slotKey) {
    const navLabel = SLOTS[slotKey].navLabel;
    const clicked = await evaluate(
      '(() => {' +
        'const label = ' + JSON.stringify(navLabel) + ';' +
        'const item = Array.from(document.querySelectorAll(".sidebar-subitem")).find((element) => element.textContent.includes(label));' +
        'if (!item) return false;' +
        'item.click();' +
        'return true;' +
      '})()',
      '点击 ' + navLabel + ' 子项',
    );
    assert.equal(clicked, true, '侧边栏找不到 ' + navLabel + ' 子项');
    await waitFor('!!document.querySelector(".dash-agent-model-trigger")', 'agent 窗口模型触发器出现', 12000);
  }

  async function openAgentWindow(slotKey) {
    await clickSidebarChat();
    await clickAgentSubItem(slotKey);
  }

  async function createNewSession() {
    await waitFor('(() => { const button = document.querySelector(".dash-session-new-btn"); return !!button && !button.disabled; })()', '新建会话按钮可用', 12000);
    await evaluate('document.querySelector(".dash-session-new-btn").click(); true', '点击新建会话');
    await waitFor('!!document.querySelector(".dash-session-item.active")', '新建会话已激活', 12000);
  }

  async function openPopup() {
    const alreadyOpen = await evaluate('!!document.querySelector(".dash-agent-model-pop")');
    if (!alreadyOpen) {
      await evaluate('document.querySelector(".dash-agent-model-trigger").click(); true', '打开模型弹层');
      await waitFor('!!document.querySelector(".dash-agent-model-pop")', '模型弹层出现', 12000);
    }
  }

  async function closePopupByEscape() {
    await evaluate('window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); true', 'Escape 关闭弹层');
    await waitFor('!document.querySelector(".dash-agent-model-pop")', '弹层已关闭', 8000);
  }

  async function rowCount() {
    return evaluate('document.querySelectorAll(".dash-agent-model-row").length');
  }

  /** 弹层里的模型行：[{ group, id, name, isFollow, selected }]（group 为渠道组标题而不是 provider id）。 */
  async function popupRows() {
    return evaluate(
      '(() => {' +
        'const rows = [];' +
        'const pop = document.querySelector(".dash-agent-model-pop");' +
        'if (!pop) return rows;' +
        'let group = "";' +
        'for (const element of Array.from(pop.querySelectorAll(".dash-agent-model-group-label, .dash-agent-model-row"))) {' +
          'if (element.classList.contains("dash-agent-model-group-label")) {' +
            'group = element.textContent.split("·")[0].trim();' +
            'continue;' +
          '}' +
          'const isFollow = element.classList.contains("dash-agent-model-row-local");' +
          'const idElement = element.querySelector(".dash-agent-model-row-id");' +
          'const nameElement = element.querySelector(".dash-agent-model-row-name");' +
          'rows.push({' +
            'group: isFollow ? "" : group,' +
            'id: idElement ? idElement.textContent.trim() : "",' +
            'name: nameElement ? nameElement.textContent.trim() : "",' +
            'isFollow: isFollow,' +
            'selected: !!element.querySelector(".dash-agent-model-check")' +
          '});' +
        '}' +
        'return rows;' +
      '})()',
      '读取弹层模型行',
    );
  }

  async function groupLabels() {
    return evaluate('Array.from(document.querySelectorAll(".dash-agent-model-group-label")).map((element) => element.textContent.trim())');
  }

  async function hasSearchInput() {
    return evaluate('!!document.querySelector(".dash-agent-model-search")');
  }

  async function setSearch(text) {
    await evaluate(
      '(() => {' +
        'const input = document.querySelector(".dash-agent-model-search");' +
        'if (!input) return false;' +
        'input.value = ' + JSON.stringify(text) + ';' +
        'input.dispatchEvent(new Event("input", { bubbles: true }));' +
        'return true;' +
      '})()',
      '搜索框输入 ' + text,
    );
  }

  /** 按渠道组标题（单组时传 "" 忽略）+ 模型 id 点击模型行。 */
  async function clickModelRow(providerGroup, modelId) {
    const clicked = await evaluate(
      '(() => {' +
        'const pop = document.querySelector(".dash-agent-model-pop");' +
        'if (!pop) return false;' +
        'const wantedGroup = ' + JSON.stringify((providerGroup || '').toLowerCase()) + ';' +
        'const wantedId = ' + JSON.stringify(modelId) + ';' +
        'let group = "";' +
        'for (const element of Array.from(pop.querySelectorAll(".dash-agent-model-group-label, .dash-agent-model-row"))) {' +
          'if (element.classList.contains("dash-agent-model-group-label")) {' +
            'group = element.textContent.split("·")[0].trim().toLowerCase();' +
            'continue;' +
          '}' +
          'if (element.classList.contains("dash-agent-model-row-local")) continue;' +
          'const idElement = element.querySelector(".dash-agent-model-row-id");' +
          'const id = idElement ? idElement.textContent.trim() : "";' +
          'if (id !== wantedId) continue;' +
          'if (wantedGroup !== "" && group !== wantedGroup) continue;' +
          'element.click();' +
          'return true;' +
        '}' +
        'return false;' +
      '})()',
      '点击模型 ' + (providerGroup ? providerGroup + '/' : '') + modelId,
    );
    assert.equal(clicked, true, '弹层里找不到模型行 ' + (providerGroup ? providerGroup + '/' : '') + modelId);
  }

  async function chipLabels() {
    return evaluate('Array.from(document.querySelectorAll(".dash-agent-model-effort")).map((element) => element.textContent.trim())');
  }

  async function clickEffort(label) {
    const clicked = await evaluate(
      '(() => {' +
        'const label = ' + JSON.stringify(label) + ';' +
        'const chip = Array.from(document.querySelectorAll(".dash-agent-model-effort")).find((element) => element.textContent.trim() === label);' +
        'if (!chip) return false;' +
        'chip.click();' +
        'return true;' +
      '})()',
      '点击思考强度 ' + label,
    );
    assert.equal(clicked, true, '弹层里找不到思考强度 chip ' + label);
  }

  async function triggerValue() {
    return evaluate('(() => { const value = document.querySelector(".dash-agent-model-value"); return value ? value.textContent.trim() : ""; })()');
  }

  async function triggerSub() {
    return evaluate('(() => { const sub = document.querySelector(".dash-agent-model-sub"); return sub ? sub.textContent.trim() : ""; })()');
  }

  async function bottomPill() {
    return evaluate('(() => { const pill = document.querySelector(".dash-agent-meta-pill"); return pill ? pill.textContent.trim() : ""; })()');
  }

  function waitTriggerValue(expected, description) {
    return waitFor(
      '(() => { const value = document.querySelector(".dash-agent-model-value"); return !!value && value.textContent.trim() === ' + JSON.stringify(expected) + '; })()',
      description || ('触发器显示 ' + expected),
      12000,
    );
  }

  async function storedPreference(slotKey) {
    const raw = await evaluate('localStorage.getItem(' + JSON.stringify('dash.agent-window.' + slotKey) + ') || "{}"');
    return JSON.parse(raw);
  }

  async function probeCount() {
    return evaluate('window.__TAURI_INTERNALS__.__catalogProbe.count');
  }

  async function bindRecords() {
    return evaluate('window.__TAURI_INTERNALS__.__catalogProbe.binds');
  }

  /** 从 mock 会话里查：(slot 的会话列表) -> 匹配字段。 */
  async function mockSessions(slotKey) {
    return evaluate(
      '(async () => {' +
        'const sessions = await window.__TAURI_INTERNALS__.invoke("list_chat_sessions", { slot: ' + JSON.stringify(slotKey) + ' });' +
        'return (sessions || []).map((session) => ({' +
          'id: session.id, model: session.model, provider: session.provider,' +
          'reasoning_effort: session.reasoning_effort, title: session.title' +
        '}));' +
      '})()',
      '读取 mock 会话',
    );
  }

  async function waitForMockSession(slotKey, predicateSource, description) {
    return waitFor(
      '(async () => {' +
        'const sessions = await window.__TAURI_INTERNALS__.invoke("list_chat_sessions", { slot: ' + JSON.stringify(slotKey) + ' });' +
        'return (sessions || []).some((session) => (' + predicateSource + '));' +
      '})()',
      description,
      12000,
    );
  }

  const cleanup = async () => {
    try { cdp && cdp.close(); } catch { /* ignore */ }
    if (chromeProcess && chromeProcess.pid) {
      killTree(chromeProcess.pid);
      await sleep(300);
    }
    if (viteProcess && serverOwned) {
      killTree(viteProcess.pid);
      await sleep(300);
    }
    const removed = removeDirWithRetry(TEMP_PROFILE);
    const port1420Free = await isPortFree(1420);
    console.log('[cleanup] chrome=' + (chromeProcess ? 'killed' : 'none') + ' vite=' + (serverOwned ? 'killed' : 'reused') + ' profileRemoved=' + removed + ' port1420Free=' + port1420Free);
    if (!removed) console.warn('[cleanup] 临时 user-data-dir 未能删除：' + TEMP_PROFILE);
  };

  try {
    // 1) dev server：已有则复用，否则自己起
    if (await isHttpUp(BASE_URL + '/')) {
      console.log('[setup] 复用已运行的 dev server：' + BASE_URL);
    } else {
      console.log('[setup] 启动 vite dev server（端口 1420 strictPort）...');
      viteProcess = spawn(process.execPath, [path.join(ROOT, 'node_modules', 'vite', 'bin', 'vite.js'), '--port', '1420', '--strictPort'], {
        cwd: ROOT,
        stdio: ['ignore', 'pipe', 'pipe'],
        windowsHide: true,
      });
      serverOwned = true;
      let viteLog = '';
      viteProcess.stdout.setEncoding('utf8');
      viteProcess.stderr.setEncoding('utf8');
      viteProcess.stdout.on('data', (chunk) => { viteLog += chunk; });
      viteProcess.stderr.on('data', (chunk) => { viteLog += chunk; });
      try {
        await waitForHttpReady(BASE_URL + '/', 30000);
      } catch (error) {
        throw new Error(error.message + '；vite 输出：' + viteLog.slice(-2000));
      }
      console.log('[setup] dev server 就绪：' + BASE_URL);
    }

    // 2) Chrome headless
    if (!fs.existsSync(CHROME_PATH)) throw new Error('找不到本机 Chrome：' + CHROME_PATH);
    console.log('[setup] 启动 Chrome headless（CDP 端口 ' + CDP_PORT + '）...');
    chromeProcess = spawn(CHROME_PATH, [
      '--headless=new',
      '--remote-debugging-port=' + CDP_PORT,
      '--remote-debugging-address=127.0.0.1',
      '--user-data-dir=' + TEMP_PROFILE,
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-gpu',
      '--disable-extensions',
      '--disable-background-networking',
      '--disable-default-apps',
      '--disable-sync',
      '--mute-audio',
      '--hide-scrollbars',
      '--window-size=1280,900',
      'about:blank',
    ], { stdio: 'ignore', windowsHide: true });

    const version = await waitForHttpReady('http://127.0.0.1:' + CDP_PORT + '/json/version', 25000);
    const versionPayload = JSON.parse(version.body);
    console.log('[setup] Chrome：' + versionPayload['Browser']);

    // 3) 新建 page target 并 attach（flatten session），再显式导航到应用
    cdp = await CdpConnection.connect(versionPayload.webSocketDebuggerUrl);
    const target = await cdp.send('Target.createTarget', { url: 'about:blank' });
    const attached = await cdp.send('Target.attachToTarget', { targetId: target.targetId, flatten: true });
    sessionId = attached.sessionId;
    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);
    await cdp.send('Log.enable', {}, sessionId).catch(() => {});
    await cdp.send('Page.navigate', { url: BASE_URL + '/' }, sessionId).catch(() => {});
    try {
      await waitFor(APP_READY, '应用首屏加载', 60000);
    } catch (error) {
      try {
        const diagnostics = await evaluate('(() => ({ href: location.href, readyState: document.readyState, title: document.title, appHtmlLength: (document.querySelector("#app") || { innerHTML: "" }).innerHTML.length, bodyText: (document.body ? document.body.innerText : "").slice(0, 600) }))()');
        console.error('[debug] 首屏加载诊断：' + JSON.stringify(diagnostics));
      } catch (diagnosticError) {
        console.error('[debug] 首屏加载诊断失败：' + diagnosticError.message);
      }
      throw error;
    }
    console.log('[setup] 应用已加载（mock 模式）');

    const results = {};

    // ===== 4) dsh：v2 多渠道重点验收 =====
    cdp.phase = 'dsh / 多渠道弹层';
    console.log('\n=== [dsh] v2 多渠道模型目录验收 ===');
    const expected = SLOTS.dsh;
    await resetSlot('dsh');
    await installProbe();
    await openAgentWindow('dsh');
    await createNewSession();

    const callsBefore = await probeCount();
    await openPopup();
    const expectedRows = 1 + expected.totalModels;
    await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + expectedRows, 'dsh 多渠道目录渲染（' + expectedRows + ' 行）', 20000);

    const rows = await popupRows();
    assert.equal(rows[0].isFollow, true, '首行必须是「跟随本机」，实际：' + JSON.stringify(rows.slice(0, 3)));
    assert.equal(rows.length, expectedRows, 'dsh 模型行数应为 ' + expectedRows + '（含跟随本机），实际 ' + rows.length);
    const labels = await groupLabels();
    const expectedGroupLabels = expected.groups.map((group) => group.label + ' · ' + group.count);
    assert.deepEqual(labels, expectedGroupLabels, 'dsh 渠道分组标题与数量不符，实际：' + JSON.stringify(labels));
    assert.equal(await hasSearchInput(), true, '模型总数 ' + expected.totalModels + ' > 12，必须出现搜索框');
    const localHint = await evaluate('(() => { const element = document.querySelector(".dash-agent-model-row-local .dash-agent-model-row-desc"); return element ? element.textContent.trim() : ""; })()');
    assert.ok(localHint.includes(expected.localHint), '「跟随本机」说明应含「' + expected.localHint + '」，实际：' + localHint);

    const dshIds = rows.filter((row) => !row.isFollow).map((row) => row.id);
    const dshNames = rows.filter((row) => !row.isFollow).map((row) => row.name);
    assert.ok(dshIds.includes('claude-opus-5'), 'dsh 目录应含 opendesgin 渠道的 claude-opus-5');
    assert.ok(dshNames.includes('Claude Opus 5'), 'dsh 目录应含 Claude Opus 5 展示名');
    const flashRows = rows.filter((row) => row.id === expected.sameIdModel);
    assert.equal(flashRows.length, 3, '同一 id deepseek-v4.1-flash 应出现在 3 个渠道，实际：' + JSON.stringify(flashRows.map((row) => row.group)));
    const flashGroups = flashRows.map((row) => row.group.toLowerCase()).sort();
    assert.deepEqual(flashGroups, ['opendesgin', 'qwen', 'together'], '(provider,id) 唯一键应区分同 id 的不同渠道，实际：' + JSON.stringify(flashGroups));

    // 初始高亮的本机模型（opendesgin / deepseek-v4.1-flash）应给出 6 档思考强度 + 跟随本机默认。
    const localChips = await chipLabels();
    assert.equal(localChips[0], '跟随本机默认', '第一个思考强度 chip 必须是「跟随本机默认」，实际：' + JSON.stringify(localChips));
    for (const effort of expected.localEfforts) {
      assert.ok(localChips.includes(effort), 'dsh 缺少思考强度「' + effort + '」，实际：' + JSON.stringify(localChips));
    }
    console.log('  - 渠道分组：' + labels.join(' / '));
    console.log('  - 模型总数：' + (rows.length - 1) + '，同 id deepseek-v4.1-flash 渠道：' + flashGroups.join(', '));
    console.log('  - 本机默认降级提示：' + localHint);
    const multiShot = await screenshot('dsh-picker-multichannel.png');
    console.log('  - 截图：' + path.relative(ROOT, multiShot));

    // 4.1 搜索过滤：opus -> 只剩 opendesgin 的 claude-opus-5
    await setSearch('opus');
    await waitFor('document.querySelectorAll(".dash-agent-model-row").length === 2', '搜索 opus 过滤后只剩 1 个模型', 8000);
    const filteredGroups = await groupLabels();
    assert.deepEqual(filteredGroups, ['opendesgin · 1'], '搜索后应只保留 opendesgin 分组，实际：' + JSON.stringify(filteredGroups));
    const filteredIds = (await popupRows()).filter((row) => !row.isFollow).map((row) => row.id);
    assert.deepEqual(filteredIds, ['claude-opus-5'], '搜索 opus 应命中 claude-opus-5，实际：' + JSON.stringify(filteredIds));
    const searchShot = await screenshot('dsh-picker-search-opus.png');
    console.log('  - 搜索 opus 过滤结果：' + filteredIds.join(', ') + '（截图 ' + path.relative(ROOT, searchShot) + '）');
    await setSearch('');
    await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + expectedRows, '清空搜索后目录恢复', 8000);
    // 列表可滚动：截图记录底部 DeepSeek / Together / Qwen 分组标题（组标题数量与名称由上面的断言保证）。
    await evaluate('(() => { const list = document.querySelector(".dash-agent-model-list"); if (!list) return false; list.scrollTop = list.scrollHeight; return true; })()', '弹层列表滚动到底部');
    await sleep(220);
    const groupsShot = await screenshot('dsh-picker-groups-bottom.png');
    console.log('  - 渠道分组（列表底部）：' + (await groupLabels()).join(' / ') + '（截图 ' + path.relative(ROOT, groupsShot) + '）');
    await evaluate('(() => { const list = document.querySelector(".dash-agent-model-list"); if (list) list.scrollTop = 0; return true; })()', '弹层列表回到顶部');

    // 4.2 选中 claude-opus-5（opendesgin）：触发器 / 思考强度 / 底部信息条 / 持久化
    await clickModelRow(expected.opus.provider, expected.opus.modelId);
    await waitTriggerValue(expected.opus.triggerValue);
    await clickEffort(expected.opus.effortLabel);
    await waitFor(
      '(() => { const sub = document.querySelector(".dash-agent-model-sub"); return !!sub && sub.textContent.includes("思考：" + ' + JSON.stringify(expected.opus.effortLabel) + '); })()',
      '触发器副标题显示思考强度 ' + expected.opus.effortLabel,
      8000,
    );
    const opusPill = await bottomPill();
    assert.ok(
      opusPill.includes('模型：' + expected.opus.modelName + '（' + expected.opus.provider + '）'),
      '底部信息条应显示「模型：Claude Opus 5（opendesgin）」，实际：' + opusPill,
    );
    assert.ok(opusPill.includes('思考：' + expected.opus.effortLabel), '底部信息条应显示思考强度，实际：' + opusPill);
    const prefAfterOpus = await storedPreference('dsh');
    assert.equal(prefAfterOpus.model, expected.opus.modelId, '窗口偏好应持久化模型 id');
    assert.equal(prefAfterOpus.provider, expected.opus.provider, '窗口偏好应持久化 provider');
    assert.equal(prefAfterOpus.effort, expected.opus.effortId, '窗口偏好应持久化思考强度 id');
    await waitForMockSession(
      'dsh',
      'session.model === ' + JSON.stringify(expected.opus.modelId) + ' && session.provider === ' + JSON.stringify(expected.opus.provider) + ' && session.reasoning_effort === ' + JSON.stringify(expected.opus.effortId),
      '会话绑定落库 claude-opus-5/opendesgin/high',
    );
    const opusShot = await screenshot('dsh-picker-selected-opendesgin.png');
    console.log('  - 触发器：' + (await triggerValue()) + ' / ' + (await triggerSub()));
    console.log('  - 底部信息条：' + opusPill);
    console.log('  - 窗口偏好：' + JSON.stringify(prefAfterOpus));
    console.log('  - 截图：' + path.relative(ROOT, opusShot));
    const rowsAfterOpus = await popupRows();
    const selectedRows = rowsAfterOpus.filter((row) => row.selected);
    assert.equal(selectedRows.length, 1, '选中 opus 后应只有一个勾选行，实际：' + JSON.stringify(selectedRows));
    assert.deepEqual(
      { group: selectedRows[0].group, id: selectedRows[0].id },
      { group: 'opendesgin', id: expected.opus.modelId },
      '勾选行必须是 opendesgin / claude-opus-5',
    );

    // 4.3 同一 id deepseek-v4.1-flash：together 与 opendesgin 可区分、选择正确，chips 按 (provider,model) 更新
    await setSearch(expected.sameIdModel);
    await waitFor('document.querySelectorAll(".dash-agent-model-row").length === 4', '搜索同 id 模型出现 3 个渠道条目', 8000);
    const sameIdRows = await popupRows();
    assert.equal(sameIdRows.filter((row) => !row.isFollow).length, 3, '同 id 应出现 3 个渠道条目');
    await clickModelRow(expected.together.provider, expected.sameIdModel);
    await waitTriggerValue(expected.together.triggerValue);
    const togetherChips = await chipLabels();
    assert.equal(togetherChips.length, 6, 'Together 渠道模型应有 5 档思考强度 + 跟随本机默认，实际：' + JSON.stringify(togetherChips));
    assert.ok(!togetherChips.includes('关闭'), 'Together 渠道模型不支持 off，chips 不应出现「关闭」：' + JSON.stringify(togetherChips));
    const togetherPill = await bottomPill();
    assert.ok(togetherPill.includes('（Together）'), '底部信息条应显示 Together 渠道，实际：' + togetherPill);
    const prefTogether = await storedPreference('dsh');
    assert.equal(prefTogether.provider, 'together', '窗口偏好持久化 provider=together');
    const togetherShot = await screenshot('dsh-picker-same-id-together.png');
    console.log('  - Together 渠道选中：' + (await triggerValue()) + '，chips=' + JSON.stringify(togetherChips));
    console.log('  - 截图：' + path.relative(ROOT, togetherShot));

    await clickModelRow(expected.opendesginFlash.provider, expected.sameIdModel);
    await waitTriggerValue(expected.opendesginFlash.triggerValue);
    const opendesginChips = await chipLabels();
    assert.equal(opendesginChips.length, 7, 'opendesgin 渠道模型应有 6 档思考强度 + 跟随本机默认，实际：' + JSON.stringify(opendesginChips));
    assert.ok(opendesginChips.includes('关闭'), 'opendesgin 渠道模型支持 off，chips 应出现「关闭」：' + JSON.stringify(opendesginChips));
    assert.equal((await storedPreference('dsh')).provider, 'opendesgin', '切回 opendesgin 后窗口偏好 provider=opendesgin');
    console.log('  - opendesgin 渠道选中：' + (await triggerValue()) + '，chips=' + JSON.stringify(opendesginChips));

    // 4.4 只改 effort：bind 只传 reasoningEffort（不传 model/provider），会话其它字段不变
    const bindsBeforeEffort = (await bindRecords()).length;
    await clickEffort(expected.finalEffortLabel);
    await waitFor(
      '(() => { const sub = document.querySelector(".dash-agent-model-sub"); return !!sub && sub.textContent.includes("思考：" + ' + JSON.stringify(expected.finalEffortLabel) + '); })()',
      '触发器副标题更新为 ' + expected.finalEffortLabel,
      8000,
    );
    await waitForMockSession(
      'dsh',
      'session.model === ' + JSON.stringify(expected.sameIdModel) + ' && session.provider === "opendesgin" && session.reasoning_effort === ' + JSON.stringify(expected.finalEffortId),
      '仅改 effort 后会话 reasoning_effort 更新且 model/provider 不变',
    );
    const bindsAfterEffort = await bindRecords();
    assert.ok(bindsAfterEffort.length > bindsBeforeEffort, '只改 effort 也应触发一次 set_chat_session_binding');
    const lastBind = bindsAfterEffort[bindsAfterEffort.length - 1];
    assert.equal(lastBind.reasoningEffort, expected.finalEffortId, '只改 effort 时 bind 必须带 reasoningEffort，实际：' + JSON.stringify(lastBind));
    assert.equal(lastBind.model, null, '只改 effort 时 bind 不应带 model，实际：' + JSON.stringify(lastBind));
    assert.equal(lastBind.provider, null, '只改 effort 时 bind 不应带 provider，实际：' + JSON.stringify(lastBind));
    console.log('  - 仅改 effort 的 bind 入参：' + JSON.stringify(lastBind));
    const effortShot = await screenshot('dsh-picker-effort-only.png');
    console.log('  - 截图：' + path.relative(ROOT, effortShot));
    await closePopupByEscape();

    // 4.5 reload 后窗口偏好恢复（provider 一起恢复），新建会话继承
    await reloadApp('reload 验证偏好持久化');
    await installProbe();
    await openAgentWindow('dsh');
    await createNewSession();
    await waitTriggerValue(expected.opendesginFlash.triggerValue, 'reload 后新建会话从窗口偏好恢复模型+渠道');
    const reloadSub = await triggerSub();
    assert.ok(reloadSub.includes('思考：' + expected.finalEffortLabel), 'reload 后新建会话应沿用窗口偏好的思考强度，实际：' + reloadSub);
    const prefAfterReload = await storedPreference('dsh');
    assert.equal(prefAfterReload.model, expected.sameIdModel, 'reload 后 localStorage 模型仍在');
    assert.equal(prefAfterReload.provider, 'opendesgin', 'reload 后 localStorage provider 仍在');
    assert.equal(prefAfterReload.effort, expected.finalEffortId, 'reload 后 localStorage 思考强度仍在');
    await openPopup();
    await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + expectedRows, 'reload 后目录渲染', 15000);
    const reloadRows = await popupRows();
    const reloadSelected = reloadRows.filter((row) => row.selected);
    assert.equal(reloadSelected.length, 1, 'reload 后应只有一个勾选行，实际：' + JSON.stringify(reloadSelected));
    assert.deepEqual(
      { group: reloadSelected[0].group, id: reloadSelected[0].id },
      { group: 'opendesgin', id: expected.sameIdModel },
      'reload 后勾选行必须是 opendesgin / deepseek-v4.1-flash',
    );
    const reloadPill = await bottomPill();
    assert.ok(reloadPill.includes('（opendesgin）'), 'reload 后底部信息条应显示 opendesgin 渠道，实际：' + reloadPill);
    const reloadShot = await screenshot('dsh-picker-reload-persisted.png');
    console.log('  - reload 后触发器：' + (await triggerValue()) + ' / ' + reloadSub + '，底部：' + reloadPill);
    console.log('  - 截图：' + path.relative(ROOT, reloadShot));
    await closePopupByEscape();
    const dshCatalogCalls = await probeCount();
    console.log('  - 目录请求次数（reload 后）：' + dshCatalogCalls);

    results.dsh = {
      groups: labels,
      totalModels: rows.length - 1,
      sameIdGroups: flashGroups,
      opusTrigger: expected.opus.triggerValue,
      opusBottom: opusPill,
      togetherChips,
      opendesginChips,
      effortOnlyBind: lastBind,
      reloadTrigger: await triggerValue(),
      reloadBottom: reloadPill,
      preference: prefAfterReload,
    };

    // ===== 5) codex / claude 回归 =====
    for (const slotKey of ['codex', 'claude']) {
      const slotExpected = SLOTS[slotKey];
      cdp.phase = slotKey + ' / 回归';
      console.log('\n=== [' + slotKey + '] 单组回归验收 ===');
      await resetSlot(slotKey);
      await installProbe();
      await openAgentWindow(slotKey);
      await createNewSession();
      const slotCallsBefore = await probeCount();
      await openPopup();
      const slotRows = 1 + slotExpected.totalModels;
      await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + slotRows, slotKey + ' 目录渲染', 20000);
      const simpleRows = await popupRows();
      const simpleLabels = await groupLabels();
      assert.equal(simpleRows[0].isFollow, true, slotKey + ' 首行必须是「跟随本机」');
      assert.equal(simpleRows.length, slotRows, slotKey + ' 模型行数应为 ' + slotRows + '，实际 ' + simpleRows.length);
      assert.deepEqual(simpleLabels, [], slotKey + ' 不应出现渠道分组标题（单组），实际：' + JSON.stringify(simpleLabels));
      assert.equal(await hasSearchInput(), false, slotKey + ' 模型数 ' + slotExpected.totalModels + ' <= 12，不应出现搜索框');
      const simpleChips = await chipLabels();
      assert.equal(simpleChips[0], '跟随本机默认', slotKey + ' 第一个思考强度 chip 必须是「跟随本机默认」');
      for (const effort of slotExpected.localEfforts) {
        assert.ok(simpleChips.includes(effort), slotKey + ' 缺少思考强度「' + effort + '」，实际：' + JSON.stringify(simpleChips));
      }
      const simpleLocalHint = await evaluate('(() => { const element = document.querySelector(".dash-agent-model-row-local .dash-agent-model-row-desc"); return element ? element.textContent.trim() : ""; })()');
      assert.ok(simpleLocalHint.includes(slotExpected.localHint), slotKey + '「跟随本机」说明应含「' + slotExpected.localHint + '」，实际：' + simpleLocalHint);

      await clickModelRow('', slotExpected.pickModelId);
      await waitTriggerValue(slotExpected.pickModelName, slotKey + ' 触发器应只显示模型名（无渠道后缀）');
      await clickEffort(slotExpected.pickEffortLabel);
      await waitFor(
        '(() => { const sub = document.querySelector(".dash-agent-model-sub"); return !!sub && sub.textContent.includes("思考：" + ' + JSON.stringify(slotExpected.pickEffortLabel) + '); })()',
        slotKey + ' 副标题显示思考强度',
        8000,
      );
      const simplePill = await bottomPill();
      assert.ok(simplePill.includes('模型：' + slotExpected.pickModelName), slotKey + ' 底部信息条应显示模型名，实际：' + simplePill);
      assert.ok(simplePill.includes('思考：' + slotExpected.pickEffortLabel), slotKey + ' 底部信息条应显示思考强度，实际：' + simplePill);
      assert.ok(!simplePill.includes('（'), slotKey + ' 单组窗口不应显示渠道后缀，实际：' + simplePill);
      const simplePref = await storedPreference(slotKey);
      assert.equal(simplePref.model, slotExpected.pickModelId, slotKey + ' 窗口偏好模型 id');
      assert.equal(simplePref.provider || '', '', slotKey + ' 窗口偏好不应写入 provider（单组无渠道）');
      assert.equal(simplePref.effort, slotExpected.pickEffortId, slotKey + ' 窗口偏好思考强度 id');
      await waitForMockSession(
        slotKey,
        'session.model === ' + JSON.stringify(slotExpected.pickModelId) + ' && session.reasoning_effort === ' + JSON.stringify(slotExpected.pickEffortId) + ' && !session.provider',
        slotKey + ' 会话绑定落库',
      );

      // 目录探测次数（Dashboard 挂载预取命中缓存）+ 刷新按钮 force 重新探测（放在自定义模型 ID 提交前，弹层仍打开）
      const slotCallsAfter = await probeCount();
      assert.ok(
        slotCallsAfter - slotCallsBefore <= 1,
        slotKey + ' 打开弹层最多探测一次目录（Dashboard 挂载预取会命中前端缓存，实际 +' + (slotCallsAfter - slotCallsBefore) + '）',
      );
      const callsBeforeRefresh = await probeCount();
      await evaluate('document.querySelector(".dash-agent-model-refresh").click(); true', '点击刷新按钮');
      await waitFor('window.__TAURI_INTERNALS__.__catalogProbe.count === ' + (callsBeforeRefresh + 1), slotKey + ' 刷新按钮 force 重新探测目录', 12000);
      await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + slotRows, slotKey + ' 刷新后目录仍在', 12000);
      const regressionShot = await screenshot(slotKey + '-picker-regression.png');
      console.log('  - 模型：' + simpleRows.filter((row) => !row.isFollow).map((row) => row.name).join(' / '));
      console.log('  - 思考强度：' + simpleChips.join(' / '));
      console.log('  - 触发器：' + (await triggerValue()) + ' / ' + (await triggerSub()));
      console.log('  - 底部信息条：' + simplePill);
      console.log('  - 截图：' + path.relative(ROOT, regressionShot));

      if (slotExpected.customModelId) {
        await evaluate(
          '(() => {' +
            'const input = document.querySelector(".dash-agent-model-custom-input");' +
            'if (!input) return false;' +
            'input.value = ' + JSON.stringify(slotExpected.customModelId) + ';' +
            'input.dispatchEvent(new Event("input", { bubbles: true }));' +
            'return true;' +
          '})()',
          '填写自定义模型 ID',
        );
        await evaluate('document.querySelector(".dash-agent-model-custom-input").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); true', '提交自定义模型 ID');
        await waitTriggerValue(slotExpected.customModelId, slotKey + ' 自定义模型 id 原样显示');
        const customPref = await storedPreference(slotKey);
        assert.equal(customPref.model, slotExpected.customModelId, slotKey + ' 自定义模型 id 应写入窗口偏好');
        const customShot = await screenshot(slotKey + '-custom-model.png');
        console.log('  - 自定义模型 ID：' + slotExpected.customModelId + '（截图 ' + path.relative(ROOT, customShot) + '）');
      }

      // 提交自定义模型 ID 后弹层会自动关闭；codex 没有自定义输入，这里手动关闭。
      await closePopupByEscape();

      results[slotKey] = {
        groups: simpleLabels,
        models: simpleRows.filter((row) => !row.isFollow).map((row) => row.name),
        efforts: simpleChips,
        trigger: await triggerValue(),
        bottom: simplePill,
        preference: simplePref,
      };
    }

    // ===== 6) 小窗口 340x400 弹层不溢出（记录，不作硬性失败） =====
    cdp.phase = '小窗口布局';
    let smallWindow = { checked: false };
    try {
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 340, height: 400, deviceScaleFactor: 1, mobile: false }, sessionId);
      await evaluate('localStorage.setItem("dash.chat-slot", "dsh"); true', '小窗口切回 dsh');
      await reloadApp('小窗口 reload');
      await openAgentWindow('dsh');
      await openPopup();
      await waitFor('document.querySelectorAll(".dash-agent-model-row").length >= ' + (1 + DSH_TOTAL_MODELS), '小窗口目录渲染', 15000);
      await sleep(300);
      const rect = await evaluate(
        '(() => {' +
          'const pop = document.querySelector(".dash-agent-model-pop");' +
          'if (!pop) return null;' +
          'const box = pop.getBoundingClientRect();' +
          'return { left: box.left, top: box.top, right: box.right, bottom: box.bottom, width: box.width, height: box.height, innerWidth: window.innerWidth, innerHeight: window.innerHeight };' +
        '})()',
      );
      if (rect) {
        smallWindow = {
          checked: true,
          viewport: rect.innerWidth + 'x' + rect.innerHeight,
          box: Math.round(rect.width) + 'x' + Math.round(rect.height) + ' @ (' + Math.round(rect.left) + ',' + Math.round(rect.top) + ')',
          overflows: rect.left < -1 || rect.top < -1 || rect.right > rect.innerWidth + 1 || rect.bottom > rect.innerHeight + 1,
        };
        const shot = await screenshot('dsh-small-window-340x400.png');
        console.log('  - 340x400 弹层：' + JSON.stringify(smallWindow) + '（截图 ' + path.relative(ROOT, shot) + '）');
      }
      await cdp.send('Emulation.clearDeviceMetricsOverride', {}, sessionId);
    } catch (error) {
      smallWindow = { checked: false, error: error.message };
      console.warn('  ! 小窗口布局检查跳过：' + error.message);
      try { await cdp.send('Emulation.clearDeviceMetricsOverride', {}, sessionId); } catch { /* ignore */ }
    }

    // ===== 7) 页面错误汇总 =====
    const pageErrors = cdp.pageErrors;
    const consoleErrors = cdp.consoleErrors;
    assert.deepEqual(pageErrors, [], '页面出现未捕获异常：' + JSON.stringify(pageErrors));
    const fatalConsole = consoleErrors.filter((entry) => /Uncaught|Vue Global Error|is not a function|Cannot read/i.test(entry.text));
    assert.deepEqual(fatalConsole, [], '页面出现致命 console error：' + JSON.stringify(fatalConsole));
    if (consoleErrors.length) console.warn('[warn] 捕获到 ' + consoleErrors.length + ' 条普通 console.error（非致命）：' + JSON.stringify(consoleErrors.slice(0, 5)));

    console.log('\n=== 通过：v2 多渠道模型目录（provider 分组/切换/持久化）浏览器验收 ===');
    console.log(JSON.stringify({ slots: results, smallWindow: smallWindow, screenshots: screenshots.map((file) => path.relative(ROOT, file)), pageErrors: pageErrors.length, consoleErrors: consoleErrors.length }, null, 2));
  } finally {
    await cleanup();
  }
}

main().catch((error) => {
  console.error('\n[FAIL] ' + (error && error.stack ? error.stack : error));
  process.exitCode = 1;
});
