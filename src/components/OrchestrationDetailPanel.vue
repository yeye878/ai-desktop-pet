<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useOrchestrationStore } from "../stores/orchestration";
import { BACKEND_LABELS, getArtifact, getEvents, STATE_LABELS, type OrchestrationEvent } from "../services/orchestration";
import AgentActivity from "./AgentActivity.vue";
import ReviewReportPanel from "./ReviewReportPanel.vue";

const orchestration = useOrchestrationStore();
const run = computed(() => (orchestration.detailRunId ? orchestration.runs[orchestration.detailRunId] : undefined));
const streams = computed(() => (run.value ? orchestration.streams(run.value.run_id) : []));
const events = ref<OrchestrationEvent[]>([]);
const artifactContents = ref<Record<string, { id: string; content: string }[]>>({});
const showEvents = ref(false);

watch(() => orchestration.detailRunId, async (id) => {
  events.value = [];
  artifactContents.value = {};
  showEvents.value = false;
  if (!id) return;
  await orchestration.refresh(id).catch(() => {});
  events.value = await getEvents(id, 0).catch(() => []);
});

watch(() => run.value?.latest_seq, async (seq) => {
  const id = orchestration.detailRunId;
  if (!id || !seq) return;
  const last = events.value[events.value.length - 1]?.seq ?? 0;
  if (seq > last) {
    const more = await getEvents(id, last).catch(() => []);
    if (more.length) events.value.push(...more);
  }
});

async function loadArtifact(setId: string, revision: number) {
  const key = `${setId}@${revision}`;
  if (artifactContents.value[key]) return;
  const result = await getArtifact(setId, revision).catch(() => null);
  if (result) artifactContents.value[key] = result.items;
}
function close() {
  orchestration.detailRunId = null;
}
const nodeStateLabel: Record<string, string> = {
  pending: "待启动", running: "进行中", completed: "完成", failed: "失败", blocked: "受阻", cancelled: "取消", skipped: "跳过",
};
</script>

<template>
  <Teleport to="body">
    <div v-if="run" class="orch-detail-mask" @click.self="close">
      <section class="orch-detail" role="dialog" aria-label="智能调度任务详情">
        <header>
          <h3>任务详情 · {{ STATE_LABELS[run.state] }}</h3>
          <button type="button" @click="close">关闭</button>
        </header>
        <p class="orch-detail-request">{{ run.original_request }}</p>
        <p v-if="run.policy" class="orch-detail-policy">
          分级依据：{{ run.policy.rule_hits.join("；") || "无额外规则命中" }}
          · 调用 {{ run.usage_stats.agent_invocations ?? 0 }} 次 · 工具 {{ run.usage_stats.tool_calls ?? 0 }} 次
        </p>
        <h4>节点</h4>
        <ul class="orch-detail-nodes">
          <li v-for="node in run.nodes" :key="node.node_id">
            <code>{{ node.node_id }}</code>
            <span class="node-kind">{{ node.kind }}</span>
            <span v-if="node.assigned_agent_name" class="node-agent">
              {{ node.assigned_agent_name }}<template v-if="node.assigned_backend"> · {{ BACKEND_LABELS[node.assigned_backend] ?? node.assigned_backend }}</template>
            </span>
            <span :class="['node-state', node.state]">{{ nodeStateLabel[node.state ?? "pending"] ?? node.state }}</span>
            <span class="node-objective">{{ node.objective }}</span>
          </li>
        </ul>
        <template v-if="streams.length">
          <h4>执行过程</h4>
          <div v-for="stream in streams" :key="stream.invocationId" class="orch-detail-stream">
            <div class="orch-detail-stream-head">
              <code>{{ stream.nodeId }}</code>
              <span>{{ stream.agentName }}</span>
              <span class="node-backend">{{ BACKEND_LABELS[stream.backend] ?? stream.backend }}</span>
            </div>
            <AgentActivity :items="stream.items" :active="stream.status === 'running'" compact hide-stop />
          </div>
        </template>
        <h4>审核</h4>
        <p v-if="!run.reviews.length" class="orch-detail-empty">该任务无需独立审核或尚未审核。</p>
        <ReviewReportPanel v-for="(report, index) in run.reviews" :key="index" :report="report" />
        <h4>交付物</h4>
        <div v-for="artifact in run.artifacts" :key="`${artifact.artifact_set_id}@${artifact.revision}`" class="orch-detail-artifact">
          <button type="button" @click="loadArtifact(artifact.artifact_set_id, artifact.revision)">
            v{{ artifact.revision }} · {{ artifact.content_hash.slice(0, 8) }}
          </button>
          <pre v-for="item in artifactContents[`${artifact.artifact_set_id}@${artifact.revision}`] ?? []" :key="item.id">{{ item.content }}</pre>
        </div>
        <h4>
          事件
          <button type="button" class="orch-detail-toggle" @click="showEvents = !showEvents">{{ showEvents ? "收起" : "展开" }}</button>
        </h4>
        <ol v-if="showEvents" class="orch-detail-events">
          <li v-for="event in events" :key="event.seq">
            <code>#{{ event.seq }}</code> {{ event.type }}
            <span v-if="event.node_id">· {{ event.node_id }}</span>
          </li>
        </ol>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.orch-detail-mask { position: fixed; inset: 0; background: rgba(20, 22, 26, .35); display: grid; place-items: center; z-index: 90; }
.orch-detail { width: min(680px, 92vw); max-height: 84vh; overflow-y: auto; background: #fff; border-radius: 10px; padding: 14px 16px; font-size: 12px; color: #30343b; }
.orch-detail header { display: flex; align-items: center; justify-content: space-between; }
.orch-detail h3 { margin: 0; font-size: 14px; }
.orch-detail h4 { margin: 12px 0 4px; font-size: 12px; }
.orch-detail header button, .orch-detail-toggle { border: 1px solid #d5d8de; background: transparent; border-radius: 4px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
.orch-detail-request { color: #5f6368; white-space: pre-wrap; }
.orch-detail-policy { color: #8a8f98; }
.orch-detail-nodes { list-style: none; margin: 0; padding: 0; }
.orch-detail-nodes li { display: flex; gap: 6px; align-items: baseline; margin: 4px 0; flex-wrap: wrap; }
.node-kind { color: #267a9c; }
.node-agent { font-size: 10px; border-radius: 3px; padding: 0 5px; background: rgba(33, 132, 90, .12); color: #21845a; }
.node-backend { font-size: 10px; color: #8a8f98; }
.orch-detail-stream { margin: 6px 0; padding: 6px 8px; border: 1px solid #eceef2; border-radius: 6px; background: #fafbfc; }
.orch-detail-stream-head { display: flex; gap: 6px; align-items: center; margin-bottom: 4px; font-size: 11px; }
.node-state { font-size: 10px; border-radius: 3px; padding: 0 5px; background: rgba(115, 123, 138, .14); color: #5f6368; }
.node-state.completed { background: rgba(33, 132, 90, .12); color: #21845a; }
.node-state.failed, .node-state.blocked { background: rgba(194, 76, 76, .12); color: #c24c4c; }
.node-objective { color: #5f6368; }
.orch-detail-empty { color: #8a8f98; }
.orch-detail-artifact button { border: 1px solid #d5d8de; background: transparent; border-radius: 4px; padding: 2px 8px; font-size: 11px; cursor: pointer; margin: 4px 0; }
.orch-detail-artifact pre { background: rgba(115, 123, 138, .06); border-radius: 4px; padding: 8px; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 240px; overflow-y: auto; }
.orch-detail-events { margin: 4px 0; padding-left: 18px; color: #5f6368; }
</style>