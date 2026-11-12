<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from "vue";
import {
  effortLabel,
  effortOptionsFor,
  fetchAgentModelCatalog,
  modelOptionFor,
  providerLabel,
  type AgentModelCatalog,
  type AgentModelOption,
  type AgentModelSlot,
} from "../services/agentModels";
import EffortSlider from "./EffortSlider.vue";

const props = withDefaults(
  defineProps<{
    slot: AgentModelSlot;
    /** 当前会话/窗口偏好里的模型 id；空串 = 跟随本机。 */
    model: string;
    /** 当前会话/窗口偏好里的渠道 provider id；空串 = 跟随本机渠道。 */
    provider: string;
    /** 当前会话/窗口偏好里的思考强度 id；空串 = 跟随本机默认。 */
    effort: string;
    disabled?: boolean;
    embedded?: boolean;
  }>(),
  { provider: "", disabled: false, embedded: false },
);

const emit = defineEmits<{
  select: [payload: { model: string; provider: string; effort: string }];
}>();

const rootRef = ref<HTMLDivElement | null>(null);
const popupRef = ref<HTMLDivElement | null>(null);
const popupStyle = ref<Record<string, string>>({});
const catalog = ref<AgentModelCatalog | null>(null);
const loading = ref(false);
const loadError = ref("");
const open = ref(false);
const customDraft = ref("");
const searchQuery = ref("");
/** 弹层里高亮的模型，以 (provider, id) 为键；下方展示它的思考强度。 */
const activeProviderId = ref("");
const activeModelId = ref("");

const currentModel = computed(() => (props.model || "").trim());
const currentProvider = computed(() => (props.provider || "").trim());
const currentEffort = computed(() => (props.effort || "").trim());

const modelLabel = computed(() => {
  const id = currentModel.value;
  if (!id) return "跟随本机";
  const option = modelOptionFor(catalog.value, id, currentProvider.value);
  return option?.name || id;
});

const channelLabel = computed(() => providerLabel(catalog.value, currentProvider.value));
/**
 * 显式选中了渠道（provider 非空）时把渠道名拼到 pill 上，保证一眼看出渠道；
 * 「跟随本机」和 claude/codex（无 provider）不拼，避免冗余。
 */
const showChannel = computed(() => !!currentModel.value && !!currentProvider.value && !!channelLabel.value);

const modelTitle = computed(() => {
  if (!currentModel.value) return "跟随本机默认模型与渠道";
  return currentProvider.value ? currentProvider.value + " / " + currentModel.value : currentModel.value;
});

const effortText = computed(() => (currentEffort.value ? effortLabel(currentEffort.value) : "跟随本机"));

const localHint = computed(() => {
  const value = catalog.value;
  const label = value?.local_model_label || value?.local_model || "";
  if (label) return "本机默认：" + label;
  return value?.error ? "目录不可用，可手输模型 ID" : "使用本机配置文件里的默认渠道与模型";
});

const totalModels = computed(() => catalog.value?.models.length ?? 0);
const searchVisible = computed(() => totalModels.value > 12);
const keyword = computed(() => searchQuery.value.trim().toLowerCase());

const activeOption = computed<AgentModelOption | undefined>(() =>
  modelOptionFor(catalog.value, activeModelId.value, activeProviderId.value),
);
const activeEfforts = computed(() => effortOptionsFor(catalog.value, activeModelId.value, activeProviderId.value));
const activeEffortHint = computed(() => {
  if (!activeModelId.value) return "";
  if (!activeOption.value) return "目录里没有这个模型，可直接输入 id 使用";
  if (!activeEfforts.value.length) return "该模型不支持选择思考强度";
  return "";
});

function matchesKeyword(model: AgentModelOption): boolean {
  if (!keyword.value) return true;
  const text = [model.name, model.id, model.provider || "", model.provider_label || ""].join(" ").toLowerCase();
  return text.includes(keyword.value);
}

