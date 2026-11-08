<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch, type Component } from "vue";
import {
  ChevronRight, Clock, FileText, Globe, Info, ListChecks, LoaderCircle,
  Pencil, Search, Square, Terminal, Wrench,
} from "@lucide/vue";
import { formatArguments, legacyActivity, toolStatusLabel, type AgentActivityFileChange, type AgentActivityItem, type AgentActivityTool } from "../services/agentActivity";
import { toolLabel } from "../services/tools";
import { renderMessageSegments } from "../services/markdown";
import { extractMentionNames } from "../services/mentions";

const props = withDefaults(defineProps<{
  items?: AgentActivityItem[];
  tools?: AgentActivityTool[];
  thinking?: string;
  active?: boolean;
  waiting?: boolean;
  awaitingAnswer?: boolean;
  compact?: boolean;
}>(), { active: false, waiting: false, awaitingAnswer: false, compact: false });
const emit = defineEmits<{ stop: [] }>();

/** 单次输出超过这个字符数就先截断，避免一屏塞满日志。 */
const OUTPUT_LIMIT = 800;

const entries = computed<AgentActivityItem[]>(() => {
  if (props.items !== undefined) return props.items;
  const legacy = legacyActivity(props.thinking);
  if (!props.tools?.length) return legacy;
  return [...legacy.filter((item) => item.kind !== "tool"), ...props.tools.map((tool) => ({ kind: "tool" as const, tool }))];
});
const tools = computed(() => entries.value.filter((item) => item.kind === "tool").map((item) => item.tool));
const isWaiting = computed(() => props.waiting || tools.value.some((tool) => tool.status === "waiting"));
const completed = computed(() => tools.value.filter((tool) => tool.status === "completed").length);
const status = computed(() => {
  if (props.awaitingAnswer || tools.value.some((tool) => tool.tool_name === "ask_user" && tool.status === "waiting")) return "等待回答";
  if (isWaiting.value) return "等待确认";
  if (tools.value.some((tool) => tool.status === "running")) return "正在执行";
  if (entries.value[entries.value.length - 1]?.kind === "text") return "正在回复";
  return entries.value.length ? "正在思考" : "正在处理";
});

/**
 * 过程文字（模型输出的中间段落）同样按 Markdown 渲染，
 * 否则流式输出阶段用户会先看到满屏的 ** 和 #，收尾时才突然变干净。
 */
function commentaryHtml(text: string): string {
  return renderMessageSegments(text, {
    names: extractMentionNames(text),
    mention: { htmlTag: '<span class="activity-mention">' },
  })
    .map((segment) => segment.html)
    .join("");
}

