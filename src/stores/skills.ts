import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface Skill {
  id: string;
  name: string;
  description: string;
  system_prompt: string;
  allowed_tools: string[];
  /** 保留字段兼容旧数据；当前不再用于激活判定，激活完全由用户手动控制。 */
  keywords: string[];
  is_active: boolean;
  created_at: number;
  updated_at: number;
}

export const useSkillsStore = defineStore("skills", () => {
  const skills = ref<Skill[]>([]);
  const customPrompt = ref<string>("");
  const loading = ref(false);

  const activeSkill = computed(
    () => skills.value.find((s) => s.is_active) || null
  );

  async function load() {
    loading.value = true;
    try {
      skills.value = await invoke<Skill[]>("list_skills");
      customPrompt.value = await invoke<string>("get_custom_system_prompt");
    } finally {
      loading.value = false;
    }
  }

  async function saveCustomPrompt(text: string) {
    await invoke("set_custom_system_prompt", { text });
    customPrompt.value = text;
  }

  async function upsert(skill: Skill) {
    await invoke("save_skill", { skill });
    await load();
  }

  async function remove(id: string) {
    await invoke("delete_skill", { id });
    await load();
  }

  async function setActive(id: string | null) {
    await invoke("set_active_skill", { id });
    await load();
  }

  return {
    skills,
    customPrompt,
    loading,
    activeSkill,
    load,
    saveCustomPrompt,
    upsert,
    remove,
    setActive,
  };
});
