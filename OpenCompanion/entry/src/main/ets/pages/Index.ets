import { AppContext } from '../commons/AppContext';
import { bus, Ev, BusEvent } from '../commons/EventBus';
import { BehaviorEngine } from '../pet/BehaviorEngine';
import { ChatService } from '../core/ChatService';
import { AsrController } from '../voice/AsrController';
import { TtsController } from '../voice/TtsController';
import { HomePage } from './HomePage';
import { ChatPage } from './ChatPage';
import { KnowledgePage } from './KnowledgePage';
import { AgentsPage } from './AgentsPage';
import { EvolutionPage } from './EvolutionPage';
import { SettingsPanel } from './SettingsPage';

class NavItem {
  key: string = '';
  label: string = '';
  icon: string = '';

  constructor(key: string, label: string, icon: string) {
    this.key = key;
    this.label = label;
    this.icon = icon;
  }
}

@Entry
@Component
struct Index {
  @State page: string = 'home';
  @State wide: boolean = AppContext.vpWidth >= 720;
  @State showSettings: boolean = false;
  @State initDone: boolean = false;

  aboutToAppear(): void {
    bus.on(Ev.NAVIGATE, (e: BusEvent) => {
      const d = e.data as Record<string, string> | null;
      if (d !== null && d !== undefined) {
        const target = d['page'];
        if (target !== undefined) {
          this.page = target;
        }
      }
    });
    bus.on(Ev.VP_WIDTH, (_e: BusEvent) => {
      this.wide = AppContext.vpWidth >= 720;
    });
    // 首帧后延迟初始化（冷启动优化：首帧前不做任何重初始化）
    AppContext.postFrame(() => {
      AppContext.initSettings().then(() => {
        return ChatService.init();
      }).then(() => {
        BehaviorEngine.start();
        AsrController.preheat();
        TtsController.preheat();
        this.initDone = true;
      }).catch((_e: Error) => {
        this.initDone = true;
      });
    }, 60);
  }

  private navItems(): NavItem[] {
    return [
      new NavItem('home', '伙伴', '🐹'),
      new NavItem('chat', '对话', '💬'),
      new NavItem('kb', '知识库', '📚'),
      new NavItem('agents', '智能体', '🤖'),
      new NavItem('evo', '进化', '🌱')
    ];
  }

  @Builder
  PageBody() {
    if (this.page === 'chat') {
      ChatPage()
    } else if (this.page === 'kb') {
      KnowledgePage()
    } else if (this.page === 'agents') {
      AgentsPage()
    } else if (this.page === 'evo') {
      EvolutionPage()
    } else {
      HomePage()
    }
  }

  build() {
    Stack() {
      Column() {
        // 顶栏
        Row() {
          Text('🐹 智能伙伴')
            .fontSize(18)
            .fontWeight(FontWeight.Bold)
            .fontColor('#5B4A4F')
          Blank()
          Text(BehaviorEngine.moodEmoji())
            .fontSize(20)
          Button('⚙')
            .type(ButtonType.Circle)
            .backgroundColor('#FFE3EA')
            .fontColor('#C2557E')
            .width(34)
            .height(34)
            .onClick(() => {
              this.showSettings = true;
            })
        }
        .width('100%')
        .height(48)
        .padding({ left: 16, right: 12 })

        // 主内容
        Row() {
          if (this.wide) {
            Column({ space: 4 }) {
              ForEach(this.navItems(), (item: NavItem) => {
                Row() {
                  Text(item.icon).fontSize(20)
                  Text(item.label)
                    .fontSize(14)
                    .fontColor(this.page === item.key ? '#C2557E' : '#8A7A80')
                    .fontWeight(this.page === item.key ? FontWeight.Bold : FontWeight.Normal)
                }
                .width('100%')
                .height(44)
                .borderRadius(12)
                .justifyContent(FlexAlign.Start)
                .padding({ left: 14 })
                .backgroundColor(this.page === item.key ? '#FFE3EA' : Color.Transparent)
                .onClick(() => {
                  this.page = item.key;
                })
              }, (item: NavItem) => item.key)
            }
            .width(120)
            .height('100%')
            .padding(8)

            Column() {
              this.PageBody()
            }
            .layoutWeight(1)
            .height('100%')
          } else {
            Column() {
              this.PageBody()
            }
            .layoutWeight(1)
            .width('100%')
          }
        }
        .layoutWeight(1)
        .width('100%')

        // 手机底部导航
        if (!this.wide) {
          Row() {
            ForEach(this.navItems(), (item: NavItem) => {
              Column({ space: 2 }) {
                Text(item.icon).fontSize(20)
                Text(item.label)
                  .fontSize(10)
                  .fontColor(this.page === item.key ? '#C2557E' : '#8A7A80')
              }
              .layoutWeight(1)
              .height(52)
              .justifyContent(FlexAlign.Center)
              .onClick(() => {
                this.page = item.key;
              })
            }, (item: NavItem) => item.key)
          }
          .width('100%')
          .height(56)
          .backgroundColor('#FFFFFF')
          .border({ width: { top: 0.5 }, color: '#F0E4E8' })
        }
      }
      .width('100%')
      .height('100%')
      .backgroundColor('#FDF8F6')

      // 设置面板（浮层）
      if (this.showSettings) {
        this.SettingsOverlay()
      }
    }
    .width('100%')
    .height('100%')
  }

  @Builder
  SettingsOverlay() {
    Column() {
      SettingsPanel({
        onClose: () => {
          this.showSettings = false;
        }
      })
    }
    .width(this.wide ? '72%' : '100%')
    .height('92%')
    .backgroundColor('#FFFFFF')
    .borderRadius(20)
    .clip(true)
  }
}