/** dsh 按 provider 分组（组标题带渠道名与模型数）；codex/claude 没有 provider，单组不显示标题。 */
const groups = computed<Array<{ key: string; label: string; models: AgentModelOption[] }>>(() => {
  const result: Array<{ key: string; label: string; models: AgentModelOption[] }> = [];
  for (const model of catalog.value?.models ?? []) {
    if (!matchesKeyword(model)) continue;
    const key = (model.provider || "").trim();
    let group = result.find((item) => item.key === key);
    if (!group) {
      group = { key, label: (model.provider_label || model.provider || "").trim(), models: [] };
      result.push(group);
    }
    group.models.push(model);
  }
  return result;
});

const visibleModels = computed<AgentModelOption[]>(() => groups.value.flatMap((group) => group.models));

const emptyHint = computed(() => {
  if (keyword.value) return "没有匹配的模型，换个关键词试试。";
  if (loading.value) return "正在读取本机模型目录…";
  return "没有探测到可选模型，可直接输入模型 ID。";
});

const isLocalSelected = computed(() => !currentModel.value && !currentProvider.value);

function modelRowKey(model: AgentModelOption): string {
  return (model.provider || "") + ":" + model.id;
}

function isHighlighted(model: AgentModelOption): boolean {
  return (model.provider || "") === activeProviderId.value && model.id === activeModelId.value;
}

function isSelected(model: AgentModelOption): boolean {
  return (model.provider || "") === currentProvider.value && model.id === currentModel.value;
}

/** 新的 (provider, model) 是否支持当前 effort；不支持时回退 ""（跟随本机默认）。 */
function compatibleEffort(provider: string, modelId: string, effort: string): string {
  const id = (modelId || "").trim();
  const wanted = (effort || "").trim();
  if (!id || !wanted) return "";
  const options = effortOptionsFor(catalog.value, id, provider);
  if (!options.length) {
    // 目录外的自定义模型无法判断能力：保留用户已有的 effort 选择。
    return modelOptionFor(catalog.value, id, provider) ? "" : wanted;
  }
  return options.some((option) => option.id === wanted) ? wanted : "";
}

function close(): void {
  if (props.embedded) return;
  open.value = false;
  customDraft.value = "";
  searchQuery.value = "";
}

/** 并发/换窗口时丢弃过期响应，避免把上一个 slot 的目录画到当前窗口。 */
let loadToken = 0;

async function loadCatalog(force: boolean): Promise<void> {
  const slot = props.slot;
  const token = ++loadToken;
  loading.value = true;
  try {
    const next = await fetchAgentModelCatalog(slot, force);
    if (token !== loadToken) return;
    if (next.error && next.models.length === 0 && catalog.value?.models.length) {
      // 刷新失败时保留上一次可用目录，只提示错误。
      loadError.value = next.error;
    } else {
      catalog.value = next;
      loadError.value = next.error;
    }
  } catch (error) {
    if (token !== loadToken) return;
    loadError.value = "模型目录加载失败：" + (error instanceof Error ? error.message : String(error));
  } finally {
    if (token === loadToken) loading.value = false;
  }
}

function updatePopupPosition(): void {
  if (props.embedded || !open.value || !rootRef.value) return;
  const anchor = rootRef.value.getBoundingClientRect();
  const width = Math.min(370, window.innerWidth - 24);
  const height = Math.min(popupRef.value?.offsetHeight || 560, window.innerHeight - 24);
  popupStyle.value = {
    width: `${width}px`,
    left: `${Math.max(12, Math.min(anchor.right - width, window.innerWidth - width - 12))}px`,
    top: `${Math.max(12, Math.min(anchor.bottom + 8, window.innerHeight - height - 12))}px`,
    maxHeight: `${Math.min(560, window.innerHeight - 24)}px`,
  };
}

function handleWindowScroll(event: Event): void {
  if (!(event.target instanceof Node) || !popupRef.value?.contains(event.target)) updatePopupPosition();
}

