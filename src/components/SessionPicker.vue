<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Check, ChevronDown, CornerDownLeft, Search, X } from "@lucide/vue";
import AppIcon from "./AppIcon.vue";
import { useSessionRuntimeStore } from "../stores/sessionRuntime";
import type { ChatSession } from "../stores/sessions";
import {
  displayTitle,
  duplicateSuffixes,
  filterSessions,
  groupSessions,
  sessionMeta,
} from "../services/sessionLabels";

/**
 * 自定义会话选择器：多窗格标题栏与空窗格共用的会话下拉，替代原生 <select>。
 * 原生 select 只能显示 title || agent_name，未命名会话在三个 agent 槽位里都只剩
 * claude/codex/dsh，完全无法区分；这里用 Teleport 弹层展示分组、消息数、相对时间、
 * 运行状态与内容预览，并支持搜索、键盘操作与完整的无障碍属性。
 */
const props = withDefaults(
  defineProps<{
    sessions: ChatSession[];
    /** 当前会话 id；空串表示该窗格还没有绑定会话。 */
    modelValue: string;
    disabled?: boolean;
    variant?: "header" | "empty";
    placeholder?: string;
  }>(),
  { disabled: false, variant: "header", placeholder: "选择会话" },
);

const emit = defineEmits<{ select: [sessionId: string] }>();

const runtime = useSessionRuntimeStore();

/** 每个组件实例一套独立的 DOM id 前缀，多窗格同时打开时 aria 引用不会互相串。 */
const uid = "session-picker-" + Math.random().toString(36).slice(2, 8);
const panelId = uid + "-panel";
const listboxId = uid + "-listbox";
const headingId = uid + "-heading";

const triggerRef = ref<HTMLButtonElement | null>(null);
const panelRef = ref<HTMLDivElement | null>(null);
const searchRef = ref<HTMLInputElement | null>(null);
const open = ref(false);
const query = ref("");
/** 弹层里高亮的会话 id；键盘上下移动与鼠标悬停都会更新它。 */
const activeId = ref("");
const placement = ref<"down" | "up">("down");
const panelStyle = ref<Record<string, string>>({});

const current = computed<ChatSession | undefined>(() =>
  props.sessions.find((session) => session.id === props.modelValue),
);
const currentTitle = computed(() => (current.value ? displayTitle(current.value) : props.placeholder));
const suffixes = computed(() => duplicateSuffixes(props.sessions));
const filtered = computed(() => filterSessions(props.sessions, query.value));
const groups = computed(() => groupSessions(filtered.value));
const visibleIds = computed(() =>
  groups.value.flatMap((group) => group.sessions.map((session) => session.id)),
);
const hasQuery = computed(() => query.value.trim() !== "");
const countLabel = computed(() =>
  hasQuery.value ? visibleIds.value.length + " / " + props.sessions.length : String(props.sessions.length),
);
const activeDescendantId = computed(() => (activeId.value ? optionId(activeId.value) : undefined));

/** 行 id 由会话 id 派生；session id 不是合法的 DOM id 字符时统一换成连字符。 */
function optionId(sessionId: string): string {
  return uid + "-option-" + (sessionId.replace(/[^A-Za-z0-9_-]/g, "-") || "unknown");
}

/** 头像规则与 SessionPane 一致：主对话用桌宠，内置 agent 用品牌图标，其余用自定义头像。 */
function avatarIcon(session: ChatSession | undefined): string {
  if (!session || session.slot === "main") return "brand-pet";
  if (session.agent_id === "builtin-claude" || session.slot === "claude") return "claude";
  if (session.agent_id === "builtin-codex" || session.slot === "codex") return "codex";
  if (session.agent_id === "builtin-dsh" || session.slot === "dsh") return "dsh";
  return session.agent_avatar || "🤖";
}

/** 运行状态直接读同窗格共用的运行时 store；会话还没加载过时视为空闲。 */
function isBusy(sessionId: string): boolean {
  return !!runtime.peek(sessionId)?.turn.loading;
}

