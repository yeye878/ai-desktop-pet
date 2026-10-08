const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');

const root = path.resolve(__dirname, '..');
function load(file) {
  const exports = {};
  const source = ts.transpileModule(fs.readFileSync(path.join(root, file), 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2021 },
  }).outputText;
  vm.runInNewContext(source, { exports, require: (id) => id.startsWith('.')
    ? load(path.posix.join(path.posix.dirname(file), id + '.ts')) : require(id), Date, console });
  return exports;
}
const plain = (value) => JSON.parse(JSON.stringify(value));
const activity = load('src/services/agentActivity.ts');
const cache = load('src/services/agentActivityCache.ts');
const { reactive, effect } = require('vue');
const tool = (status, extra = {}) => ({ id: 'read-1', tool_name: 'read_file', status, summary: '读取 src/main.ts', arguments: '{"path":"src/main.ts"}', ...extra });

test('commentary, reasoning and tools retain arrival order; final answer is not duplicated', () => {
  const run = activity.createAgentActivity();
  run.appendText('先检查配置。');
  run.appendThinking('确认入口');
  run.upsertTool(tool('requested'));
  run.appendThinking('\n[调用工具] 读取 src/main.ts\n{"path":"src/main.ts"}\n');
  run.upsertTool(tool('running'));
  run.upsertTool(tool('completed', { output: '文件内容' }));
  run.appendThinking('[执行结果]\n文件内容\n');
  run.appendText('配置已确认。');
  const result = run.finish('先检查配置。配置已确认。');
  assert.equal(result.displayContent, '配置已确认。');
  assert.deepEqual(plain(result.activity).map((item) => item.kind), ['text', 'thinking', 'tool']);
  assert.equal(result.activity[2].tool.output, '文件内容');
});

test('terminal tools never regress on delayed updates, null fields do not erase output', () => {
  const run = activity.createAgentActivity();
  run.upsertTool(tool('completed', { output: 'done', approved: true }));
  run.upsertTool(tool('running', { output: null, approved: null }));
  assert.equal(run.items.length, 1);
  assert.equal(run.items[0].tool.status, 'completed');
  assert.equal(run.items[0].tool.output, 'done');
  assert.equal(run.items[0].tool.approved, true);
});

test('delayed authorization updates do not regress a running tool', () => {
  const run = activity.createAgentActivity();
  run.upsertTool(tool('running'));
  run.upsertTool(tool('waiting'));
  assert.equal(run.items[0].tool.status, 'running');
});

test('failure preserves partial reply and marks only unfinished tools interrupted', () => {
  const run = activity.createAgentActivity();
  run.appendText('已检查一半');
  run.upsertTool(tool('running'));
  run.upsertTool({ ...tool('denied'), id: 'write-2' });
  const result = run.finish('已中止', 'aborted');
  assert.equal(result.activity[0].text, '已检查一半');
  assert.equal(result.activity[1].tool.status, 'interrupted');
  assert.equal(result.activity[2].tool.status, 'denied');
  assert.equal(result.displayContent, '已中止');
});

test('plain/legacy replies and final-only protocols keep their exact answer', () => {
  const run = activity.createAgentActivity();
  run.appendText('中文\n[QUOTE]引用[/QUOTE]');
  const result = run.finish('中文\n[QUOTE]引用[/QUOTE]');
  assert.equal(result.displayContent, '中文\n[QUOTE]引用[/QUOTE]');
  assert.equal(result.activity.length, 0);
  run.reset();
  run.appendThinking('先分析');
  assert.equal(run.finish('直接回答').displayContent, '直接回答');
});

test('legacy history restores structured calls without exposing tool JSON as reasoning', () => {
  const thinking = '检查入口\n\n[调用工具] 读取文件 src/main.ts\n{\n "path": "src/main.ts"\n}\n[执行结果]\nhello\n';
  const items = activity.legacyActivity(thinking);
  assert.equal(items[0].kind, 'thinking');
  assert.equal(items[0].text.trim(), '检查入口');
  assert.equal(items[1].kind, 'tool');
  assert.equal(items[1].tool.path, 'src/main.ts');
  assert.equal(items[1].tool.output.trim(), 'hello');
});

