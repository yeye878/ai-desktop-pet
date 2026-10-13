// 工具的稳定 ID，与 Rust 端 `src-tauri/src/direct_api/tools.rs::tool_definitions()` 顺序一致。
// 排除智能体工坊自己的管理工具（create_agent / update_agent / list_agents），
// 与 `lib.rs::tool_purpose_for_factory()` 的工坊目录同口径。
//
// 这份清单决定工坊「免确认工具」勾选器能看到哪些工具，而且 `Settings.vue` 保存技能时会用它
// 过滤 `allowed_tools` —— **不在此清单里的工具会被静默丢弃**。因此两边必须严格一致：
// `tools.rs` 的 `frontend_tool_contract_matches_rust_definitions()` 会比对集合，漏一个即测试失败。
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
  "browser_extract",
  "browser_click",
  "browser_type",
  "browser_press",
  "browser_scroll",
  "browser_back",
  "browser_forward",
  "browser_status",
  "browser_close",
  "create_scheduled_task",
  "list_scheduled_tasks",
  "search_memory",
  "save_memory",
  "file_search",
  "code_search",
  "edit_file",
  "delete_memory",
  "create_docx",
  "ask_user",
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
  browser_extract: "浏览器提取",
  browser_click: "浏览器点击",
  browser_type: "浏览器输入",
  browser_press: "浏览器按键",
  browser_scroll: "浏览器滚动",
  browser_back: "浏览器后退",
  browser_forward: "浏览器前进",
  browser_status: "浏览器状态",
  browser_close: "关闭浏览器",
  create_scheduled_task: "新建定时任务",
  list_scheduled_tasks: "列出定时任务",
  search_memory: "搜索记忆",
  save_memory: "保存记忆",
  file_search: "搜索文件",
  code_search: "搜索代码内容",
  edit_file: "局部修改文件",
  delete_memory: "删除记忆",
  create_docx: "生成 Word 文档",
  ask_user: "询问用户",
};

export function toolLabel(name: string): string {
  return (TOOL_LABELS as Record<string, string>)[name] ?? name;
}
