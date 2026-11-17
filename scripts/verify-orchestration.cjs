const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');
const { createPinia, setActivePinia } = require('pinia');
const root = path.resolve(__dirname, '..');

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

const terminal = ['succeeded', 'failed', 'review_failed', 'cancelled'];

function snapshot(overrides = {}) {
  return {
    task_id: 'task-1', session_id: 'session-main', original_request: '修复记忆库 bug',
    task_revision: 1, run_id: 'run-1', state: 'executing', state_revision: 3,
    policy: { difficulty: 'L2', risk: 'R1', force_review: true, max_rework: 2, max_agent_invocations: 20, rule_hits: [], policy_version: 1 },
    usage_stats: { agent_invocations: 2, tool_calls: 1 },
    final_artifact_set_id: null, status_message: '',
    nodes: [
      { node_id: 'assess', kind: 'assess', state: 'completed' },
      { node_id: 'work-1', kind: 'work', state: 'running', rework_round: 0 },
      { node_id: 'review-final', kind: 'review', state: 'pending' },
    ],
    artifacts: [], reviews: [], latest_seq: 5,
    ...overrides,
  };
}

function fixture() {
  const handlers = new Map();
  const calls = [];
  let mode = 'direct';
  let current = snapshot();
  const mocks = {
    '@tauri-apps/api/core': { invoke: async (cmd, args) => {
      calls.push({ cmd, args });
      if (cmd === 'get_orchestration_mode') return mode;
      if (cmd === 'set_orchestration_mode') { mode = args.mode; return mode; }
      if (cmd === 'get_orchestrated_run') return current;
      if (cmd === 'list_orchestrated_runs') return [{ run_id: current.run_id, task_id: current.task_id, state: current.state, state_revision: current.state_revision, status_message: '', created_at: 1, updated_at: 2, request: current.original_request }];
      if (cmd === 'cancel_orchestrated_run') { current = { ...current, state: 'cancelling' }; return true; }
      if (cmd === 'get_orchestration_events') return [];
      return null;
    } },
    '@tauri-apps/api/event': { listen: async (event, handler) => {
      handlers.set(event, handler);
      return () => handlers.delete(event);
    } },
  };
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = load('src/stores/orchestration.ts', mocks).useOrchestrationStore(pinia);
  const emit = (event, payload) => { assert.ok(handlers.has(event), event); handlers.get(event)({ payload }); };
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
  return { store, emit, calls, wait, setSnapshot: next => { current = next; }, getMode: () => mode };
}

test('模式默认 direct，可切换 adaptive 并持久化', async () => {
  const { store, calls } = fixture();
  await store.start();
  assert.equal(store.mode, 'direct');
  await store.setMode('adaptive');
  assert.equal(store.mode, 'adaptive');
  assert.ok(calls.some(c => c.cmd === 'set_orchestration_mode' && c.args.mode === 'adaptive'));
});

test('run-started 与事件驱动快照刷新，终态后 isRunning=false', async () => {
  const { store, emit, wait, setSnapshot } = fixture();
  await store.start();
  await store.setMode('adaptive');
  emit('orchestration-run-started', { session_id: 'session-main', task_id: 'task-1', run_id: 'run-1' });
  await wait(350);
  assert.equal(store.activeRun('session-main')?.run_id, 'run-1');
  assert.equal(store.isRunning('session-main'), true);
  setSnapshot(snapshot({ state: 'succeeded', state_revision: 9, final_artifact_set_id: 'art-1' }));
  emit('orchestration-run-finished', { run_id: 'run-1' });
  await wait(350);
  assert.equal(store.isRunning('session-main'), false);
  assert.equal(store.activeRun('session-main')?.state, 'succeeded');
});

test('取消携带当前 state_revision（幂等取消协议）', async () => {
  const { store, emit, wait, calls } = fixture();
  await store.start();
  emit('orchestration-run-started', { session_id: 'session-main', task_id: 'task-1', run_id: 'run-1' });
  await wait(350);
  await store.cancel('run-1');
  const cancel = calls.find(c => c.cmd === 'cancel_orchestrated_run');
  assert.equal(cancel.args.runId, 'run-1');
  assert.equal(cancel.args.expectedRevision, 3);
});

