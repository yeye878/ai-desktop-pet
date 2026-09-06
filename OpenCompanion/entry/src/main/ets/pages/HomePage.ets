import { BehaviorEngine } from '../pet/BehaviorEngine';
import { PetCanvas } from '../pet/PetCanvas';
import { bus, Ev } from '../commons/EventBus';
import { PerfTrace } from '../commons/PerfTrace';
import { LlmProfiles } from '../core/LlmClient';
import { KbService } from '../knowledge/KbService';
import { Ink, Radius, Shadow } from '../ui/DesignTokens';
import { AppIcon, Card, Chip, PillButton, StatusLine } from '../ui/Widgets';

/**
 * 首页：桌宠主视觉（呼吸光环）+ 情绪状态 + 快捷操作 + 运行状态自检。
 */
@Component
export struct HomePage {
  @State moodText: string = '今天也要元气满满哦';
  @State stateLabel: string = '待机中';
  @State apiReady: boolean = false;
  @State kbReady: boolean = false;
  @State coldMs: number = -1;
  @State tick: number = 0;
  @State breathScale: number = 1;
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentLight') accentLight: string = '#FFE3EA';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
  private timer: number = -1;

  aboutToAppear(): void {
    this.refresh();
    this.timer = setInterval(() => {
      this.refresh();
      this.tick++;
    }, 1500);
    // 呼吸光环：无限往复缩放（纯 transform，不触发布局）
    setTimeout(() => {
      animateTo({
        duration: 2600,
        iterations: -1,
        playMode: PlayMode.Alternate,
        curve: Curve.EaseInOut
      }, () => {
        this.breathScale = 1.12;
      });
    }, 200);
  }

  aboutToDisappear(): void {
    if (this.timer >= 0) {
      clearInterval(this.timer);
    }
  }

  private refresh(): void {
    this.moodText = BehaviorEngine.moodText;
    const stateMap: Record<string, string> = {
      'idle': '待机中', 'waving': '和你打招呼', 'happy': '很开心', 'sleeping': '睡得香',
      'thinking': '思考中', 'speaking': '说话中', 'confused': '有点懵', 'sad': '有点低落'
    };
    this.stateLabel = stateMap[BehaviorEngine.state] ?? '待机中';
    this.apiReady = LlmProfiles.list().length > 0;
    this.kbReady = KbService.isKitReady();
    this.coldMs = PerfTrace.lastMs('冷启动·可交互');
  }

  private go(page: string): void {
    bus.emit(Ev.NAVIGATE, { 'page': page } as Record<string, string>);
  }

  private say(text: string): void {
    bus.emit(Ev.NAVIGATE, { 'page': 'chat', 'draft': text } as Record<string, string>);
  }

