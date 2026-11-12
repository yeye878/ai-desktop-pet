<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Send, Square, X, ArrowUpRight, GripHorizontal, Plus, Settings2, Paperclip } from "@lucide/vue";
import AppIcon from "./AppIcon.vue";
import AttachmentTray from "./AttachmentTray.vue";
import { useAttachmentsStore } from "../stores/attachments";
import { useDraftStore } from "../stores/drafts";
import { getFileIcon } from "../services/attachments";
import AgentActivity from "./AgentActivity.vue";
import AgentModelPicker from "./AgentModelPicker.vue";
import SessionPermissionPicker from "./SessionPermissionPicker.vue";
import UserQuestion from "./UserQuestion.vue";
import { renderMessageSegments } from "../services/markdown";
import { useSessionRuntimeStore } from "../stores/sessionRuntime";
import { useSessionsStore, SLOT_DEFAULT_AGENT, type ChatSession, type ChatSlot, type BindSessionPatch } from "../stores/sessions";
import { effortLabel, type AgentModelSlot } from "../services/agentModels";

/**
 * 同屏多窗格里的一个会话窗格。
 * paneId 是窗格身份（布局持久化用），sessionId 是数据身份；
 * 关闭窗格只移除视图，不停止、不删除会话。
 */
const props = defineProps<{ paneId: string; sessionId: string; backend?: string; fileOver?: boolean }>();
const emit = defineEmits<{
  assign: [paneId: string, sessionId: string];
  close: [paneId: string];
  openSingle: [sessionId: string];
  paneDrop: [targetPaneId: string, sourcePaneId: string];
}>();

const runtime = useSessionRuntimeStore();
const sessions = useSessionsStore();
const attachments = useAttachmentsStore();
const attachmentDraft = computed(() => attachments.ensure(props.sessionId));
const filesLoading = computed(() => attachmentDraft.value.files.some((file) => file.status === "loading"));
// 输入框草稿存在 store 里：控制台页面切到别的页面再切回来时，输入到一半的内容不丢
const drafts = useDraftStore();
const draftKey = computed(() => `session:${props.sessionId || props.paneId}`);
const input = computed({
  get: () => drafts.get(draftKey.value),
  set: (value: string) => drafts.set(draftKey.value, value),
});
const messagesRef = ref<HTMLDivElement | null>(null);
const dialogRef = ref<HTMLDialogElement | null>(null);
const dialogMode = ref<"settings" | "new" | null>(null);
const settingsTab = ref<"model" | "permission">("model");
const newSlot = ref<ChatSlot>("claude");
const creating = ref(false);
const savingModel = ref(false);
const paneError = ref("");
const settingsError = ref("");
const customModel = ref("");
let disposed = false;

const newSessionOptions: { slot: ChatSlot; label: string }[] = [
  { slot: "main", label: "主对话" },
  { slot: "claude", label: "Claude Code" },
  { slot: "codex", label: "Codex" },
  { slot: "dsh", label: "DeepSeek Harness" },
];

const session = computed<ChatSession | undefined>(() => sessions.find(props.sessionId));
const state = computed(() => (props.sessionId ? runtime.peek(props.sessionId) : undefined));
const messages = computed(() => state.value?.messages ?? []);
const turn = computed(() => state.value?.turn);
const loading = computed(() => !!turn.value?.loading);
const pendingConfirm = computed(() => state.value?.pendingConfirm ?? null);
const pendingQuestion = computed(() => state.value?.pendingQuestion ?? null);
const confirming = ref(false);
const dragging = ref(false);
const isMainSession = computed(() => session.value?.slot === "main");
const canEditModel = computed(() => !!session.value && !isMainSession.value);
const canEditPermission = computed(() =>
  !!session.value && ["claude_code", "codex", "dsh"].includes(props.backend || ""),
);
const hasSessionSettings = computed(() => canEditModel.value || canEditPermission.value);
const modelSlot = computed<AgentModelSlot | null>(() => {
  if (!canEditModel.value) return null;
  if (props.backend === "claude_code") return "claude";
  if (props.backend === "codex" || props.backend === "dsh") return props.backend;
  return null;
});
const settingsSummary = computed(() => {
  if (isMainSession.value) return session.value?.permission ? `权限 ${session.value.permission}` : "主对话设置";
  return [
    session.value?.model || "跟随默认模型",
    session.value?.provider,
    session.value?.reasoning_effort ? `思考 ${effortLabel(session.value.reasoning_effort)}` : "",
  ].filter(Boolean).join(" · ");
});

