import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { DEFAULT_VOICE_SETTINGS } from "./services/voice";
import { DEFAULT_TTS_SETTINGS } from "./services/tts";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      metadata?: unknown;
    };
  }
}

type MockPayload = Record<string, unknown> | undefined;

const settingStore = new Map<string, string>([
  ["voice_settings", JSON.stringify(DEFAULT_VOICE_SETTINGS)],
  ["tts_settings", JSON.stringify(DEFAULT_TTS_SETTINGS)],
]);

const edgeVoices = [
  { id: "zh-CN-XiaoxiaoNeural", name: "晓晓 - 中文普通话", language: "zh-CN", gender: "Female" },
  { id: "zh-CN-YunxiNeural", name: "云希 - 中文普通话", language: "zh-CN", gender: "Male" },
  { id: "en-US-JennyNeural", name: "Jenny - English US", language: "en-US", gender: "Female" },
  { id: "ja-JP-NanamiNeural", name: "Nanami - 日本語", language: "ja-JP", gender: "Female" },
];

function getMockSetting(args: MockPayload) {
  const key = String(args?.key || "");
  return settingStore.get(key) || "";
}

function setMockSetting(args: MockPayload) {
  const key = String(args?.key || "");
  const value = String(args?.value || "");
  if (key) settingStore.set(key, value);
  return null;
}

function handleMockCommand(cmd: string, args: MockPayload) {
  if (cmd.startsWith("plugin:")) {
    if (cmd === "plugin:window|get_all_windows") return ["main"];
    if (cmd === "plugin:window|scale_factor") return 1;
    if (cmd === "plugin:window|outer_position" || cmd === "plugin:window|inner_position") return { x: 120, y: 80 };
    if (cmd === "plugin:window|outer_size" || cmd === "plugin:window|inner_size") return { width: 1180, height: 760 };
    if (cmd === "plugin:window|cursor_position") return { x: 200, y: 200 };
    if (cmd === "plugin:global-shortcut|is_registered") return false;
    return null;
  }

  switch (cmd) {
    case "get_system_info":
      return { cpu: 18.6, memory: 42.3 };
    case "get_current_model":
      return "Claude Code / preview";
    case "get_backend_type":
      return "claude_code";
    case "get_api_config":
      return {
        api_key: "",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        confirm_enabled: true,
      };
    case "check_claude_status":
      return {
        logged_in: true,
        token_preview: "mock-token-preview",
        model: "Claude 3.5 Sonnet",
        base_url: "local preview",
      };
    case "get_skin":
      return "default";
    case "get_font_color":
    case "get_user_avatar":
      return "";
    case "get_personality":
      return "gentle";
    case "get_profession":
      return "companion";
    case "get_memories":
      return [
        { key: "偏好", value: "喜欢简洁、温暖、低噪声的桌面体验" },
        { key: "工作方式", value: "希望控制台像原生桌面软件一样可靠" },
      ];
    case "get_setting_value":
      return getMockSetting(args);
    case "set_setting_value":
      return setMockSetting(args);
    case "tts_list_voices":
      return edgeVoices;
    case "test_api_connection":
      return "Preview connection ok";
    default:
      return null;
  }
}

export function installDevTauriMock() {
  if (!import.meta.env.DEV) return;
  if (typeof window === "undefined") return;
  if (window.__TAURI_INTERNALS__?.metadata) return;

  mockWindows("main");
  mockIPC((cmd, args) => handleMockCommand(cmd, args as MockPayload), {
    shouldMockEvents: true,
  });
}