function openPopup(): void {
  if (props.disabled || props.embedded) return;
  open.value = true;
  activeProviderId.value = currentProvider.value;
  activeModelId.value = currentModel.value;
  searchQuery.value = "";
  void loadCatalog(false);
}

function togglePopup(): void {
  if (props.disabled || props.embedded) return;
  if (open.value) close();
  else openPopup();
}

function selectModel(model: AgentModelOption): void {
  if (props.disabled) return;
  const provider = (model.provider || "").trim();
  activeProviderId.value = provider;
  activeModelId.value = model.id;
  emit("select", {
    model: model.id,
    provider,
    effort: compatibleEffort(provider, model.id, currentEffort.value),
  });
}

function selectFollowLocal(): void {
  if (props.disabled) return;
  activeProviderId.value = "";
  activeModelId.value = "";
  emit("select", { model: "", provider: "", effort: "" });
  close();
}

function selectEffort(effortId: string): void {
  if (props.disabled) return;
  const model = currentModel.value || (effortId ? catalog.value?.local_model || "" : "");
  const provider = currentProvider.value || (model && !currentModel.value ? catalog.value?.local_provider || "" : "");
  emit("select", {
    model,
    provider,
    effort: compatibleEffort(provider, model, effortId),
  });
}

function submitCustom(): void {
  if (props.disabled) return;
  const id = customDraft.value.trim();
  if (!id) return;
  const provider = currentProvider.value;
  activeProviderId.value = provider;
  activeModelId.value = id;
  emit("select", { model: id, provider, effort: compatibleEffort(provider, id, currentEffort.value) });
  close();
}

const customPlaceholder = computed(() =>
  currentProvider.value ? "自定义模型 ID（" + currentProvider.value + "）…" : "自定义模型 ID…（Enter 提交）",
);

function refresh(): void {
  if (props.disabled || loading.value) return;
  void loadCatalog(true);
}

function moveActive(delta: number): void {
  if (props.disabled) return;
  const models = visibleModels.value;
  const keys = ["", ...models.map(modelRowKey)];
  const currentKey = activeModelId.value ? activeProviderId.value + ":" + activeModelId.value : "";
  const index = keys.indexOf(currentKey);
  const next = index < 0 ? 0 : (index + delta + keys.length) % keys.length;
  const key = keys[next];
  if (!key) {
    activeProviderId.value = "";
    activeModelId.value = "";
    return;
  }
  const separator = key.indexOf(":");
  activeProviderId.value = key.slice(0, separator);
  activeModelId.value = key.slice(separator + 1);
}

function selectHighlighted(): void {
  if (props.disabled) return;
  if (!activeModelId.value) {
    selectFollowLocal();
    return;
  }
  const model = visibleModels.value.find(
    (item) => (item.provider || "") === activeProviderId.value && item.id === activeModelId.value,
  ) ?? modelOptionFor(catalog.value, activeModelId.value, activeProviderId.value);
  if (model) selectModel(model);
}

function handleInputEscape(event: KeyboardEvent, fromSearch = false): void {
  if (props.embedded || props.disabled) return;
  event.stopPropagation();
  event.preventDefault();
  if (fromSearch && searchQuery.value) searchQuery.value = "";
  else close();
}

function handleDocumentPointerDown(event: PointerEvent): void {
  if (props.embedded || !open.value) return;
  const root = rootRef.value;
  const target = event.target as Node | null;
  if (target && (root?.contains(target) || popupRef.value?.contains(target))) return;
  close();
}

