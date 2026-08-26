import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// exit 后保留条目的宽限期：后端 spawn_waiter 在 exit 时会从 procs 移除，
// 前端保留这 N 毫秒让 UI 展示 exitCode，之后从本地 store + logs 同时清理，避免僵尸堆积。
const EXITED_PRUNE_DELAY_MS = 5_000;

export interface ProcessInfo {
  id: string;
  label: string;
  command: string;
  args: string[];
  pid: number | null;
  startedAt: number;
  status: string;
  exitCode: number | null;
  bufferTruncated: boolean;
}

export interface SpawnParams {
  label: string;
  command: string;
  args: string[];
  cwd?: string | null;
  env?: Record<string, string>;
  appendNewline?: boolean;
}

export interface StdoutEvent { id: string; line: string }
export interface StderrEvent { id: string; line: string }
export interface ExitEvent { id: string; code: number | null; signal: number | null; label: string }

export const useSubprocessStore = defineStore("subprocess", () => {
  const processes = ref<ProcessInfo[]>([]);
  const logs = ref<Record<string, { line: string; stderr: boolean }[]>>({});
  const loading = ref(false);

  let unlistenStdout: UnlistenFn | null = null;
  let unlistenStderr: UnlistenFn | null = null;
  let unlistenExit: UnlistenFn | null = null;

  async function ensureListeners() {
    if (unlistenStdout) return;
    unlistenStdout = await listen<StdoutEvent>("subproc-stdout", (e) => {
      const { id, line } = e.payload;
      (logs.value[id] ||= []).push({ line, stderr: false });
    });
    unlistenStderr = await listen<StderrEvent>("subproc-stderr", (e) => {
      const { id, line } = e.payload;
      (logs.value[id] ||= []).push({ line, stderr: true });
    });
    unlistenExit = await listen<ExitEvent>("subproc-exit", (e) => {
      const { id, code } = e.payload;
      const p = processes.value.find((x) => x.id === id);
      if (p) {
        p.status = "exited";
        p.exitCode = code;
      }
      // 后端已 remove(&id)，前端保留 EXITED_PRUNE_DELAY_MS 给 UI 展示结果再清理，
      // 避免 processes 数组无限堆积僵尸、logs 内存也跟着泄漏。
      setTimeout(() => {
        const idx = processes.value.findIndex((x) => x.id === id);
        if (idx !== -1 && processes.value[idx].status === "exited") {
          processes.value.splice(idx, 1);
        }
        delete logs.value[id];
      }, EXITED_PRUNE_DELAY_MS);
    });
  }

  async function refresh() {
    await ensureListeners();
    loading.value = true;
    try {
      processes.value = await invoke<ProcessInfo[]>("list_subprocesses");
    } finally {
      loading.value = false;
    }
  }

  async function spawn(params: SpawnParams): Promise<ProcessInfo> {
    await ensureListeners();
    const info = await invoke<ProcessInfo>("spawn_subprocess", { params });
    processes.value.push(info);
    logs.value[info.id] = [];
    return info;
  }

  async function send(id: string, text: string, appendNewline = true) {
    await invoke("send_stdin", { id, text, appendNewline });
  }

  async function kill(id: string, force = false) {
    const process = processes.value.find((item) => item.id === id);
    if (process && process.status !== "running") return;
    if (process) process.status = "stopping";
    try {
      await invoke("kill_subprocess", { id, force });
    } catch (error) {
      if (process) {
        const msg = error instanceof Error ? error.message : String(error);
        // 后端返回"进程 X 不存在"意味着进程已自行退出/kill 时已被 spawn_waiter 移除，
        // 此时回滚到 running 会留下永远发不出 send_stdin 的幽灵；标记为 exited。
        if (msg.includes("不存在")) {
          process.status = "exited";
        } else {
          process.status = "running";
        }
      }
      throw error;
    }
  }

  async function clearLogs(id: string) {
    delete logs.value[id];
  }

  function dispose() {
    unlistenStdout?.();
    unlistenStderr?.();
    unlistenExit?.();
    unlistenStdout = unlistenStderr = unlistenExit = null;
  }

  return {
    processes,
    logs,
    loading,
    refresh,
    spawn,
    send,
    kill,
    clearLogs,
    ensureListeners,
    dispose,
  };
});
