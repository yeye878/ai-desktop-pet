/**
 * 独立验证：Codex 轨迹（tool activity）与思考（thinking）在真实事件流下的行为。
 *
 * 设计原则：不复制"实现者的说法"，而是把 scripts/fixtures 下真实采集的
 * codex exec --json 事件（Codex CLI 0.162.0，本机实跑，路径已脱敏），
 * 按下面两个源头文件的映射规则重放，再断言前端模型
 * （src/services/agentActivity.ts + agentActivityCache.ts）的顺序 / 终态 / 缓存行为：
 *   - src-tauri/src/cli_activity.rs::CliActivityParser::codex()  （tool 事件）
 *   - src-tauri/src/codex.rs::EventParser::parse_line()          （thinking / text 增量）
 * 下面的 codexActivityEvents / createDeltaParser 是这两段 Rust 的等价移植，
 * 标注为 [PORT]；Rust 侧真正的行为由 cargo test --lib 覆盖，见测试报告。
 *
 * 明确边界（不要夸大）：
 *   - 不启动 GUI、不使用 Playwright、不渲染 DOM、不做 340px 真实布局。
 *   - 涉及渲染的断言只到模型层或 .vue 源码字符串层，测试名里带 [MODEL] / [STATIC]。
 */
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
  vm.runInNewContext(source, {
    exports,
    require: (id) => id.startsWith('.')
      ? load(path.posix.join(path.posix.dirname(file), id + '.ts')) : require(id),
    Date, console,
  });
  return exports;
}

const plain = (value) => JSON.parse(JSON.stringify(value));
const activity = load('src/services/agentActivity.ts');
const cache = load('src/services/agentActivityCache.ts');

// 真实采集的 codex exec --json 事件（Codex CLI 0.162.0），随仓库提供，路径已脱敏。
const FIXTURE_STDOUT2 = path.join(root, 'scripts', 'fixtures', 'codex-exec-json-0.162.0.jsonl');

function readFixture(file) {
  assert.ok(fs.existsSync(file), 'missing real fixture: ' + file);
  // 夹具写入时已去掉 UTF-8 BOM；这里仍做一次防御性清理，因为 PowerShell 的
  // Out-File -Encoding utf8 会给首行加 BOM，而 JSON.parse 与 serde_json 都会拒绝该行。
  return fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, '').split('\n').filter((line) => line.trim());
}

// ---------------------------------------------------------------- [PORT] Rust 映射
/** serde_json::to_string_pretty 的等价输出：2 空格缩进 + 键名排序（默认 BTreeMap）。 */
function sortKeys(value) {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value && typeof value === 'object') {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = sortKeys(value[key]);
    return out;
  }
  return value;
}
function jsonPretty(value) { return JSON.stringify(sortKeys(value), null, 2); }

/** cli_activity.rs::compact() */
function compact(text) {
  const chars = Array.from(text);
  let value = chars.slice(0, 3000).join('');
  if (chars.length > 3000) value += '\n... [已截断]';
  return value;
}

/** cli_activity.rs::text_content() */
function textContent(value) {
  if (typeof value === 'string') return compact(value);
  if (Array.isArray(value)) {
    return compact(value.map((block) => (block && typeof block.text === 'string' ? block.text : null))
      .filter((entry) => entry !== null).join('\n'));
  }
  return '';
}

/** cli_activity.rs::plan_items() */
function planItems(item) {
  for (const key of ['items', 'plan', 'todos']) if (Array.isArray(item[key])) return item[key];
  return [];
}

/** cli_activity.rs::file_changes()：只保留 path 为字符串的条目，kind 允许缺失/null。 */
function fileChanges(item) {
  if (!Array.isArray(item.changes)) return null;
  return item.changes
    .filter((change) => change && typeof change.path === 'string')
    .map((change) => ({ path: change.path, kind: typeof change.kind === 'string' ? change.kind : null }));
}

/**
 * [PORT] cli_activity.rs::CliActivityParser::codex()
 * state: { completed: Set, notices: Set }，对应 Rust 的 completed / notices。
 * 注意：read_stream_with_activity 每个回合新建一次 parser，所以去重是回合级的。
 */
