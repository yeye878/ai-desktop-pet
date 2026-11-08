import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 控制台里的对话窗口槽位：主对话 + 三个内置 CLI agent 窗口。 */
export type ChatSlot = "main" | "claude" | "codex" | "dsh";
export type ChatSessionKind = "main" | "agent";

export interface ChatSession {
  id: string;
  title: string;
  kind: ChatSessionKind;
  slot: ChatSlot;
  agent_id: string;
  agent_name: string;
  agent_avatar: string;
  model: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  preview: string;
}

export interface CreateSessionOptions {
  agentId?: string;
  model?: string;
  title?: string;
}

export interface BindSessionPatch {
  agentId?: string;
  agentName?: string;
  agentAvatar?: string;
  model?: string;
}

/** 三个 agent 窗口的默认内置角色 id（与后端 builtin_agents 对齐）。 */
export const SLOT_DEFAULT_AGENT: Record<Exclude<ChatSlot, "main">, string> = {
  claude: "builtin-claude",
  codex: "builtin-codex",
  dsh: "builtin-dsh",
};

export const AGENT_SLOTS: Array<Exclude<ChatSlot, "main">> = ["claude", "codex", "dsh"];

function sortSessions(items: ChatSession[]): ChatSession[] {
  return [...items].sort((left, right) => right.updated_at - left.updated_at);
}

/**
 * 会话列表 store。主对话与三个 agent 窗口共用；后端 chat_sessions 是唯一事实来源。
 * 任何窗口的新建/改名/删除都会通过 chat-sessions-changed 事件同步到这里。
 */
export const useSessionsStore = defineStore("sessions", () => {
  const sessions = ref<ChatSession[]>([]);
  const activeMainId = ref("");
  const loading = ref(false);
  let loadVersion = 0;
  let unlisteners: UnlistenFn[] = [];
  let reloadTimer: ReturnType<typeof setTimeout> | null = null;

  function bySlot(slot: ChatSlot): ChatSession[] {
    return sortSessions(sessions.value.filter((session) => session.slot === slot));
  }

  function find(id: string | null | undefined): ChatSession | undefined {
    if (!id) return undefined;
    return sessions.value.find((session) => session.id === id);
  }

  function activeMain(): ChatSession | undefined {
    return find(activeMainId.value) ?? bySlot("main")[0];
  }

  function upsert(session: ChatSession) {
    const index = sessions.value.findIndex((item) => item.id === session.id);
    if (index >= 0) sessions.value[index] = { ...sessions.value[index], ...session };
    else sessions.value.push(session);
  }

  async function load(): Promise<void> {
    const version = ++loadVersion;
    loading.value = true;
    try {
      const [list, active] = await Promise.all([
        invoke<ChatSession[]>("list_chat_sessions", {}),
        invoke<ChatSession | null>("get_active_chat_session").catch(() => null),
      ]);
      if (version !== loadVersion) return;
      sessions.value = sortSessions(Array.isArray(list) ? list : []);
      const activeId = active?.id && sessions.value.some((session) => session.id === active.id)
        ? active.id
        : (activeMainId.value && sessions.value.some((session) => session.id === activeMainId.value)
          ? activeMainId.value
          : bySlot("main")[0]?.id ?? "");
      activeMainId.value = activeId;
    } finally {
      if (version === loadVersion) loading.value = false;
    }
  }

  async function create(slot: ChatSlot, options: CreateSessionOptions = {}): Promise<ChatSession> {
    const session = await invoke<ChatSession>("create_chat_session", {
      slot,
      agentId: options.agentId ?? "",
      model: options.model ?? "",
      title: options.title ?? "",
    });
    upsert(session);
    if (session.slot === "main") activeMainId.value = session.id;
    return session;
  }

  async function setActiveMain(id: string): Promise<ChatSession> {
    const session = await invoke<ChatSession>("set_active_chat_session", { id });
    activeMainId.value = session.id;
    upsert(session);
    return session;
  }

  async function rename(id: string, title: string): Promise<ChatSession> {
    const session = await invoke<ChatSession>("rename_chat_session", { id, title });
    upsert(session);
    return session;
  }

  async function remove(id: string): Promise<void> {
    const result = await invoke<{ active_main_session_id?: string }>("delete_chat_session", { id });
    sessions.value = sessions.value.filter((session) => session.id !== id);
    if (result?.active_main_session_id) activeMainId.value = result.active_main_session_id;
  }

  async function bind(id: string, patch: BindSessionPatch): Promise<ChatSession> {
    const session = await invoke<ChatSession>("set_chat_session_binding", {
      id,
      agentId: patch.agentId,
      agentName: patch.agentName,
      agentAvatar: patch.agentAvatar,
      model: patch.model,
    });
    upsert(session);
    return session;
  }

  /** 首个用户消息落库后后端会改标题/预览，这里只是刷新列表，不阻塞发送。 */
  function scheduleReload(delay = 180) {
    if (reloadTimer !== null) clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => {
      reloadTimer = null;
      void load().catch((error) => console.error("刷新会话列表失败", error));
    }, delay);
  }

  async function startSync(): Promise<void> {
    stopSync();
    const stopList = await listen("chat-sessions-changed", () => scheduleReload());
    const stopActive = await listen<{ session?: ChatSession }>("chat-session-changed", ({ payload }) => {
      if (payload?.session) {
        upsert(payload.session);
        if (payload.session.slot === "main") activeMainId.value = payload.session.id;
      }
      scheduleReload();
    });
    const stopCleared = await listen("chat-history-cleared", () => scheduleReload(0));
    unlisteners.push(stopList, stopActive, stopCleared);
    await load();
  }

  function stopSync(): void {
    unlisteners.splice(0).forEach((unlisten) => unlisten());
    if (reloadTimer !== null) {
      clearTimeout(reloadTimer);
      reloadTimer = null;
    }
  }

  return {
    sessions,
    activeMainId,
    loading,
    load,
    bySlot,
    find,
    activeMain,
    create,
    setActiveMain,
    rename,
    remove,
    bind,
    scheduleReload,
    startSync,
    stopSync,
  };
});