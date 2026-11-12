/**
 * 按 sessionId 隔离的会话运行时：消息、进行中回合、待处理交互。
 * 单窗格与同屏多窗格读取同一份会话状态，布局切换不重新创建消息或回合。
 * 全局 AI 事件只注册一次，按 session_id 路由到对应会话。
 */
import { defineStore } from "pinia";
import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { createAgentActivity, legacyActivity, type AgentActivityItem } from "../services/agentActivity";
import { restoreActivityPresentation, saveActivityPresentation } from "../services/agentActivityCache";
import type { Message, MessageAgent, ToolEventItem, AiFinishedPayload } from "./chat";
import { messageAgentFromPayload } from "./chat";
import { useSessionsStore } from "./sessions";
import type { ToolConfirmPayload, AskUserPayload } from "../services/pendingInteractions";
import type { CollaborationReply, CollaborationState } from "../services/collaboration";
import { attachmentPayload, attachmentTags, buildMessageWithFiles, type PendingFile } from "../services/attachments";
import { mergeSessionHistory, visibleMessageContent } from "../services/sessionHistory";
import { truncateQuoteContent } from "../services/quoteParser";
import type { QuotedMessage, FileAttachment } from "./chat";

export interface SessionTurn {
  loading: boolean;
  thinking: string;
  streamingAnswer: string;
  toolEvents: ToolEventItem[];
  activity: ReturnType<typeof createAgentActivity>;
}

export interface SessionRuntime {
  sessionId: string;
  messages: Message[];
  turn: SessionTurn;
  loaded: boolean;
  pendingConfirm: ToolConfirmPayload | null;
  pendingQuestion: AskUserPayload | null;
  /** 主会话协同：当前 run_id 与已落窗格的 turn_id（去重）。 */
  collaborationRunId: string;
  collaborationTurns: Set<string>;
}

function newTurn(): SessionTurn {
  return {
    loading: false,
    thinking: "",
    streamingAnswer: "",
    toolEvents: [],
    activity: createAgentActivity(reactive<AgentActivityItem[]>([])),
  };
}

let nextLocalMessageId = 1_000_000;
let presentationStorage: Storage | undefined;
try { presentationStorage = globalThis.localStorage; } catch { /* ignore */ }

function historyToMessage(item: any, sessionId: string): Message {
  const timestamp = typeof item?.created_at === "number" ? item.created_at : Date.now();
  const message: Message = {
    id: typeof item?.id === "number" ? item.id : nextLocalMessageId++,
    role: item?.role === "assistant" ? "assistant" : item?.role === "system" ? "system" : "user",
    content: item?.content || "",
    thinking: item?.thinking || undefined,
    timestamp: timestamp < 1e12 ? timestamp * 1000 : timestamp,
    quote: item?.quoted_content
      ? { role: item?.quoted_role === "assistant" ? "assistant" : "user", content: item.quoted_content }
      : undefined,
    agent: item?.agent_name
      ? { id: item.agent_id || undefined, name: item.agent_name, avatar: item.agent_avatar || undefined }
      : undefined,
    session_id: sessionId,
  };
  if (message.role === "assistant") {
    const restored = restoreActivityPresentation(presentationStorage, message);
    if (restored) Object.assign(message, restored);
  }
  return message;
}

