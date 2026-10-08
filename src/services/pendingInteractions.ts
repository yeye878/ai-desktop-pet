import { ref, type Ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ToolConfirmPayload {
  id: string;
  tool_name: string;
  arguments: string;
  summary?: string;
  command?: string | null;
  path?: string | null;
}
export interface AskUserPayload {
  id: string;
  question: string;
  options: { label: string; description?: string | null }[];
}
export interface PendingInteractions {
  tool_confirms: ToolConfirmPayload[];
  questions: AskUserPayload[];
}

type Kind = "tool" | "question";
type Request = ToolConfirmPayload | AskUserPayload;

// Subscribe first, then recover requests missed while a window was loading.
// A snapshot must not replace an event observed while its IPC was in flight.
export function subscribePendingInteraction<T extends Request>(
  kind: Kind,
  pending: Ref<T | null>,
  onResolved: (id: string) => void = () => {},
): () => void {
  const listeners: UnlistenFn[] = [];
  let stopped = false;
  let revision = 0;
  const receive = (payload: T) => { if (!stopped) { ++revision; pending.value = payload; } };
  const event = kind === "tool" ? "ai-tool-confirm" : "ai-ask-user";
  void (async () => {
    for (const register of [
      () => listen<T>(event, ({ payload }) => receive(payload)),
      () => listen<{ id: string }>(`${event}-resolved`, ({ payload }) => {
        if (stopped) return;
        ++revision;
        if (pending.value?.id === payload.id) { pending.value = null; onResolved(payload.id); }
      }),
      ...(kind === "tool" ? [() => listen<T>("tool-confirm-payload", ({ payload }) => receive(payload))] : []),
    ]) {
      const unlisten = await register();
      if (stopped) { unlisten(); return; }
      listeners.push(unlisten);
    }
    const before = revision;
    const snapshot = await invoke<PendingInteractions>("get_pending_interactions");
    if (stopped || revision !== before) return;
    const items = kind === "tool" ? snapshot?.tool_confirms : snapshot?.questions;
    pending.value = (items?.find((item) => item.id === pending.value?.id) || items?.[0] || null) as T | null;
  })().catch((error) => console.error("加载待处理请求失败", error));
  return () => { stopped = true; listeners.splice(0).forEach((unlisten) => unlisten()); };
}

export function createInteractionSubmission<T extends { id: string }>(
  pending: Ref<T | null>,
  send: (id: string, value: string | boolean) => Promise<unknown>,
) {
  const submittingId = ref<string | null>(null);
  const error = ref("");
  async function submit(value: string | boolean): Promise<string | null> {
    const id = pending.value?.id;
    if (!id || submittingId.value === id) return null;
    submittingId.value = id;
    error.value = "";
    try {
      await send(id, value);
      if (pending.value?.id === id) pending.value = null;
      return id;
    } catch (reason) {
      if (pending.value?.id === id) error.value = String(reason);
      return null;
    } finally {
      if (submittingId.value === id) submittingId.value = null;
    }
  }
  return { submittingId, error, submit };
}
