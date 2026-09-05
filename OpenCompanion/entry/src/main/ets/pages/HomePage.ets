import { BehaviorEngine } from '../pet/BehaviorEngine';
import { ThemeEngine } from '../pet/ThemeEngine';
import { PetCanvas } from '../pet/PetCanvas';
import { ChatService } from '../core/ChatService';
import { bus, Ev, BusEvent } from '../commons/EventBus';
import { LlmProfiles } from '../core/LlmClient';
import { KbService } from '../knowledge/KbService';

/**
 * 首页：桌宠伙伴 + 情绪 + 快捷操作 + 状态卡片
 */
@Component
export struct HomePage {
  @State moodText: string = '今天也要元气满满哦';
  @State stateLabel: string = '待机中';
  @State apiReady: boolean = false;
  @State kbReady: boolean = false;
  @State tick: number = 0;
  private timer: number = -1;

  aboutToAppear(): void {
    this.refresh();
    this.timer = setInterval(() => {
      this.refresh();
      this.tick++;
    }, 1500);
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
      'thinking': '思考中…', 'speaking': '说话中', 'confused': '有点懵', 'sad': '有点低落'
    };
    this.stateLabel = stateMap[BehaviorEngine.state] ?? '待机中';
    this.apiReady = LlmProfiles.list().length > 0;
    this.kbReady = KbService.isKitReady();
  }

  build() {
    Scroll() {
      Column({ space: 14 }) {
        // 桌宠
        Column({ space: 10 }) {
          PetCanvas({ sizeVp: 210 })
          Text(this.moodText)
            .fontSize(15)
            .fontColor('#8A7A80')
          Row({ space: 8 }) {
            Text(`${BehaviorEngine.moodEmoji()} ${this.stateLabel}`)
              .fontSize(13)
              .fontColor('#C2557E')
              .backgroundColor('#FFE3EA')
              .borderRadius(10)
              .padding({ left: 10, right: 10, top: 4, bottom: 4 })
            Text(`❤️ ${Math.round((BehaviorEngine.happiness + 1) * 50)}%`)
              .fontSize(13)
              .fontColor('#C2557E')
              .backgroundColor('#FFE3EA')
              .borderRadius(10)
              .padding({ left: 10, right: 10, top: 4, bottom: 4 })
            Text(`⚡ ${Math.round(BehaviorEngine.energy)}%`)
              .fontSize(13)
              .fontColor('#8A6A1F')
              .backgroundColor('#FFF3D6')
              .borderRadius(10)
              .padding({ left: 10, right: 10, top: 4, bottom: 4 })
          }
        }
        .width('100%')
        .margin({ top: 6 })

        // 快捷操作
        Row({ space: 10 }) {
          Button('👋 打招呼')
            .onClick(() => {
              BehaviorEngine.greet();
              this.refresh();
            })
          Button('🖐 摸摸头')
            .onClick(() => {
              BehaviorEngine.pet();
              this.refresh();
            })
          Button(BehaviorEngine.state === 'sleeping' ? '⏰ 唤醒' : '😴 睡觉')
            .onClick(() => {
              if (BehaviorEngine.state === 'sleeping') {
                BehaviorEngine.wake();
              } else {
                BehaviorEngine.sleepNow();
              }
              this.refresh();
            })
        }
        .width('100%')
        .justifyContent(FlexAlign.Center)

        // 主要入口
        Row({ space: 10 }) {
          Button('💬 开始对话')
            .backgroundColor('#F76C8A')
            .onClick(() => {
              bus.emit(Ev.NAVIGATE, { 'page': 'chat' } as Record<string, string>);
            })
          Button('📚 知识库')
            .backgroundColor('#FFD166')
            .fontColor('#5B4A1F')
            .onClick(() => {
              bus.emit(Ev.NAVIGATE, { 'page': 'kb' } as Record<string, string>);
            })
          Button('🤖 智能体')
            .backgroundColor('#9D7BFF')
            .onClick(() => {
              bus.emit(Ev.NAVIGATE, { 'page': 'agents' } as Record<string, string>);
            })
        }
        .width('100%')
        .justifyContent(FlexAlign.Center)

        // 状态卡
        Column({ space: 6 }) {
          Text('状态自检').fontSize(13).fontColor('#8A7A80').width('100%')
          Row({ space: 8 }) {
            Text(this.apiReady ? '✅ LLM API 已配置' : '⚠️ 未配置 LLM API（设置 → API）')
              .fontSize(13)
              .fontColor(this.apiReady ? '#3E8E5A' : '#B0670A')
            Text(this.kbReady ? '✅ 鸿蒙 RAG 检索可用' : 'ℹ️ 知识库用本地检索')
              .fontSize(13)
              .fontColor(this.kbReady ? '#3E8E5A' : '#8A7A80')
          }
          .width('100%')
          Text('复合指令示例：拍一下这个文档，帮我总结重点（对话页 → 📷 → 发送）')
            .fontSize(12)
            .fontColor('#8A7A80')
            .width('100%')
        }
        .width('100%')
        .padding(12)
        .borderRadius(14)
        .backgroundColor('#FFFFFF')
      }
      .width('100%')
      .padding(16)
      .constraintSize({ minHeight: '100%' })
    }
    .width('100%')
    .height('100%')
    .scrollBar(BarState.Off)
  }
}