test('bindSession 用列表+快照补齐（窗口重载不丢任务）', async () => {
  const { store } = fixture();
  await store.start();
  await store.bindSession('session-main');
  assert.equal(store.activeRun('session-main')?.run_id, 'run-1');
  assert.equal(store.activeRun('session-main')?.state, 'executing');
});

test('终态集合与后端合同一致', () => {
  const service = load('src/services/orchestration.ts', { '@tauri-apps/api/core': { invoke: async () => null } });
  assert.deepEqual(JSON.parse(JSON.stringify(service.TERMINAL_STATES)), terminal);
  assert.equal(service.STATE_LABELS.review_failed, '审核未通过');
});

test('跨窗口模式变更广播与 clearSessions 生效', async () => {
  const { store, emit } = fixture();
  await store.start();
  emit('orchestration-mode-changed', 'adaptive');
  assert.equal(store.mode, 'adaptive');
  await store.bindSession('session-main');
  assert.ok(store.activeRun('session-main'));
  store.clearSessions();
  assert.equal(store.activeRun('session-main'), undefined);
});

test('实时收集节点过程流与派工结论并在完成后汇总挂载且剥离内部封套', async () => {
  const { store, emit } = fixture();
  await store.start();
  emit('orchestration-event', {
    run_id: 'run-live',
    type: 'assessment.completed',
    payload: {
      difficulty: 'L2',
      risk: 'R1',
      recommended_agent_id: 'builtin-codex',
      recommended_agent_name: 'codex',
      recommended_agent_avatar: '⌘',
      recommended_backend: 'codex',
      recommended_reason: '工程实现交由 Codex',
    },
  });
  assert.equal(store.handoff('run-live')?.recommendedAgentName, 'codex');

  emit('orchestration-activity', {
    session_id: 'session-main',
    run_id: 'run-live',
    node_id: 'work-1',
    role_kind: 'work',
    invocation_id: 'inv-work-1',
    attempt: 1,
    agent_id: 'builtin-codex',
    agent_name: 'codex',
    agent_avatar: '⌘',
    backend: 'codex',
    kind: 'started',
  });
  emit('orchestration-activity', {
    session_id: 'session-main',
    run_id: 'run-live',
    node_id: 'work-1',
    role_kind: 'work',
    invocation_id: 'inv-work-1',
    attempt: 1,
    agent_id: 'builtin-codex',
    agent_name: 'codex',
    agent_avatar: '⌘',
    backend: 'codex',
    kind: 'thinking',
    text: '先检查代码位置',
  });
  emit('orchestration-activity', {
    session_id: 'session-main',
    run_id: 'run-live',
    node_id: 'work-1',
    role_kind: 'work',
    invocation_id: 'inv-work-1',
    attempt: 1,
    agent_id: 'builtin-codex',
    agent_name: 'codex',
    agent_avatar: '⌘',
    backend: 'codex',
    kind: 'tool',
    tool: { id: 't1', tool_name: 'read_file', status: 'completed', summary: '读取文件', path: 'src/main.rs' },
  });
  emit('orchestration-activity', {
    session_id: 'session-main',
    run_id: 'run-live',
    node_id: 'work-1',
    role_kind: 'work',
    invocation_id: 'inv-work-1',
    attempt: 1,
    agent_id: 'builtin-codex',
    agent_name: 'codex',
    agent_avatar: '⌘',
    backend: 'codex',
    kind: 'text',
    text: '已完成修复。\n\n```json\n{"summary":"修复完成","artifacts":[]}\n```',
  });
  emit('orchestration-activity', {
    session_id: 'session-main',
    run_id: 'run-live',
    node_id: 'work-1',
    role_kind: 'work',
    invocation_id: 'inv-work-1',
    attempt: 1,
    agent_id: 'builtin-codex',
    agent_name: 'codex',
    agent_avatar: '⌘',
    backend: 'codex',
    kind: 'completed',
  });

  const streams = store.streams('run-live');
  assert.equal(streams.length, 1);
  assert.equal(streams[0].status, 'completed');
  assert.equal(streams[0].items.at(-1).text, '已完成修复。');
  const combined = store.buildRunActivity('run-live', '已完成修复。');
  assert.ok(combined.some(item => item.kind === 'tool' && item.tool.tool_name === 'read_file'));
  assert.ok(!combined.some(item => item.kind === 'text' && item.text.includes('"artifacts"')));
});