/** 第二行信息按 sessionMeta 的中文分隔符拆开，方便把「运行中」单独渲染成暖色小标。 */
function metaParts(session: ChatSession): string[] {
  const text = sessionMeta(session, { busy: isBusy(session.id), suffix: suffixes.value[session.id] });
  return text ? text.split(" · ") : [];
}

/**
 * 面板 fixed 定位：相对触发器，右侧溢出左移、下方空间不足向上展开。
 * 每次都按面板实测高度重新计算，避免切换搜索结果的多少时贴边或遮挡触发器。
 */
function updatePosition(): void {
  const trigger = triggerRef.value;
  if (!open.value || !trigger) return;
  const anchor = trigger.getBoundingClientRect();
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const margin = 8;
  const gap = 6;
  const width = Math.min(340, viewportWidth - margin * 2);
  const maxHeight = Math.min(360, viewportHeight - 24);
  const measured = panelRef.value?.offsetHeight ?? 0;
  const height = Math.min(measured || maxHeight, maxHeight);
  let left = Math.min(anchor.left, viewportWidth - width - margin);
  left = Math.max(margin, left);
  const spaceBelow = viewportHeight - anchor.bottom - gap - margin;
  const spaceAbove = anchor.top - gap - margin;
  placement.value =
    height <= spaceBelow || (height > spaceAbove && spaceBelow >= spaceAbove) ? "down" : "up";
  let top = placement.value === "down" ? anchor.bottom + gap : anchor.top - height - gap;
  top = Math.max(margin, Math.min(top, viewportHeight - height - margin));
  panelStyle.value = {
    width: width + "px",
    maxHeight: maxHeight + "px",
    left: left + "px",
    top: top + "px",
  };
}

function openPanel(): void {
  if (props.disabled || open.value) return;
  open.value = true;
  query.value = "";
  const bound = props.sessions.some((session) => session.id === props.modelValue);
  activeId.value = (bound ? props.modelValue : "") || visibleIds.value[0] || "";
  // 先按估算高度摆一次避免首帧闪到错误位置，挂载后再按实测高度校正。
  updatePosition();
  void nextTick(() => {
    if (!open.value) return;
    updatePosition();
    searchRef.value?.focus();
  });
}

/** 关闭面板；restoreFocus 为 true 时把焦点还给触发器（外部点击关闭则不抢焦点）。 */
function close(restoreFocus = true): void {
  if (!open.value) return;
  open.value = false;
  query.value = "";
  activeId.value = "";
  if (restoreFocus) {
    void nextTick(() => {
      if (!open.value) triggerRef.value?.focus();
    });
  }
}

function toggle(): void {
  if (props.disabled) return;
  if (open.value) close(true);
  else openPanel();
}

function moveActive(delta: number): void {
  const ids = visibleIds.value;
  if (!ids.length) return;
  const index = ids.indexOf(activeId.value);
  const next = index < 0 ? (delta > 0 ? 0 : ids.length - 1) : (index + delta + ids.length) % ids.length;
  activeId.value = ids[next];
  scrollActiveIntoView();
}

function jumpActive(where: "first" | "last"): void {
  const ids = visibleIds.value;
  if (!ids.length) return;
  activeId.value = where === "first" ? ids[0] : ids[ids.length - 1];
  scrollActiveIntoView();
}

/** 高亮行只滚动到刚好可见，避免整个列表跳动。 */
function scrollActiveIntoView(): void {
  if (!activeId.value) return;
  const element = document.getElementById(optionId(activeId.value));
  if (element) element.scrollIntoView({ block: "nearest" });
}

function selectActive(): void {
  const id = activeId.value || visibleIds.value[0];
  if (!id) return;
  const session = props.sessions.find((item) => item.id === id);
  if (session) choose(session);
}

function choose(session: ChatSession): void {
  if (props.disabled) return;
  emit("select", session.id);
  close(true);
}

function onOptionHover(sessionId: string): void {
  activeId.value = sessionId;
}

/** 清空搜索后把焦点交回搜索框，便于继续输入。 */
function clearQuery(): void {
  query.value = "";
  void nextTick(() => searchRef.value?.focus());
}

