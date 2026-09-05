import { LlmClient, LlmProfiles } from './LlmClient';
import { AgentLoop, LoopOptions, LoopResult, LoopCallbacks } from './AgentLoop';
import { AgentRepo } from './AgentRepo';
import { ToolRegistry } from './ToolRegistry';
import { bus, Ev } from '../commons/EventBus';
import { Log } from '../commons/Log';
import { AgentProfile, CancelToken, JsonUtil, LlmMessage, ToolCallReq } from './Types';

interface SubTask {
  agentName: string;
  subtask: string;
}

/**
 * 多智能体团队编排（对应桌面版 collab/：/team 规划 → 并行执行 → 综合汇报）。
 * 对应命题要求 F4「自主分解复杂任务」。
 */
export class TeamOrchestrator {
  static readonly MARKER: string = '/team';

  static isTeamCommand(text: string): boolean {
    const t = text.trim();
    return t.startsWith(TeamOrchestrator.MARKER) || t.startsWith('/组队') || t.startsWith('/协作');
  }

  /** @param text 已剥离 @提及 与 /team 前缀的任务描述 */
  static async run(taskText: string, mentioned: AgentProfile[], token: CancelToken,
    trace: (line: string) => void, onThinking: (d: string) => void, onContent: (d: string) => void): Promise<string> {
    const t0 = Date.now();
    const clock = (): string => {
      return `[+${((Date.now() - t0) / 1000).toFixed(1)}s]`;
    };
    const profile = await LlmProfiles.active();
    if (profile === null) {
      return '未配置 LLM API，无法组队。请到「设置」配置。';
    }
    let agents = mentioned;
    if (agents.length < 2) {
      const all = await AgentRepo.list();
      agents = all.filter((a: AgentProfile) => a.isBuiltin);
    }
    trace(`${clock()} 👥 [规划] 组队模式：${agents.map((a: AgentProfile) => a.avatar + a.name).join(' + ')}`);

    // ① 规划：LLM 把任务拆解到各成员（DAG 分解）
    let subtasks: SubTask[] = [];
    try {
      const agentDesc = agents.map((a: AgentProfile) => `${a.name}(${a.description})`).join('；');
      const planRaw = await LlmClient.chatOnce(profile,
        '你是多智能体团队的队长。把任务分解给队员执行。只输出严格 JSON：{"subtasks":[{"agent":"队员名","subtask":"分配给该队员的具体子任务"}]}，' +
        '不要输出其它文字。', `队员：${agentDesc}\n任务：${taskText}`, 900);
      const jsonText = JsonUtil.extractJson(planRaw);
      if (jsonText) {
        const obj = JsonUtil.parseObj(jsonText);
        if (obj) {
          const arr = obj['subtasks'] as Array<Object>;
          if (arr) {
            arr.forEach((o: Object) => {
              const m = o as Record<string, Object>;
              const st: SubTask = {
                agentName: typeof m['agent'] === 'string' ? m['agent'] as string : '',
                subtask: typeof m['subtask'] === 'string' ? m['subtask'] as string : ''
              };
              if (st.agentName.length > 0 && st.subtask.length > 0) {
                subtasks.push(st);
              }
            });
          }
        }
      }
    } catch (e) {
      Log.warn(`plan failed: ${(e as Error).message}`);
    }
    if (token.cancelled) {
      return '(已中断)';
    }
    if (subtasks.length === 0) {
      // 规划失败退化为全员拿完整任务（与桌面版策略一致）
      subtasks = agents.map((a: AgentProfile) => {
        const st: SubTask = { agentName: a.name, subtask: taskText };
        return st;
      });
    }
    trace(`${clock()} 📋 [派发] 任务图分解为 ${subtasks.length} 个子任务：`);
    subtasks.forEach((st: SubTask) => {
      trace(`${clock()} 　· ${st.agentName} ← ${st.subtask}`);
    });

    // ② 并行执行各成员（能力白名单隔离；组队模式下高风险工具自动拒绝，对应 CONFIRM_GATE 单槽约束）
    //    工程保障：单成员 90s 超时 + 失败重试 1 次；单成员异常不拖垮也不挂死整队（方案 4.4）
    trace(`${clock()} ▶ [执行] ${subtasks.length} 个子任务并行开始`);
    const reports: string[] = [];
    const jobs: Promise<void>[] = [];
    subtasks.forEach((st: SubTask) => {
      let agent: AgentProfile | null = null;
      agents.forEach((a: AgentProfile) => {
        if (a.name === st.agentName) {
          agent = a;
        }
      });
      if (agent === null) {
        reports.push(`【${st.agentName}】(无该成员)`);
        return;
      }
      const a: AgentProfile = agent;
      jobs.push((async (): Promise<void> => {
        let lastErr = '';
        for (let attempt = 1; attempt <= 2; attempt++) {
          if (token.cancelled) {
            return;
          }
          // 每个成员独立的取消令牌：超时/用户中断只取消自己，不影响其他成员
          const childToken = new CancelToken();
          token.onCancel(() => {
            childToken.cancel();
          });
          try {
            trace(`${clock()} [${a.name}] 第${attempt}次尝试…`);
            const opts = new LoopOptions();
            opts.profile = profile;
            opts.systemPrompt = `${a.systemPrompt}\n当前处于多智能体协作模式：只负责你被分配的子任务，输出你的工作报告（不要寒暄）。`;
            opts.history = [];
            opts.userText = st.subtask;
            opts.tools = ToolRegistry.defsFor(a.allowedTools);
            opts.allowConfirm = false;
            opts.maxTurns = 8;
            opts.token = childToken;
            const cbs: LoopCallbacks = {
              onThinking: (_d: string) => {
              },
              onContent: (_d: string) => {
              },
              onToolTrace: (line: string) => {
                trace(`${clock()} [${a.name}] ${line}`);
              }
            };
            opts.callbacks = cbs;
            const r: LoopResult = await TeamOrchestrator.withTimeout(AgentLoop.run(opts), 90000, `${a.name}`);
            reports.push(`【${a.name}】\n${r.content.length > 0 ? r.content : '(无输出)'}`);
            trace(`${clock()} ✔ [${a.name}] 完成`);
            return;
          } catch (e) {
            lastErr = (e as Error).message;
            childToken.cancel(); // 超时/失败后终止该成员的底层请求
            Log.warn(`agent ${a.name} attempt ${attempt} failed: ${lastErr}`);
          }
        }
        reports.push(`【${a.name}】执行异常: ${lastErr}`);
        trace(`${clock()} ✖ [${a.name}] 异常: ${lastErr}`);
      })());
    });
    await Promise.all(jobs);
    if (token.cancelled) {
      return '(已中断)';
    }

    // ③ 综合汇报（流式输出给用户）
    trace(`${clock()} 🧩 [汇总] 队长综合 ${reports.length} 份成员报告…`);
    const capped: string[] = [];
    reports.forEach((r: string) => {
      capped.push(r.length > 2500 ? r.substring(0, 2500) : r);
    });
    const final = await new Promise<string>((resolve: (v: string) => void) => {
      const runId = `team_${Date.now()}`;
      token.onCancel(() => {
        LlmClient.cancel(runId);
      });
      LlmClient.chatStream(profile, TeamOrchestrator.synthMessages(taskText, capped), null, {
        onThinking: (d: string) => {
          onThinking(d);
        },
        onContent: (d: string) => {
          onContent(d);
        },
        onToolCalls: (_c: ToolCallReq[]) => {
        },
        onDone: (full: string, _c: ToolCallReq[]) => {
          resolve(full);
        },
        onError: (msg: string) => {
          Log.error(`team synth error: ${msg}`);
          resolve(`(综合汇报失败: ${msg})\n\n各成员报告：\n${capped.join('\n\n')}`);
        }
      }, runId);
    });
    trace(`${clock()} ✔ [完成] 总耗时 ${((Date.now() - t0) / 1000).toFixed(1)}s`);
    bus.emit(Ev.AI_FINISHED, { 'agentName': '智能体团队', 'agentAvatar': '👥' } as Record<string, string>);
    return final;
  }

  /** 单成员执行超时保护：超时抛错触发重试/放弃，但不挂死整队 */
  private static withTimeout(p: Promise<LoopResult>, ms: number, label: string): Promise<LoopResult> {
    return new Promise<LoopResult>((resolve: (v: LoopResult) => void, reject: (e: Error) => void) => {
      const timer = setTimeout(() => {
        reject(new Error(`${label} 超时（${Math.round(ms / 1000)}s）`));
      }, ms);
      p.then((v: LoopResult) => {
        clearTimeout(timer);
        resolve(v);
      }).catch((e: Error) => {
        clearTimeout(timer);
        reject(e);
      });
    });
  }

  private static synthMessages(task: string, reports: string[]): LlmMessage[] {
    const sys = new LlmMessage();
    sys.role = 'system';
    sys.content = '你是智能体团队的队长。下面是队员们的工作报告，请综合成一份给用户的最终回复：' +
      '先一句话说明团队完成了什么，再分点呈现关键结果与建议，最后给出下一步建议。保持简洁。';
    const user = new LlmMessage();
    user.role = 'user';
    user.content = `任务：${task}\n\n成员报告：\n${reports.join('\n\n')}`;
    return [sys, user];
  }
}
