import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Agent {
  id: string;
  name: string;
  avatar: string;
  description: string;
  system_prompt: string;
  model: string;
  backend: "direct_api" | "claude_code" | "codex" | "dsh";
  api_profile_id: string;
  is_builtin: boolean;
  allowed_tools: string[];
  created_at: number;
  updated_at: number;
}

/** AI 生成智能体定义时返回的规格（前端预填表单供用户确认后保存） */
export interface AgentSpec {
  name: string;
  avatar: string;
  description: string;
  system_prompt: string;
  allowed_tools: string[];
}

export const useAgentsStore = defineStore("agents", () => {
  const agents = ref<Agent[]>([]);
  const loading = ref(false);
  let loadVersion = 0;
  let syncVersion = 0;
  let unlisten: UnlistenFn | null = null;

  async function load() {
    const version = ++loadVersion;
    loading.value = true;
    try {
      const result = await invoke<Agent[]>("list_agents");
      if (version === loadVersion) agents.value = result;
    } finally {
      if (version === loadVersion) loading.value = false;
    }
  }

  function stopSync() {
    ++syncVersion;
    unlisten?.();
    unlisten = null;
  }

  async function startSync() {
    stopSync();
    const version = syncVersion;
    const stop = await listen("agents-changed", () => {
      void load().catch((error) => console.error("同步智能体列表失败", error));
    });
    // A window may close while Tauri is registering the listener.
    if (version !== syncVersion) {
      stop();
      return;
    }
    unlisten = stop;
    // Subscribe before loading, so changes during initial loading are not lost.
    await load();
  }

  async function upsert(agent: Agent) {
    await invoke("save_agent", { agent });
    await load();
  }

  async function remove(id: string) {
    await invoke("delete_agent", { id });
    await load();
  }

  /** 让 AI 根据自然语言描述生成智能体定义 */
  async function generateSpec(description: string): Promise<AgentSpec> {
    return await invoke<AgentSpec>("generate_agent_spec", { description });
  }

  function findByName(name: string): Agent | undefined {
    return agents.value.find((agent) => agent.id === name || (agent.is_builtin
      ? agent.name.toLowerCase() === name.toLowerCase() : agent.name === name));
  }

  function mentionName(agent: Agent): string {
    return !agent.is_builtin && ["claude", "codex", "dsh"].includes(agent.name.toLowerCase()) ? agent.id : agent.name;
  }

  return { agents, loading, load, startSync, stopSync, upsert, remove, generateSpec, findByName, mentionName };
});