/** 面板内键盘：Esc 关闭、上下移动、搜索框里的 Home/End 与 Enter。 */
function onPanelKeydown(event: KeyboardEvent): void {
  if (!open.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    close(true);
    return;
  }
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    moveActive(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.target !== searchRef.value) return;
  if (event.key === "Home" || event.key === "End") {
    event.preventDefault();
    jumpActive(event.key === "Home" ? "first" : "last");
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    selectActive();
  }
}

function onTriggerKeydown(event: KeyboardEvent): void {
  if (props.disabled) return;
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  if (!open.value) openPanel();
  else moveActive(event.key === "ArrowDown" ? 1 : -1);
}

function onDocumentPointerDown(event: PointerEvent): void {
  if (!open.value) return;
  const target = event.target as Node | null;
  if (!target) return;
  if (triggerRef.value?.contains(target) || panelRef.value?.contains(target)) return;
  close(false);
}

function onWindowKeydown(event: KeyboardEvent): void {
  if (!open.value || event.key !== "Escape") return;
  event.preventDefault();
  close(true);
}

/** 面板自身滚动时不跟手重排，页面/其它容器滚动才重新定位。 */
function onWindowScroll(event: Event): void {
  if (!open.value) return;
  if (event.target instanceof Node && panelRef.value?.contains(event.target)) return;
  updatePosition();
}

watch(visibleIds, (ids) => {
  if (!open.value) return;
  if (activeId.value && !ids.includes(activeId.value)) activeId.value = ids[0] || "";
  void nextTick(updatePosition);
});
watch(
  () => props.disabled,
  (value) => {
    if (value) close(false);
  },
);
watch(
  () => props.modelValue,
  (value) => {
    if (open.value && value && visibleIds.value.includes(value)) activeId.value = value;
  },
);

onMounted(() => {
  document.addEventListener("pointerdown", onDocumentPointerDown, true);
  window.addEventListener("keydown", onWindowKeydown);
  window.addEventListener("resize", updatePosition);
  window.addEventListener("scroll", onWindowScroll, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onDocumentPointerDown, true);
  window.removeEventListener("keydown", onWindowKeydown);
  window.removeEventListener("resize", updatePosition);
  window.removeEventListener("scroll", onWindowScroll, true);
});
</script>

