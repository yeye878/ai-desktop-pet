/**
 * 进程内事件总线（对应桌面版 Tauri app_handle.emit / listen 的前后端事件机制。
 * 鸿蒙为单进程 ArkTS，直接用回调注册表实现，无需跨进程。）
 */
export class BusEvent {
  name: string;
  data: Object | null;

  constructor(name: string, data: Object | null = null) {
    this.name = name;
    this.data = data;
  }
}

export type BusListener = (e: BusEvent) => void;

export class EventBus {
  private listeners: Map<string, Set<BusListener>> = new Map<string, Set<BusListener>>();

  on(name: string, l: BusListener): void {
    let set = this.listeners.get(name);
    if (!set) {
      set = new Set<BusListener>();
      this.listeners.set(name, set);
    }
    set.add(l);
  }

  off(name: string, l: BusListener): void {
    const set = this.listeners.get(name);
    if (set) {
      set.delete(l);
    }
  }

  emit(name: string, data: Object | null = null): void {
    const set = this.listeners.get(name);
    if (!set) {
      return;
    }
    set.forEach((l: BusListener) => {
      try {
        l(new BusEvent(name, data));
      } catch (e) {
        // 监听器异常不阻断其它监听器
      }
    });
  }
}

export const bus: EventBus = new EventBus();

/** 事件名常量 */
export class Ev {
  static readonly CHAT_CHANGED: string = 'chat_changed';
  static readonly AI_DELTA: string = 'ai_delta';
  static readonly AI_TOOL: string = 'ai_tool';
  static readonly AI_STARTED: string = 'ai_started';
  static readonly AI_FINISHED: string = 'ai_finished';
  static readonly AI_ERROR: string = 'ai_error';
  static readonly TOOL_CONFIRM: string = 'tool_confirm';
  static readonly ASK_USER: string = 'ask_user';
  static readonly NAVIGATE: string = 'navigate';
  static readonly EVOLUTION_CHANGED: string = 'evolution_changed';
  static readonly AGENTS_CHANGED: string = 'agents_changed';
  static readonly KB_CHANGED: string = 'kb_changed';
  static readonly VOICE_PARTIAL: string = 'voice_partial';
  static readonly VOICE_STATE: string = 'voice_state';
  static readonly VP_WIDTH: string = 'vp_width';
}
