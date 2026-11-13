import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  CHAT_BACKGROUNDS,
  CHAT_BG_CHANGED_EVENT,
  CHAT_BG_CUSTOM_STORAGE_KEY,
  CHAT_BG_INTENSITY_STORAGE_KEY,
  CHAT_BG_STORAGE_KEY,
  chatBackgroundPreset,
  normalizeChatBgIntensity,
  normalizeChatBackgroundId,
  readChatBackground,
  readChatBgIntensity,
  readCustomChatBackground,
  type ChatBackgroundId,
} from "../services/chatBackgrounds";

/**
 * 对话背景的响应式视图状态。
 *
 * 背景设置存在 localStorage，多个组件都要用（对话页、多窗格、浮动聊天窗、外观页）。
 * 以前每个组件各写一份 ref + storage 监听，多窗格（SessionPane）干脆忘了接，
 * 于是「同屏多窗口」下背景不生效。这里收敛成一个 composable：
 *   - 只读 localStorage 一次，之后靠事件同步
 *   - 跨 webview 靠 storage 事件，同窗口靠 CHAT_BG_CHANGED_EVENT
 *   - 组件卸载时自动解绑，不会泄漏监听
 */
export function useChatBackground() {
  const chatBg = ref<ChatBackgroundId>(readChatBackground());
  const customBgImage = ref(readCustomChatBackground());
  const chatBgIntensity = ref(readChatBgIntensity());

  const preset = computed(() => chatBackgroundPreset(chatBg.value));
  const hasBackground = computed(() => chatBg.value !== "none");
  const hasCustomImage = computed(() => Boolean(customBgImage.value));
  const presets = computed(() => CHAT_BACKGROUNDS);

  function syncFromStorage() {
    chatBg.value = readChatBackground();
    customBgImage.value = readCustomChatBackground();
    chatBgIntensity.value = readChatBgIntensity();
  }

  function handleStorage(event: StorageEvent) {
    if (event.key === CHAT_BG_STORAGE_KEY) {
      chatBg.value = normalizeChatBackgroundId(event.newValue);
    } else if (event.key === CHAT_BG_CUSTOM_STORAGE_KEY) {
      customBgImage.value = event.newValue || "";
    } else if (event.key === CHAT_BG_INTENSITY_STORAGE_KEY) {
      chatBgIntensity.value = normalizeChatBgIntensity(event.newValue);
    }
  }

  function handleLocalChange() {
    syncFromStorage();
  }

  onMounted(() => {
    syncFromStorage();
    window.addEventListener("storage", handleStorage);
    window.addEventListener(CHAT_BG_CHANGED_EVENT, handleLocalChange);
  });

  onBeforeUnmount(() => {
    window.removeEventListener("storage", handleStorage);
    window.removeEventListener(CHAT_BG_CHANGED_EVENT, handleLocalChange);
  });

  return {
    chatBg,
    customBgImage,
    chatBgIntensity,
    preset,
    presets,
    hasBackground,
    hasCustomImage,
  };
}
