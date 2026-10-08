import { defineStore } from "pinia";
import { reactive, ref } from "vue";
import { createAgentActivity, type AgentActivityItem } from "../services/agentActivity";
import { clearActivityPresentations, restoreActivityPresentation, saveActivityPresentation } from "../services/agentActivityCache";
import type { CollaborationReply, CollaborationState } from "../services/collaboration";

export interface FileAttachment {
  name: string;
  isImage: boolean;
  extension: string;
}

export interface ToolEventItem {
  id: string;
  tool_name: string;
  status: string;
  summary: string;
  arguments?: string | null;
  command?: string | null;
  path?: string | null;
  output?: string | null;
  approved?: boolean | null;
}

export interface QuotedMessage {
  /** 被引用消息的说话者 */
  role: "user" | "assistant";
  /** 被引用消息的原文 */
  content: string;
}

export interface MessageAgent {
  id?: string;
  name: string;
  avatar?: string;
}

/** Matches the Rust AiFinishedPayload event, including its snake_case fields. */
export interface AiFinishedPayload {
  text: string;
  thinking?: string | null;
  agent_id?: string | null;
  agent_name?: string | null;
  agent_avatar?: string | null;
}

export function messageAgentFromPayload(payload: AiFinishedPayload): MessageAgent | null {
  return payload.agent_name
    ? { id: payload.agent_id || undefined, name: payload.agent_name, avatar: payload.agent_avatar || undefined }
    : null;
}

export interface Message {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  timestamp: number;
  files?: FileAttachment[];
  toolEvents?: ToolEventItem[];
  activity?: AgentActivityItem[];
  displayContent?: string;
  /** 用户引用的前文对话（微信式引用） */
  quote?: QuotedMessage;
  /** 回复来自哪个智能体（@ 提及后由该智能体作答） */
  agent?: MessageAgent | null;
}

export type MessageDraft = Omit<Message, "id">;

export const useChatStore = defineStore("chat", () => {
  let presentationStorage: Storage | undefined;
  try { presentationStorage = globalThis.localStorage; } catch { /* Storage can be disabled by the WebView. */ }
  const messages = ref<Message[]>([]);
  const isLoading = ref(false);
  const activity = createAgentActivity(reactive<AgentActivityItem[]>([]));
  const collaborationEnabled = ref(true);
  const collaboration = ref<CollaborationState | null>(null);
  const completedTurns = new Set<string>();
  let nextId = 1;

  function addMessage(
    role: "user" | "assistant" | "system",
    content: string,
    thinking?: string,
    files?: FileAttachment[],
    toolEvents?: ToolEventItem[],
    quote?: QuotedMessage,
    agent?: MessageAgent | null,
  ) {
    const message: Message = {
      id: nextId++,
      role,
      content,
      thinking,
      files,
      toolEvents,
      quote,
      agent,
      timestamp: Date.now(),
    };
    messages.value.push(message);
    return messages.value[messages.value.length - 1];
  }

  function addSystemMessage(content: string) {
    return addMessage("system", content);
  }

  function setMessages(items: MessageDraft[]) {
    nextId = 1;
    messages.value = items.map((item) => {
      const timestamp = item.timestamp < 1e12 ? item.timestamp * 1000 : item.timestamp;
      const message = { id: nextId++, ...item, timestamp };
      return item.role === "assistant" ? { ...message, ...restoreActivityPresentation(presentationStorage, message) } : message;
    });
  }

  function clearMessages() {
    messages.value = [];
    nextId = 1;
    collaboration.value = null;
    completedTurns.clear();
    activity.reset();
    clearActivityPresentations(presentationStorage);
  }

  function receiveCollaborationState(state: CollaborationState): boolean {
    const current = collaboration.value;
    if (current?.status === "running" && current.run_id !== state.run_id && state.status !== "running") return false;
    if (current?.run_id !== state.run_id) completedTurns.clear();
    if (current?.run_id !== state.run_id || current?.turn !== state.turn) activity.reset();
    collaboration.value = state;
    isLoading.value = state.status === "running";
    return true;
  }

  function receiveCollaborationReply(reply: CollaborationReply): boolean {
    if (collaboration.value?.run_id !== reply.run_id || collaboration.value.status !== "running" || completedTurns.has(reply.turn_id)) return false;
    completedTurns.add(reply.turn_id);
    const message = addMessage("assistant", reply.text, reply.thinking || undefined, undefined, undefined, undefined, messageAgentFromPayload(reply));
    captureActivity(message, reply.failed ? "failed" : "completed");
    return true;
  }

  function captureActivity(message: Message, reason: "completed" | "failed" | "aborted" = "completed") {
    if (activity.items.length) {
      const presentation = activity.finish(message.content, reason);
      Object.assign(message, presentation);
      saveActivityPresentation(presentationStorage, { ...message, ...presentation });
    }
    activity.reset();
  }

  return { messages, isLoading, activity, captureActivity, collaborationEnabled, collaboration, receiveCollaborationState, receiveCollaborationReply, addMessage, addSystemMessage, setMessages, clearMessages };
});
