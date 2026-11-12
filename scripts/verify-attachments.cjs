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

const metadata = paths => paths.map(p => ({ path: p, name: p.split(/[\\/]/).pop(), size: 1024, extension: p.split('.').pop(), exists: true, is_file: true, is_dir: false }));
function fixture(options = {}) {
  const calls = [];
  const invoke = async (command, args) => {
    calls.push({ command, args: plain(args) });
    if (command === 'get_file_metadata') return options.metadata ? options.metadata(args.paths) : metadata(args.paths);
    if (command === 'read_file_as_data_url') return 'data:image/png;base64,broken';
  };
  const mocks = {
    '@tauri-apps/api/core': { invoke, convertFileSrc: value => 'asset:' + value },
    '@tauri-apps/plugin-dialog': { open: options.open || (async () => null) },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    './sessions': { useSessionsStore: () => ({ find: id => ({ id, slot: id === 'main' ? 'main' : 'codex', agent_id: 'builtin-codex', model: 'test-model' }), scheduleReload() {} }) },
  };
  const pinia = createPinia();
  setActivePinia(pinia);
  const cache = new Map();
  return {
    store: load('src/stores/attachments.ts', mocks, cache).useAttachmentsStore(pinia),
    runtime: load('src/stores/sessionRuntime.ts', mocks, cache).useSessionRuntimeStore(pinia),
    calls,
  };
}

test('附件按会话隔离，布局视图重新读取时仍是同一份草稿', async () => {
  const { store } = fixture();
  await store.add('a', ['C:/资料/报告.pdf', 'C:/资料/截图.PNG']);
  await store.add('b', ['C:/资料/代码.ts']);
  const draft = store.ensure('a');
  assert.equal(store.ensure('a'), draft);
  assert.equal(draft.files[1].isImage, true);
  store.remove('a', draft.files[0].id);
  assert.deepEqual(plain(draft.files.map(f => f.name)), ['截图.PNG']);
  assert.equal(store.ensure('b').files.length, 1);
  store.clear('a');
  assert.equal(store.ensure('a').files.length, 0);
  assert.equal(store.ensure('b').files.length, 1);
});

test('并发拖入会立即占位、去重，始终最多 5 个', async () => {
  const resolvers = [];
  const { store, calls } = fixture({ metadata: paths => new Promise(resolve => resolvers.push(() => resolve(metadata(paths)))) });
  const first = store.add('a', ['C:/a.txt', 'C:/b.txt', 'C:/c.txt']);
  const second = store.add('a', ['c:\\A.txt', 'C:/d.txt', 'C:/e.txt', 'C:/f.txt']);
  assert.equal(store.ensure('a').files.length, 5);
  assert.ok(store.ensure('a').files.every(f => f.status === 'loading'));
  assert.equal(calls.filter(c => c.command === 'get_file_metadata').flatMap(c => c.args.paths).length, 5);
  resolvers.forEach(resolve => resolve());
  await Promise.all([first, second]);
  assert.equal(store.ensure('a').files.length, 5);
  assert.match(store.ensure('a').error, /最多/);
});

test('读取元数据时移除或清空文件，旧请求不会恢复文件或写入另一个会话', async () => {
  let complete;
  const { store } = fixture({ metadata: paths => new Promise(resolve => { complete = () => resolve(metadata(paths)); }) });
  const adding = store.add('a', ['C:/a.txt', 'C:/b.txt']);
  store.clear('a');
  store.ensure('b');
  complete();
  await adding;
  assert.equal(store.ensure('a').files.length, 0);
  assert.equal(store.ensure('b').files.length, 0);
});

test('读取元数据时移除其中一个文件，其他文件仍正常就绪', async () => {
  let complete;
  const { store } = fixture({ metadata: paths => new Promise(resolve => { complete = () => resolve(metadata(paths)); }) });
  const adding = store.add('a', ['C:/a.txt', 'C:/b.txt']);
  store.remove('a', store.ensure('a').files[0].id);
  complete();
  await adding;
  assert.deepEqual(plain(store.ensure('a').files.map(f => [f.name, f.status])), [['b.txt', 'ready']]);
});

test('元数据获取或文件选择失败会保留明确错误，且选择器允许重试', async () => {
  const { store } = fixture({ metadata: async () => { throw new Error('读取失败'); }, open: async () => { throw new Error('选择失败'); } });
  await store.add('a', ['C:/a.txt']);
  assert.equal(store.ensure('a').files.length, 0);
  assert.match(store.ensure('a').error, /获取文件信息失败/);
  await store.pick('a');
  assert.equal(store.ensure('a').picking, false);
  assert.match(store.ensure('a').error, /选择文件失败/);
});