const toolAliases: Record<string, string> = {
  Read: "读取文件", Write: "写入文件", Edit: "修改文件", Bash: "执行命令", Glob: "查找文件", Grep: "搜索代码",
  WebSearch: "网页搜索", WebFetch: "读取网页",
  run_command: "执行命令", edit_file: "修改文件", write_file: "写入文件", read_file: "读取文件",
  list_directory: "列目录", code_search: "搜索代码", file_search: "搜索文件", web_search: "网页搜索",
  read_webpage: "读取网页", update_plan: "更新任务清单", codex_notice: "提示", ask_user: "询问用户",
  create_docx: "生成文档", save_memory: "保存记忆", search_memory: "搜索记忆",
};
/** MCP 工具名已经是 "server / tool" 形态，直接展示比查表更准确。 */
function isMcpTool(name: string): boolean {
  return name.includes(" / ");
}
function activityToolLabel(name: string): string {
  return isMcpTool(name) ? name : toolAliases[name] || toolLabel(name);
}
function toolIcon(tool: AgentActivityTool): Component {
  const name = tool.tool_name;
  if (name === "run_command" || name === "Bash") return Terminal;
  if (name === "edit_file" || name === "write_file" || name === "Write" || name === "Edit") return Pencil;
  if (name === "read_file" || name === "Read" || name === "list_directory" || name === "Glob") return FileText;
  if (name === "code_search" || name === "Grep" || name === "file_search") return Search;
  if (name === "web_search" || name === "WebSearch" || name === "WebFetch" || name === "read_webpage") return Globe;
  if (name === "update_plan") return ListChecks;
  if (name === "codex_notice" || name === "ask_user") return Info;
  return Wrench;
}
/** 行尾目标：优先命令，其次文件路径，最后才是摘要。 */
function toolTarget(tool: AgentActivityTool): string {
  if (tool.command) return tool.command;
  if (tool.path) return tool.path;
  const summary = tool.summary?.trim();
  if (!summary || isMcpTool(tool.tool_name)) return "";
  return summary === tool.tool_name || summary === activityToolLabel(tool.tool_name) ? "" : summary;
}
/** 详情里的补充说明：和标题、命令、路径重复的摘要不再重复展示。 */
function toolSummary(tool: AgentActivityTool): string {
  const summary = tool.summary?.trim();
  if (!summary) return "";
  if (summary === tool.tool_name || summary === activityToolLabel(tool.tool_name)) return "";
  if (summary === tool.command?.trim() || summary === tool.path?.trim()) return "";
  return summary;
}
function formatDuration(ms: number | null | undefined): string {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
  return `${Math.floor(seconds / 60)}m${String(Math.round(seconds % 60)).padStart(2, "0")}s`;
}
function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  if (total < 60) return `${total}s`;
  return `${Math.floor(total / 60)}m${String(total % 60).padStart(2, "0")}s`;
}
function exitFailed(tool: AgentActivityTool): boolean {
  return typeof tool.exit_code === "number" && tool.exit_code !== 0;
}
const changeKindLabels: Record<string, string> = { add: "新增", update: "修改", delete: "删除", move: "移动", rename: "重命名", copy: "复制" };
function changeKindLabel(kind?: string | null): string {
  return kind ? changeKindLabels[kind] ?? kind : "变更";
}

type PlanStep = { text: string; done: boolean; current: boolean };
function planArray(value: unknown): unknown[] | null {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  for (const key of ["plan", "steps", "items", "todos", "tasks"]) {
    const candidate = record[key];
    if (Array.isArray(candidate)) return candidate;
    if (candidate && typeof candidate === "object") {
      const nested = planArray(candidate);
      if (nested) return nested;
    }
  }
  return null;
}
/** update_plan 的参数按清单渲染，而不是甩一段 JSON 给用户。 */
function planSteps(tool: AgentActivityTool): PlanStep[] {
  if (tool.tool_name !== "update_plan" || !tool.arguments) return [];
  let parsed: unknown;
  try { parsed = JSON.parse(tool.arguments); } catch { return []; }
  const list = planArray(parsed);
  if (!list) return [];
  const steps: PlanStep[] = [];
  for (const raw of list) {
    if (typeof raw === "string") {
      if (raw.trim()) steps.push({ text: raw.trim(), done: false, current: false });
      continue;
    }
    if (!raw || typeof raw !== "object") continue;
    const record = raw as Record<string, unknown>;
    const text = ["step", "text", "title", "content", "task", "description", "name"].map((key) => record[key]).find((value) => typeof value === "string" && value.trim());
    if (typeof text !== "string") continue;
    const state = typeof record.status === "string" ? record.status : "";
    const done = record.completed === true || record.done === true || record.checked === true || ["completed", "done", "finished"].includes(state);
    const current = !done && ["in_progress", "running", "active", "current"].includes(state);
    steps.push({ text: text.trim(), done, current });
  }
  return steps;
}

/** 展开过的长输出按工具 id 记住，状态刷新时不会被重新截断。 */
const expandedOutput = ref<Record<string, boolean>>({});
function outputChars(tool: AgentActivityTool): number {
  return Array.from(tool.output ?? "").length;
}
function outputTruncated(tool: AgentActivityTool): boolean {
  return outputChars(tool) > OUTPUT_LIMIT;
}
function outputText(tool: AgentActivityTool): string {
  const chars = Array.from(tool.output ?? "");
  if (expandedOutput.value[tool.id] || chars.length <= OUTPUT_LIMIT) return chars.join("");
  return `${chars.slice(0, OUTPUT_LIMIT).join("")}…`;
}
function toggleOutput(id: string) {
  expandedOutput.value = { ...expandedOutput.value, [id]: !expandedOutput.value[id] };
}