  build() {
    Scroll() {
      Column({ space: 14 }) {
        // ---- 主视觉：呼吸光环 + 桌宠 ----
        Column({ space: 12 }) {
          Stack() {
            // 氛围光
            Circle({ width: 216, height: 216 })
              .fill(this.accentTint)
              .scale({ x: this.breathScale, y: this.breathScale })
            Circle({ width: 168, height: 168 })
              .fill(this.accentLight)
              .opacity(0.55)
            // 呼吸细环
            Circle({ width: 196, height: 196 })
              .fill(Color.Transparent)
              .stroke(this.accentTint)
              .strokeWidth(1.5)
              .scale({ x: this.breathScale, y: this.breathScale })
            PetCanvas({ sizeVp: 190 })
          }
          .width(232)
          .height(220)

          Text(this.moodText)
            .fontSize(16)
            .fontWeight(FontWeight.Medium)
            .fontColor(Ink.text)
            .textAlign(TextAlign.Center)

          Row({ space: 6 }) {
            Chip({ label: this.stateLabel, variant: 'accent' })
            Chip({ label: `${Math.round((BehaviorEngine.happiness + 1) * 50)}%`, icon: $r('sys.symbol.heart_fill'), variant: 'accent' })
            Chip({ label: `${Math.round(BehaviorEngine.energy)}%`, icon: $r('sys.symbol.bolt_fill'), variant: 'sunken' })
          }
        }
        .width('100%')
        .margin({ top: 4 })

        // ---- 快捷互动 ----
        Row({ space: 10 }) {
          this.QuickAction($r('sys.symbol.hand'), '打招呼', (): void => {
            BehaviorEngine.greet();
            this.refresh();
          })
          this.QuickAction($r('sys.symbol.hand_point_up_tap'), '摸摸头', (): void => {
            BehaviorEngine.pet();
            this.refresh();
          })
          this.QuickAction(
            BehaviorEngine.state === 'sleeping' ? $r('sys.symbol.sun_max') : $r('sys.symbol.moon_fill'),
            BehaviorEngine.state === 'sleeping' ? '唤醒' : '睡觉',
            (): void => {
              if (BehaviorEngine.state === 'sleeping') {
                BehaviorEngine.wake();
              } else {
                BehaviorEngine.sleepNow();
              }
              this.refresh();
            }
          )
        }
        .width('100%')
        .justifyContent(FlexAlign.Center)

        // ---- 主要入口 ----
        Column({ space: 10 }) {
          PillButton({
            label: '开始对话',
            icon: $r('sys.symbol.message_fill'),
            variant: 'ink',
            heightVp: 48,
            onTap: (): void => {
              this.go('chat');
            }
          })
          Row({ space: 10 }) {
            PillButton({
              label: '知识库',
              fill: true,
              icon: $r('sys.symbol.doc_text'),
              variant: 'soft',
              heightVp: 40,
              fontSizeVp: 13.5,
              onTap: (): void => {
                this.go('kb');
              }
            })
              .layoutWeight(1)
            PillButton({
              label: '智能体',
              icon: $r('sys.symbol.person_2'),
              variant: 'soft',
              heightVp: 40,
              fontSizeVp: 13.5,
              fill: true,
              onTap: (): void => {
                this.go('agents');
              }
            })
              .layoutWeight(1)
          }
          .width('100%')
        }
        .width('100%')

        // ---- 试试对它说（可点，直接带入对话） ----
        Card({ padVp: 14 }) {
          Column({ space: 10 }) {
            Text('试试对它说')
              .fontSize(13)
              .fontWeight(FontWeight.Medium)
              .fontColor(Ink.text2)
              .width('100%')
            this.SayChip('@学习助手 帮我复习一下重点概念')
            this.SayChip('拍一下这个文档，帮我总结重点')
          }
          .width('100%')
        }

        // ---- 运行状态 ----
        Card({ padVp: 16 }) {
          Column({ space: 2 }) {
            Text('运行状态')
              .fontSize(13)
              .fontWeight(FontWeight.Medium)
              .fontColor(Ink.text2)
              .width('100%')
              .padding({ bottom: 6 })
            StatusLine({
              icon: $r('sys.symbol.link'),
              label: 'LLM API',
              value: this.apiReady ? '已配置' : '去设置配置',
              tone: this.apiReady ? 'ok' : 'warn'
            })
            StatusLine({
              icon: $r('sys.symbol.doc_text'),
              label: '端侧 RAG 检索',
              value: this.kbReady ? '可用' : '本地检索兜底',
              tone: this.kbReady ? 'ok' : 'muted'
            })
            if (this.coldMs >= 0) {
              StatusLine({
                icon: $r('sys.symbol.hourglass'),
                label: '冷启动·可交互',
                value: `${(this.coldMs / 1000).toFixed(2)}s`,
                tone: this.coldMs <= 3000 ? 'ok' : 'warn'
              })
            }
          }
          .width('100%')
        }
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 4 })
      .constraintSize({ minHeight: '100%' })
    }
    .width('100%')
    .height('100%')
    .scrollBar(BarState.Off)
  }

  @Builder
  QuickAction(icon: Resource, label: string, action: () => void) {
    Column({ space: 6 }) {
      Column() {
        AppIcon({ icon: icon, sizeVp: 20, color: Ink.text2, weight: 600 })
      }
      .width(52)
      .height(52)
      .borderRadius(26)
      .backgroundColor(Ink.card)
      .border({ width: 0.5, color: Ink.border })
      .shadow(Shadow.hair)
      .justifyContent(FlexAlign.Center)
      .alignItems(HorizontalAlign.Center)
      .onClick(() => {
        action();
      })
      Text(label)
        .fontSize(11)
        .fontColor(Ink.text3)
    }
    .alignItems(HorizontalAlign.Center)
  }

  @Builder
  SayChip(text: string) {
    Row({ space: 6 }) {
      Text(text)
        .fontSize(13)
        .fontColor(Ink.text)
        .layoutWeight(1)
      AppIcon({ icon: $r('sys.symbol.arrow_right'), sizeVp: 14, color: this.accentDeep, weight: 600 })
    }
    .width('100%')
    .padding({ left: 12, right: 12, top: 10, bottom: 10 })
    .borderRadius(Radius.inner)
    .backgroundColor(Ink.sunken)
    .onClick(() => {
      this.say(text);
    })
  }
}
