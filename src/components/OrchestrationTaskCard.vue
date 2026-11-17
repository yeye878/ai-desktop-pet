<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useOrchestrationStore } from "../stores/orchestration";
import {
  BACKEND_LABELS,
  DIFFICULTY_LABELS,
  ROLE_KIND_LABELS,
  STATE_LABELS,
  TERMINAL_STATES,
} from "../services/orchestration";
import AgentActivity from "./AgentActivity.vue";
import AppIcon from "./AppIcon.vue";

const props = defineProps<{ sessionId: string }>();
const orchestration = useOrchestrationStore();
const actionBusy = ref(false);
const run = computed(() => orchestration.activeRun(props.sessionId));
const running = computed(() => !!run.value && !TERMINAL_STATES.includes(run.value.state));
const resumable = computed(() => !!run.value && ["blocked", "waiting_input", "interrupted"].includes(run.value.state));
const policy = computed(() => run.value?.policy ?? null);
const workNodes = computed(() => run.value?.nodes.filter((n) => n.kind === "work") ?? []);
const planNode = computed(() => run.value?.nodes.find((n) => n.kind === "plan"));
const integrateNode = computed(() => run.value?.nodes.find((n) => n.kind === "integrate"));
const reworkRound = computed(() =>
  workNodes.value.reduce((max, n) => Math.max(max, (n as { rework_round?: number }).rework_round ?? 0), 0),
);
const assessDone = computed(() => run.value?.nodes.find((n) => n.node_id === "assess")?.state === "completed");
const workDone = computed(() => workNodes.value.length > 0 && workNodes.value.every((n) => n.state === "completed"));
const reviewNode = computed(() => run.value?.nodes.find((n) => n.node_id === "review-final"));
const latestReview = computed(() => run.value?.reviews[run.value.reviews.length - 1]);
const blockingCount = computed(() => latestReview.value?.issues.filter((i) => i.severity !== "minor").length ?? 0);

const streams = computed(() => (run.value ? orchestration.streams(run.value.run_id) : []));
const handoff = computed(() => (run.value ? orchestration.handoff(run.value.run_id) : undefined));
const primaryAssignee = computed(() => {
  const node = integrateNode.value ?? workNodes.value[workNodes.value.length - 1];
  if (node?.assigned_agent_name) {
    return {
      id: node.assigned_agent_id || "",
      name: node.assigned_agent_name,
      avatar: node.assigned_agent_avatar || "🤖",
      backend: node.assigned_backend || "",
      reason: handoff.value?.recommendedReason || "",
    };
  }
  if (handoff.value) {
    return {
      id: handoff.value.recommendedAgentId,
      name: handoff.value.recommendedAgentName,
      avatar: handoff.value.recommendedAgentAvatar,
      backend: handoff.value.recommendedBackend,
      reason: handoff.value.recommendedReason,
    };
  }
  return null;
});

const userProcessToggle = ref<boolean | null>(null);
watch(() => run.value?.run_id, () => {
  userProcessToggle.value = null;
});
const showProcess = computed(() => {
  if (userProcessToggle.value !== null) return userProcessToggle.value;
  return running.value;
});

function toggleProcess() {
  userProcessToggle.value = !showProcess.value;
}

function agentIconName(agentId: string, fallbackAvatar: string): string {
  if (agentId === "main") return "brand-pet";
  if (agentId === "builtin-claude") return "claude";
  if (agentId === "builtin-codex") return "codex";
  if (agentId === "builtin-dsh") return "dsh";
  return fallbackAvatar || "🤖";
}

function backendLabel(backend: string): string {
  return BACKEND_LABELS[backend] || backend;
}

function roleLabel(roleKind: string): string {
  return ROLE_KIND_LABELS[roleKind] || roleKind;
}

function streamPlaceholder(roleKind: string): string {
  if (roleKind === "assess") return "正在评估任务难度、风险与匹配合适智能体…";
  if (roleKind === "plan") return "正在拆解子任务计划与分配执行者…";
  if (roleKind === "review" || roleKind === "review_action") return "正在独立核对产物与工具证据…";
  if (roleKind === "integrate") return "正在汇总并集成各执行者产物…";
  return "正在思考与执行任务…";
}

