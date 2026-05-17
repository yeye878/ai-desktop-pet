import { defineStore } from "pinia";
import { ref } from "vue";

export interface Message {
  id: number;
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  timestamp: number;
}

export type MessageDraft = Omit<Message, "id">;

export const useChatStore = defineStore("chat", () => {
  const messages = ref<Message[]>([]);
  const isLoading = ref(false);
  let nextId = 1;

  function addMessage(role: "user" | "assistant", content: string, thinking?: string) {
    const message = {
      id: nextId++,
      role,
      content,
      thinking,
      timestamp: Date.now(),
    };
    messages.value.push(message);
    return message;
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

  return { messages, isLoading, addMessage, setMessages, clearMessages };
});
