<script setup lang="ts">
import { computed } from "vue";
import { Check, ChevronRight, Circle, CircleAlert, CircleX, Clock, FileText, LoaderCircle, Square, Terminal } from "@lucide/vue";
import { formatArguments, legacyActivity, toolStatusLabel, type AgentActivityItem, type AgentActivityTool } from "../services/agentActivity";
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
function statusIcon(status: string) {
  if (status === "completed") return Check;
  if (status === "failed") return CircleAlert;
  if (["denied", "interrupted"].includes(status)) return CircleX;
  if (["running", "requested", "approved"].includes(status)) return LoaderCircle;
  if (status === "waiting") return Clock;
  return Circle;
}
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
function activityToolLabel(name: string) {
  const aliases: Record<string, string> = { Read: "读取文件", Write: "写入文件", Edit: "修改文件", Bash: "执行命令", Glob: "查找文件", Grep: "搜索代码", WebSearch: "网页搜索", WebFetch: "读取网页", edit_file: "修改文件", code_search: "搜索代码", ask_user: "询问用户", create_docx: "生成文档" };
  return aliases[name] || toolLabel(name);
}
</script>

<template>
  <section v-if="entries.length || active" :class="['agent-activity', { compact }]" aria-label="工作过程">
    <div v-if="active" class="activity-status" role="status" aria-live="polite">
      <Clock v-if="isWaiting" :size="14" />
      <LoaderCircle v-else :size="14" class="activity-spin" />
      <span>{{ status }}</span>
      <span v-if="tools.length" class="activity-count">{{ completed }}/{{ tools.length }}</span>
      <button type="button" class="activity-stop" title="停止生成" aria-label="停止生成" @click.stop="emit('stop')"><Square :size="12" fill="currentColor" /></button>
    </div>
    <ol class="activity-list">
      <li v-for="(entry, index) in entries" :key="entry.kind === 'tool' ? entry.tool.id : `entry-${index}`" :class="['activity-entry', entry.kind]">
        <div v-if="entry.kind === 'text'" class="activity-commentary md-body" v-html="commentaryHtml(entry.text)"></div>
        <details v-else-if="entry.kind === 'thinking'" class="activity-reasoning">
          <summary><ChevronRight :size="12" class="activity-chevron" /><span>思考过程</span></summary>
          <p>{{ entry.text }}</p>
        </details>
        <details v-else :class="['activity-tool', entry.tool.status]">
          <summary :title="entry.tool.summary || entry.tool.tool_name">
            <component :is="statusIcon(entry.tool.status)" :size="14" :class="['activity-tool-icon', { 'activity-spin': active && ['running', 'requested', 'approved'].includes(entry.tool.status) }]" />
            <span class="activity-tool-label">{{ activityToolLabel(entry.tool.tool_name) }}</span>
            <code v-if="entry.tool.command || entry.tool.path" class="activity-target">{{ entry.tool.command || entry.tool.path }}</code>
            <span v-else-if="entry.tool.summary && entry.tool.summary !== entry.tool.tool_name" class="activity-target">{{ entry.tool.summary }}</span>
            <span class="activity-tool-state">{{ entry.tool.tool_name === 'ask_user' && entry.tool.status === 'waiting' ? '等待回答' : toolStatusLabel(entry.tool.status) }}</span>
            <ChevronRight :size="12" class="activity-chevron" />
          </summary>
          <div class="activity-detail">
            <p v-if="entry.tool.summary" class="activity-summary">{{ entry.tool.summary }}</p>
            <div v-if="entry.tool.command" class="activity-field"><Terminal :size="13" /><pre>{{ entry.tool.command }}</pre></div>
            <div v-if="entry.tool.path" class="activity-field"><FileText :size="13" /><pre>{{ entry.tool.path }}</pre></div>
            <details v-if="entry.tool.arguments" class="activity-arguments"><summary>参数</summary><pre>{{ formatArguments(entry.tool.arguments) }}</pre></details>
            <div v-if="entry.tool.output" class="activity-output"><span>输出</span><pre>{{ entry.tool.output }}</pre></div>
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
.activity-stop { margin-left: auto; width: 28px; height: 28px; display: grid; place-items: center; border: 1px solid #d5d8de; background: transparent; color: inherit; border-radius: 4px; cursor: pointer; flex: 0 0 28px; }
.activity-stop:hover { background: #eceef1; }
.activity-list { list-style: none; padding: 0; margin: 0; }
.activity-entry { min-width: 0; padding: 4px 0; }
.activity-commentary { margin: 0; white-space: normal; overflow-wrap: anywhere; color: var(--dash-text-primary, #30343b); font-size: 13px; }
.activity-commentary .md-p:last-child { margin-bottom: 0; }
.activity-commentary .md-list { margin: 3px 0 5px; }
.activity-quote { border-left: 2px solid #c4c9d1; padding-left: 9px; margin: 5px 0; color: var(--dash-text-secondary, #5f6368); }
.activity-mention { color: #267a9c; font-weight: 500; }
summary { cursor: pointer; list-style: none; user-select: none; }
summary::-webkit-details-marker { display: none; }
summary:focus-visible, button:focus-visible { outline: 2px solid #4b91bf; outline-offset: 2px; }
.activity-reasoning > summary { display: flex; align-items: center; gap: 5px; width: fit-content; }
.activity-reasoning p { margin: 7px 0 4px 7px; padding-left: 10px; border-left: 2px solid #dfe2e6; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 220px; overflow-y: auto; }
.activity-tool > summary { display: flex; align-items: center; gap: 7px; min-height: 30px; min-width: 0; border-radius: 4px; padding: 2px 4px; }
.activity-tool > summary:hover { background: rgba(100, 110, 128, .08); }
.activity-tool-icon { flex-shrink: 0; color: #767b84; }
.completed .activity-tool-icon { color: #21845a; }
.failed .activity-tool-icon, .denied .activity-tool-icon { color: #c24c4c; }
.running .activity-tool-icon { color: #287fa7; }
.waiting .activity-tool-icon { color: #a57924; }
.activity-tool-label { min-width: 0; max-width: 45%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--dash-text-primary, #30343b); font-weight: 500; }
.activity-target { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11px; font-family: Consolas, ui-monospace, monospace; }
.activity-tool-state { flex-shrink: 0; font-size: 10px; margin-left: auto; }
.activity-chevron { flex-shrink: 0; transition: transform .15s; }
details[open] > summary > .activity-chevron { transform: rotate(90deg); }
.activity-detail { margin: 3px 0 6px 10px; padding: 6px 0 6px 15px; border-left: 1px solid #dfe2e6; min-width: 0; }
.activity-summary { margin: 0 0 7px; overflow-wrap: anywhere; }
.activity-field { display: flex; align-items: flex-start; gap: 6px; margin-bottom: 7px; min-width: 0; }
.activity-field svg { flex-shrink: 0; margin-top: 3px; }
pre { font: 11px/1.6 Consolas, ui-monospace, monospace; white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word; margin: 5px 0 0; max-height: 280px; overflow: auto; color: var(--dash-text-primary, #30343b); user-select: text; }
.activity-field pre { margin: 0; min-width: 0; }
.activity-output { margin-top: 8px; }
.activity-output > span, .activity-arguments > summary { font-size: 11px; color: #747985; }
.activity-output pre, .activity-arguments pre { background: rgba(115, 123, 138, .06); padding: 8px; border-radius: 4px; }
.compact { font-size: 11px; }
.compact .activity-tool > summary { flex-wrap: wrap; column-gap: 5px; }
.compact .activity-target { flex-basis: 60%; order: 5; margin-left: 19px; }
.compact .activity-commentary { font-size: 12px; }
@keyframes activity-rotate { to { transform: rotate(360deg); } }
.activity-spin { animation: activity-rotate 1.4s linear infinite; }
@media (prefers-reduced-motion: reduce) { .activity-spin { animation: none; } .activity-chevron { transition: none; } }
</style>