async function stop() {
  if (!run.value || actionBusy.value) return;
  actionBusy.value = true;
  try {
    await orchestration.cancel(run.value.run_id);
  } finally {
    actionBusy.value = false;
  }
}
async function resume() {
  if (!run.value || actionBusy.value) return;
  actionBusy.value = true;
  try {
    await orchestration.resume(run.value.run_id);
  } finally {
    actionBusy.value = false;
  }
}
function openDetail() {
  if (run.value) orchestration.detailRunId = run.value.run_id;
}
</script>

<template>
  <div v-if="run" :class="['orch-task-card', run.state]">
    <header class="orch-task-head">
      <strong>智能调度</strong>
      <span v-if="policy" class="orch-task-tag">{{ DIFFICULTY_LABELS[policy.difficulty] }}任务</span>
      <span v-if="policy" class="orch-task-tag risk">{{ policy.risk }}</span>
      <span v-if="policy?.force_review" class="orch-task-tag review">需要独立审核</span>
      <span v-if="primaryAssignee" class="orch-task-tag assignee">
        <AppIcon :name="agentIconName(primaryAssignee.id, primaryAssignee.avatar)" :size="11" />
        <span>{{ primaryAssignee.name }}</span>
      </span>
      <span class="orch-task-state">{{ STATE_LABELS[run.state] }}</span>
    </header>
    <div class="orch-task-phases">
      <span :class="{ done: assessDone }">{{ assessDone ? "✓" : "●" }} 任务评估</span>
      <span v-if="planNode" :class="{ done: planNode.state === 'completed' }">
        {{ planNode.state === "completed" ? "✓" : "●" }} 生成计划
      </span>
      <span :class="{ done: workDone }">{{ workDone ? "✓" : "●" }} 执行</span>
      <span v-if="integrateNode" :class="{ done: integrateNode.state === 'completed' }">
        {{ integrateNode.state === "completed" ? "✓" : "●" }} 集成
      </span>
      <span v-if="reviewNode" :class="{ done: reviewNode.state === 'completed' && latestReview?.verdict === 'pass' }">
        {{ latestReview?.verdict === "pass" ? "✓" : "●" }} 独立审核
      </span>
    </div>
    <p v-if="assessDone && primaryAssignee" class="orch-task-handoff">
      交由 <strong>{{ primaryAssignee.name }}</strong>
      <span v-if="primaryAssignee.backend" class="orch-task-backend">（{{ backendLabel(primaryAssignee.backend) }}）</span>
      执行<span v-if="primaryAssignee.reason"> · {{ primaryAssignee.reason }}</span>
    </p>
    <div v-if="showProcess && streams.length" class="orch-task-streams">
      <section v-for="stream in streams" :key="stream.invocationId" class="orch-stream-item">
        <header class="orch-stream-head">
          <AppIcon :name="agentIconName(stream.agentId, stream.agentAvatar)" :size="12" />
          <strong>{{ stream.agentName }}</strong>
          <span class="orch-stream-role">{{ roleLabel(stream.roleKind) }}</span>
          <span class="orch-stream-backend">{{ backendLabel(stream.backend) }}</span>
          <span :class="['orch-stream-state', stream.status]">
            {{ stream.status === "running" ? "进行中" : stream.status === "completed" ? "已完成" : "失败" }}
          </span>
        </header>
        <p v-if="stream.status === 'running' && !stream.items.length" class="orch-stream-placeholder">
          {{ streamPlaceholder(stream.roleKind) }}
        </p>
        <AgentActivity
          v-if="stream.items.length || stream.status === 'running'"
          :items="stream.items"
          :active="stream.status === 'running'"
          compact
          hide-stop
        />
        <p v-if="stream.error" class="orch-stream-error">{{ stream.error }}</p>
      </section>
    </div>
    <p v-if="reworkRound > 0 && running" class="orch-task-note">
      审核发现 {{ blockingCount }} 项必需修改，正在返工（{{ reworkRound }}/{{ policy?.max_rework ?? 2 }}）
    </p>
    <p v-if="run.status_message && !running && run.state !== 'succeeded'" class="orch-task-note">{{ run.status_message }}</p>
    <p v-if="run.state === 'succeeded'" class="orch-task-note ok">验收通过 · 交付版本 v{{ run.artifacts[run.artifacts.length - 1]?.revision ?? 1 }}</p>
    <footer class="orch-task-actions">
      <button v-if="streams.length" type="button" @click="toggleProcess">{{ showProcess ? "收起过程" : "过程记录" }}</button>
      <button type="button" @click="openDetail">查看详情</button>
      <button v-if="latestReview && latestReview.issues.length" type="button" @click="openDetail">查看问题</button>
      <button v-if="resumable" type="button" :disabled="actionBusy" @click="resume">恢复</button>
      <button v-if="running" type="button" class="stop" :disabled="actionBusy" @click="stop">停止</button>
    </footer>
  </div>