<template>
  <div class="session-picker" :class="{ 'is-empty-variant': variant === 'empty' }">
    <button
      ref="triggerRef"
      type="button"
      class="session-picker-trigger"
      :class="{ 'is-open': open }"
      :disabled="disabled"
      :title="current ? '切换此窗格显示的会话' : placeholder"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :aria-controls="panelId"
      @click="toggle"
      @keydown="onTriggerKeydown"
    >
      <span v-if="current" class="session-picker-trigger-avatar" aria-hidden="true">
        <AppIcon :name="avatarIcon(current)" :size="variant === 'empty' ? 16 : 14" />
      </span>
      <span class="session-picker-trigger-label" :class="{ 'is-placeholder': !current }">{{ currentTitle }}</span>
      <ChevronDown :size="12" class="session-picker-caret" aria-hidden="true" />
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        :id="panelId"
        ref="panelRef"
        class="session-picker-panel"
        :class="placement === 'up' ? 'is-up' : 'is-down'"
        :style="panelStyle"
        role="dialog"
        :aria-labelledby="headingId"
        :data-placement="placement"
        @keydown="onPanelKeydown"
      >
        <header class="session-picker-head">
          <span :id="headingId" class="session-picker-heading">选择会话</span>
          <span class="session-picker-count" :aria-label="'共 ' + sessions.length + ' 个会话'">{{ countLabel }}</span>
          <button
            type="button"
            class="session-picker-close"
            aria-label="关闭会话选择器"
            @click="close(true)"
          >
            <X :size="14" aria-hidden="true" />
          </button>
        </header>

        <div class="session-picker-search-row">
          <Search :size="13" class="session-picker-search-icon" aria-hidden="true" />
          <input
            ref="searchRef"
            v-model="query"
            class="session-picker-search"
            type="text"
            role="combobox"
            aria-label="搜索会话"
            aria-autocomplete="list"
            :aria-expanded="open"
            :aria-controls="listboxId"
            :aria-activedescendant="activeDescendantId"
            placeholder="搜索标题、内容或智能体"
            autocomplete="off"
            spellcheck="false"
          />
          <button
            v-if="query"
            type="button"
            class="session-picker-clear"
            aria-label="清空搜索"
            @click="clearQuery"
          >
            <X :size="12" aria-hidden="true" />
          </button>
        </div>

        <div :id="listboxId" class="session-picker-list" role="listbox" aria-label="会话列表">
          <template v-if="groups.length">
            <section
              v-for="group in groups"
              :key="group.slot"
              class="session-picker-group"
              role="group"
              :aria-label="group.label"
            >
              <div class="session-picker-group-head">
                <span class="session-picker-group-label">{{ group.label }}</span>
                <span class="session-picker-group-count">{{ group.sessions.length }}</span>
              </div>
              <button
                v-for="item in group.sessions"
                :id="optionId(item.id)"
                :key="item.id"
                type="button"
                class="session-picker-option"
                :class="{ 'is-current': item.id === modelValue, 'is-active': item.id === activeId }"
                :data-session-id="item.id"
                role="option"
                :aria-selected="item.id === modelValue"
                tabindex="-1"
                @click="choose(item)"
                @pointerenter="onOptionHover(item.id)"
              >
                <span class="session-picker-option-avatar" aria-hidden="true">
                  <AppIcon :name="avatarIcon(item)" :size="16" />
                </span>
                <span class="session-picker-option-main">
                  <span class="session-picker-option-title" :title="displayTitle(item)">{{ displayTitle(item) }}</span>
                  <span class="session-picker-meta">
                    <span
                      v-for="(part, partIndex) in metaParts(item)"
                      :key="partIndex"
                      class="session-picker-meta-part"
                      :class="{ 'is-running': part === '运行中' }"
                    >{{ part }}</span>
                  </span>
                  <span v-if="item.preview && item.preview.trim()" class="session-picker-option-preview" :title="item.preview.trim()">{{ item.preview.trim() }}</span>
                </span>
                <span v-if="item.id === modelValue" class="session-picker-option-check" aria-hidden="true">
                  <Check :size="14" />
                </span>
              </button>
            </section>
          </template>
          <div v-else class="session-picker-empty">
            <p class="session-picker-empty-title">{{ hasQuery ? "没有匹配的会话" : "还没有会话" }}</p>
            <p class="session-picker-empty-hint">
              {{ hasQuery ? "试试其它关键词，或清空搜索查看全部会话。" : "先在任意窗格新建一个对话。" }}
            </p>
          </div>
        </div>

        <footer v-if="groups.length" class="session-picker-hint" aria-hidden="true">
          <span><span class="session-picker-hint-key">↑↓</span> 移动</span>
          <span><CornerDownLeft :size="11" /> 选择</span>
          <span><span class="session-picker-hint-key">Esc</span> 关闭</span>
        </footer>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* ---------- 触发器 ---------- */
