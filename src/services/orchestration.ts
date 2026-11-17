/** 智能调度 IPC 类型与命令封装（契约同步 src-tauri/src/orchestration/contracts.rs）。 */
import { invoke } from "@tauri-apps/api/core";

export type Difficulty = "L0" | "L1" | "L2" | "L3";
export type Risk = "R0" | "R1" | "R2" | "R3";
export type OrchestrationMode = "direct" | "collaborative" | "adaptive";

export type RunState =
  | "queued" | "assessing" | "planning" | "executing" | "integrating"
  | "validating" | "reviewing" | "rework_required" | "waiting_input" | "blocked"
  | "cancelling" | "cancelled" | "succeeded" | "failed" | "review_failed"
  | "interrupted";

export const TERMINAL_STATES: RunState[] = ["succeeded", "failed", "review_failed", "cancelled"];

export interface AcceptanceCriterion {
  id: string;
  requirement: string;
  required: boolean;
  verification: "structural" | "command" | "source_check" | "model_review" | "user";
  evidence_requirements: string[];
}

export interface EffectivePolicy {
  difficulty: Difficulty;
  risk: Risk;
  force_review: boolean;
  pre_review_external_actions: boolean;
  max_concurrency: number;
  max_rework: number;
  max_transient_retries: number;
  confidence_threshold: number;
  rule_hits: string[];
  policy_version: number;
}

export interface ReviewCheck {
  criterion_id: string;
  status: "pass" | "fail" | "unverified";
  evidence_refs: string[];
  explanation: string;
}

export interface ReviewIssue {
  id: string;
  severity: "blocker" | "major" | "minor";
  criterion_id: string;
  artifact_ref: string;
  responsible_node_id: string;
  description: string;
  required_fix: string;
  verification: string;
}

export interface ReviewReport {
  schema_version: number;
  task_revision: number;
  artifact_set_id: string;
  artifact_hash: string;
  verdict: "pass" | "rework" | "blocked";
  checks: ReviewCheck[];
  issues: ReviewIssue[];
}

export interface TaskNodeSpec {
  node_id: string;
  kind: string;
  dependencies: string[];
  assigned_agent_id?: string | null;
  assigned_agent_name?: string | null;
  assigned_agent_avatar?: string | null;
  assigned_backend?: string | null;
  objective: string;
  criterion_ids: string[];
  read_scopes: string[];
  write_scopes: string[];
  [key: string]: unknown;
}

export interface OrchestrationActivityTool {
  id: string;
  tool_name: string;
  status: string;
  summary?: string | null;
  arguments?: string | null;
  command?: string | null;
  path?: string | null;
  output?: string | null;
  approved?: boolean | null;
  exit_code?: number | null;
  duration_ms?: number | null;
  changes?: { path: string; kind?: string | null }[] | null;
}

export interface OrchestrationActivityEvent {
  session_id: string;
  run_id: string;
  node_id: string;
  role_kind: string;
  invocation_id: string;
  attempt: number;
  agent_id: string;
  agent_name: string;
  agent_avatar: string;
  backend: string;
  kind: "started" | "thinking" | "text" | "tool" | "completed" | "failed";
  text?: string | null;
  tool?: OrchestrationActivityTool | null;
}

export interface RunSnapshot {
  task_id: string;
  session_id: string;
  original_request: string;
  task_revision: number;
  run_id: string;
  state: RunState;
  state_revision: number;
  policy: EffectivePolicy | null;
  usage_stats: { agent_invocations?: number; tool_calls?: number };
  final_artifact_set_id: string | null;
  status_message: string;
  nodes: (TaskNodeSpec & { state?: string })[];
  artifacts: { artifact_set_id: string; revision: number; content_hash: string; known_limitations?: string[] }[];
  reviews: ReviewReport[];
  latest_seq: number;
}

export interface OrchestrationEvent {
  event_id: string;
  session_id: string;
  task_id: string;
  run_id: string;
  node_id: string;
  invocation_id: string;
  attempt: number;
  seq: number;
  state_revision: number;
  type: string;
  payload: Record<string, unknown>;
  created_at: number;
}

export interface RunListItem {
  run_id: string;
  task_id: string;
  state: RunState;
  state_revision: number;
  status_message: string;
  created_at: number;
  updated_at: number;
  request: string;
}

export const startOrchestratedTask = (request: string, idempotencyKey: string, sessionId?: string) =>
  invoke<{ task_id: string; run_id: string; session_id: string }>("start_orchestrated_task", { request, idempotencyKey, sessionId });

export const getRun = (runId: string) => invoke<RunSnapshot>("get_orchestrated_run", { runId });

export const listRuns = (sessionId: string) => invoke<RunListItem[]>("list_orchestrated_runs", { sessionId });

export const getEvents = (runId: string, afterSeq: number) =>
  invoke<OrchestrationEvent[]>("get_orchestration_events", { runId, afterSeq });

export const cancelRun = (runId: string, expectedRevision: number) =>
  invoke<boolean>("cancel_orchestrated_run", { runId, expectedRevision });

export const resumeRun = (runId: string, expectedRevision: number) =>
  invoke<{ task_id: string; run_id: string; previous_run_id: string }>("resume_orchestrated_run", { runId, expectedRevision });

export const amendTask = (taskId: string, expectedRevision: number, change: string) =>
  invoke<{ task_revision: number }>("amend_orchestrated_task", { taskId, expectedRevision, change });

export const getArtifact = (artifactSetId: string, revision: number) =>
  invoke<{ manifest: unknown; items: { id: string; content: string }[] }>("get_orchestration_artifact", { artifactSetId, revision });

export const getMode = () => invoke<OrchestrationMode>("get_orchestration_mode");

export const setMode = (mode: OrchestrationMode) => invoke<OrchestrationMode>("set_orchestration_mode", { mode });

export const STATE_LABELS: Record<RunState, string> = {
  queued: "排队中",
  assessing: "任务评估",
  planning: "生成计划",
  executing: "执行中",
  integrating: "集成中",
  validating: "必要验证",
  reviewing: "独立审核",
  rework_required: "返工中",
  waiting_input: "等待输入",
  blocked: "受阻",
  cancelling: "停止中",
  cancelled: "已取消",
  succeeded: "验收通过",
  failed: "失败",
  review_failed: "审核未通过",
  interrupted: "中断待核对",
};

export const DIFFICULTY_LABELS: Record<Difficulty, string> = {
  L0: "简单",
  L1: "普通",
  L2: "困难",
  L3: "复杂",
};

export const BACKEND_LABELS: Record<string, string> = {
  direct_api: "直连 API",
  claude_code: "Claude Code",
  codex: "Codex",
  dsh: "DeepSeek Harness",
};

export const ROLE_KIND_LABELS: Record<string, string> = {
  assess: "任务评估",
  plan: "生成计划",
  work: "执行",
  integrate: "产物集成",
  review: "独立审核",
  review_action: "动作前审",
  execute_action: "执行动作",
  verify_result: "结果核对",
};