async function openDialog(mode: "settings" | "new") {
  if (creating.value || savingModel.value) return;
  dialogMode.value = mode;
  settingsTab.value = canEditModel.value ? "model" : "permission";
  settingsError.value = "";
  paneError.value = "";
  customModel.value = session.value?.model || "";
  await nextTick();
  if (!disposed && dialogMode.value === mode) dialogRef.value?.showModal();
}

function closeDialog() {
  dialogRef.value?.close();
  dialogMode.value = null;
}

function onDialogCancel(event: Event) {
  if (creating.value) event.preventDefault();
}

async function createConversation() {
  if (creating.value || savingModel.value) return;
  const previousId = props.sessionId;
  const previous = session.value;
  const slot = previous?.slot ?? newSlot.value;
  creating.value = true;
  paneError.value = "";
  try {
    const created = await sessions.create(slot, {
      agentId: previous?.agent_id || (slot === "main" ? "" : SLOT_DEFAULT_AGENT[slot]),
      model: previous?.model || "",
      provider: previous?.provider || "",
      reasoningEffort: previous?.reasoning_effort || "",
      activate: false,
    });
    if (disposed) return;
    if (props.sessionId !== previousId) {
      paneError.value = "新对话已创建，可从会话列表选择。";
      return;
    }
    closeDialog();
    emit("assign", props.paneId, created.id);
  } catch (cause) {
    if (!disposed) paneError.value = "新建对话失败：" + (cause instanceof Error ? cause.message : String(cause));
  } finally {
    creating.value = false;
  }
}

function startNewConversation() {
  if (session.value) void createConversation();
  else void openDialog("new");
}

async function selectModel(payload: { model: string; provider: string; effort: string }) {
  const target = session.value;
  if (!target || loading.value || savingModel.value || creating.value) return;
  const patch: BindSessionPatch = {};
  if (payload.model !== target.model) patch.model = payload.model;
  if (payload.provider !== target.provider) patch.provider = payload.provider;
  if (payload.effort !== target.reasoning_effort) patch.reasoningEffort = payload.effort;
  if (!Object.keys(patch).length) return;
  savingModel.value = true;
  settingsError.value = "";
  try {
    await sessions.bind(target.id, patch);
  } catch (cause) {
    if (!disposed && props.sessionId === target.id) {
      settingsError.value = "保存模型失败：" + (cause instanceof Error ? cause.message : String(cause));
    }
  } finally {
    savingModel.value = false;
  }
}

/**
 * 指针拖拽换位。Tauri 窗口开启了原生文件拖放，HTML5 drag 事件会被拦截，
 * 所以用 pointerdown/move/up 自己实现：拖标题栏，松手时落在哪个窗格就与它交换。
 */
let dragPointer: { startX: number; startY: number; active: boolean } | null = null;
let hoveredPane: HTMLElement | null = null;

function clearHover() {
  hoveredPane?.classList.remove("drag-over");
  hoveredPane = null;
}
function onHandlePointerDown(event: PointerEvent) {
  if (event.button !== 0 || creating.value) return;
  const target = event.target as HTMLElement | null;
  if (target?.closest("button, select, input, a, option")) return;
  dragPointer = { startX: event.clientX, startY: event.clientY, active: false };
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}
function onPointerMove(event: PointerEvent) {
  if (!dragPointer) return;
  if (!dragPointer.active) {
    if (Math.hypot(event.clientX - dragPointer.startX, event.clientY - dragPointer.startY) < 6) return;
    dragPointer.active = true;
    dragging.value = true;
    document.body.style.cursor = "grabbing";
  }
  const under = document.elementFromPoint(event.clientX, event.clientY)?.closest(".session-pane") as HTMLElement | null;
  const next = under && under.dataset.paneId !== props.paneId ? under : null;
  if (next !== hoveredPane) {
    clearHover();
    hoveredPane = next;
    hoveredPane?.classList.add("drag-over");
  }
}
function onPointerUp() {
  window.removeEventListener("pointermove", onPointerMove);
  const wasActive = dragPointer?.active === true;
  dragPointer = null;
  dragging.value = false;
  document.body.style.cursor = "";
  const targetId = hoveredPane?.dataset.paneId || "";
  clearHover();
  if (wasActive && targetId && targetId !== props.paneId) emit("paneDrop", targetId, props.paneId);
}
onBeforeUnmount(() => {
  disposed = true;
  closeDialog();
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  clearHover();
  if (dragging.value) document.body.style.cursor = "";
});