test('切到单窗格再返回同屏布局，不丢失原会话绑定', () => {
  const component = fs.readFileSync(path.join(root, 'src/components/Dashboard.vue'), 'utf8');
  const script = component.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile('Dashboard.ts', script, ts.ScriptTarget.Latest, true);
  const declaration = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'setWorkspaceLayout');
  const panes = { value: ['a', 'b', 'c', 'd'] };
  const layout = { value: 'four' };
  const compiled = ts.transpileModule(declaration.getText(parsed) + '\nexports.set = setWorkspaceLayout;', {
    compilerOptions: { module: ts.ModuleKind.CommonJS },
  }).outputText;
  const exports = {};
  vm.runInNewContext(compiled, {
    exports, workspacePanes: panes, workspaceLayout: layout, currentSessionId: { value: 'c' },
    layoutPaneCount: value => value === 'single' ? 1 : 4, saveWorkspaceLayout() {}, saveWorkspacePanes() {},
  });
  exports.set('single');
  exports.set('four');
  assert.deepEqual(plain(panes.value), ['a', 'b', 'c', 'd']);
});

test('无效路径与超大文件有明确反馈，目录和正常文件仍可使用', async () => {
  const { store } = fixture({ metadata: paths => metadata(paths).map((m, i) => ({ ...m, exists: i !== 0, size: i === 1 ? 51 * 1024 * 1024 : 1024, is_file: i !== 2, is_dir: i === 2 })) });
  await store.add('a', ['C:/missing.txt', 'C:/large.zip', 'C:/folder', 'C:/ok.md']);
  assert.deepEqual(plain(store.ensure('a').files.map(f => f.name)), ['folder', 'ok.md']);
  assert.equal(store.ensure('a').files[0].isDir, true);
  assert.match(store.ensure('a').error, /路径不存在.*文件过大/);
});

test('文件选择器捕获发起会话，取消不清空草稿，重复点击只打开一次', async () => {
  let complete;
  let opens = 0;
  const { store } = fixture({ open: () => { opens++; return new Promise(resolve => { complete = resolve; }); } });
  const picking = store.pick('a');
  await store.pick('a');
  store.ensure('b');
  complete(['C:/picked.pdf']);
  await picking;
  assert.equal(opens, 1);
  assert.equal(store.ensure('a').files.length, 1);
  assert.equal(store.ensure('b').files.length, 0);
  const canceled = store.pick('a');
  complete(null);
  await canceled;
  assert.equal(store.ensure('a').files.length, 1);
});

test('图片预览失败不影响发送，失败的数据 URL 不会无限重试', async () => {
  const { store, calls } = fixture();
  await store.add('a', ['C:/image.png']);
  const file = store.ensure('a').files[0];
  await store.previewFallback('a', file);
  await store.previewFallback('a', file);
  assert.equal(calls.filter(c => c.command === 'read_file_as_data_url').length, 1);
  assert.equal(file.previewUrl, '');
  assert.equal(file.status, 'ready');
});

test('原生拖放按缩放后的实际位置定位窗格，空窗格与间隙不会串投', () => {
  const { attachmentDropTarget } = load('src/services/attachments.ts');
  const panes = [
    { paneId: 'left', sessionId: 'b', rect: { left: 10, right: 200, top: 100, bottom: 500 } },
    { paneId: 'right', sessionId: 'a', rect: { left: 210, right: 400, top: 100, bottom: 500 } },
    { paneId: 'empty', sessionId: '', rect: { left: 410, right: 600, top: 100, bottom: 500 } },
  ];
  for (const scale of [1, 1.25, 1.5, 2]) {
    assert.equal(attachmentDropTarget({ x: 250 * scale, y: 200 * scale }, scale, panes).sessionId, 'a');
    assert.equal(attachmentDropTarget({ x: 205 * scale, y: 200 * scale }, scale, panes), null);
    assert.equal(attachmentDropTarget({ x: 450 * scale, y: 200 * scale }, scale, panes).sessionId, '');
  }
});

test('多窗格允许仅发附件，图片载荷与目录路径发给正确会话并显示文件标签', async () => {
  const { store, runtime, calls } = fixture();
  await store.add('agent', ['C:/image.png', 'C:/report.pdf']);
  await runtime.send('agent', '', [...store.ensure('agent').files]);
  const send = calls.find(c => c.command === 'send_to_ai').args;
  assert.equal(send.sessionId, 'agent');
  assert.equal(send.targetAgentId, 'builtin-codex');
  assert.equal(send.attachments.length, 2);
  assert.equal(send.attachments[0].isImage, true);
  assert.match(send.message, /C:\/image.png/);
  assert.equal(runtime.peek('agent').messages[0].files.length, 2);
  assert.equal(runtime.peek('agent').messages[0].content, '(已发送文件)');
  assert.equal(runtime.peek('main'), undefined);
  await runtime.send('agent', '重复提交', []);
  assert.equal(calls.filter(c => c.command === 'send_to_ai').length, 1);
});

test('元数据未就绪时阻止发送，纯文本发送保持原状', async () => {
  const { store, runtime, calls } = fixture();
  await store.add('agent', ['C:/a.txt']);
  store.ensure('agent').files[0].status = 'loading';
  await runtime.send('agent', '', store.ensure('agent').files);
  assert.equal(calls.filter(c => c.command === 'send_to_ai').length, 0);
  await runtime.send('main', '你好');
  const send = calls.find(c => c.command === 'send_to_ai').args;
  assert.equal(send.message, '你好');
  assert.deepEqual(send.attachments, []);
});
