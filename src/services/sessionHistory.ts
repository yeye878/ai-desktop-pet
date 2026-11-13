import type { Message } from "../stores/chat";

export function visibleMessageContent(message: Pick<Message, "role" | "content">): string {
  if (message.role !== "user") return message.content;
  const content = message.content.split("\n\n---\n[用户附加了以下本地文件]")[0];
  return content === "[用户附加了文件，但没有输入文字]" ? "(已发送文件)" : content;
}

/**
 * 数据库历史可能早于本地流式事件，也可能没有保存中断回合。
 * 按消息顺序对齐两份记录，保留本地独有回复及完整轨迹，不按正文全局去重。
 */
export function mergeSessionHistory(history: Message[], local: Message[]): Message[] {
  const matches = (a: Message, b: Message) => a.role === b.role && visibleMessageContent(a) === visibleMessageContent(b);
  const lengths = Array.from({ length: history.length + 1 }, () => new Uint32Array(local.length + 1));
  for (let h = history.length - 1; h >= 0; h--) {
    for (let l = local.length - 1; l >= 0; l--) {
      lengths[h][l] = matches(history[h], local[l]) ? 1 + lengths[h + 1][l + 1]
        : Math.max(lengths[h + 1][l], lengths[h][l + 1]);
    }
  }
  const merged: Message[] = [];
  let h = 0;
  let l = 0;
  while (h < history.length && l < local.length) {
    if (matches(history[h], local[l])) {
      const saved = history[h++];
      const live = local[l++];
      merged.push({
        ...saved,
        ...live,
        id: saved.id,
        files: live.files ?? saved.files,
        thinking: live.thinking || saved.thinking,
        agent: live.agent ?? saved.agent,
        activity: live.activity?.length ? live.activity : saved.activity,
        toolEvents: live.toolEvents?.length ? live.toolEvents : saved.toolEvents,
        displayContent: live.displayContent ?? saved.displayContent,
      });
    } else if (lengths[h + 1][l] >= lengths[h][l + 1]) {
      merged.push(history[h++]);
    } else {
      merged.push(local[l++]);
    }
  }
  return merged.concat(history.slice(h), local.slice(l));
}