function codexActivityEvents(line, state) {
  let value;
  try { value = JSON.parse(line); } catch { return []; }
  const eventType = typeof value.type === 'string' ? value.type : '';
  if (!['item.started', 'item.updated', 'item.completed'].includes(eventType)) return [];
  const item = value.item && typeof value.item === 'object' ? value.item : {};
  const id = item.id;
  if (typeof id !== 'string') return [];
  if (state.completed.has(id)) return [];
  const kind = typeof item.type === 'string' ? item.type : '';
  if (kind === 'reasoning') return [];
  if (kind === 'error') {
    if (eventType !== 'item.completed') return [];
    const message = typeof item.message === 'string' ? item.message : '';
    if (!message || state.notices.has(message)) return [];
    state.notices.add(message);
    state.completed.add(id);
    const summary = message.split('\n').map((part) => part.trim()).find((part) => part) || 'Codex 提示';
    return [{
      id, tool_name: 'codex_notice', status: 'notice', summary, arguments: '{}',
      command: null, path: null, output: compact(message), approved: null,
      exit_code: null, duration_ms: null, changes: null,
    }];
  }
  let toolName;
  let args;
  let output = null;
  let changes = null;
  if (kind === 'command_execution') {
    toolName = 'run_command';
    args = { command: item.command === undefined ? null : item.command };
    output = typeof item.aggregated_output === 'string' ? compact(item.aggregated_output) : null;
  } else if (kind === 'file_change') {
    toolName = 'edit_file';
    args = { changes: item.changes === undefined ? null : item.changes };
    changes = fileChanges(item);
  } else if (kind === 'web_search') {
    toolName = 'web_search';
    args = { query: item.query === undefined ? null : item.query, action: item.action === undefined ? null : item.action };
  } else if (kind === 'mcp_tool_call') {
    toolName = (typeof item.server === 'string' ? item.server : 'MCP') + ' / ' + (typeof item.tool === 'string' ? item.tool : 'tool');
    args = item.arguments === undefined ? null : item.arguments;
    output = item.error && typeof item.error.message === 'string'
      ? compact(item.error.message)
      : (item.result ? textContent(item.result.content) : null);
  } else if (kind === 'todo_list') {
    toolName = 'update_plan';
    args = { plan: planItems(item) };
  } else {
    return [];
  }
  const exitCode = Number.isInteger(item.exit_code) ? item.exit_code : null;
  const durationMs = Number.isInteger(item.duration_ms) && item.duration_ms >= 0 ? item.duration_ms : null;
  const failed = item.status === 'failed'
    || (exitCode !== null && exitCode !== 0)
    || !!(item.error && typeof item.error === 'object');
  const status = failed ? 'failed' : (eventType === 'item.completed' ? 'completed' : 'running');
  if (eventType === 'item.completed') state.completed.add(id);
  const command = args && typeof args.command === 'string' ? args.command : null;
  const filePath = changes && changes.length ? changes[0].path : null;
  const summary = kind === 'todo_list' ? '更新任务清单' : (command || filePath || toolName);
  return [{
    id, summary, tool_name: toolName, status, arguments: jsonPretty(args),
    command, path: filePath, output, approved: null, exit_code: exitCode,
    duration_ms: durationMs, changes,
  }];
}

// ---------------------------------------------------------------- [PORT] EventParser
/** codex.rs::text_field() */
function textField(value) {
  if (typeof value === 'string') return value.length ? value : null;
  if (!Array.isArray(value)) return null;
  const joined = value.map((entry) => {
    if (typeof entry === 'string') return entry;
    if (entry && typeof entry === 'object' && typeof entry.text === 'string') return entry.text;
    return null;
  }).filter((entry) => entry !== null).join('\n');
  return joined.length ? joined : null;
}

/** codex.rs::reasoning_text() */
function reasoningText(item) {
  for (const key of ['text', 'summary_text', 'raw_content']) {
    const value = textField(item[key]);
    if (value !== null) return value;
  }
  return null;
}

/** codex.rs::increment()：按最长公共前缀返回新增部分。 */
function increment(previous, text) {
  const left = Array.from(previous);
  const right = Array.from(text);
  let index = 0;
  while (index < left.length && index < right.length && left[index] === right[index]) index += 1;
  return right.slice(index).join('');
}

/** [PORT] codex.rs::EventParser 的相关状态机。 */
function createDeltaParser() {
  const completedItems = new Set();
  const reasoningItems = new Map();
  let lastReasoningId = null;
  let turnCompleted = false;
  const state = { full_text: '', full_thinking: '' };
  const emitThinking = (deltas, id, delta) => {
    if (!delta) return;
    const separate = id === '' || lastReasoningId !== id;
    if (separate && state.full_thinking !== '') state.full_thinking += '\n\n';
    state.full_thinking += delta;
    lastReasoningId = id;
    deltas.push({ kind: 'thinking', text: delta });
  };
  return {
    state,
    isTurnCompleted: () => turnCompleted,
    parse(line) {
      const deltas = [];
      let event;
      try { event = JSON.parse(line); } catch { return deltas; }
      const eventType = typeof event.type === 'string' ? event.type : '';
      if (eventType === 'turn.completed') { turnCompleted = true; return deltas; }
      if (!['item.started', 'item.updated', 'item.completed'].includes(eventType)) return deltas;
      const item = event.item && typeof event.item === 'object' ? event.item : {};
      const id = typeof item.id === 'string' ? item.id : '';
      const kind = typeof item.type === 'string' ? item.type : '';
      if (kind === 'reasoning') {
        if (eventType === 'item.completed') {
          if (completedItems.has(id)) return deltas;
          completedItems.add(id);
          const text = reasoningText(item) || '';
          const previous = reasoningItems.get(id) || '';
          reasoningItems.delete(id);
          emitThinking(deltas, id, increment(previous, text));
        } else {
          const text = reasoningText(item);
          if (text === null) return deltas;
          const previous = reasoningItems.get(id) || '';
          // F1 修复（codex.rs:266）：比已缓存文本更短的事件直接忽略，否则缓存被改短后，
          // 定稿事件会按最长公共前缀把中段重复补发一次。
          if (Array.from(text).length <= Array.from(previous).length) return deltas;
          const delta = increment(previous, text);
          reasoningItems.set(id, text);
          emitThinking(deltas, id, delta);
        }
        return deltas;
      }
      if (kind === 'agent_message') {
        // F5 修复（codex.rs:278）：agent_message 只在 item.completed 处理，
        // 避免 started/updated 提前占用 completed_items 让定稿被丢弃（最终回复截断）。
        if (eventType !== 'item.completed') return deltas;
        if (completedItems.has(id)) return deltas;
        completedItems.add(id);
        const text = typeof item.text === 'string' ? item.text : '';
        if (!text) return deltas;
        if (state.full_text !== '') state.full_text += '\n\n';
        state.full_text += text;
        deltas.push({ kind: 'text', text });
      }
      return deltas;
    },
  };
}