type ToolView = {
  tool: AgentActivityTool;
  label: string;
  target: string;
  icon: Component;
  duration: string;
  exitFailed: boolean;
  state: string;
  spinning: boolean;
  summary: string;
  steps: PlanStep[];
  changes: AgentActivityFileChange[];
  showArguments: boolean;
  outputText: string;
  outputTruncated: boolean;
  outputChars: number;
  expanded: boolean;
};
function toolView(tool: AgentActivityTool): ToolView {
  const steps = planSteps(tool);
  return {
    tool,
    label: activityToolLabel(tool.tool_name),
    target: toolTarget(tool),
    icon: toolIcon(tool),
    duration: formatDuration(tool.duration_ms),
    exitFailed: exitFailed(tool),
    state: tool.tool_name === "ask_user" && tool.status === "waiting" ? "等待回答" : toolStatusLabel(tool.status),
    spinning: props.active && ["running", "requested", "approved"].includes(tool.status),
    summary: toolSummary(tool),
    steps,
    changes: tool.changes ?? [],
    showArguments: !!tool.arguments && !steps.length,
    outputText: outputText(tool),
    outputTruncated: outputTruncated(tool),
    outputChars: outputChars(tool),
    expanded: !!expandedOutput.value[tool.id],
  };
}

type ActivityRow =
  | { kind: "text"; key: string; html: string }
  | { kind: "thinking"; key: string; text: string; chars: number; streaming: boolean; open: boolean | undefined }
  | { kind: "tool"; key: string; status: string; view: ToolView };

const rows = computed<ActivityRow[]>(() => {
  const lastIndex = entries.value.length - 1;
  // 流式中：回合还在跑（active）且这条思考是最后一条 → 自动展开。
  // open 只在这两种情况间切换：undefined 让用户手动展开后不被后续重渲染关掉。
  return entries.value.map((entry, index) => {
    if (entry.kind === "text") return { kind: "text" as const, key: `text-${index}`, html: commentaryHtml(entry.text) };
    if (entry.kind === "thinking") {
      const live = props.active && index === lastIndex;
      return { kind: "thinking" as const, key: `thinking-${index}`, text: entry.text, chars: Array.from(entry.text).length, streaming: live, open: live ? true : undefined };
    }
    return { kind: "tool" as const, key: `tool-${entry.tool.id}`, status: entry.tool.status, view: toolView(entry.tool) };
  });
});

// 头部运行时长：active 时每秒走一格，结束后不再计时。
const startedAt = ref<number | null>(null);
const now = ref(0);
let timer: ReturnType<typeof setInterval> | undefined;
const elapsed = computed(() => (startedAt.value === null || !now.value ? 0 : Math.max(0, now.value - startedAt.value)));
const elapsedLabel = computed(() => (props.active ? formatElapsed(elapsed.value) : ""));
watch(() => props.active, (active) => {
  if (active) {
    startedAt.value = Date.now();
    now.value = Date.now();
    timer = setInterval(() => { now.value = Date.now(); }, 1000);
  } else {
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
  }
}, { immediate: true });
onBeforeUnmount(() => { if (timer !== undefined) clearInterval(timer); });
</script>

