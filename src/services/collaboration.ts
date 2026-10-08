import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AiFinishedPayload, MessageAgent, useChatStore } from "../stores/chat";

export interface CollaborationState {
  run_id: string;
  status: "running" | "completed" | "aborted" | "limited" | "failed";
  active_agent: MessageAgent | null;
  queued: MessageAgent[];
  turn: number;
  max_turns: number;
  message: string;
}
export interface CollaborationReply extends AiFinishedPayload {
  run_id: string;
  turn_id: string;
  failed: boolean;
}

/** Register immediately; the returned cleanup also handles closing mid-registration. */
export function subscribeCollaboration(
  chat: ReturnType<typeof useChatStore>,
  onReply: (reply: CollaborationReply) => void,
  onState: (state: CollaborationState, newTurn: boolean) => void,
): () => void {
  let stopped = false;
  const listeners: UnlistenFn[] = [];
  let observedEvent = false;
  const stateReceived = (state: CollaborationState) => {
    const newTurn = chat.collaboration?.run_id !== state.run_id || chat.collaboration?.turn !== state.turn;
    if (chat.receiveCollaborationState(state)) onState(state, newTurn);
  };
  void (async () => {
    for (const register of [
      () => listen<CollaborationState>("collaboration-state", ({ payload }) => {
        if (stopped) return;
        observedEvent = true;
        stateReceived(payload);
      }),
      () => listen<CollaborationReply>("collaboration-reply", ({ payload }) => {
        if (!stopped && chat.receiveCollaborationReply(payload)) onReply(payload);
      }),
    ]) {
      const unlisten = await register();
      if (stopped) { unlisten(); return; }
      listeners.push(unlisten);
    }
    const current = await invoke<CollaborationState | null>("get_collaboration_state");
    if (!stopped && !observedEvent && current) stateReceived(current);
  })().catch((error) => console.error("加载协同状态失败", error));
  return () => { stopped = true; listeners.splice(0).forEach((unlisten) => unlisten()); };
}