/** 复刻 codex.rs::read_stream_with_activity 的每行投递顺序：先 tool 事件，再 thinking/text 增量。 */
function replay(lines) {
  const run = activity.createAgentActivity();
  const activityState = { completed: new Set(), notices: new Set() };
  const parser = createDeltaParser();
  const trace = [];
  for (const line of lines) {
    for (const tool of codexActivityEvents(line, activityState)) {
      run.upsertTool(tool);
      trace.push('tool:' + tool.id + ':' + tool.status);
    }
    for (const delta of parser.parse(line)) {
      if (delta.kind === 'thinking') run.appendThinking(delta.text); else run.appendText(delta.text);
      trace.push(delta.kind);
    }
  }
  return { run, parser, activityState, trace };
}

// 注意：items 由 vm 沙箱内的模块创建，跨 realm 的数组原型不同，
// 直接 deepStrictEqual 会报 "same structure but not reference-equal"，所以统一走 plain()。
const kinds = (items) => plain(items.map((item) => item.kind));
const toolItems = (items) => plain(items.filter((item) => item.kind === 'tool').map((item) => item.tool));

// ---------------------------------------------------------------- 1. 真实顺序
test('real Codex turn order (reasoning -> command -> file_change -> reasoning -> answer) is preserved', () => {
  const lines = [
    '{"type":"thread.started","thread_id":"01a13874-fe8d-7ad1-a47d-dd45d8cb2e6c"}',
    '{"type":"turn.started"}',
    '{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"我会先查看当前目录。"}}',
    '{"type":"item.completed","item":{"id":"item_4","type":"reasoning","text":"先确认命令执行方式。"}}',
    '{"type":"item.started","item":{"id":"item_5","type":"command_execution","command":"Get-ChildItem","aggregated_output":"","exit_code":null,"status":"in_progress"}}',
    '{"type":"item.completed","item":{"id":"item_5","type":"command_execution","command":"Get-ChildItem","aggregated_output":"ok","exit_code":0,"duration_ms":12,"status":"completed"}}',
    '{"type":"item.started","item":{"id":"item_6","type":"file_change","changes":[{"path":"trace-note.txt","kind":"add"}],"status":"in_progress"}}',
    '{"type":"item.completed","item":{"id":"item_6","type":"file_change","changes":[{"path":"trace-note.txt","kind":"add"}],"status":"completed"}}',
    '{"type":"item.completed","item":{"id":"item_7","type":"reasoning","text":"文件已写入，汇报结果。"}}',
    '{"type":"item.completed","item":{"id":"item_8","type":"agent_message","text":"任务完成。"}}',
    '{"type":"turn.completed","usage":{"input_tokens":96970}}',
  ];
  const { run, trace } = replay(lines);
  assert.deepEqual(kinds(run.items), ['text', 'thinking', 'tool', 'tool', 'thinking', 'text'],
    'activity items 必须按事件到达顺序排列且不丢项');
  assert.deepEqual(trace, [
    'text',
    'thinking',
    'tool:item_5:running', 'tool:item_5:completed',
    'tool:item_6:running', 'tool:item_6:completed',
    'thinking',
    'text',
  ], 'thinking 必须排在其后的 agent_message 之前，命令/文件事件不得整体后移');
  const tools = toolItems(run.items);
  assert.equal(tools.length, 2, '同一 id 的 started/completed 合并为一条，不重复');
  assert.deepEqual(tools.map((tool) => tool.id), ['item_5', 'item_6']);
  assert.equal(tools[0].status, 'completed');
  assert.equal(tools[0].exit_code, 0);
  assert.equal(tools[0].duration_ms, 12);
  assert.equal(tools[0].output, 'ok');
  assert.deepEqual(plain(tools[1].changes), [{ path: 'trace-note.txt', kind: 'add' }]);
});