export const useSessionRuntimeStore = defineStore("sessionRuntime", () => {
  const runtimes = reactive<Record<string, SessionRuntime>>({});
  let listeners: UnlistenFn[] = [];
  let started = false;
  const loads = new Map<string, Promise<void>>();
  const turnRevisions = new Map<string, number>();
  function touch(sessionId: string) {
    turnRevisions.set(sessionId, (turnRevisions.get(sessionId) ?? 0) + 1);
  }

  function ensure(sessionId: string): SessionRuntime {
    if (!runtimes[sessionId]) {
      runtimes[sessionId] = {
        sessionId,
        messages: [],
        turn: newTurn(),
        loaded: false,
        pendingConfirm: null,
        pendingQuestion: null,
        collaborationRunId: "",
        collaborationTurns: new Set<string>(),
      };
    }
    return runtimes[sessionId];
  }

  function peek(sessionId: string): SessionRuntime | undefined {
    return sessionId ? runtimes[sessionId] : undefined;
  }

  function remove(sessionId: string) {
    delete runtimes[sessionId];
    loads.delete(sessionId);
    turnRevisions.delete(sessionId);
  }

  function load(sessionId: string, force = false): Promise<void> {
    if (!sessionId) return Promise.resolve();
    const pending = loads.get(sessionId);
    if (pending) return pending;
    const runtime = ensure(sessionId);
    if (runtime.loaded && !force) return Promise.resolve();
    const request = loadHistory(sessionId, runtime).finally(() => {
      if (loads.get(sessionId) === request) loads.delete(sessionId);
    });
    loads.set(sessionId, request);
    return request;
  }

  async function loadHistory(sessionId: string, runtime: SessionRuntime): Promise<void> {
    try {
      const history = await invoke<any[] | null>("get_chat_session_messages", { sessionId, limit: 200 });
      if (peek(sessionId) !== runtime) return;
      if (Array.isArray(history)) {
        const merged = mergeSessionHistory(history.map((item) => historyToMessage(item, sessionId)), runtime.messages);
        runtime.messages.splice(0, runtime.messages.length, ...merged);
        runtime.loaded = true;
      }
    } catch (error) {
      console.error("加载会话历史失败", error);
    }
    if (peek(sessionId) === runtime) await restoreActive(sessionId);
  }

  async function restoreActive(sessionId: string): Promise<void> {
    const runtime = ensure(sessionId);
    const revision = turnRevisions.get(sessionId) ?? 0;
    try {
      const active = await invoke<{ thinking?: string; session_id?: string } | null>("get_active_chat", { sessionId });
      if (peek(sessionId) !== runtime || revision !== (turnRevisions.get(sessionId) ?? 0)) return;
      if (!active || active.session_id !== sessionId) return;
      const turn = runtime.turn;
      // 已有实时状态比后端的启动快照完整，不得清空工具、流式正文或覆盖思考增量。
      if (turn.loading || turn.activity.items.length || turn.streamingAnswer || turn.thinking) return;
      turn.loading = true;
      turn.thinking = active.thinking || "";
      if (active.thinking && !turn.activity.items.length) turn.activity.items.push(...legacyActivity(active.thinking));
    } catch { /* 无 Tauri 环境忽略。 */ }
  }

  async function send(sessionId: string, text: string, files: PendingFile[] = [], quote?: QuotedMessage): Promise<void> {
    const content = text.trim();
    if ((!content && !files.length && !quote) || !sessionId || files.some((file) => file.status !== "ready")) return;
    const runtime = ensure(sessionId);
    if (runtime.turn.loading) return;
    const sessions = useSessionsStore();
    const session = sessions.find(sessionId);
    if (!session) return;
    runtime.messages.push({
      id: nextLocalMessageId++,
      role: "user",
      content: content || (quote ? "（引用消息）" : "(已发送文件)"),
      files: files.length ? attachmentTags(files) : undefined,
      quote,
      timestamp: Date.now(),
      session_id: sessionId,
    });
    runtime.turn.loading = true;
    runtime.turn.thinking = "";
    runtime.turn.streamingAnswer = "";
    runtime.turn.toolEvents = [];
    runtime.turn.activity.reset();
    touch(sessionId);
    const isAgent = session.slot !== "main";
    try {
      await invoke("send_to_ai", {
        message: buildMessageWithFiles(content, files),
        attachments: attachmentPayload(files),
        quotedRole: quote?.role,
        quotedContent: quote ? truncateQuoteContent(quote.content) : undefined,
        mentionAgents: [],
        collaborationEnabled: session.slot === "main",
        sessionId,
        targetAgentId: isAgent ? session.agent_id || undefined : undefined,
        model: session.model || "",
      });
    } catch (error) {
      settle(sessionId, "出错了: " + String(error), { reason: "failed" });
    }
    sessions.scheduleReload();
  }

  function settle(
    sessionId: string,
    content: string,
    options: { thinking?: string; agent?: MessageAgent | null; reason?: "completed" | "failed" | "aborted" } = {},
  ) {
    const runtime = ensure(sessionId);
    touch(sessionId);
    const turn = runtime.turn;
    const last = runtime.messages[runtime.messages.length - 1];
    if (!(last?.role === "assistant" && last.content === content && last.session_id === sessionId)) {
      const message: Message = {
        id: nextLocalMessageId++,
        role: "assistant",
        content,
        thinking: options.thinking || turn.thinking || undefined,
        timestamp: Date.now(),
        toolEvents: turn.toolEvents.length ? JSON.parse(JSON.stringify(turn.toolEvents)) : undefined,
        agent: options.agent ?? null,
        session_id: sessionId,
      };
      if (turn.activity.items.length) {
        const presentation = turn.activity.finish(content, options.reason ?? "completed");
        Object.assign(message, presentation);
        saveActivityPresentation(presentationStorage, { ...message, ...presentation });
      }
      runtime.messages.push(message);
    }
    turn.loading = false;
    turn.thinking = "";
    turn.streamingAnswer = "";
    turn.toolEvents = [];
    turn.activity.reset();
    runtime.pendingConfirm = null;
    runtime.pendingQuestion = null;
  }

  async function abort(sessionId: string): Promise<void> {
    const runtime = peek(sessionId);
    if (runtime?.turn.loading) settle(sessionId, "已中止", { reason: "aborted" });
    try { await invoke("abort_ai", { sessionId }); } catch { /* ignore */ }
  }

  async function confirm(sessionId: string, approved: boolean): Promise<void> {
    const runtime = peek(sessionId);
    const id = runtime?.pendingConfirm?.id;
    if (!id) return;
    await invoke("confirm_tool", { id, approved });
    if (runtime.pendingConfirm?.id === id) runtime.pendingConfirm = null;
  }

  async function answer(sessionId: string, value: string): Promise<void> {
    const runtime = peek(sessionId);
    const id = runtime?.pendingQuestion?.id;
    if (!id) return;
    await invoke("answer_question", { id, answer: value });
    if (runtime.pendingQuestion?.id === id) runtime.pendingQuestion = null;
  }

  /** 全局事件入口：只在首个窗格使用时注册，按 session_id 路由。 */
  async function start(): Promise<void> {
    if (started) return;
    started = true;
    const route = (sessionId: string | undefined | null): SessionRuntime | null => {
      const id = sessionId || useSessionsStore().activeMainId;
      if (!id) return null;
      touch(id);
      return ensure(id);
    };
    listeners.push(await listen<{ session_id?: string; text?: string }>("ai-started", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (runtime) runtime.turn.loading = true;
    }));
    listeners.push(await listen<{ session_id?: string; text?: string } | string>("ai-thinking", ({ payload }) => {
      const sessionId = typeof payload === "string" ? undefined : payload?.session_id;
      const text = typeof payload === "string" ? payload : payload?.text || "";
      const runtime = route(sessionId);
      if (!runtime || !text) return;
      runtime.turn.loading = true;
      runtime.turn.thinking += text;
      runtime.turn.activity.appendThinking(text);
    }));
    listeners.push(await listen<{ session_id?: string; text?: string }>("ai-answer-delta", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime || !payload?.text) return;
      runtime.turn.loading = true;
      runtime.turn.streamingAnswer += payload.text;
      runtime.turn.activity.appendText(payload.text);
    }));
    listeners.push(await listen<ToolEventItem & { session_id?: string }>("ai-tool-event", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime) return;
      runtime.turn.loading = true;
      const tool = payload as ToolEventItem;
      const index = runtime.turn.toolEvents.findIndex((item) => item.id === tool.id);
      if (index >= 0) runtime.turn.toolEvents[index] = { ...runtime.turn.toolEvents[index], ...tool };
      else runtime.turn.toolEvents.push(tool);
      runtime.turn.activity.upsertTool(tool);
    }));
    listeners.push(await listen<AiFinishedPayload & { session_id?: string }>("ai-finished", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime) return;
      settle(runtime.sessionId, payload.text || "", {
        thinking: payload.thinking || undefined,
        agent: messageAgentFromPayload(payload),
        reason: "completed",
      });
      useSessionsStore().scheduleReload();
    }));
    listeners.push(await listen<{ session_id?: string; message?: string; aborted?: boolean; thinking?: string }>("ai-error", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime) return;
      const turn = runtime.turn;
      let text: string;
      if (payload.aborted) text = "已中止";
      else if (turn.thinking || turn.activity.items.length) text = "⚠️ " + (payload.message || "出错");
      else text = "出错了: " + (payload.message || "未知错误");
      settle(runtime.sessionId, text, {
        thinking: payload.thinking || turn.thinking || undefined,
        reason: payload.aborted ? "aborted" : "failed",
      });
      useSessionsStore().scheduleReload();
    }));
    listeners.push(await listen<ToolConfirmPayload>("ai-tool-confirm", ({ payload }) => {
      if (!payload?.session_id) return;
      ensure(payload.session_id).pendingConfirm = payload;
    }));
    listeners.push(await listen<{ id: string }>("ai-tool-confirm-resolved", ({ payload }) => {
      for (const runtime of Object.values(runtimes)) {
        if (runtime.pendingConfirm?.id === payload.id) runtime.pendingConfirm = null;
      }
    }));
    listeners.push(await listen<AskUserPayload>("ai-ask-user", ({ payload }) => {
      if (!payload?.session_id) return;
      ensure(payload.session_id).pendingQuestion = payload;
    }));
    listeners.push(await listen<{ id: string }>("ai-ask-user-resolved", ({ payload }) => {
      for (const runtime of Object.values(runtimes)) {
        if (runtime.pendingQuestion?.id === payload.id) runtime.pendingQuestion = null;
      }
    }));
    // 其它视图（单窗格 / 聊天子窗口）发出的用户消息与定时任务系统消息；
    // 发送方自己已乐观插入，按最后一条消息去重，避免重复。
    listeners.push(await listen<{ session_id?: string; role?: string; content?: string; files?: FileAttachment[]; fileAttachments?: FileAttachment[]; quote?: { role?: string; content?: string } }>("sync-chat-message", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime || !payload?.content) return;
      const role: Message["role"] = payload.role === "assistant" ? "assistant" : payload.role === "system" ? "system" : "user";
      const last = runtime.messages[runtime.messages.length - 1];
      if (last && last.role === role && visibleMessageContent(last) === visibleMessageContent({ role, content: payload.content })) return;
      runtime.messages.push({
        id: nextLocalMessageId++,
        role,
        content: payload.content,
        files: payload.files ?? payload.fileAttachments,
        timestamp: Date.now(),
        quote: payload.quote?.content
          ? { role: payload.quote.role === "assistant" ? "assistant" : "user", content: payload.quote.content }
          : undefined,
        session_id: runtime.sessionId,
      });
      if (role === "user") {
        runtime.turn.loading = true;
        runtime.turn.thinking = "";
        runtime.turn.streamingAnswer = "";
        runtime.turn.toolEvents = [];
        runtime.turn.activity.reset();
      }
    }));
    listeners.push(await listen<CollaborationState & { session_id?: string }>("collaboration-state", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime) return;
      if (runtime.collaborationRunId !== payload.run_id) {
        runtime.collaborationRunId = payload.run_id;
        runtime.collaborationTurns.clear();
        runtime.turn.activity.reset();
        runtime.turn.toolEvents = [];
      }
      runtime.turn.loading = payload.status === "running";
      if (payload.status !== "running") {
        runtime.turn.thinking = "";
        runtime.turn.streamingAnswer = "";
        runtime.pendingConfirm = null;
        runtime.pendingQuestion = null;
        useSessionsStore().scheduleReload();
      }
    }));
    listeners.push(await listen<CollaborationReply & { session_id?: string }>("collaboration-reply", ({ payload }) => {
      const runtime = route(payload?.session_id);
      if (!runtime || runtime.collaborationTurns.has(payload.turn_id)) return;
      runtime.collaborationTurns.add(payload.turn_id);
      const message: Message = {
        id: nextLocalMessageId++,
        role: "assistant",
        content: payload.text,
        thinking: payload.thinking || undefined,
        timestamp: Date.now(),
        agent: messageAgentFromPayload(payload),
        session_id: runtime.sessionId,
      };
      if (runtime.turn.activity.items.length) {
        const presentation = runtime.turn.activity.finish(payload.text, payload.failed ? "failed" : "completed");
        Object.assign(message, presentation);
        saveActivityPresentation(presentationStorage, { ...message, ...presentation });
      }
      runtime.messages.push(message);
      runtime.turn.thinking = "";
      runtime.turn.streamingAnswer = "";
      runtime.turn.toolEvents = [];
      runtime.turn.activity.reset();
    }));
  }

  function stop(): void {
    listeners.splice(0).forEach((unlisten) => unlisten());
    started = false;
  }

  function clear() {
    for (const sessionId of Object.keys(runtimes)) remove(sessionId);
  }

  return { runtimes, ensure, peek, remove, clear, load, send, abort, settle, confirm, answer, start, stop };
});
