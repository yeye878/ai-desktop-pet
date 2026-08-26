import { defineStore } from "pinia";
import { ref } from "vue";

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
  arguments?: string;
  command?: string;
  path?: string;
  output?: string;
  approved?: boolean;
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

export interface Message {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  timestamp: number;
  files?: FileAttachment[];
  toolEvents?: ToolEventItem[];
  /** 用户引用的前文对话（微信式引用） */
  quote?: QuotedMessage;
  /** 回复来自哪个智能体（@ 提及后由该智能体作答） */
  agent?: MessageAgent | null;
}

export type MessageDraft = Omit<Message, "id">;

export const useChatStore = defineStore("chat", () => {
  const messages = ref<Message[]>([]);
  const isLoading = ref(false);
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
    const message = {
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
    return message;
  }

  function addSystemMessage(content: string) {
    return addMessage("system", content);
  }

  function setMessages(items: MessageDraft[]) {
    nextId = 1;
    messages.value = items.map((item) => ({
      id: nextId++,
      ...item,
    }));
  }

  function clearMessages() {
    messages.value = [];
    nextId = 1;
  }

  return { messages, isLoading, addMessage, addSystemMessage, setMessages, clearMessages };
});
