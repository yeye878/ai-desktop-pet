import { AskOption } from './Types';
import { bus, Ev } from '../commons/EventBus';

interface GateWaiter {
  resolve: (v: string) => void;
  reject: (e: Error) => void;
}

/**
 * 交互闸门：工具确认 / 追问用户（对应桌面版 pending_confirms / pending_questions 的 oneshot 通道机制，
 * 鸿蒙侧以 Promise + 事件通知 UI 实现）
 */
export class Gates {
  private static confirms: Map<string, GateWaiter> = new Map<string, GateWaiter>();
  private static asks: Map<string, GateWaiter> = new Map<string, GateWaiter>();
  private static seq: number = 1;

  /** 请求工具执行确认；60s 超时视为拒绝 */
  static requestToolConfirm(toolName: string, argsText: string): Promise<boolean> {
    return new Promise<boolean>((resolve: (v: boolean) => void, reject: (e: Error) => void) => {
      const id = `c${Gates.seq++}`;
      const waiter: GateWaiter = {
        resolve: (v: string) => resolve(v === '1'),
        reject: reject
      };
      Gates.confirms.set(id, waiter);
      bus.emit(Ev.TOOL_CONFIRM, { 'id': id, 'tool': toolName, 'args': argsText } as Record<string, string>);
      setTimeout(() => {
        if (Gates.confirms.delete(id)) {
          resolve(false); // 超时默认拒绝，安全第一
        }
      }, 60000);
    });
  }

  /** UI 调用：解决确认 */
  static resolveToolConfirm(id: string, approved: boolean): void {
    const w = Gates.confirms.get(id);
    if (w) {
      Gates.confirms.delete(id);
      w.resolve(approved ? '1' : '0');
    }
  }

  /** 请求用户回答（ask_user 工具） */
  static requestUserAnswer(question: string, options: AskOption[]): Promise<string> {
    return new Promise<string>((resolve: (v: string) => void, reject: (e: Error) => void) => {
      const id = `a${Gates.seq++}`;
      const waiter: GateWaiter = {
        resolve: resolve,
        reject: reject
      };
      Gates.asks.set(id, waiter);
      const opts: Array<Object> = [];
      options.forEach((o: AskOption) => {
        const m: Record<string, string> = { 'label': o.label, 'description': o.description };
        opts.push(m);
      });
      bus.emit(Ev.ASK_USER, { 'id': id, 'question': question, 'options': opts } as Record<string, Object>);
      setTimeout(() => {
        if (Gates.asks.delete(id)) {
          resolve('(用户未回应，已跳过)');
        }
      }, 120000);
    });
  }

  /** UI 调用：回答用户提问 */
  static resolveAskUser(id: string, answer: string): void {
    const w = Gates.asks.get(id);
    if (w) {
      Gates.asks.delete(id);
      w.resolve(answer);
    }
  }

  /** 用户中断时清理所有挂起闸门 */
  static abortAll(): void {
    Gates.confirms.forEach((w: GateWaiter, k: string) => {
      w.reject(new Error('__ABORTED__'));
    });
    Gates.confirms.clear();
    Gates.asks.forEach((w: GateWaiter, k: string) => {
      w.resolve('(用户中断)');
    });
    Gates.asks.clear();
  }
}