// ---------------------------------------------------------------- 2. 真实夹具回放
test('real codex exec --json fixture replays with output/exit_code/changes intact and no terminal regression', () => {
  const lines = readFixture(FIXTURE_STDOUT2);
  assert.equal(lines.length, 16, 'stdout2.jsonl 夹具应有 16 行真实事件');

  const { run, parser } = replay(lines);
  assert.deepEqual(kinds(run.items), ['tool', 'text', 'tool', 'thinking', 'tool', 'tool', 'tool', 'text']);

  const tools = toolItems(run.items);
  assert.deepEqual(tools.map((tool) => tool.id), ['item_0', 'item_3', 'item_5', 'item_6', 'item_7']);
  const notices = tools.filter((tool) => tool.tool_name === 'codex_notice');
  assert.equal(notices.length, 1, 'item_0/item_1 文本相同 -> 同一回合内只报一次提示');
  assert.equal(notices[0].status, 'notice');

  const command3 = tools.find((tool) => tool.id === 'item_3');
  assert.equal(command3.tool_name, 'run_command');
  assert.equal(command3.status, 'failed');
  assert.equal(command3.exit_code, -1);
  assert.ok(command3.output.includes('helper_unknown_error'), '真实 aggregated_output 必须保留');
  assert.equal(command3.duration_ms == null, true, '真实夹具没有 duration_ms，不得臆造');

  for (const id of ['item_6', 'item_7']) {
    const file = tools.find((tool) => tool.id === id);
    assert.equal(file.tool_name, 'edit_file');
    assert.equal(file.status, 'failed', 'file_change 的 status=failed 必须映射为 failed');
    assert.equal(file.changes.length, 1);
    assert.equal(file.changes[0].kind, 'add');
    assert.ok(file.changes[0].path.endsWith('trace-note.txt'));
    assert.equal(file.path, file.changes[0].path);
  }

  const reasoning = lines.map((line) => JSON.parse(line))
    .find((event) => event.item && event.item.id === 'item_4').item.text;
  const thinking = run.items.filter((item) => item.kind === 'thinking');
  assert.equal(thinking.length, 1);
  assert.equal(thinking[0].text, reasoning);
  assert.equal(parser.state.full_thinking, reasoning);

  const answers = lines.map((line) => JSON.parse(line))
    .filter((event) => event.item && event.item.type === 'agent_message').map((event) => event.item.text);
  assert.deepEqual(plain(run.items.filter((item) => item.kind === 'text').map((item) => item.text)), answers);

  // 反例 A：同一 id 的 item.completed 重复到达 + 迟到的 item.started（exit_code 为 null）
  const before = plain(tools);
  const realById = new Map(lines.map((line) => JSON.parse(line))
    .filter((event) => event.item && event.item.id).map((event) => [event.item.id, event.item]));
  const lateStarted = (id) => JSON.stringify({
    type: 'item.started',
    item: { id, type: 'command_execution', command: realById.get(id).command, aggregated_output: '', exit_code: null, status: 'in_progress' },
  });
  const duplicated = [lines[6], lateStarted('item_3'), lateStarted('item_5')];
  for (const line of duplicated) {
    const seen = { completed: new Set(tools.map((tool) => tool.id)), notices: new Set() };
    assert.deepEqual(codexActivityEvents(line, seen), [], 'Rust 侧 completed id 会整体丢弃重复/迟到事件');
    for (const tool of codexActivityEvents(line, { completed: new Set(), notices: new Set() })) run.upsertTool(tool);
  }
  const after = plain(toolItems(run.items));
  assert.equal(after.length, before.length, '重复/迟到事件不得新增轨迹条目');
  for (const [index, tool] of after.entries()) {
    assert.equal(tool.status, before[index].status, '终态不得回退');
    assert.equal(tool.exit_code, before[index].exit_code, 'exit_code 不得被 null 覆盖');
    assert.deepEqual(tool.output, before[index].output, 'output 不得被清空');
    assert.deepEqual(tool.changes, before[index].changes, 'changes 不得被清空');
  }
});

// ---------------------------------------------------------------- 3. 终态与 0 值
test('late null/empty updates never regress a terminal tool nor erase zero-valued fields', () => {
  const run = activity.createAgentActivity();
  run.upsertTool({ id: 'c9', tool_name: 'run_command', status: 'running', summary: 'cargo test', arguments: '{}' });
  run.upsertTool({
    id: 'c9', tool_name: 'run_command', status: 'completed', summary: 'cargo test', arguments: '{}',
    command: 'cargo test', output: 'ok', exit_code: 0, duration_ms: 0,
    changes: [{ path: 'a.rs', kind: null }],
  });
  run.upsertTool({
    id: 'c9', tool_name: 'run_command', status: 'running', summary: 'cargo test', arguments: '{}',
    output: null, exit_code: null, duration_ms: null, changes: [], command: null,
  });
  const tool = toolItems(run.items)[0];
  assert.equal(run.items.length, 1, '同一 id 不产生第二条');
  assert.equal(tool.status, 'completed', 'running 不得覆盖终态');
  assert.equal(tool.exit_code, 0, 'exit_code=0 是合法值，迟到的 null 不得抹掉');
  assert.equal(tool.duration_ms, 0, 'duration_ms=0 是合法值');
  assert.equal(tool.output, 'ok');
  assert.equal(tool.command, 'cargo test');
  assert.deepEqual(plain(tool.changes), [{ path: 'a.rs', kind: null }], '空数组不得清空已上报的变更清单');

  // 反例 B：exit_code 为 null 但 status 为 failed（stdout.jsonl 的 file_change 就是这个形状）
  const failedNoCode = codexActivityEvents(
    '{"type":"item.completed","item":{"id":"f9","type":"file_change","status":"failed","changes":[{"path":"x.rs","kind":null}]}}',
    { completed: new Set(), notices: new Set() },
  );
  assert.equal(failedNoCode[0].status, 'failed', 'status=failed 且没有 exit_code 仍须判失败');
  assert.equal(failedNoCode[0].exit_code, null);
  assert.deepEqual(failedNoCode[0].changes, [{ path: 'x.rs', kind: null }], 'kind 为 null 的变更必须保留');

  // 反例 C：changes 里缺少 path 的条目按 Rust filter_map 语义丢弃
  const dropped = codexActivityEvents(
    '{"type":"item.completed","item":{"id":"f10","type":"file_change","changes":[{"kind":"add"},{"path":"keep.rs","kind":"update"}]}}',
    { completed: new Set(), notices: new Set() },
  );
  assert.deepEqual(dropped[0].changes, [{ path: 'keep.rs', kind: 'update' }]);
});

