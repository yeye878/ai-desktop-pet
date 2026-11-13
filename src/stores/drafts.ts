import { defineStore } from "pinia";
import { ref } from "vue";

/** 草稿最多保留的会话数，避免长期使用后无限堆积。 */
const MAX_DRAFTS = 50;

/**
 * 输入框草稿仓库。
 *
 * 控制台页面在切换到「智能体 / 记忆库 / 设置」等其它页面时会被 v-if 销毁重建，
 * 组件里的 ref 随之丢失。草稿放到 store 里，页面切回来时按会话恢复未发送的内容。
 */
export const useDraftStore = defineStore("drafts", () => {
  const drafts = ref<Record<string, string>>({});

  function get(key: string): string {
    if (!key) return "";
    return drafts.value[key] ?? "";
  }

  function set(key: string, value: string): void {
    if (!key) return;
    if (value) {
      drafts.value[key] = value;
    } else {
      delete drafts.value[key];
    }
    const keys = Object.keys(drafts.value);
    if (keys.length > MAX_DRAFTS) {
      for (const stale of keys.slice(0, keys.length - MAX_DRAFTS)) delete drafts.value[stale];
    }
  }

  function remove(key: string): void {
    if (key) delete drafts.value[key];
  }

  return { get, set, remove };
});