<template>
  <section v-if="rows.length || active" :class="['agent-activity', { compact }]" aria-label="工作过程">
    <div v-if="active" class="activity-status" role="status" aria-live="polite">
      <Clock v-if="isWaiting" :size="14" />
      <LoaderCircle v-else :size="14" class="activity-spin" />
      <span>{{ status }}</span>
      <span v-if="tools.length" class="activity-count">{{ completed }}/{{ tools.length }}</span>
      <span v-if="elapsedLabel" class="activity-elapsed">{{ elapsedLabel }}</span>
      <button type="button" class="activity-stop" title="停止生成" aria-label="停止生成" @click.stop="emit('stop')"><Square :size="12" fill="currentColor" /></button>
    </div>
    <ol class="activity-list">
      <li v-for="row in rows" :key="row.key" :class="['activity-entry', row.kind]">
        <span class="activity-node" :class="row.kind === 'tool' ? row.status : row.kind" aria-hidden="true"></span>
        <div v-if="row.kind === 'text'" class="activity-commentary md-body" v-html="row.html"></div>
        <details v-else-if="row.kind === 'thinking'" class="activity-reasoning" :open="row.open">
          <summary>
            <ChevronRight :size="12" class="activity-chevron" />
            <span class="activity-reasoning-title">思考过程</span>
            <span v-if="row.streaming" class="activity-streaming">思考中</span>
            <span class="activity-meta">{{ row.chars }} 字</span>
          </summary>
          <p>{{ row.text }}</p>
        </details>
        <details v-else :class="['activity-tool', row.status]">
          <summary :title="row.view.tool.summary || row.view.tool.tool_name">
            <component :is="row.view.icon" :size="14" :class="['activity-tool-icon', { 'activity-spin': row.view.spinning }]" />
            <span class="activity-tool-label">{{ row.view.label }}</span>
            <code v-if="row.view.target" class="activity-target">{{ row.view.target }}</code>
            <span v-if="row.view.duration" class="activity-duration">{{ row.view.duration }}</span>
            <span v-if="row.view.exitFailed" class="activity-exit">退出码 {{ row.view.tool.exit_code }}</span>
            <span class="activity-tool-state">{{ row.view.state }}</span>
            <ChevronRight :size="12" class="activity-chevron" />
          </summary>
          <div class="activity-detail">
            <p v-if="row.view.summary" class="activity-summary">{{ row.view.summary }}</p>
            <div v-if="row.view.tool.command" class="activity-field"><Terminal :size="13" /><pre>{{ row.view.tool.command }}</pre></div>
            <div v-if="row.view.tool.path" class="activity-field"><FileText :size="13" /><pre>{{ row.view.tool.path }}</pre></div>
            <div v-if="row.view.changes.length" class="activity-field activity-changes">
              <FileText :size="13" />
              <ul>
                <li v-for="(change, index) in row.view.changes" :key="`${change.path}-${index}`">
                  <span class="activity-change-kind" :class="change.kind || 'update'">{{ changeKindLabel(change.kind) }}</span>
                  <code>{{ change.path }}</code>
                </li>
              </ul>
            </div>
            <ul v-if="row.view.steps.length" class="activity-plan">
              <li v-for="(step, index) in row.view.steps" :key="index" :class="{ done: step.done, current: step.current }">
                <span class="activity-plan-mark" aria-hidden="true">{{ step.done ? '✓' : '•' }}</span>
                <span class="activity-plan-text">{{ step.text }}</span>
              </li>
            </ul>
            <details v-if="row.view.showArguments" class="activity-arguments"><summary>参数</summary><pre>{{ formatArguments(row.view.tool.arguments) }}</pre></details>
            <div v-if="row.view.tool.output" class="activity-output">
              <span>输出</span>
              <pre>{{ row.view.outputText }}</pre>
              <button v-if="row.view.outputTruncated" type="button" class="activity-more" @click="toggleOutput(row.view.tool.id)">
                {{ row.view.expanded ? '收起' : `展开全部（共 ${row.view.outputChars} 字）` }}
              </button>
            </div>
          </div>
        </details>
      </li>
    </ol>
  </section>
</template>