const title = computed(() => session.value?.agent_name || session.value?.title || "选择会话");
const subtitle = computed(() => {
  if (!session.value) return "空窗格";
  const slot = session.value.slot === "main" ? "主对话" : session.value.slot;
  return `${slot} · ${session.value.title || "新会话"}`;
});
const avatar = computed(() => session.value?.agent_avatar || (session.value?.slot === "main" ? "🐾" : "🤖"));

/** 同一会话只允许出现在一个窗格，由父组件去重；这里提供全部会话供换绑。 */
const options = computed(() => sessions.sessions);

watch(
  () => props.sessionId,
  (id) => {
    input.value = "";
    paneError.value = "";
    closeDialog();
    if (id) void runtime.load(id);
  },
  { immediate: true },
);
watch(() => props.backend, closeDialog);

watch(
  () => [messages.value.length, loading.value, turn.value?.streamingAnswer?.length ?? 0, turn.value?.thinking.length ?? 0],
  async () => {
    await nextTick();
    const el = messagesRef.value;
    if (el) el.scrollTop = el.scrollHeight;
  },
);

function onAssign(event: Event) {
  const id = (event.target as HTMLSelectElement).value;
  if (!creating.value && id && id !== props.sessionId) emit("assign", props.paneId, id);
}

async function send() {
  const text = input.value.trim();
  if ((!text && !attachmentDraft.value.files.length) || !props.sessionId || loading.value || filesLoading.value) return;
  const files = [...attachmentDraft.value.files];
  input.value = "";
  attachments.clear(props.sessionId);
  await runtime.send(props.sessionId, text, files);
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    void send();
  }
}

async function confirm(approved: boolean) {
  if (confirming.value) return;
  confirming.value = true;
  try { await runtime.confirm(props.sessionId, approved); } finally { confirming.value = false; }
}

function rendered(content: string) {
  return renderMessageSegments(content).map((segment) => segment.html).join("");
}
</script>

