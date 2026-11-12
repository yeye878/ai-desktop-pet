const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');
const { createPinia, setActivePinia } = require('pinia');
const root = path.resolve(__dirname, '..');
const plain = value => JSON.parse(JSON.stringify(value));

function load(file, mocks = {}, cache = new Map()) {
  if (cache.has(file)) return cache.get(file);
  const exports = {};
  cache.set(file, exports);
  const source = ts.transpileModule(fs.readFileSync(path.join(root, file), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(source, { exports, console, setTimeout, clearTimeout, require: name => {
    if (mocks[name]) return mocks[name];
    if (name.startsWith('.')) return load(path.relative(root, path.resolve(root, path.dirname(file), name + '.ts')), mocks, cache);
    return require(name);
  } });
  return exports;
}

function fixture(options = {}) {
  const handlers = new Map();
  const calls = [];
  const mocks = {
    '@tauri-apps/api/core': { invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      if (cmd === 'get_chat_session_messages') return options.history ? options.history(args) : [];
      if (cmd === 'get_active_chat') return options.active ? options.active(args) : null;
      if (cmd === 'send_to_ai' && options.send) return options.send(args);
    } },
    '@tauri-apps/api/event': { listen: async (event, handler) => {
      handlers.set(event, handler);
      return () => handlers.delete(event);
    } },
    './sessions': { useSessionsStore: () => ({ activeMainId: options.activeMainId || '', find: id => ({ id, slot: id.startsWith('main') ? 'main' : 'claude', agent_id: 'builtin-claude', model: 'sonnet' }), scheduleReload() {} }) },
  };
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = load('src/stores/sessionRuntime.ts', mocks).useSessionRuntimeStore(pinia);
  const emit = (event, payload) => { assert.ok(handlers.has(event), event); handlers.get(event)({ payload }); };
  return { store, emit, calls };
}

const thinking = (emit, id, text = '定位代码入口') => emit('ai-thinking', { session_id: id, text });
const text = (emit, id, text = '我继续检查生成智能体链路。') => emit('ai-answer-delta', { session_id: id, text });
const tool = (emit, id) => emit('ai-tool-event', { session_id: id, id: 'tool-1', tool_name: 'read_file', status: 'completed', summary: '读取生成入口', path: 'D:/ai-desktop-pet/src/main.ts', output: 'entry' });
const user = (emit, id, content = '继续') => emit('sync-chat-message', { session_id: id, role: 'user', content });

test('中断回复和思考工具记录保留在会话运行时，历史重载不会覆盖', async () => {
  const { store, emit } = fixture({ history: async () => [{ id: 1, role: 'user', content: '继续', created_at: 100 }] });
  await store.start();
  user(emit, 'claude'); thinking(emit, 'claude'); text(emit, 'claude'); tool(emit, 'claude');
  emit('ai-error', { session_id: 'claude', message: '已中断', aborted: true });
  const runtime = store.peek('claude');
  assert.equal(runtime.messages.length, 2);
  assert.equal(runtime.messages[1].content, '已中止');
  const activity = plain(runtime.messages[1].activity);
  await store.load('claude');
  await store.load('claude', true);
  assert.equal(store.peek('claude'), runtime);
  assert.equal(runtime.messages.length, 2);
  assert.deepEqual(plain(runtime.messages[1].activity), activity);
  assert.equal(runtime.messages[1].thinking, '定位代码入口');
  assert.equal(runtime.turn.loading, false);
});

test('历史获取期间收到最终回复，迟到的历史不会抹掉新回复', async () => {
  let complete;
  const { store, emit } = fixture({ history: () => new Promise(resolve => { complete = resolve; }) });
  await store.start();
  const loading = store.load('claude');
  user(emit, 'claude'); thinking(emit, 'claude'); tool(emit, 'claude');
  emit('ai-finished', { session_id: 'claude', text: '已修复生成入口', thinking: '定位代码入口' });
  complete([{ id: 4, role: 'user', content: '继续', created_at: 100 }]);
  await loading;
  const runtime = store.peek('claude');
  assert.equal(runtime.messages.length, 2);
  assert.equal(runtime.messages[1].content, '已修复生成入口');
  assert.equal(runtime.messages[1].toolEvents[0].output, 'entry');
  assert.equal(runtime.turn.loading, false);
});

test('历史已包含回复时只保留一条，同时保留本地工具轨迹与毫秒时间', async () => {
  const now = Date.now();
  const { store, emit } = fixture({ history: async () => [
    { id: 1, role: 'user', content: '继续', created_at: Math.floor(now / 1000) },
    { id: 2, role: 'assistant', content: '完成', thinking: '定位代码入口', created_at: Math.floor(now / 1000) },
  ] });
  await store.start();
  user(emit, 'claude'); thinking(emit, 'claude'); tool(emit, 'claude');
  emit('ai-finished', { session_id: 'claude', text: '完成', thinking: '定位代码入口' });
  await store.load('claude');
  const messages = store.peek('claude').messages;
  assert.equal(messages.length, 2);
  assert.equal(messages[1].id, 2);
  assert.equal(messages[1].activity.length, 2);
  assert.ok(messages[1].timestamp >= now);
});

test('运行中重载不会清空思考、流式回复和工具记录，同会话并发加载共用请求', async () => {
  const { store, emit, calls } = fixture({ active: async () => ({ session_id: 'claude', thinking: '旧快照' }) });
  await store.start();
  user(emit, 'claude'); thinking(emit, 'claude'); text(emit, 'claude'); tool(emit, 'claude');
  const turn = store.peek('claude').turn;
  const activity = plain(turn.activity.items);
  await Promise.all([store.load('claude'), store.load('claude')]);
  assert.equal(calls.filter(c => c.cmd === 'get_chat_session_messages').length, 1);
  assert.equal(turn.thinking, '定位代码入口');
  assert.equal(turn.streamingAnswer, '我继续检查生成智能体链路。');
  assert.deepEqual(plain(turn.activity.items), activity);
});

test('迟到的 active 快照不能把已完成的回合恢复成运行中', async () => {
  let complete;
  const { store, emit } = fixture({ active: () => new Promise(resolve => { complete = resolve; }) });
  await store.start();
  const loading = store.load('claude');
  await new Promise(resolve => setImmediate(resolve));
  user(emit, 'claude'); thinking(emit, 'claude');
  emit('ai-finished', { session_id: 'claude', text: '完成' });
  complete({ session_id: 'claude', thinking: '旧的思考快照' });
  await loading;
  assert.equal(store.peek('claude').turn.loading, false);
  assert.equal(store.peek('claude').turn.thinking, '');
});

test('删除会话后旧历史响应不得复活会话', async () => {
  let complete;
  const { store } = fixture({ history: () => new Promise(resolve => { complete = resolve; }) });
  const loading = store.load('claude');
  store.remove('claude');
  complete([{ id: 1, role: 'assistant', content: '旧回复', created_at: 100 }]);
  await loading;
  assert.equal(store.peek('claude'), undefined);
});

test('历史加载失败仍可重试，不留下已加载假状态', async () => {
  let failed = true;
  const { store } = fixture({ history: async () => {
    if (failed) { failed = false; throw new Error('测试历史请求失败'); }
    return [{ id: 1, role: 'assistant', content: '重试恢复', created_at: 100 }];
  } });
  await store.load('claude');
  assert.equal(store.peek('claude').loaded, false);
  await store.load('claude');
  assert.equal(store.peek('claude').messages[0].content, '重试恢复');
});

test('相同正文的连续回合不会被全局去重，中断记录保持原顺序', () => {
  const { mergeSessionHistory } = load('src/services/sessionHistory.ts');
  const message = (id, role, content) => ({ id, role, content, timestamp: id });
  const history = [message(1, 'user', '继续'), message(2, 'user', '继续'), message(3, 'assistant', '完成')];
  const local = [message(100, 'user', '继续'), message(101, 'assistant', '已中止'), message(102, 'user', '继续'), message(103, 'assistant', '完成')];
  assert.deepEqual(plain(mergeSessionHistory(history, local).map(m => m.content)), ['继续', '已中止', '继续', '完成']);
});

test('正文与附件提示的后端广播不会重复用户消息，也不会抹掉已到达轨迹', async () => {
  const { store, emit } = fixture();
  await store.start();
  const files = [{ id: 'a', name: 'a.pdf', path: 'C:/a.pdf', size: 100, isImage: false, isDir: false, extension: 'pdf', previewUrl: '', status: 'ready' }];
  await store.send('claude', '检查附件', files, { role: 'assistant', content: '前文' });
  thinking(emit, 'claude');
  user(emit, 'claude', '检查附件\n\n---\n[用户附加了以下本地文件]\na.pdf');
  const runtime = store.peek('claude');
  assert.equal(runtime.messages.length, 1);
  assert.equal(runtime.messages[0].files[0].name, 'a.pdf');
  assert.equal(runtime.messages[0].quote.content, '前文');
  assert.equal(runtime.turn.thinking, '定位代码入口');
});

test('主会话与 agent 会话共享同样的视图状态机制，实时事件不会串会话', async () => {
  const { store, emit } = fixture();
  await store.start();
  user(emit, 'main-a'); thinking(emit, 'main-a', '主思考'); text(emit, 'main-a', '主输出');
  user(emit, 'claude'); thinking(emit, 'claude', 'agent 思考');
  emit('ai-finished', { session_id: 'claude', text: 'agent 完成' });
  assert.equal(store.peek('main-a').turn.loading, true);
  assert.equal(store.peek('main-a').turn.streamingAnswer, '主输出');
  assert.equal(store.peek('claude').turn.loading, false);
  assert.equal(store.peek('claude').messages.at(-1).content, 'agent 完成');
  tool(emit, 'tool-only');
  assert.equal(store.peek('tool-only').turn.loading, true);
});

test('旧版无 session_id 的事件回退到活跃主对话，同屏和单窗格仍可显示', async () => {
  const { store, emit } = fixture({ activeMainId: 'main-a' });
  await store.start();
  emit('sync-chat-message', { role: 'user', content: '兼容旧事件' });
  emit('ai-thinking', '旧版思考');
  emit('ai-answer-delta', { text: '旧版增量回复' });
  assert.equal(store.peek('main-a').turn.thinking, '旧版思考');
  emit('ai-finished', { text: '旧版最终回复' });
  assert.equal(store.peek('main-a').messages.at(-1).content, '旧版最终回复');
  assert.equal(store.peek('main-a').messages.at(-1).session_id, 'main-a');
  assert.equal(store.peek('main-a').turn.loading, false);
});