// ---------------------------------------------------------------- 4. notice / plan [MODEL]
test('codex_notice and update_plan stay renderable at the model layer [MODEL]', () => {
  assert.equal(activity.toolStatusLabel('notice'), '提示');
  assert.equal(activity.toolStatusLabel('failed'), '失败');
  const notice = codexActivityEvents(
    '{"type":"item.completed","item":{"id":"n1","type":"error","message":"Codex is ignoring 3 unrecognized configuration settings.\\n  user (config.toml): network_access is ignored."}}',
    { completed: new Set(), notices: new Set() },
  )[0];
  assert.equal(notice.tool_name, 'codex_notice');
  assert.equal(notice.status, 'notice');
  assert.equal(notice.summary, 'Codex is ignoring 3 unrecognized configuration settings.');
  assert.ok(notice.output.includes('network_access'));
  assert.equal(notice.exit_code, null);

  const plan = codexActivityEvents(
    '{"type":"item.completed","item":{"id":"p1","type":"todo_list","items":[{"text":"第一步","completed":true},{"text":"第二步","completed":false}]}}',
    { completed: new Set(), notices: new Set() },
  )[0];
  assert.equal(plan.tool_name, 'update_plan');
  assert.equal(plan.summary, '更新任务清单');
  const parsed = JSON.parse(plan.arguments);
  assert.ok(Array.isArray(parsed.plan), 'arguments.plan 必须是数组');
  assert.deepEqual(plain(parsed.plan), [{ text: '第一步', completed: true }, { text: '第二步', completed: false }]);
  assert.ok(planArray(parsed), 'AgentActivity.vue 的 planArray 必须能从该结构取到清单');
  function planArray(value) {
    if (Array.isArray(value)) return value;
    if (!value || typeof value !== 'object') return null;
    for (const key of ['plan', 'steps', 'items', 'todos', 'tasks']) {
      const candidate = value[key];
      if (Array.isArray(candidate)) return candidate;
      if (candidate && typeof candidate === 'object') { const nested = planArray(candidate); if (nested) return nested; }
    }
    return null;
  }

  // F4 修复（agentActivity.ts:28）：notice 已是终态，中止后保持 notice，
  // 也不会被迟到的 running/authorized 更新覆盖。
  const aborted = activity.createAgentActivity();
  aborted.upsertTool(notice);
  const finished = aborted.finish('已中止', 'aborted');
  assert.equal(finished.activity[0].tool.status, 'notice', 'F4 已修：中止后 notice 必须保持 notice');
  aborted.upsertTool({ ...notice, status: 'running' });
  assert.equal(aborted.items[0].tool.status, 'notice', '终态 notice 不接受迟到的 running');
  assert.equal(activity.toolStatusLabel('notice'), '提示');

  // 与 Rust 单测固定的 pretty 输出交叉核对（证明 [PORT] 的序列化与 serde_json 一致）
  const search = codexActivityEvents(
    '{"type":"item.started","item":{"id":"s1","type":"web_search","query":"测试"}}',
    { completed: new Set(), notices: new Set() },
  )[0];
  assert.equal(search.arguments, '{\n  "action": null,\n  "query": "测试"\n}',
    '必须与 cli_activity.rs 测试里固定的 serde_json 输出一致');
});

// ---------------------------------------------------------------- 5. 缓存兼容
test('activity cache restores new-field and legacy tool items and drops wrong types', () => {
  const map = new Map();
  const storage = {
    getItem: (key) => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => map.set(key, value),
    removeItem: (key) => map.delete(key),
  };
  const message = {
    content: '正文', thinking: '', timestamp: 500000, agent: { id: 'a', name: 'x' },
    displayContent: '正文',
    activity: [
      { kind: 'thinking', text: '思考' },
      { kind: 'tool', tool: {
        id: 'item_3', tool_name: 'run_command', status: 'failed', summary: 'Get-ChildItem',
        arguments: '{"command":"Get-ChildItem"}', command: 'Get-ChildItem', path: null,
        output: 'Failed to create unified exec process', exit_code: -1, duration_ms: 12,
        changes: [{ path: 'a.rs', kind: null }],
      } },
      { kind: 'tool', tool: { id: 'n1', tool_name: 'codex_notice', status: 'notice', summary: '提示', arguments: '{}', output: '告警' } },
      { kind: 'text', text: '正文' },
    ],
  };
  cache.saveActivityPresentation(storage, message);
  const restored = cache.restoreActivityPresentation(storage, { ...message, timestamp: 501000 });
  assert.ok(restored, '带新字段的缓存必须能恢复');
  assert.deepEqual(plain(restored.activity), plain(message.activity));

  const legacy = {
    content: '旧', thinking: '', timestamp: 600000, agent: null, displayContent: '旧',
    activity: [{ kind: 'tool', tool: { id: 'l1', tool_name: 'read_file', status: 'completed', summary: '读取', arguments: '{"path":"a.ts"}', output: 'ok' } }],
  };
  const legacyStorage = {
    getItem: () => JSON.stringify([{ identity: JSON.stringify(['旧', '', '']), timestamp: 600000, displayContent: '旧', activity: legacy.activity }]),
    setItem: () => {}, removeItem: () => {},
  };
  const legacyRestored = cache.restoreActivityPresentation(legacyStorage, legacy);
  assert.ok(legacyRestored, '旧格式缓存必须继续恢复');
  assert.deepEqual(plain(legacyRestored.activity), legacy.activity);

  for (const bad of [
    { exit_code: 'x' }, { duration_ms: Number.POSITIVE_INFINITY }, { changes: [{ kind: 'add' }] },
  ]) {
    const raw = JSON.stringify([{
      identity: 'k', timestamp: 1, displayContent: 'x',
      activity: [{ kind: 'tool', tool: { id: 'b', tool_name: 'run_command', status: 'completed', ...bad } }],
    }]);
    assert.equal(cache.restoreActivityPresentation({ getItem: () => raw }, { content: 'x', timestamp: 1 }),
      undefined, 'invalid cached tool must be dropped: ' + JSON.stringify(bad));
  }

  assert.doesNotThrow(() => cache.restoreActivityPresentation({ getItem: () => '{"version":2}' }, message));
});