function handleWindowKeydown(event: KeyboardEvent): void {
  if (props.embedded || props.disabled || !open.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    close();
    return;
  }
  const target = event.target as HTMLElement | null;
  const tag = target?.tagName;
  if (target?.closest(".effort-slider") || tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    moveActive(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    selectHighlighted();
  }
}

watch(
  [currentModel, currentProvider, () => props.embedded],
  ([model, provider]) => {
    if (props.embedded || open.value) {
      activeModelId.value = model;
      activeProviderId.value = provider;
    }
  },
  { immediate: true },
);
watch(
  () => props.disabled,
  (value) => {
    if (value) close();
  },
);
watch(
  () => props.slot,
  () => {
    loadToken += 1;
    loading.value = false;
    open.value = false;
    customDraft.value = "";
    searchQuery.value = "";
    activeModelId.value = currentModel.value;
    activeProviderId.value = currentProvider.value;
    catalog.value = null;
    loadError.value = "";
    void loadCatalog(false);
  },
);

watch([open, activeEfforts, groups, loadError], async () => {
  if (!props.embedded && open.value) {
    await nextTick();
    updatePopupPosition();
  }
}, { flush: "post" });

onMounted(() => {
  watch(() => props.embedded, (embedded, _, onCleanup) => {
    if (embedded) return;
    window.addEventListener("resize", updatePopupPosition);
    window.addEventListener("scroll", handleWindowScroll, true);
    document.addEventListener("pointerdown", handleDocumentPointerDown, true);
    window.addEventListener("keydown", handleWindowKeydown);
    onCleanup(() => {
      window.removeEventListener("resize", updatePopupPosition);
      window.removeEventListener("scroll", handleWindowScroll, true);
      document.removeEventListener("pointerdown", handleDocumentPointerDown, true);
      window.removeEventListener("keydown", handleWindowKeydown);
    });
  }, { immediate: true });
  // 窗口挂载即预取目录（service 自带缓存）：重启后 pill 直接显示展示名而不是原始 id。
  void loadCatalog(false);
});
</script>

<template>
  <div ref="rootRef" class="dash-agent-model-picker" :class="{ 'is-embedded': embedded }">
    <button
      v-if="!embedded"
      type="button"
      class="dash-agent-field dash-agent-model-trigger"
      :class="{ 'is-open': open }"
      :disabled="disabled"
      :aria-expanded="open"
      aria-haspopup="dialog"
      :title="modelTitle"
      @click="togglePopup"
    >
      <span class="dash-agent-field-label">模型</span>
      <span class="dash-agent-model-trigger-text">
        <span class="dash-agent-model-value">
          {{ modelLabel }}
        </span>
        <span class="dash-agent-model-sub"><span v-if="showChannel" class="dash-agent-model-channel">{{ channelLabel }} · </span>思考 {{ effortText }}</span>
      </span>
      <span v-if="loading && !catalog && open" class="dash-agent-model-spinner" aria-hidden="true" />
      <span v-else class="dash-agent-model-caret" aria-hidden="true">
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
      </span>
    </button>

    <Teleport to="body" :disabled="embedded">
    <div
      v-if="embedded || open"
      ref="popupRef"
      class="dash-agent-model-pop"
      :class="{ 'is-embedded': embedded }"
      :style="embedded ? undefined : popupStyle"
      :role="embedded ? 'group' : 'dialog'"
      aria-label="模型与思考强度"
    >
      <div class="dash-agent-model-pop-head">
        <span class="dash-agent-model-pop-title">
          模型<span v-if="totalModels" class="dash-agent-model-pop-count">{{ totalModels }}</span>
        </span>
        <button
          type="button"
          class="dash-agent-model-refresh"
          :disabled="disabled || loading"
          title="重新探测本机模型目录"
          @click="refresh"
        >
          <span v-if="loading">…</span>
          <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 0 0-15.4-6.4L3 8" /><path d="M3 3v5h5" /><path d="M3 12a9 9 0 0 0 15.4 6.4L21 16" /><path d="M16 16h5v5" /></svg>
        </button>
      </div>

      <input
        v-if="searchVisible"
        v-model="searchQuery"
        class="dash-agent-model-search"
        placeholder="搜索模型 / 渠道…"
        :disabled="disabled"
        spellcheck="false"
        @keydown.esc="handleInputEscape($event, true)"
      />

      <div v-if="loadError" class="dash-agent-model-error">{{ loadError }}</div>

      <div class="dash-agent-model-list" role="listbox" aria-label="可选模型">
        <button
          type="button"
          :class="['dash-agent-model-row', 'dash-agent-model-row-local', { active: isLocalSelected }]"
          :disabled="disabled"
          @click="selectFollowLocal"
        >
          <span class="dash-agent-model-row-main">
            <span class="dash-agent-model-row-name">跟随本机</span>
            <span class="dash-agent-model-row-desc">{{ localHint }}</span>
          </span>
          <span v-if="isLocalSelected" class="dash-agent-model-check" aria-label="已选中">✓</span>
        </button>

        <template v-for="group in groups" :key="group.key || 'default'">
          <div v-if="group.label" class="dash-agent-model-group-label">
            {{ group.label }} · {{ group.models.length }}
          </div>
          <button
            v-for="model in group.models"
            :key="modelRowKey(model)"
            type="button"
            :class="['dash-agent-model-row', { active: isHighlighted(model) }]"
            :disabled="disabled"
            @click="selectModel(model)"
          >
            <span class="dash-agent-model-row-main">
              <span class="dash-agent-model-row-name">{{ model.name }}</span>
              <span class="dash-agent-model-row-id">{{ model.id }}</span>
              <span v-if="model.description" class="dash-agent-model-row-desc">{{ model.description }}</span>
            </span>
            <span v-if="isSelected(model)" class="dash-agent-model-check" aria-label="已选中">✓</span>
          </button>
        </template>

        <div v-if="!groups.length" class="dash-agent-model-empty">{{ emptyHint }}</div>
      </div>

      <div v-if="activeEfforts.length" class="dash-agent-model-efforts">
        <EffortSlider
          :options="activeEfforts"
          :model-value="currentEffort"
          :disabled="disabled"
          @change="selectEffort"
        />
      </div>
      <div v-else-if="activeEffortHint" class="dash-agent-model-efforts-note">{{ activeEffortHint }}</div>

      <div class="dash-agent-model-custom">
        <input
          v-model="customDraft"
          class="dash-agent-model-custom-input"
          :placeholder="customPlaceholder"
          :disabled="disabled"
          spellcheck="false"
          @keydown.enter.stop.prevent="submitCustom"
          @keydown.esc="handleInputEscape"
        />
        <button
          type="button"
          class="dash-agent-model-custom-btn"
          :disabled="disabled || !customDraft.trim()"
          @click="submitCustom"
        >
          使用
        </button>
      </div>
    </div>
    </Teleport>
  </div>
</template>

<style scoped>
.dash-agent-model-picker.is-embedded {
  position: static;
  display: block;
  width: 100%;
  min-width: 0;
  max-width: 100%;
}
.dash-agent-model-pop.is-embedded {
  position: static;
  inset: auto;
  z-index: auto;
  box-sizing: border-box;
  width: 100%;
  min-width: 0;
  max-width: 100%;
  max-height: none;
  overflow: visible;
  overflow-wrap: anywhere;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: none;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  box-shadow: none;
  animation: none;
}
.dash-agent-model-pop.is-embedded > * {
  box-sizing: border-box;
  min-width: 0;
  flex-shrink: 0;
}
.dash-agent-model-pop.is-embedded .dash-agent-model-list {
  flex: none;
  max-height: min(360px, 45vh);
  overscroll-behavior: auto;
}
.dash-agent-model-pop.is-embedded .dash-agent-model-row { box-sizing: border-box; }
.dash-agent-model-pop.is-embedded .dash-agent-model-row-id,
.dash-agent-model-pop.is-embedded .dash-agent-model-row-desc { white-space: normal; }
.dash-agent-model-pop.is-embedded .dash-agent-model-efforts { margin-inline: 0; }
.dash-agent-model-row:disabled,
.dash-agent-model-search:disabled,
.dash-agent-model-custom-input:disabled { opacity: 0.6; cursor: default; }
</style>