<style scoped>
.agent-activity { color: var(--dash-text-secondary, #5f6368); font-size: 12px; line-height: 1.6; width: 100%; min-width: 0; letter-spacing: 0; margin: 0 0 10px; }
.activity-status { display: flex; align-items: center; gap: 7px; min-height: 30px; color: var(--dash-text-primary, #30343b); margin-bottom: 6px; }
.activity-count { color: var(--dash-text-secondary, #686b73); font-variant-numeric: tabular-nums; font-size: 11px; }
.activity-elapsed { color: var(--dash-text-secondary, #686b73); font-variant-numeric: tabular-nums; font-size: 11px; }
.activity-stop { margin-left: auto; width: 28px; height: 28px; display: grid; place-items: center; border: 1px solid #d5d8de; background: transparent; color: inherit; border-radius: 4px; cursor: pointer; flex: 0 0 28px; }
.activity-stop:hover { background: #eceef1; }

/* 时间线：左侧竖线 + 每个条目一个节点，节点颜色跟随状态。 */
.activity-list { list-style: none; padding: 0; margin: 0; position: relative; }
.activity-list::before { content: ""; position: absolute; left: 7px; top: 12px; bottom: 12px; width: 1px; background: #e3e6ea; }
.activity-entry { position: relative; display: grid; grid-template-columns: 15px minmax(0, 1fr); column-gap: 8px; min-width: 0; }
/* 网格子项的自动最小尺寸是 min-content，必须显式归零，长命令/长路径才不会撑破容器。 */
.activity-entry > * { min-width: 0; }
.activity-entry + .activity-entry { margin-top: 5px; }
.activity-node { justify-self: center; align-self: start; width: 7px; height: 7px; margin-top: 8px; border-radius: 50%; background: #b6bbc4; }
.activity-node.thinking { background: #98a0b4; }
.activity-node.text { background: #4b91bf; }
.activity-node.completed { background: #21845a; }
.activity-node.failed, .activity-node.denied { background: #c24c4c; }
.activity-node.running, .activity-node.requested, .activity-node.approved { background: #287fa7; }
.activity-node.waiting { background: #a57924; }
.activity-node.notice { background: #8a8f98; }
.activity-node.interrupted, .activity-node.skipped { background: #b6bbc4; }

.activity-commentary { margin: 0; white-space: normal; overflow-wrap: anywhere; color: var(--dash-text-primary, #30343b); font-size: 13px; }
.activity-commentary .md-p:last-child { margin-bottom: 0; }
.activity-commentary .md-list { margin: 3px 0 5px; }
.activity-quote { border-left: 2px solid #c4c9d1; padding-left: 9px; margin: 5px 0; color: var(--dash-text-secondary, #5f6368); }
.activity-mention { color: #267a9c; font-weight: 500; }
summary { cursor: pointer; list-style: none; user-select: none; }
summary::-webkit-details-marker { display: none; }
summary:focus-visible, button:focus-visible { outline: 2px solid #4b91bf; outline-offset: 2px; }

/* 思考过程：流式中自动展开，结束后可折叠；正文等宽灰底、可滚动。 */
.activity-reasoning > summary { display: flex; align-items: center; gap: 5px; width: fit-content; max-width: 100%; min-height: 24px; }
.activity-reasoning-title { color: var(--dash-text-primary, #30343b); font-weight: 500; }
.activity-streaming { flex-shrink: 0; font-size: 10px; color: #287fa7; background: rgba(40, 127, 167, .12); border-radius: 3px; padding: 0 4px; }
.activity-meta { flex-shrink: 0; font-size: 10px; color: #8a8f98; font-variant-numeric: tabular-nums; }
.activity-reasoning p { margin: 6px 0 4px; padding: 8px 10px; border-radius: 4px; background: rgba(115, 123, 138, .08); font-family: Consolas, ui-monospace, monospace; font-size: 11px; white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word; max-height: 220px; overflow-y: auto; color: var(--dash-text-primary, #30343b); user-select: text; }

.activity-tool > summary { display: flex; align-items: center; gap: 7px; min-height: 30px; min-width: 0; border-radius: 4px; padding: 2px 4px; }
.activity-tool > summary:hover { background: rgba(100, 110, 128, .08); }
.activity-tool-icon { flex-shrink: 0; color: #767b84; }
.completed .activity-tool-icon { color: #21845a; }
.failed .activity-tool-icon, .denied .activity-tool-icon { color: #c24c4c; }
.running .activity-tool-icon { color: #287fa7; }
.waiting .activity-tool-icon { color: #a57924; }
.notice .activity-tool-icon { color: #8a8f98; }
.activity-tool-label { flex-shrink: 0; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dash-text-primary, #30343b); font-weight: 500; }
.activity-target { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11px; font-family: Consolas, ui-monospace, monospace; }
.activity-duration { flex-shrink: 0; font-size: 10px; color: #8a8f98; font-variant-numeric: tabular-nums; }
.activity-exit { flex-shrink: 0; font-size: 10px; color: #c24c4c; background: rgba(194, 76, 76, .1); border-radius: 3px; padding: 0 4px; font-variant-numeric: tabular-nums; }
.activity-tool-state { flex-shrink: 0; font-size: 10px; margin-left: auto; }
.activity-chevron { flex-shrink: 0; transition: transform .15s; }
details[open] > summary > .activity-chevron { transform: rotate(90deg); }

.activity-detail { margin: 3px 0 6px; padding: 6px 0 6px 12px; border-left: 1px solid #dfe2e6; min-width: 0; }
.activity-summary { margin: 0 0 7px; overflow-wrap: anywhere; }
.activity-field { display: flex; align-items: flex-start; gap: 6px; margin-bottom: 7px; min-width: 0; }
.activity-field svg { flex-shrink: 0; margin-top: 3px; }
pre { font: 11px/1.6 Consolas, ui-monospace, monospace; white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word; margin: 5px 0 0; max-height: 280px; overflow: auto; color: var(--dash-text-primary, #30343b); user-select: text; }
.activity-field pre { margin: 0; min-width: 0; }
.activity-output { margin-top: 8px; }
.activity-output > span, .activity-arguments > summary { font-size: 11px; color: #747985; }
.activity-output pre, .activity-arguments pre { background: rgba(115, 123, 138, .06); padding: 8px; border-radius: 4px; }
.activity-more { display: block; margin-top: 4px; padding: 0; border: 0; background: transparent; color: #267a9c; font-size: 11px; cursor: pointer; }
.activity-more:hover { text-decoration: underline; }

/* 文件变更：add/update/delete 用中文标注 + 状态色。 */
.activity-changes ul { list-style: none; margin: 0; padding: 0; min-width: 0; }
.activity-changes li { display: flex; align-items: baseline; gap: 6px; min-width: 0; margin-bottom: 3px; }
.activity-change-kind { flex-shrink: 0; font-size: 10px; border-radius: 3px; padding: 0 4px; color: #5f6368; background: rgba(115, 123, 138, .14); }
.activity-change-kind.add { color: #21845a; background: rgba(33, 132, 90, .12); }
.activity-change-kind.update { color: #287fa7; background: rgba(40, 127, 167, .12); }
.activity-change-kind.delete { color: #c24c4c; background: rgba(194, 76, 76, .12); }
.activity-changes code { min-width: 0; font: 11px/1.6 Consolas, ui-monospace, monospace; overflow-wrap: anywhere; }

/* 任务清单：✓ 已完成 / • 未完成，进行中的一步加重。 */
.activity-plan { list-style: none; margin: 0 0 7px; padding: 0; }
.activity-plan li { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
.activity-plan-mark { flex-shrink: 0; width: 12px; color: #8a8f98; }
.activity-plan li.done .activity-plan-mark { color: #21845a; }
.activity-plan li.done .activity-plan-text { color: var(--dash-text-secondary, #5f6368); }
.activity-plan li.current .activity-plan-text { color: var(--dash-text-primary, #30343b); font-weight: 500; }
.activity-plan-text { min-width: 0; overflow-wrap: anywhere; }

.compact { font-size: 11px; }
.compact .activity-entry { grid-template-columns: 13px minmax(0, 1fr); column-gap: 6px; }
.compact .activity-list::before { left: 6px; }
.compact .activity-node { width: 6px; height: 6px; margin-top: 7px; }
.compact .activity-tool > summary { flex-wrap: wrap; column-gap: 5px; }
.compact .activity-target { flex-basis: 60%; order: 5; margin-left: 19px; }
.compact .activity-chevron { order: 6; }
.compact .activity-tool-label { max-width: 55%; }
.compact .activity-commentary { font-size: 12px; }

@keyframes activity-rotate { to { transform: rotate(360deg); } }
.activity-spin { animation: activity-rotate 1.4s linear infinite; }
@media (prefers-reduced-motion: reduce) { .activity-spin { animation: none; } .activity-chevron { transition: none; } }
</style>