// ---------------------------------------------------------------- 6. [PORT] 反例探针
test('[PORT] F1/F5 counterexamples: my original repros now behave correctly', () => {
  // F1 原始复现：先全文 -> 再更短 -> 定稿给回全文
  const parser = createDeltaParser();
  assert.deepEqual(parser.parse('{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一段思考内容"}}'),
    [{ kind: 'thinking', text: '第一段思考内容' }]);
  assert.deepEqual(parser.parse('{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一段"}}'), [],
    'F1 已修：更短的 updated 被忽略');
  assert.equal(parser.state.full_thinking, '第一段思考内容');
  assert.deepEqual(parser.parse('{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一段思考内容，结论"}}'),
    [{ kind: 'thinking', text: '，结论' }], 'F1 已修：定稿只补尾部，不再重复中段');
  assert.equal(parser.state.full_thinking, '第一段思考内容，结论');
  assert.deepEqual(parser.parse('{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一段思考内容，结论"}}'), []);

  // F1 变体：更短的 updated + 定稿与已流出内容完全一致 -> 必须零增量
  const variant = createDeltaParser();
  variant.parse('{"type":"item.updated","item":{"id":"r9","type":"reasoning","text":"第一段思考内容，结论"}}');
  variant.parse('{"type":"item.updated","item":{"id":"r9","type":"reasoning","text":"第一段思考内容"}}');
  assert.deepEqual(variant.parse('{"type":"item.completed","item":{"id":"r9","type":"reasoning","text":"第一段思考内容，结论"}}'), [],
    'F1 已修：定稿与已流出内容一致时不补发');
  assert.equal(variant.state.full_thinking, '第一段思考内容，结论');

  // F5 原始复现：正文用 started/updated 增量推送，定稿带全文
  const message = createDeltaParser();
  assert.deepEqual(message.parse('{"type":"item.updated","item":{"id":"m1","type":"agent_message","text":"前半段"}}'), [],
    'F5 已修：updated 的 agent_message 不再冒充定稿');
  assert.deepEqual(message.parse('{"type":"item.started","item":{"id":"m1","type":"agent_message","text":"前半段"}}'), []);
  assert.equal(message.state.full_text, '', 'started/updated 不得写入 full_text');
  assert.deepEqual(message.parse('{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"前半段，后半段。"}}'),
    [{ kind: 'text', text: '前半段，后半段。' }]);
  assert.equal(message.state.full_text, '前半段，后半段。', 'F5 已修：定稿不再被 completed_items 丢弃');
  assert.deepEqual(message.parse('{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"前半段，后半段。"}}'), []);
  assert.equal(message.state.full_text, '前半段，后半段。');

  // F5 的两个不同 id 定稿都要保留
  const two = createDeltaParser();
  two.parse('{"type":"item.completed","item":{"id":"m3","type":"agent_message","text":"第一段"}}');
  two.parse('{"type":"item.completed","item":{"id":"m4","type":"agent_message","text":"第二段"}}');
  assert.equal(two.state.full_text, '第一段\n\n第二段');

  // F5 修复的已知取舍：若某 CLI 只推 updated 且从不发 completed，正文会是空的（修复前是"只有第一段"）。
  const updatesOnly = createDeltaParser();
  assert.deepEqual(updatesOnly.parse('{"type":"item.updated","item":{"id":"m2","type":"agent_message","text":"只有增量"}}'), []);
  assert.equal(updatesOnly.state.full_text, '');

  // 残余边界 1（修复前后结果一致，不是 F1 引入的回归；真实夹具不出现，Codex 的 reasoning 只增长）：
  // 非"前缀扩展"的改写（这里用等长文本举例）本来就会被前缀法拼出多余内容，
  // 修复只是让这个等长更新本身不再单独产生增量。
  const equalLength = createDeltaParser();
  equalLength.parse('{"type":"item.updated","item":{"id":"r5","type":"reasoning","text":"ABC"}}');
  assert.deepEqual(equalLength.parse('{"type":"item.updated","item":{"id":"r5","type":"reasoning","text":"ABD"}}'), [],
    '等长改写被忽略');
  assert.deepEqual(equalLength.parse('{"type":"item.completed","item":{"id":"r5","type":"reasoning","text":"ABD"}}'),
    [{ kind: 'thinking', text: 'D' }], '残余边界：会补出 D，full_thinking=ABCD 而不是 ABD');
  assert.equal(equalLength.state.full_thinking, 'ABCD');

  // F1 修复的残余边界 2（修复前就存在）：更长但不是前缀扩展的更新整段追加。
  const rewrite = createDeltaParser();
  rewrite.parse('{"type":"item.updated","item":{"id":"r6","type":"reasoning","text":"ABCD"}}');
  assert.deepEqual(rewrite.parse('{"type":"item.updated","item":{"id":"r6","type":"reasoning","text":"ABXYZW"}}'),
    [{ kind: 'thinking', text: 'XYZW' }]);
  assert.equal(rewrite.state.full_thinking, 'ABCDXYZW');

  const fallback = createDeltaParser();
  assert.deepEqual(fallback.parse('{"type":"item.completed","item":{"id":"r2","type":"reasoning","text":"","summary_text":"摘要"}}'),
    [{ kind: 'thinking', text: '摘要' }]);
  assert.deepEqual(fallback.parse('{"type":"item.completed","item":{"id":"r3","type":"reasoning","raw_content":[{"type":"text","text":"甲"},{"type":"text","text":"乙"}]}}'),
    [{ kind: 'thinking', text: '甲\n乙' }]);

  const tools = { completed: new Set(), notices: new Set() };
  const line = '{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"x","aggregated_output":"o","exit_code":1,"status":"failed"}}';
  assert.equal(codexActivityEvents(line, tools).length, 1);
  assert.equal(codexActivityEvents(line, tools).length, 0);

  const notices = { completed: new Set(), notices: new Set() };
  const noticeLine = '{"type":"item.completed","item":{"id":"e1","type":"error","message":"same"}}';
  assert.equal(codexActivityEvents(noticeLine, notices).length, 1);
  assert.equal(codexActivityEvents(noticeLine.replace('e1', 'e2'), notices).length, 0, '同回合同文本去重');
  assert.equal(codexActivityEvents(noticeLine.replace('e1', 'e3').replace('same', 'other'), notices).length, 1);
  assert.equal(codexActivityEvents('{"type":"item.completed","item":{"id":"e9","type":"error","message":"same"}}',
    { completed: new Set(), notices: new Set() }).length, 1, '新回合必须重新提示，去重不能永久生效');
});