test('unknown tool/status, invalid JSON, denied and skipped results remain inspectable', () => {
  const run = activity.createAgentActivity();
  run.upsertTool({ id: 'custom', tool_name: 'custom_plugin', status: 'custom_state', arguments: 'invalid{', summary: '' });
  assert.equal(run.items[0].tool.arguments, 'invalid{');
  assert.equal(activity.toolStatusLabel('custom_state'), 'custom_state');
  assert.equal(activity.toolStatusLabel('failed'), '失败');
  assert.equal(activity.formatArguments('{"path":"a"}'), '{\n  "path": "a"\n}');
  assert.equal(activity.formatArguments('invalid{'), 'invalid{');
});

test('many tools and chunked Chinese text do not collapse independent calls or lose text', () => {
  const run = activity.createAgentActivity();
  for (let i = 0; i < 200; i++) {
    run.appendText(`步骤${i}。`);
    run.upsertTool({ ...tool('completed'), id: `read-${i}` });
  }
  for (let i = 0; i < 1000; i++) run.appendText('好');
  const result = run.finish(Array.from({ length: 200 }, (_, i) => `步骤${i}。`).join('') + '好'.repeat(1000));
  assert.equal(result.displayContent, '好'.repeat(1000));
  assert.equal(result.activity.filter((item) => item.kind === 'tool').length, 200);
});

test('scroll following stops when inspecting older messages', () => {
  assert.equal(activity.isNearBottom({ scrollHeight: 1000, scrollTop: 500, clientHeight: 300 }), false);
  assert.equal(activity.isNearBottom({ scrollHeight: 1000, scrollTop: 690, clientHeight: 300 }), true);
});

test('known log markers are filtered only when delivered as log records', () => {
  const run = activity.createAgentActivity();
  run.appendThinking('正常推理中讨论 [调用工具] 的格式');
  run.appendThinking('[等待授权] 执行命令\n');
  assert.equal(run.items.length, 1);
  assert.equal(run.items[0].text, '正常推理中讨论 [调用工具] 的格式');
});

test('incremental updates invalidate Vue renders even when loading remains true', () => {
  const run = activity.createAgentActivity(reactive([]));
  let rendered = '';
  effect(() => { rendered = run.items.map((item) => item.kind === 'tool' ? item.tool.status : item.text).join('|'); });
  run.appendText('先');
  run.appendText('检查');
  assert.equal(rendered, '先检查');
  run.upsertTool(tool('running'));
  assert.equal(rendered, '先检查|running');
  run.upsertTool(tool('completed'));
  assert.equal(rendered, '先检查|completed');
  run.reset();
  assert.equal(rendered, '');
});

test('a final-only settlement replaces the streamed final segment without losing commentary', () => {
  const run = activity.createAgentActivity();
  run.appendText('先检查');
  run.upsertTool(tool('completed'));
  run.appendText('已完成');
  const result = run.finish('已完成');
  assert.equal(result.activity.length, 2);
  assert.equal(result.displayContent, '已完成');
});

test('presentation cache restores matching history only; storage failure never blocks chat', () => {
  const map = new Map();
  const storage = { getItem: (key) => map.get(key), setItem: (key, value) => map.set(key, value), removeItem: (key) => map.delete(key) };
  const message = { content: '全部正文', thinking: '分析', timestamp: 123_000, agent: { id: 'a', name: '助手' }, displayContent: '最终正文', activity: [{ kind: 'tool', tool: tool('completed') }] };
  cache.saveActivityPresentation(storage, message);
  assert.equal(cache.restoreActivityPresentation(storage, { ...message, timestamp: 124_000 }).displayContent, '最终正文');
  assert.equal(cache.restoreActivityPresentation(storage, { ...message, timestamp: 130_000 }), undefined);
  assert.equal(cache.restoreActivityPresentation(storage, { ...message, agent: { id: 'b', name: '助手' } }), undefined);
  assert.doesNotThrow(() => cache.saveActivityPresentation({ getItem() { throw Error('disabled'); } }, message));
  cache.clearActivityPresentations(storage);
  assert.equal(map.size, 0);
});

test('corrupt presentation cache safely falls back without changing conversation text', () => {
  const message = { content: '原文', timestamp: 123000 };
  for (const raw of ['invalid{', '{"version":2}', '[{"identity":"x","timestamp":123000,"displayContent":"x","activity":[{"kind":"tool"}]}]']) {
    assert.equal(cache.restoreActivityPresentation({ getItem: () => raw }, message), undefined);
  }
});