</template>

<style scoped>
.orch-task-card { border: 1px solid #d5d8de; border-radius: 8px; background: #fff; padding: 10px 12px; margin: 8px 0; font-size: 12px; color: #30343b; min-width: 0; }
.orch-task-card.succeeded { border-color: #21845a66; }
.orch-task-card.review_failed, .orch-task-card.failed { border-color: #c24c4c66; }
.orch-task-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.orch-task-tag { display: inline-flex; align-items: center; gap: 4px; font-size: 10px; border-radius: 3px; padding: 1px 6px; background: rgba(40, 127, 167, .12); color: #267a9c; }
.orch-task-tag.risk { background: rgba(165, 121, 36, .12); color: #a57924; }
.orch-task-tag.review { background: rgba(115, 123, 138, .14); color: #5f6368; }
.orch-task-tag.assignee { background: rgba(33, 132, 90, .12); color: #21845a; }
.orch-task-state { margin-left: auto; font-weight: 600; }
.orch-task-phases { display: flex; gap: 12px; flex-wrap: wrap; margin: 8px 0 4px; color: #8a8f98; }
.orch-task-phases .done { color: #21845a; }
.orch-task-handoff { margin: 4px 0 6px; color: #4a505a; font-size: 11.5px; line-height: 1.5; }
.orch-task-backend { color: #7a808a; }
.orch-task-streams { display: flex; flex-direction: column; gap: 8px; margin: 8px 0; padding-top: 8px; border-top: 1px dashed #e2e5eb; min-width: 0; }
.orch-stream-item { border: 1px solid #eceef2; border-radius: 6px; background: #fafbfc; padding: 7px 9px; min-width: 0; }
.orch-stream-head { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; margin-bottom: 5px; font-size: 11px; }
.orch-stream-role { padding: 0 5px; border-radius: 3px; background: rgba(40, 127, 167, .1); color: #267a9c; font-size: 10px; }
.orch-stream-backend { color: #8a8f98; font-size: 10px; }
.orch-stream-state { margin-left: auto; font-size: 10px; color: #287fa7; }
.orch-stream-state.completed { color: #21845a; }
.orch-stream-state.failed { color: #c24c4c; }
.orch-stream-placeholder { margin: 2px 0 4px; color: #7a808a; font-size: 11px; }
.orch-stream-error { margin: 4px 0 0; color: #c24c4c; font-size: 11px; }
.orch-task-note { margin: 4px 0; color: #5f6368; }
.orch-task-note.ok { color: #21845a; }
.orch-task-actions { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
.orch-task-actions button { border: 1px solid #d5d8de; background: transparent; border-radius: 4px; padding: 3px 10px; font-size: 11px; cursor: pointer; color: #30343b; }
.orch-task-actions button.stop { color: #c24c4c; border-color: #c24c4c66; }
</style>