.session-picker { display: inline-flex; min-width: 0; max-width: 100%; }
.session-picker-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  max-width: 100%;
  padding: 3px 6px;
  border: 1px solid transparent;
  border-radius: var(--dash-radius-sm, 8px);
  background: transparent;
  color: var(--dash-text-secondary, #556173);
  font: inherit;
  font-size: 11px;
  line-height: 1.4;
  cursor: pointer;
  transition:
    background var(--dash-dur-fast, 120ms) var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1)),
    border-color var(--dash-dur-fast, 120ms) var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1));
}
.session-picker-trigger:hover:not(:disabled),
.session-picker-trigger:focus-visible,
.session-picker-trigger.is-open { background: #fff; border-color: rgba(120, 110, 100, 0.22); }
.session-picker-trigger:focus-visible { outline: 2px solid rgba(var(--pet-primary-rgb, 204, 112, 82), 0.5); outline-offset: 1px; }
.session-picker-trigger:disabled { opacity: 0.55; cursor: default; }
.session-picker-trigger-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 6px;
  background: rgba(var(--pet-primary-rgb, 204, 112, 82), 0.08);
  color: var(--dash-text-secondary, #556173);
  flex-shrink: 0;
}
.session-picker-trigger-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.session-picker-trigger-label.is-placeholder { color: var(--dash-text-muted, #8792a3); }
.session-picker-caret {
  flex-shrink: 0;
  color: var(--dash-text-muted, #8792a3);
  transition: transform var(--dash-dur-base, 200ms) var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1));
}
.session-picker-trigger.is-open .session-picker-caret { transform: rotate(180deg); }
/* variant='empty'：空窗格里的 CTA，白底、带边框、更宽更大。
   宽度放在根元素上，百分比才能相对父窗格内容宽度解析，触发器再撑满根元素。 */
.session-picker.is-empty-variant { display: flex; width: min(260px, 100%); }
.session-picker.is-empty-variant .session-picker-trigger {
  width: 100%;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 10px;
  border-color: rgba(120, 110, 100, 0.20);
  background: #fff;
  color: var(--dash-text-primary, #20293a);
  font-size: 12px;
}
.session-picker.is-empty-variant .session-picker-trigger:hover:not(:disabled),
.session-picker.is-empty-variant .session-picker-trigger:focus-visible,
.session-picker.is-empty-variant .session-picker-trigger.is-open { border-color: rgba(120, 110, 100, 0.32); background: #fffdf8; }
.session-picker.is-empty-variant .session-picker-trigger-avatar { width: 22px; height: 22px; border-radius: 7px; }

/* ---------- 面板 ---------- */
.session-picker-panel {
  position: fixed;
  z-index: 90;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
  width: min(340px, calc(100vw - 16px));
  max-height: min(360px, calc(100vh - 24px));
  overflow: hidden;
  border: 1px solid rgba(120, 110, 100, 0.16);
  border-radius: 14px;
  background: #fff;
  box-shadow: 0 18px 48px rgba(33, 48, 74, 0.18), 0 2px 8px rgba(33, 48, 74, 0.08);
  color: var(--dash-text-primary, #20293a);
  animation: session-picker-in 170ms var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1));
  transform-origin: top right;
}
.session-picker-panel.is-up { animation-name: session-picker-in-up; transform-origin: bottom right; }
@keyframes session-picker-in {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}
@keyframes session-picker-in-up {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}
@media (prefers-reduced-motion: reduce) {
  .session-picker-panel { animation: none; }
  .session-picker-trigger,
  .session-picker-caret { transition: none; }
}

/* ---------- 面板头部 ---------- */
.session-picker-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 8px 7px 12px;
  border-bottom: 1px solid var(--dash-divider, rgba(33, 48, 74, 0.09));
}
.session-picker-heading { font-size: 12px; font-weight: 650; letter-spacing: 0.01em; color: var(--dash-text-secondary, #556173); }
.session-picker-count { margin-left: auto; font-size: 10px; color: var(--dash-text-muted, #8792a3); font-variant-numeric: tabular-nums; }
.session-picker-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--dash-text-muted, #8792a3);
  cursor: pointer;
}
.session-picker-close:hover { background: rgba(33, 48, 74, 0.05); color: var(--dash-text-primary, #20293a); }
.session-picker-close:focus-visible { outline: 2px solid rgba(var(--pet-primary-rgb, 204, 112, 82), 0.5); outline-offset: 1px; }

/* ---------- 搜索 ---------- */
.session-picker-search-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 10px 5px;
  padding: 5px 8px;
  border: 1px solid var(--dash-divider, rgba(33, 48, 74, 0.09));
  border-radius: var(--dash-radius-sm, 8px);
  background: #fbfaf7;
  transition: border-color var(--dash-dur-fast, 120ms) var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1));
}
.session-picker-search-row:focus-within {
  border-color: rgba(var(--pet-primary-rgb, 204, 112, 82), 0.34);
  background: #fff;
  box-shadow: 0 0 0 3px rgba(var(--pet-primary-rgb, 204, 112, 82), 0.08);
}
.session-picker-search-icon { flex-shrink: 0; color: var(--dash-text-muted, #8792a3); }
.session-picker-search {
  flex: 1;
  min-width: 0;
  padding: 1px 0;
  border: 0;
  background: transparent;
  color: var(--dash-text-primary, #20293a);
  font: inherit;
  font-size: 12px;
  outline: none;
}
.session-picker-search::placeholder { color: var(--dash-text-muted, #8792a3); }
.session-picker-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: var(--dash-text-muted, #8792a3);
  cursor: pointer;
  flex-shrink: 0;
}
.session-picker-clear:hover { background: rgba(33, 48, 74, 0.06); color: var(--dash-text-primary, #20293a); }
.session-picker-clear:focus-visible { outline: 2px solid rgba(var(--pet-primary-rgb, 204, 112, 82), 0.5); outline-offset: 1px; }

/* ---------- 列表 ---------- */
.session-picker-list { flex: 1; min-height: 0; overflow-y: auto; padding: 2px 6px 6px; overscroll-behavior: contain; }
.session-picker-group + .session-picker-group { margin-top: 2px; }
.session-picker-group-head { display: flex; align-items: center; gap: 6px; padding: 6px 6px 3px; }
.session-picker-group-label { font-size: 10px; font-weight: 650; letter-spacing: 0.03em; color: var(--dash-text-muted, #8792a3); }
.session-picker-group-count { font-size: 9.5px; color: var(--dash-text-muted, #8792a3); opacity: 0.72; font-variant-numeric: tabular-nums; }
.session-picker-option {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  width: 100%;
  padding: 7px 8px;
  border: 1px solid transparent;
  border-radius: var(--dash-radius-sm, 8px);
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background var(--dash-dur-fast, 120ms) var(--dash-ease-out, cubic-bezier(0.22, 1, 0.36, 1));
}
.session-picker-option:hover { background: rgba(33, 48, 74, 0.04); }
.session-picker-option.is-active { background: rgba(33, 48, 74, 0.04); border-color: rgba(33, 48, 74, 0.10); }
.session-picker-option.is-current { background: rgba(107, 139, 184, 0.09); }
.session-picker-option.is-current.is-active { background: rgba(107, 139, 184, 0.12); }
.session-picker-option:focus-visible { outline: 2px solid rgba(var(--pet-primary-rgb, 204, 112, 82), 0.5); outline-offset: 1px; }
.session-picker-option-avatar {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  margin-top: 1px;
  border-radius: var(--dash-radius-sm, 8px);
  background: rgba(120, 110, 100, 0.07);
  color: var(--dash-text-secondary, #556173);
  flex-shrink: 0;
}
.session-picker-option-main { display: flex; flex: 1; min-width: 0; flex-direction: column; gap: 2px; }
.session-picker-option-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  font-weight: 600;
  line-height: 1.3;
  color: var(--dash-text-primary, #20293a);
}
.session-picker-meta { font-size: 10px; line-height: 1.4; color: var(--dash-text-muted, #8792a3); }
.session-picker-meta-part + .session-picker-meta-part::before { content: " · "; opacity: 0.6; }
.session-picker-meta-part.is-running {
  display: inline-block;
  margin-left: 1px;
  padding: 0 5px;
  border-radius: 999px;
  background: #fff3cd;
  color: #946200;
  font-size: 9.5px;
  font-weight: 600;
  line-height: 1.55;
  vertical-align: 1px;
}
.session-picker-option-preview {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10px;
  line-height: 1.35;
  color: var(--dash-text-muted, #8792a3);
  opacity: 0.86;
}
.session-picker-option-check {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-top: 1px;
  color: rgba(107, 139, 184, 0.95);
  flex-shrink: 0;
}

/* ---------- 空态与底部提示 ---------- */
.session-picker-empty { padding: 22px 16px 24px; text-align: center; }
.session-picker-empty-title { margin: 0 0 4px; font-size: 12px; font-weight: 600; color: var(--dash-text-secondary, #556173); }
.session-picker-empty-hint { margin: 0; font-size: 11px; line-height: 1.5; color: var(--dash-text-muted, #8792a3); }
.session-picker-hint {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 5px 12px;
  border-top: 1px solid var(--dash-divider, rgba(33, 48, 74, 0.09));
  background: #fbfaf7;
  font-size: 10px;
  color: var(--dash-text-muted, #8792a3);
}
.session-picker-hint > span { display: inline-flex; align-items: center; gap: 4px; }
.session-picker-hint-key { font-size: 9.5px; color: var(--dash-text-secondary, #556173); }
</style>