// ---------------------------------------------------------------- 6b. [PORT] 与 Rust 源码对齐（防回退）
test('Rust source still contains the exact F1/F5 guards that the [PORT] mirrors', () => {
  // 本脚本的 createDeltaParser 是 codex.rs::EventParser 的移植；这条静态断言保证
  // 移植对象仍然带着 F1/F5 的修复，防止"port 绿、Rust 回退"这种假绿。
  const source = fs.readFileSync(path.join(root, 'src-tauri/src/codex.rs'), 'utf8');
  const itemFn = source.slice(source.indexOf('fn item('), source.indexOf('fn emit_thinking('));
  assert.ok(itemFn.length > 0, '未找到 EventParser::item');
  assert.match(itemFn, /if text\.chars\(\)\.count\(\) <= previous\.chars\(\)\.count\(\) \{\s*return;\s*\}/,
    'F1 防护已回退：reasoning 的"更短则忽略"分支不见了');
  const messageArm = itemFn.slice(itemFn.indexOf('"agent_message" => {'), itemFn.indexOf('_ => {}'));
  assert.ok(messageArm.length > 0, '未找到 agent_message 分支');
  assert.match(messageArm, /if event_type != "item\.completed" \{\s*return;\s*\}/,
    'F5 防护已回退：agent_message 不再限制在 item.completed');
  assert.match(itemFn, /let delta = increment\(&previous, &text\);\s*self\.emit_thinking/,
    'F1 的定稿按前缀补尾部路径应仍然存在');
});

// ---------------------------------------------------------------- 7. 前端契约 [STATIC]
test('AgentActivity.vue keeps the selectors/copy required by verify-agent-activity-browser.cjs [STATIC]', () => {
  const vue = fs.readFileSync(path.join(root, 'src/components/AgentActivity.vue'), 'utf8');
  for (const token of [
    "class=\"['agent-activity', { compact }]\"", 'aria-label="工作过程"', 'class="activity-status"',
    "['activity-tool', row.status]", 'class="activity-output"', 'class="activity-reasoning"',
    'class="activity-node"', 'class="activity-list"', 'activity-changes"', 'class="activity-plan"',
    '停止生成', 'activity-count', '思考过程', '展开全部', 'changeKindLabels', '更新任务清单',
  ]) {
    assert.ok(vue.includes(token), 'AgentActivity.vue 缺少契约片段: ' + token);
  }
  assert.match(vue, /<details v-else :class="\['activity-tool', row\.status\]">/);
  for (const state of ['running', 'waiting', 'completed', 'failed', 'denied', 'notice', 'interrupted']) {
    assert.ok(vue.includes('.' + state), 'CSS 缺少状态类: .' + state);
  }
  for (const token of [
    '.activity-entry > * { min-width: 0; }',
    'grid-template-columns: 15px minmax(0, 1fr)',
    '.compact .activity-tool > summary { flex-wrap: wrap;',
    'overflow-wrap: anywhere',
    'word-break: break-word',
    '.compact .activity-tool-label { max-width: 55%; }',
  ]) {
    assert.ok(vue.includes(token), '缺少 340px 溢出防护规则: ' + token);
  }
});