<template>
  <section
    class="session-pane"
    :class="{ dragging, 'file-over': fileOver }"
    :data-pane-id="paneId"
  >
    <div v-if="fileOver" class="session-pane-file-drop" aria-hidden="true">
      <Paperclip :size="16" />
      <span>{{ sessionId ? '松开添加到此会话' : '先选择会话，再添加文件' }}</span>
    </div>
    <header class="session-pane-head" title="按住拖动，与其它窗格换位" @pointerdown="onHandlePointerDown">
      <GripHorizontal class="session-pane-grip" :size="14" aria-hidden="true" />
      <span class="session-pane-avatar" aria-hidden="true">
        <AppIcon v-if="!session || session.slot === 'main'" name="brand-pet" :size="16" />
        <AppIcon v-else-if="session.agent_id === 'builtin-claude' || session.slot === 'claude'" name="claude" :size="16" />
        <AppIcon v-else-if="session.agent_id === 'builtin-codex' || session.slot === 'codex'" name="codex" :size="16" />
        <AppIcon v-else-if="session.agent_id === 'builtin-dsh' || session.slot === 'dsh'" name="dsh" :size="16" />
        <template v-else>{{ avatar }}</template>
      </span>
      <div class="session-pane-title">
        <strong :title="title">{{ title }}</strong>
        <small :title="subtitle">{{ subtitle }}</small>
      </div>
      <span :class="['session-pane-state', loading ? 'busy' : 'ready']">{{ loading ? "运行中" : "就绪" }}</span>
      <div class="session-pane-actions">
        <select
          class="session-pane-switch"
          :value="sessionId"
          title="切换此窗格显示的会话"
          :disabled="creating"
          @change="onAssign"
        >
          <option value="" disabled>选择会话</option>
          <option v-for="item in options" :key="item.id" :value="item.id">
            {{ item.slot === "main" ? "🐾" : (item.agent_avatar || "🤖") }} {{ item.title || item.agent_name || item.id }}
          </option>
        </select>
        <button type="button" class="session-pane-new" title="在此窗格新建对话" :disabled="creating || savingModel" @click="startNewConversation">
          <Plus :size="14" />
          <span>{{ creating ? "创建中" : "新对话" }}</span>
        </button>
        <button v-if="sessionId" type="button" title="在单窗格中打开" :disabled="creating" @click="emit('openSingle', sessionId)">
          <ArrowUpRight :size="14" />
        </button>
        <button v-if="loading" type="button" title="停止该会话当前回合" @click="runtime.abort(sessionId)">
          <Square :size="13" />
        </button>
        <button type="button" title="关闭窗格（不停止任务）" @click="emit('close', paneId)">
          <X :size="14" />
        </button>
      </div>
    </header>

    <p v-if="paneError && dialogMode !== 'new'" class="session-pane-error" role="alert">{{ paneError }}</p>
    <div v-if="!sessionId" class="session-pane-empty">
      <p>新建对话，或选择已有会话</p>
      <button type="button" class="session-pane-empty-new" :disabled="creating" @click="startNewConversation">
        <Plus :size="15" /> 新建对话
      </button>
      <select class="session-pane-switch" :disabled="creating" @change="onAssign">
        <option value="" selected disabled>选择会话</option>
        <option v-for="item in options" :key="item.id" :value="item.id">
          {{ item.slot === "main" ? "🐾" : (item.agent_avatar || "🤖") }} {{ item.title || item.agent_name || item.id }}
        </option>
      </select>
    </div>

    <div v-else ref="messagesRef" class="session-pane-messages">
      <p v-if="!messages.length && !loading" class="session-pane-hint">暂无消息，发一条开始对话。</p>
      <div v-for="msg in messages" :key="msg.id" :class="['session-pane-msg', msg.role]">
        <AgentActivity
          v-if="msg.role === 'assistant'"
          :items="msg.activity"
          :tools="msg.toolEvents"
          :thinking="msg.activity ? undefined : msg.thinking"
          compact
        />
        <div v-if="msg.role !== 'system'" class="session-pane-bubble">
          <div v-if="msg.agent && msg.role === 'assistant'" class="session-pane-agent">
            <AppIcon v-if="msg.agent.id === 'builtin-claude'" name="claude" :size="12" />
            <AppIcon v-else-if="msg.agent.id === 'builtin-codex'" name="codex" :size="12" />
            <AppIcon v-else-if="msg.agent.id === 'builtin-dsh'" name="dsh" :size="12" />
            <span v-else>{{ msg.agent.avatar || "🤖" }}</span>
            <span>{{ msg.agent.name }}</span>
          </div>
          <div class="md-body" v-html="rendered(msg.displayContent ?? msg.content)"></div>
          <div v-if="msg.files?.length" class="session-pane-sent-files">
            <span v-for="(file, index) in msg.files" :key="index" :title="file.name">{{ file.isDir ? '📂' : getFileIcon(file.extension) }} {{ file.name }}</span>
          </div>
        </div>
        <div v-else class="session-pane-system">{{ msg.content }}</div>
      </div>
      <div v-if="loading" class="session-pane-msg assistant">
        <AgentActivity
          :items="turn?.activity.items"
          :thinking="turn?.thinking"
          :active="true"
          :waiting="!!pendingConfirm || !!pendingQuestion"
          :awaiting-answer="!!pendingQuestion"
          compact
          @stop="runtime.abort(sessionId)"
        />
        <div v-if="turn?.streamingAnswer" class="session-pane-bubble">
          <div class="md-body" v-html="rendered(turn.streamingAnswer)"></div>
        </div>
      </div>
    </div>

    <div v-if="pendingConfirm" class="session-pane-confirm">
      <strong>{{ pendingConfirm.summary || pendingConfirm.tool_name }}</strong>
      <code v-if="pendingConfirm.command">{{ pendingConfirm.command }}</code>
      <div class="session-pane-confirm-actions">
        <button type="button" :disabled="confirming" @click="confirm(false)">拒绝</button>
        <button type="button" class="primary" :disabled="confirming" @click="confirm(true)">允许</button>
      </div>
    </div>
    <UserQuestion
      v-else-if="pendingQuestion"
      :question="pendingQuestion"
      compact
      @answered="() => { if (state) state.pendingQuestion = null; }"
    />

    <button v-if="sessionId && hasSessionSettings" type="button" class="session-pane-config" :title="settingsSummary + ' · 会话设置'" :disabled="creating" @click="openDialog('settings')">
      <Settings2 :size="13" />
      <span>{{ settingsSummary }}</span>
      <small>设置</small>
    </button>
    <footer v-if="sessionId" class="session-pane-composer">
      <AttachmentTray :draft-key="sessionId" />
      <div class="session-pane-input-row">
        <button type="button" class="session-pane-attach" title="添加附件（也可拖入此窗格）" aria-label="添加附件" :disabled="attachmentDraft.picking" @click="attachments.pick(sessionId)">
          <Paperclip :size="16" />
        </button>
        <input
          v-model="input"
          type="text"
          :placeholder="loading ? '该会话正在运行…' : '发消息到此会话…'"
          :disabled="loading"
          @keydown="onKeydown"
        />
        <button type="button" title="发送" aria-label="发送" :disabled="(!input.trim() && !attachmentDraft.files.length) || loading || filesLoading" @click="send">
          <Send :size="14" />
        </button>
      </div>
    </footer>
  </section>
  <Teleport to="body">
    <dialog
      ref="dialogRef"
      class="session-pane-dialog"
      :class="{ 'is-settings': dialogMode === 'settings' }"
      :aria-label="dialogMode === 'new' ? '新建对话' : '会话设置'"
      @close="dialogMode = null"
      @cancel="onDialogCancel"
      @click="(event) => { if (event.target === dialogRef && !creating) closeDialog(); }"
    >
      <template v-if="dialogMode">
        <header class="session-pane-dialog-head">
          <div>
            <h2>{{ dialogMode === "new" ? "新建对话" : "会话设置" }}</h2>
            <p>{{ dialogMode === "new" ? "创建后只在当前窗格打开" : `${title} · ${session?.title || '新对话'}` }}</p>
          </div>
          <button type="button" class="session-pane-dialog-close" aria-label="关闭设置面板" :disabled="creating" @click="closeDialog">
            <X :size="18" />
          </button>
        </header>
        <nav v-if="dialogMode === 'settings'" class="session-pane-dialog-tabs" aria-label="会话设置分类">
          <button v-if="canEditModel" type="button" :class="{ active: settingsTab === 'model' }" :aria-pressed="settingsTab === 'model'" @click="settingsTab = 'model'">模型与思考</button>
          <button v-if="canEditPermission" type="button" :class="{ active: settingsTab === 'permission' }" :aria-pressed="settingsTab === 'permission'" @click="settingsTab = 'permission'">权限</button>
        </nav>
        <div class="session-pane-dialog-body">
          <template v-if="dialogMode === 'new'">
            <label class="session-pane-dialog-field">
              对话类型
              <select v-model="newSlot" :disabled="creating">
                <option v-for="option in newSessionOptions" :key="option.slot" :value="option.slot">{{ option.label }}</option>
              </select>
            </label>
            <p class="session-pane-dialog-note">旧会话和其他窗格保持不变，正在运行的任务也会继续。</p>
            <p v-if="paneError" class="session-pane-error" role="alert">{{ paneError }}</p>
          </template>
          <template v-else-if="session">
            <p v-if="loading" class="session-pane-dialog-note" role="status">当前回合运行中，结束后可修改设置；新建对话不会中止旧任务。</p>
            <p v-if="settingsError" class="session-pane-error" role="alert">{{ settingsError }}</p>
            <template v-if="settingsTab === 'model' && canEditModel">
              <AgentModelPicker
                v-if="modelSlot"
                :key="sessionId"
                :slot="modelSlot"
                :model="session.model"
                :provider="session.provider"
                :effort="session.reasoning_effort"
                :disabled="loading || savingModel"
                embedded
                @select="selectModel"
              />
              <form v-else-if="backend === 'direct_api'" class="session-pane-model-form" @submit.prevent="selectModel({ model: customModel.trim(), provider: session.provider, effort: session.reasoning_effort })">
                <label class="session-pane-dialog-field">
                  本会话模型 ID
                  <input v-model="customModel" placeholder="留空跟随默认模型" :disabled="loading || savingModel" />
                </label>
                <p class="session-pane-dialog-note">API 渠道与思考配置沿用当前智能体设置，这里只覆盖本会话的模型。</p>
                <button type="submit" class="session-pane-dialog-primary" :disabled="loading || savingModel">{{ savingModel ? "保存中…" : "保存模型" }}</button>
              </form>
              <p v-else class="session-pane-dialog-note">当前后端的模型由连接配置管理。</p>
            </template>
            <SessionPermissionPicker
              v-else-if="settingsTab === 'permission' && canEditPermission"
              :key="sessionId"
              :session-id="sessionId"
              :backend="backend || ''"
              :disabled="loading"
              embedded
            />
            <p v-else class="session-pane-dialog-note">{{ isMainSession ? "主对话的模型、思考强度和模式在单窗格底部信息条设置。" : "当前后端没有可调整的会话设置。" }}</p>
          </template>
        </div>
        <footer class="session-pane-dialog-footer">
          <span v-if="dialogMode === 'settings'" role="status">{{ savingModel ? "正在保存…" : "更改仅作用于本会话，不影响其他窗格" }}</span>
          <button v-if="dialogMode === 'new'" type="button" class="session-pane-dialog-primary" :disabled="creating" @click="createConversation">{{ creating ? "创建中…" : "创建对话" }}</button>
          <button v-else type="button" @click="closeDialog">完成</button>
        </footer>
      </template>
    </dialog>
  </Teleport>
