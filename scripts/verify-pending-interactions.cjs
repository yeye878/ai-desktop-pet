const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const { test } = require('node:test');
const { ref } = require('vue');
const root = path.resolve(__dirname, '..');
function load(mocks = {}) {
  const exports = {};
  const source = ts.transpileModule(fs.readFileSync(path.join(root, 'src/services/pendingInteractions.ts'), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(source, { exports, require: (id) => mocks[id] || require(id), console });
  return exports;
}
const flush = () => new Promise(setImmediate);
function fixture() {
  const listeners = new Map();
  let snapshotResolve;
  const api = load({
    '@tauri-apps/api/core': { invoke: () => new Promise((resolve) => { snapshotResolve = resolve; }) },
    '@tauri-apps/api/event': { listen: async (event, callback) => {
      listeners.set(event, callback); return () => listeners.delete(event);
    } },
  });
  return { api, listeners, snapshot: (value) => snapshotResolve(value), emit: (event, payload) => listeners.get(event)?.({ payload }) };
}

test('new windows recover outstanding questions missed before listener registration', async () => {
  const { api, snapshot } = fixture();
  const pending = ref(null);
  const stop = api.subscribePendingInteraction('question', pending);
  await flush();
  snapshot({ questions: [{ id: 'q1', question: '选择方案', options: [] }], tool_confirms: [] });
  await flush(); assert.equal(pending.value.id, 'q1'); stop();
});

test('late snapshots cannot revive a resolved question or replace a newer one', async () => {
  const { api, snapshot, emit } = fixture();
  const pending = ref({ id: 'old' });
  const stop = api.subscribePendingInteraction('question', pending);
  await flush();
  emit('ai-ask-user-resolved', { id: 'old' });
  emit('ai-ask-user', { id: 'new', question: '新的问题', options: [] });
  snapshot({ questions: [{ id: 'old' }] });
  await flush(); assert.equal(pending.value.id, 'new'); stop();
});

test('initial stale confirmation is removed when the backend has no pending request', async () => {
  const { api, snapshot } = fixture();
  const pending = ref({ id: 'expired' });
  const stop = api.subscribePendingInteraction('tool', pending);
  await flush(); snapshot({ questions: [], tool_confirms: [] }); await flush();
  assert.equal(pending.value, null); stop();
});

test('old submissions and cross-window resolution cannot clear a new confirmation', async () => {
  const { api } = fixture();
  const pending = ref({ id: 'old' });
  let complete;
  const calls = [];
  const submission = api.createInteractionSubmission(pending, (id, value) => { calls.push([id, value]); return new Promise((resolve) => { complete = resolve; }); });
  const first = submission.submit(true);
  await submission.submit(false);
  assert.equal(calls.length, 1);
  pending.value = { id: 'new' }; complete(); await first;
  assert.equal(pending.value.id, 'new');
  assert.equal(submission.submittingId.value, null);
});

test('failed IPC keeps the same request visible and permits a retry', async () => {
  const { api } = fixture(); const pending = ref({ id: 'q1' });
  let attempts = 0;
  const submission = api.createInteractionSubmission(pending, async () => { if (++attempts === 1) throw Error('IPC unavailable'); });
  assert.equal(await submission.submit('中文'), null);
  assert.equal(pending.value.id, 'q1'); assert.match(submission.error.value, /IPC unavailable/);
  assert.equal(await submission.submit('中文'), 'q1'); assert.equal(pending.value, null);
});

test('unmount during subscription and during snapshot leaves no live listeners', async () => {
  const { api, snapshot, listeners } = fixture(); const pending = ref(null);
  const stop = api.subscribePendingInteraction('tool', pending);
  await flush(); stop(); snapshot({ tool_confirms: [{ id: 'late' }] }); await flush();
  assert.equal(listeners.size, 0); assert.equal(pending.value, null);
});
