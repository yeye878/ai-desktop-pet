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

export interface Message {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  timestamp: number;
  files?: FileAttachment[];
  toolEvents?: ToolEventItem[];
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
  ) {
    const message = {
      id: nextId++,
      role,
      content,
      thinking,
      files,
      toolEvents,
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