</template>

<style scoped>
.session-pane { display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; background: rgba(255,255,255,.64); border: 1px solid rgba(255,255,255,.62); border-radius: 14px; box-shadow: 0 2px 10px rgba(60,50,40,.05); transition: box-shadow .15s, opacity .15s; }
.session-pane.dragging { opacity: .55; box-shadow: none; }
.session-pane.drag-over { outline: 2px solid var(--pet-primary, #ff6b6b); outline-offset: -2px; box-shadow: 0 0 0 4px rgba(255,107,107,.12); }
.session-pane-head { display: grid; grid-template-columns: 14px 28px minmax(0, 1fr) auto; align-items: center; gap: 8px; padding: 9px 12px; border-bottom: 1px solid rgba(120,110,100,.1); flex-shrink: 0; cursor: grab; user-select: none; touch-action: none; background: rgba(255,255,255,.5); }
.session-pane-head:active { cursor: grabbing; }
.session-pane-grip { color: #b6aca1; flex-shrink: 0; }
.session-pane-avatar { width: 28px; height: 28px; display: inline-flex; align-items: center; justify-content: center; font-size: 16px; background: rgba(var(--pet-primary-rgb, 255,107,107), .1); border-radius: 9px; flex-shrink: 0; }
.session-pane-title { min-width: 0; flex: 1; display: flex; flex-direction: column; gap: 1px; }
.session-pane-title strong { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.session-pane-title small { font-size: 11px; color: #968b7f; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.session-pane-state { display: inline-flex; align-items: center; gap: 4px; font-size: 10px; padding: 3px 8px; border-radius: 999px; flex-shrink: 0; }
.session-pane-state::before { content: ""; width: 5px; height: 5px; border-radius: 50%; background: currentColor; }
.session-pane-state.busy { background: #fff3cd; color: #946200; }
.session-pane-state.busy::before { animation: pane-pulse 1s ease-in-out infinite; }
.session-pane-state.ready { background: #e6f4ea; color: #2c7a43; }
@keyframes pane-pulse { 50% { opacity: .3; } }
.session-pane-actions { grid-column: 1 / -1; display: flex; flex-wrap: wrap; align-items: center; gap: 2px; min-width: 0; }
.session-pane-actions button { display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 26px; border: none; border-radius: 7px; background: transparent; cursor: pointer; color: #7d7267; transition: background .12s, color .12s; }
.session-pane-actions button:hover { background: rgba(0,0,0,.07); color: #3d3630; }
.session-pane-switch { flex: 1; min-width: 70px; width: 0; font-size: 11px; border: 1px solid transparent; border-radius: 7px; background: transparent; color: #7d7267; padding: 3px 4px; cursor: pointer; }
.session-pane-switch:hover, .session-pane-switch:focus { border-color: rgba(120,110,100,.25); background: #fff; }
.session-pane-messages { flex: 1; min-height: 0; overflow-y: auto; padding: 12px; display: flex; flex-direction: column; gap: 10px; }
.session-pane-hint { color: #968b7f; font-size: 12px; text-align: center; margin: auto 0; }
.session-pane-empty { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; margin: 10px; border: 1.5px dashed rgba(120,110,100,.25); border-radius: 10px; color: #968b7f; font-size: 12px; }
.session-pane-empty p { margin: 0; }
.session-pane-empty .session-pane-switch { flex: none; width: min(200px, 100%); border-color: rgba(120,110,100,.3); background: #fff; padding: 6px 10px; font-size: 12px; }
.session-pane-empty-new { display: inline-flex; align-items: center; gap: 5px; padding: 8px 14px; border: 1px solid rgba(120,110,100,.18); border-radius: 9px; background: #fff; color: #3d3630; cursor: pointer; }
.session-pane-actions .session-pane-new { width: auto; padding: 0 7px; gap: 3px; color: #3d3630; background: rgba(120,110,100,.06); font-size: 11px; }
.session-pane-actions button:disabled { opacity: .45; cursor: default; }
.session-pane-msg { display: flex; flex-direction: column; gap: 4px; }
.session-pane-msg.user { align-items: flex-end; }
.session-pane-bubble { max-width: 100%; padding: 8px 11px; border-radius: 12px; background: #fff; border: 1px solid rgba(120,110,100,.1); font-size: 12.5px; line-height: 1.6; overflow-wrap: anywhere; }
.session-pane-msg.user .session-pane-bubble { background: var(--pet-primary, #ff6b6b); color: #fff; border: none; border-bottom-right-radius: 4px; }
.session-pane-msg.assistant .session-pane-bubble { border-bottom-left-radius: 4px; }
.session-pane-agent { font-size: 11px; color: #968b7f; margin-bottom: 3px; }
.session-pane-system { text-align: center; font-size: 11px; color: #968b7f; }
.session-pane-confirm { margin: 0 12px 10px; padding: 12px; border: 1px solid #f0c36d; border-radius: 12px; background: #fffaf0; display: flex; flex-direction: column; gap: 8px; font-size: 12px; flex-shrink: 0; }
.session-pane-confirm code { font-size: 11px; overflow-wrap: anywhere; }
.session-pane-confirm-actions { display: flex; gap: 8px; justify-content: flex-end; }
.session-pane-confirm-actions button { padding: 5px 14px; border-radius: 8px; border: 1px solid rgba(120,110,100,.3); background: #fff; cursor: pointer; }
.session-pane-confirm-actions .primary { background: var(--pet-primary, #ff6b6b); border-color: transparent; color: #fff; }
.session-pane-composer { display: flex; flex-direction: column; padding: 10px 12px; border-top: 1px solid rgba(120,110,100,.1); flex-shrink: 0; background: rgba(255,255,255,.5); }
.session-pane-input-row { display: flex; gap: 6px; min-width: 0; }
.session-pane-composer .session-pane-attach { width: 30px; flex-shrink: 0; color: #7d7267; background: transparent; }
.session-pane-composer .session-pane-attach:hover { background: rgba(120,110,100,.08); }
.session-pane { position: relative; }
.session-pane.file-over { outline: 2px solid var(--pet-primary, #7d9fc7); outline-offset: -2px; }
.session-pane-file-drop { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; gap: 7px; background: rgba(250,252,255,.86); color: #37567e; font-size: 12px; font-weight: 600; z-index: 5; pointer-events: none; }
.session-pane-sent-files { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 5px; }
.session-pane-sent-files span { max-width: 100%; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border: 1px solid currentColor; border-radius: 6px; padding: 2px 5px; font-size: 11px; }
.session-pane-composer input { flex: 1; min-width: 0; border: 1px solid rgba(120,110,100,.18); border-radius: 9px; padding: 8px 11px; font-size: 12.5px; background: #fff; outline: none; transition: border-color .12s, box-shadow .12s; }
.session-pane-composer input:focus { border-color: var(--pet-primary, #ff6b6b); box-shadow: 0 0 0 3px rgba(255,107,107,.12); }
.session-pane-composer button { display: inline-flex; align-items: center; justify-content: center; width: 36px; border: none; border-radius: 9px; background: var(--pet-primary, #ff6b6b); color: #fff; cursor: pointer; transition: opacity .12s, transform .12s; }
.session-pane-composer button:not(:disabled):hover { transform: translateY(-1px); }
.session-pane-composer button:disabled { opacity: .4; cursor: default; }
.session-pane-config { display: flex; align-items: center; gap: 6px; width: 100%; min-width: 0; padding: 7px 12px; border: 0; border-top: 1px solid rgba(120,110,100,.1); background: rgba(255,255,255,.6); text-align: left; color: #7d7267; font: inherit; font-size: 11px; cursor: pointer; flex-shrink: 0; }
.session-pane-config:hover { background: #fff; }
.session-pane-config svg, .session-pane-config small { flex-shrink: 0; }
.session-pane-config span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.session-pane-error { margin: 8px 12px; padding: 8px 10px; border-radius: 8px; background: #fff0ed; color: #b7472f; font-size: 12px; overflow-wrap: anywhere; }
.session-pane-dialog { width: min(560px, calc(100vw - 24px)); max-width: none; max-height: calc(100vh - 24px); max-height: calc(100dvh - 24px); padding: 0; margin: auto; box-sizing: border-box; border: 1px solid rgba(120,110,100,.16); border-radius: 18px; color: #3d3630; background: #fcfcfb; box-shadow: 0 24px 80px rgba(33,48,74,.24); font: inherit; overflow: hidden; }
.session-pane-dialog[open] { display: flex; flex-direction: column; }
.session-pane-dialog.is-settings { height: min(660px, calc(100vh - 24px)); height: min(660px, calc(100dvh - 24px)); }
.session-pane-dialog::backdrop { background: rgba(30,38,48,.3); }
.session-pane-dialog-head { display: flex; align-items: flex-start; gap: 12px; padding: 18px 20px 14px; flex-shrink: 0; }
.session-pane-dialog-head > div { flex: 1; min-width: 0; }
.session-pane-dialog-head h2 { margin: 0; font-size: 17px; font-weight: 650; }
.session-pane-dialog-head p { margin: 5px 0 0; font-size: 12px; color: #968b7f; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.session-pane-dialog-close { display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 30px; flex-shrink: 0; border: 0; border-radius: 8px; background: transparent; color: #7d7267; cursor: pointer; }
.session-pane-dialog-close:hover { background: rgba(120,110,100,.08); }
.session-pane-dialog-tabs { display: flex; gap: 4px; padding: 0 20px 12px; flex-shrink: 0; border-bottom: 1px solid rgba(120,110,100,.12); }
.session-pane-dialog-tabs button { padding: 7px 12px; border: 0; border-radius: 8px; background: transparent; color: #7d7267; font: inherit; font-size: 12px; cursor: pointer; }
.session-pane-dialog-tabs button.active { background: #e9f0f9; color: #37567e; font-weight: 600; }
.session-pane-dialog-body { flex: 1; min-height: 0; min-width: 0; overflow: auto; overscroll-behavior: contain; padding: 16px 20px; }
.session-pane-dialog-note { margin: 0 0 14px; font-size: 12px; line-height: 1.6; color: #968b7f; }
.session-pane-dialog-field { display: flex; flex-direction: column; gap: 8px; margin-bottom: 16px; font-size: 12px; font-weight: 600; }
.session-pane-dialog-field input, .session-pane-dialog-field select { width: 100%; min-width: 0; box-sizing: border-box; padding: 10px 12px; border: 1px solid rgba(120,110,100,.22); border-radius: 9px; background: #fff; color: #3d3630; font: inherit; }
.session-pane-dialog-footer { display: flex; align-items: center; justify-content: flex-end; gap: 12px; padding: 12px 20px; border-top: 1px solid rgba(120,110,100,.12); flex-shrink: 0; }
.session-pane-dialog-footer span { flex: 1; font-size: 11px; line-height: 1.5; color: #968b7f; }
.session-pane-dialog-footer button, .session-pane-dialog-primary { flex-shrink: 0; border: 1px solid rgba(120,110,100,.2); border-radius: 9px; padding: 8px 16px; background: #fff; color: #3d3630; font: inherit; font-size: 12px; cursor: pointer; }
.session-pane-dialog-footer .session-pane-dialog-primary, .session-pane-dialog-primary { background: #37567e; color: #fff; border-color: transparent; }
.session-pane-dialog button:disabled { opacity: .45; cursor: default; }
.session-pane-dialog button:focus-visible, .session-pane-dialog input:focus-visible, .session-pane-dialog select:focus-visible { outline: 2px solid #7d9fc7; outline-offset: 2px; }
:deep(.md-body) { font-size: 12.5px; }
</style>