test('compact/340px overflow: only static CSS reasoning is possible here [STATIC, approximate]', () => {
  const vue = fs.readFileSync(path.join(root, 'src/components/AgentActivity.vue'), 'utf8');
  assert.ok(vue.includes('flex-shrink: 0'), '徽标不应被压缩（依赖 flex-wrap 换行）');
  assert.ok(vue.includes('max-height: 220px; overflow-y: auto'), '思考块自身滚动，不拉长容器');
  assert.ok(!/\.activity-tool > summary \{[^}]*white-space: nowrap/.test(vue), 'summary 不应强制 nowrap');
});

// ---------------------------------------------------------------- 8. demo 夹具 × browser 断言一致性（F2/F3 复验）
test('demo fixture satisfies every ?activity=demo assertion in verify-agent-activity-browser.cjs', () => {
  const mock = fs.readFileSync(path.join(root, 'src/dev-tauri-mock.ts'), 'utf8');
  const browser = fs.readFileSync(path.join(root, 'scripts/verify-agent-activity-browser.cjs'), 'utf8');
  const oldText = '检查完成，活动流和历史消息均正常。';
  assert.ok(!browser.includes(oldText), 'F2 已修：browser 脚本不再等待旧结束语');
  assert.ok(!mock.includes(oldText), '新夹具不再输出旧结束语');

  // 阈值直接从 browser 脚本里读，避免两边各写一份数字后悄悄漂移
  const demoLine = (selector) => {
    const line = browser.split('\n').find((text) => text.includes("demo.locator('" + selector + "')"));
    assert.ok(line, 'browser 脚本缺少 demo 断言: ' + selector);
    return line;
  };
  const countEquals = (selector) => {
    const line = demoLine(selector);
    const match = /\.count\(\), (\d+)/.exec(line);
    assert.ok(match, 'browser 断言不是 count() === N: ' + line.trim());
    return Number(match[1]);
  };
  const countAtLeast = (selector) => {
    const line = demoLine(selector);
    const match = /\.count\(\) >= (\d+)/.exec(line);
    assert.ok(match, 'browser 断言不是 count() >= N: ' + line.trim());
    return Number(match[1]);
  };

  // 按 mock 的 frames 顺序回放（tool / thinking / answer 三类都参与顺序）
  const run = activity.createAgentActivity();
  for (const [event, payload] of loadDemoFrames(mock)) {
    if (event === 'ai-tool-event') run.upsertTool(payload);
    else if (event === 'ai-thinking') run.appendThinking(payload);
    else if (event === 'ai-answer-delta') run.appendText(payload.text);
  }
  const tools = toolItems(run.items);
  const byStatus = (status) => tools.filter((tool) => tool.status === status).length;

  assert.equal(byStatus('completed'), countEquals('.activity-tool.completed'), 'F3 已修：重跑用新 id，completed 数应与 browser 断言一致');
  assert.equal(byStatus('failed'), countEquals('.activity-tool.failed'));
  assert.equal(byStatus('notice'), countEquals('.activity-tool.notice'));
  assert.equal(tools.length, 6, '夹具共 6 个工具条目（4 completed / 1 failed / 1 notice）');

  // F3：失败那次的 exit_code 必须保持 1，重跑用新 id 不再覆盖它
  const firstRun = tools.find((tool) => tool.id === 'preview-test');
  assert.equal(firstRun.status, 'failed');
  assert.equal(firstRun.exit_code, 1, 'F3 已修：failed 行的 exit_code 不再被成功那次的 0 覆盖');
  assert.equal(tools.find((tool) => tool.id === 'preview-retest').status, 'completed');
  assert.ok(browser.includes("getByText('退出码 1', { exact: true })"), 'browser 脚本应断言退出码 1');
  const visibleExit = tools.filter((tool) => typeof tool.exit_code === 'number' && tool.exit_code !== 0);
  assert.equal(visibleExit.length, 1, '界面上只会出现一个退出码红标');

  // 清单 / 变更 / 思考分段
  const plan = tools.find((tool) => tool.tool_name === 'update_plan');
  assert.ok(JSON.parse(plan.arguments).plan.length >= countAtLeast('.activity-plan li'), '清单步数应满足 browser 断言');
  const changeCount = tools.reduce((total, tool) => total + (tool.changes ? tool.changes.length : 0), 0);
  assert.ok(changeCount >= countAtLeast('.activity-change-kind'), '变更标注数应满足 browser 断言');
  const thinkingCount = kinds(run.items).filter((kind) => kind === 'thinking').length;
  assert.ok(thinkingCount >= countAtLeast('.activity-reasoning'), '思考分段数应满足 browser 断言');

  function loadDemoFrames(source) {
    const start = source.indexOf('const frames');
    assert.ok(start > 0, '未找到 demo frames');
    const open = source.indexOf('= [', start) + 2;
    let depth = 0;
    let quote = null;
    let escaped = false;
    let end = open;
    for (let i = open; i < source.length; i += 1) {
      const ch = source[i];
      if (quote) {
        if (escaped) escaped = false;
        else if (ch === '\\') escaped = true;
        else if (ch === quote) quote = null;
        continue;
      }
      if (ch === '"' || ch === "'" || ch === '\u0060') { quote = ch; continue; }
      if (ch === '[') depth += 1;
      else if (ch === ']') { depth -= 1; if (depth === 0) { end = i; break; } }
    }
    return vm.runInNewContext('(' + source.slice(open, end + 1) + ')', { JSON });
  }
});
