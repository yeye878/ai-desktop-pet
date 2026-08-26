import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface Agent {
  id: string;
  name: string;
  avatar: string;
  description: string;
  system_prompt: string;
  model: string;
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

  async function load() {
    loading.value = true;
    try {
      agents.value = await invoke<Agent[]>("list_agents");
    } finally {
      loading.value = false;
    }
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
    return agents.value.find((agent) => agent.name === name);
  }

  return { agents, loading, load, upsert, remove, generateSpec, findByName };
});
