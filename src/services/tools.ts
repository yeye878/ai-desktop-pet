// 23 个工具的稳定 ID，与 Rust 端 `src-tauri/src/direct_api/tools.rs::tool_definitions()` 顺序一致
export const AVAILABLE_TOOL_NAMES = [
  "read_file",
  "write_file",
  "list_directory",
  "run_command",
  "web_search",
  "read_webpage",
  "get_weather",
  "open_app",
  "computer_screenshot",
  "computer_mouse",
  "computer_keyboard",
  "computer_wait",
  "window_list",
  "window_focus",
  "browser_open",
  "browser_navigate",
  "browser_snapshot",
  "create_scheduled_task",
  "list_scheduled_tasks",
  "search_memory",
  "save_memory",
  "file_search",
  "delete_memory",
] as const;

export type ToolName = (typeof AVAILABLE_TOOL_NAMES)[number];

export const TOOL_LABELS: Record<ToolName, string> = {
  read_file: "读取文件",
  write_file: "写入文件",
  list_directory: "列目录",
  run_command: "执行命令",
  web_search: "网页搜索",
  read_webpage: "读取网页",
  get_weather: "查询天气",
  open_app: "打开应用",
  computer_screenshot: "电脑截图",
  computer_mouse: "电脑鼠标",
  computer_keyboard: "电脑键盘",
  computer_wait: "电脑等待",
  window_list: "窗口列表",
  window_focus: "窗口聚焦",
  browser_open: "打开浏览器",
  browser_navigate: "浏览器导航",
  browser_snapshot: "浏览器快照",
  create_scheduled_task: "新建定时任务",
  list_scheduled_tasks: "列出定时任务",
  search_memory: "搜索记忆",
  save_memory: "保存记忆",
  delete_memory: "删除记忆",
  file_search: "搜索文件",
};

export function toolLabel(name: string): string {
  return (TOOL_LABELS as Record<string, string>)[name] ?? name;
}
