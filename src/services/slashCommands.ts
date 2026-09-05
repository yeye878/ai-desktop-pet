export type SlashScope = "pet" | "dashboard";

export type SlashCommandId =
  | "clear-skill"
  | "new-chat"
  | "clear-chat"
  | "skills"
  | "agents"
  | "evolution"
  | "team"
  | "chat"
  | "clipboard";

export interface SlashCommand {
  id: SlashCommandId;
  trigger: string;
  title: string;
  description: string;
  aliases: string[];
  scopes: SlashScope[];
}

export const SLASH_COMMANDS: SlashCommand[] = [
  {
    id: "clear-skill",
    trigger: "/clear-skill",
    title: "清除技能",
    description: "取消当前激活的 skill",
    aliases: ["clear", "none", "skill-off", "取消技能", "清除技能"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "new-chat",
    trigger: "/new",
    title: "新对话",
    description: "开始一个新的聊天分段",
    aliases: ["new", "restart", "新对话"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "clear-chat",
    trigger: "/clear",
    title: "清空聊天",
    description: "清空当前聊天记录",
    aliases: ["clear-chat", "清空", "清空聊天"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "skills",
    trigger: "/skills",
    title: "技能管理",
    description: "打开 skill 管理页面",
    aliases: ["skill", "技能", "技能管理"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "agents",
    trigger: "/agents",
    title: "智能体工坊",
    description: "打开智能体管理与工坊页面",
    aliases: ["agent", "agents", "智能体", "智能体工坊", "工坊"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "evolution",
    trigger: "/evolution",
    title: "桌宠进化中心",
    description: "查看默契度、画像准则、进化提案与成长历史",
    aliases: ["evolve", "evolution", "进化", "自进化", "复盘", "进化中心", "成长"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "team",
    trigger: "/team",
    title: "多智能体协作",
    description: "组队合作：@多个智能体并行分工完成一个任务",
    aliases: ["team", "组队", "协作", "多智能体", "团队"],
    scopes: ["pet", "dashboard"],
  },
  {
    id: "chat",
    trigger: "/chat",
    title: "切到聊天",
    description: "切换回聊天输入",
    aliases: ["对话", "聊天"],
    scopes: ["pet"],
  },
  {
    id: "clipboard",
    trigger: "/clipboard",
    title: "剪切板",
    description: "切到剪切板暂存区",
    aliases: ["clip", "剪切板", "暂存"],
    scopes: ["pet"],
  },
];

export function slashQueryFromInput(value: string): string | null {
  const trimmedStart = value.trimStart();
  if (!trimmedStart.startsWith("/")) return null;
  const token = trimmedStart.slice(1);
  if (/\s/.test(token)) return null;
  return token.toLowerCase();
}

export function slashCommandMatches(command: SlashCommand, query: string, scope: SlashScope) {
  if (!command.scopes.includes(scope)) return false;
  if (!query) return true;
  const haystack = [
    command.trigger,
    command.title,
    command.description,
    command.id,
    ...command.aliases,
  ].join(" ").toLowerCase();
  return haystack.includes(query);
}
