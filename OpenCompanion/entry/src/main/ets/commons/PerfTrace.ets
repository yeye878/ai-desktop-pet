import { hilog } from '@kit.PerformanceAnalysisKit';

interface MarkSpan {
  from: string;
  ms: number;
}

/**
 * 轻量性能埋点（命题 R4：冷启动 ≤3s、语音指令响应 ≤2s 的量化验收）。
 * mark 打点 → measure 计算区间 → hilog 落日志 + 内存留存供界面自检展示。
 * 全部同步内存操作，本身不引入可观测的性能开销。
 */
export class PerfTrace {
  private static marks: Map<string, number> = new Map<string, number>();
  private static spans: MarkSpan[] = [];

  static mark(name: string): void {
    PerfTrace.marks.set(name, Date.now());
  }

  /** 计算 from→to 区间耗时并记录（to 未打点时以当前时间为准） */
  static measure(spanName: string, from: string, to?: string): number {
    const t0 = PerfTrace.marks.get(from);
    if (t0 === undefined) {
      return -1;
    }
    let t1 = to !== undefined ? PerfTrace.marks.get(to) : undefined;
    if (t1 === undefined) {
      t1 = Date.now();
    }
    const ms = t1 - t0;
    const span: MarkSpan = { from: spanName, ms: ms };
    PerfTrace.spans.push(span);
    if (PerfTrace.spans.length > 64) {
      PerfTrace.spans.shift();
    }
    hilog.info(0x0C0C, 'OpenCompanion.Perf', '%{public}s = %{public}d ms', spanName, ms);
    return ms;
  }

  /** 某打点的时间戳（未打点返回当前时间），供调用方自行计算耗时 */
  static lastStamp(name: string): number {
    const t = PerfTrace.marks.get(name);
    return t === undefined ? Date.now() : t;
  }

  /** 最近一次同名区间耗时（ms），无记录返回 -1 */
  static lastMs(spanName: string): number {
    for (let i = PerfTrace.spans.length - 1; i >= 0; i--) {
      if (PerfTrace.spans[i].from === spanName) {
        return PerfTrace.spans[i].ms;
      }
    }
    return -1;
  }

  /** 埋点摘要（首页自检 / /perf 命令用） */
  static report(): string {
    if (PerfTrace.spans.length === 0) {
      return '暂无性能埋点数据';
    }
    const lines: string[] = [];
    PerfTrace.spans.forEach((s: MarkSpan) => {
      lines.push(`${s.from}: ${s.ms}ms`);
    });
    return lines.join('\n');
  }
}
