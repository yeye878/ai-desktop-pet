/** 智能调度任务 store：按 run 管理快照、实时节点过程流与事件游标；快照+seq 补齐，不只靠实时广播。 */
import { defineStore } from "pinia";
import { reactive, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as api from "../services/orchestration";
import { createAgentActivity, type AgentActivityItem, type AgentActivityTool } from "../services/agentActivity";
import type {
  OrchestrationActivityEvent,
  OrchestrationEvent,
  OrchestrationMode,
  RunSnapshot,
} from "../services/orchestration";

export interface NodeActivityStream {
  invocationId: string;
  nodeId: string;
  roleKind: string;
  attempt: number;
  agentId: string;
  agentName: string;
  agentAvatar: string;
  backend: string;
  status: "running" | "completed" | "failed";
  error?: string;
  items: AgentActivityItem[];
  activity: ReturnType<typeof createAgentActivity>;
}

export interface AssessmentHandoff {
  difficulty: string;
  risk: string;
  recommendedAgentId: string;
  recommendedAgentName: string;
  recommendedAgentAvatar: string;
  recommendedBackend: string;
  recommendedReason: string;
  rationale: string[];
}

/** 剥离 worker 末尾附加的元数据 JSON 封套，避免把内部协议字段流式展示到思考过程里。 */
function stripTrailingWorkerEnvelope(text: string): string {
  const match = /\n*```(?:json|JSON)?\s*\n(\{[\s\S]*\})\s*```\s*$/.exec(text);
  if (!match) return text;
  try {
    const parsed = JSON.parse(match[1]) as Record<string, unknown>;
    const isEnvelope =
      parsed &&
      typeof parsed === "object" &&
      ("summary" in parsed || "artifacts" in parsed) &&
      ("evidence_notes" in parsed || "limitations" in parsed || "request_subtask" in parsed || "artifacts" in parsed);
    if (!isEnvelope) return text;
    return text.slice(0, match.index).trimEnd();
  } catch {
    return text;
  }
}

export const useOrchestrationStore = defineStore("orchestration", () => {
  const mode = ref<OrchestrationMode>("direct");
  const runs = reactive<Record<string, RunSnapshot>>({});
  const activeBySession = reactive<Record<string, string>>({});
  const streamsByRun = reactive<Record<string, NodeActivityStream[]>>({});
  const handoffByRun = reactive<Record<string, AssessmentHandoff>>({});
  const detailRunId = ref<string | null>(null);
  let started = false;
  let listeners: UnlistenFn[] = [];
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  const pendingRefresh = new Set<string>();

  async function loadMode() {
    try {
      mode.value = await api.getMode();
    } catch {
      mode.value = "direct";
    }
  }

  async function setMode(next: OrchestrationMode) {
    mode.value = await api.setMode(next);
  }

  async function refresh(runId: string) {
    const snapshot = await api.getRun(runId);
    runs[runId] = snapshot;
  }

  function scheduleRefresh(runId: string) {
    pendingRefresh.add(runId);
    if (refreshTimer) return;
    refreshTimer = setTimeout(async () => {
      refreshTimer = null;
      const ids = [...pendingRefresh];
      pendingRefresh.clear();
      for (const id of ids) {
        try {
          await refresh(id);
        } catch {
          /* run 可能已被删除 */
        }
      }
    }, 250);
  }

  function ensureStream(payload: OrchestrationActivityEvent): NodeActivityStream {
    if (!streamsByRun[payload.run_id]) streamsByRun[payload.run_id] = [];
    const list = streamsByRun[payload.run_id];
    let entry = list.find((item) => item.invocationId === payload.invocation_id);
    if (!entry) {
      const items = reactive<AgentActivityItem[]>([]);
      entry = reactive({
        invocationId: payload.invocation_id,
        nodeId: payload.node_id,
        roleKind: payload.role_kind,
        attempt: payload.attempt,
        agentId: payload.agent_id,
        agentName: payload.agent_name,
        agentAvatar: payload.agent_avatar,
        backend: payload.backend,
        status: "running",
        items,
        activity: createAgentActivity(items),
      }) as NodeActivityStream;
      list.push(entry);
    }
    return entry;
  }

  function applyActivityEvent(payload: OrchestrationActivityEvent) {
    if (!payload?.run_id || !payload?.invocation_id) return;
    const entry = ensureStream(payload);
    if (payload.kind === "started") {
      entry.status = "running";
      return;
    }
    if (payload.kind === "thinking" && payload.text) {
      entry.activity.appendThinking(payload.text);
      return;
    }
    if (payload.kind === "text" && payload.text) {
      entry.activity.appendText(payload.text);
      return;
    }
    if (payload.kind === "tool" && payload.tool) {
      entry.activity.upsertTool(payload.tool as AgentActivityTool);
      return;
    }
    if (payload.kind === "completed") {
      entry.status = "completed";
      const last = entry.items[entry.items.length - 1];
      if (last?.kind === "text") {
        const cleaned = stripTrailingWorkerEnvelope(last.text);
        if (!cleaned.trim()) entry.items.pop();
        else last.text = cleaned;
      }
      return;
    }
    if (payload.kind === "failed") {
      entry.status = "failed";
      entry.error = payload.text || undefined;
      for (const item of entry.items) {
        if (item.kind === "tool" && !["completed", "denied", "skipped", "failed", "interrupted", "notice"].includes(item.tool.status)) {
          item.tool.status = "interrupted";
        }
      }
    }
  }

  function applyLifecycleEvent(payload: OrchestrationEvent) {
    if (!payload?.run_id) return;
    if (payload.type === "assessment.completed") {
      const p = payload.payload ?? {};
      handoffByRun[payload.run_id] = {
        difficulty: String(p.difficulty ?? ""),
        risk: String(p.risk ?? ""),
        recommendedAgentId: String(p.recommended_agent_id ?? "main"),
        recommendedAgentName: String(p.recommended_agent_name ?? "主助理"),
        recommendedAgentAvatar: String(p.recommended_agent_avatar ?? "🐾"),
        recommendedBackend: String(p.recommended_backend ?? ""),
        recommendedReason: String(p.recommended_reason ?? ""),
        rationale: Array.isArray(p.rationale) ? p.rationale.map(String) : [],
      };
    }
  }

  /** 将一次智能调度 run 的多节点过程整理为可挂载在最终消息上的 AgentActivityItem[]。 */
  function buildRunActivity(runId: string, finalContent: string): AgentActivityItem[] {
    const list = streamsByRun[runId] ?? [];
    const handoff = handoffByRun[runId];
    const items: AgentActivityItem[] = [];
    const combined = createAgentActivity(items);
    if (handoff) {
      const reason = handoff.recommendedReason ? `（${handoff.recommendedReason}）` : "";
      combined.appendThinking(
        `[任务评估 ${handoff.difficulty}/${handoff.risk}] 交由「${handoff.recommendedAgentName}」执行${reason}\n`,
      );
    }
    for (const stream of list) {
      const roleLabel = api.ROLE_KIND_LABELS[stream.roleKind] ?? stream.roleKind;
      if (stream.roleKind === "work" || stream.roleKind === "integrate" || stream.roleKind === "review") {
        const backendLabel = api.BACKEND_LABELS[stream.backend] ?? stream.backend;
        if (stream.items.length) {
          combined.appendThinking(`\n[${roleLabel} · ${stream.agentName} (${backendLabel})]\n`);
        }
      }
      for (const item of stream.items) {
        if (item.kind === "thinking") combined.appendThinking(item.text);
        else if (item.kind === "text") combined.appendText(item.text);
        else if (item.kind === "tool") combined.upsertTool({ ...item.tool });
      }
    }
    if (!items.length) return [];
    return combined.finish(finalContent, "completed").activity;
  }

  /** 窗口重载/断线后：列表 + 快照 + seq 补齐。 */
  async function bindSession(sessionId: string) {
    if (!sessionId) return;
    try {
      const list = await api.listRuns(sessionId);
      if (list.length) {
        activeBySession[sessionId] = list[0].run_id;
        await refresh(list[0].run_id);
      } else {
        delete activeBySession[sessionId];
      }
    } catch {
      /* 无 Tauri 环境忽略 */
    }
  }

  function clearSessions() {
    Object.keys(activeBySession).forEach((key) => delete activeBySession[key]);
    Object.keys(runs).forEach((key) => delete runs[key]);
    Object.keys(streamsByRun).forEach((key) => delete streamsByRun[key]);
    Object.keys(handoffByRun).forEach((key) => delete handoffByRun[key]);
    detailRunId.value = null;
  }

  async function start() {
    if (started) return;
    started = true;
    await loadMode();
    listeners.push(
      await listen<{ session_id: string; task_id: string; run_id: string }>("orchestration-run-started", ({ payload }) => {
        activeBySession[payload.session_id] = payload.run_id;
        scheduleRefresh(payload.run_id);
      }),
    );
    listeners.push(
      await listen<OrchestrationEvent>("orchestration-event", ({ payload }) => {
        // 按 run_id + seq 去重补齐；快照刷新以数据库为当前事实。
        applyLifecycleEvent(payload);
        scheduleRefresh(payload.run_id);
      }),
    );
    listeners.push(
      await listen<OrchestrationActivityEvent>("orchestration-activity", ({ payload }) => {
        applyActivityEvent(payload);
      }),
    );
    listeners.push(
      await listen<{ run_id: string; session_id?: string }>("orchestration-run-finished", ({ payload }) => {
        if (payload.session_id && payload.run_id) {
          activeBySession[payload.session_id] = payload.run_id;
        }
        scheduleRefresh(payload.run_id);
      }),
    );
    listeners.push(
      await listen<OrchestrationMode>("orchestration-mode-changed", ({ payload }) => {
        mode.value = payload;
      }),
    );
  }

  function stop() {
    listeners.splice(0).forEach((unlisten) => unlisten());
    started = false;
  }

  function activeRun(sessionId: string): RunSnapshot | undefined {
    const id = activeBySession[sessionId];
    return id ? runs[id] : undefined;
  }

  function streams(runId: string): NodeActivityStream[] {
    return streamsByRun[runId] ?? [];
  }

  function handoff(runId: string): AssessmentHandoff | undefined {
    return handoffByRun[runId];
  }

  function isRunning(sessionId: string): boolean {
    const run = activeRun(sessionId);
    return !!run && !api.TERMINAL_STATES.includes(run.state);
  }

  async function cancel(runId: string) {
    const snapshot = runs[runId];
    await api.cancelRun(runId, snapshot?.state_revision ?? 0);
    await refresh(runId).catch(() => scheduleRefresh(runId));
  }

  async function resume(runId: string) {
    const snapshot = runs[runId];
    const result = await api.resumeRun(runId, snapshot?.state_revision ?? 0);
    activeBySession[snapshot?.session_id ?? ""] = result.run_id;
    scheduleRefresh(result.run_id);
    return result;
  }

  return {
    mode, runs, activeBySession, streamsByRun, handoffByRun, detailRunId,
    loadMode, setMode, refresh, bindSession, clearSessions, start, stop,
    activeRun, streams, handoff, buildRunActivity, isRunning, cancel, resume,
  };
});
