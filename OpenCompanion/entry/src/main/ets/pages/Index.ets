import { AppContext } from '../commons/AppContext';
import { RdbHelper } from '../commons/RdbHelper';
import { PerfTrace } from '../commons/PerfTrace';
import { DistributedSync } from '../commons/DistributedSync';
import { Log } from '../commons/Log';
import { bus, Ev, BusEvent } from '../commons/EventBus';
import { BehaviorEngine } from '../pet/BehaviorEngine';
import { ChatService } from '../core/ChatService';
import { AsrController } from '../voice/AsrController';
import { TtsController } from '../voice/TtsController';
import { RagBridge } from '../knowledge/RagBridge';
import { AccentBridge, Ink, Radius, Shadow } from '../ui/DesignTokens';
import { AppIcon, IconButton, SheetOverlay } from '../ui/Widgets';
import { HomePage } from './HomePage';
import { ChatPage } from './ChatPage';
import { KnowledgePage } from './KnowledgePage';
import { AgentsPage } from './AgentsPage';
import { EvolutionPage } from './EvolutionPage';
import { SettingsPanel } from './SettingsPage';

class NavItem {
  key: string = '';
  label: string = '';
  icon: Resource = $r('sys.symbol.circle');
  iconFill: Resource = $r('sys.symbol.circle_fill');

  constructor(key: string, label: string, icon: Resource, iconFill: Resource) {
    this.key = key;
    this.label = label;
    this.icon = icon;
    this.iconFill = iconFill;
  }
}

/**
 * 应用外壳：墨色顶栏 + 浮动胶囊导航（手机）/ 侧栏（平板）+ 底部抽屉式设置。
 * 视觉语言见 ui/DesignTokens.ets。
 */
@Entry
@Component
struct Index {
  @State page: string = 'home';
  @State wide: boolean = AppContext.vpWidth >= 720;
  @State showSettings: boolean = false;
  @State initDone: boolean = false;
  @State mood: string = BehaviorEngine.moodEmoji();
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
  private moodTimer: number = -1;

  aboutToAppear(): void {
    AccentBridge.defaults();
    AccentBridge.sync();
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
    this.moodTimer = setInterval(() => {
      this.mood = BehaviorEngine.moodEmoji();
    }, 3000);
    // 首帧后延迟初始化（冷启动优化：首帧前不做任何重初始化）
    AppContext.postFrame(() => {
      // RdbHelper 必须最先初始化：ChatService.init → AgentRepo/loadHistory 直接查主库，
      // 缺了这一步整个初始化链静默失败（发消息不显示、页面刷新 unhandled rejection）
      RdbHelper.init().then(() => {
        return AppContext.initSettings();
      }).then(() => {
        return ChatService.init();
      }).then(() => {
        BehaviorEngine.start();
        AsrController.preheat();
        TtsController.preheat();
        this.initDone = true;
        PerfTrace.mark('interactive');
        PerfTrace.measure('冷启动·可交互', 'ability_created', 'interactive');
        // idle 阶段预热：RAG 会话池 + 分布式同步（都有静默降级，不影响主链路）
        AppContext.postFrame(() => {
          RagBridge.preheat();
          DistributedSync.ensure();
        }, 600);
      }).catch((e: Error) => {
        Log.error(`init chain failed: ${e.message}`);
        this.initDone = true;
      });
    }, 60);
  }

  aboutToDisappear(): void {
    if (this.moodTimer >= 0) {
      clearInterval(this.moodTimer);
    }
  }

  private navItems(): NavItem[] {
    return [
      new NavItem('home', '伙伴', $r('sys.symbol.house'), $r('sys.symbol.house_fill')),
      new NavItem('chat', '对话', $r('sys.symbol.message'), $r('sys.symbol.message_fill')),
      new NavItem('kb', '知识库', $r('sys.symbol.doc_text'), $r('sys.symbol.doc_text_fill')),
      new NavItem('agents', '智能体', $r('sys.symbol.person_2'), $r('sys.symbol.person_2_fill')),
      new NavItem('evo', '进化', $r('sys.symbol.leaf'), $r('sys.symbol.leaf_fill'))
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
        Row({ space: 10 }) {
          Text('OC')
            .fontSize(11)
            .fontWeight(FontWeight.Bold)
            .fontColor(Ink.onInk)
            .width(26)
            .height(26)
            .textAlign(TextAlign.Center)
            .borderRadius(8)
            .backgroundColor(Ink.text)
          Text('智能伙伴')
            .fontSize(17)
            .fontWeight(FontWeight.Bold)
            .fontColor(Ink.text)
            .layoutWeight(1)
          Text(this.mood)
            .fontSize(14)
            .width(32)
            .height(32)
            .textAlign(TextAlign.Center)
            .borderRadius(Radius.pill)
            .backgroundColor(Ink.card)
            .border({ width: 0.5, color: Ink.border })
          IconButton({
            icon: $r('sys.symbol.gearshape'),
            sizeVp: 34,
            iconVp: 17,
            bg: Ink.card,
            color: Ink.text2,
            onTap: () => {
              this.showSettings = true;
            }
          })
        }
        .width('100%')
        .height(56)
        .padding({ left: 16, right: 12 })

        if (this.wide) {
          Row() {
            // 侧栏
            Column({ space: 4 }) {
              ForEach(this.navItems(), (item: NavItem) => {
                Row({ space: 10 }) {
                  AppIcon({
                    icon: this.page === item.key ? item.iconFill : item.icon,
                    sizeVp: 19,
                    color: this.page === item.key ? this.accentDeep : Ink.text3
                  })
                  Text(item.label)
                    .fontSize(14)
                    .fontColor(this.page === item.key ? Ink.text : Ink.text2)
                    .fontWeight(this.page === item.key ? FontWeight.Medium : FontWeight.Normal)
                }
                .width('100%')
                .height(42)
                .borderRadius(Radius.inner)
                .padding({ left: 14 })
                .backgroundColor(this.page === item.key ? Ink.card : Color.Transparent)
                .animation({ duration: 220, curve: Curve.EaseOut })
                .onClick(() => {
                  this.page = item.key;
                })
              }, (item: NavItem) => item.key)
            }
            .width(172)
            .height('100%')
            .padding({ left: 12, right: 12, top: 6 })

            Column() {
              this.PageBody()
            }
            .layoutWeight(1)
            .height('100%')
          }
          .layoutWeight(1)
          .width('100%')
        } else {
          Column() {
            this.PageBody()
          }
          .layoutWeight(1)
          .width('100%')
          .padding({ bottom: 86 })
        }
      }
      .width('100%')
      .height('100%')
      .backgroundColor(Ink.bg)

      // 手机底部：浮动胶囊导航
      if (!this.wide) {
        Row({ space: 2 }) {
          ForEach(this.navItems(), (item: NavItem) => {
            Column({ space: 3 }) {
              AppIcon({
                icon: this.page === item.key ? item.iconFill : item.icon,
                sizeVp: 21,
                color: this.page === item.key ? this.accentDeep : Ink.text3,
                weight: this.page === item.key ? 700 : 500
              })
              Text(item.label)
                .fontSize(10)
                .fontColor(this.page === item.key ? Ink.text : Ink.text3)
                .fontWeight(this.page === item.key ? FontWeight.Medium : FontWeight.Normal)
            }
            .layoutWeight(1)
            .height(54)
            .justifyContent(FlexAlign.Center)
            .animation({ duration: 220, curve: Curve.EaseOut })
            .onClick(() => {
              this.page = item.key;
            })
          }, (item: NavItem) => item.key)
        }
        .width('72%')
        .padding({ left: 6, right: 6, top: 4, bottom: 4 })
        .backgroundColor(Ink.card)
        .borderRadius(Radius.pill)
        .border({ width: 0.5, color: Ink.border })
        .shadow(Shadow.soft)
        .margin({ bottom: 14 })
      }

      // 设置（底部抽屉 / 居中卡片，面板自带纱罩与关闭逻辑）
      if (this.showSettings) {
        SettingsPanel({
          onClose: () => {
            this.showSettings = false;
          }
        })
      }
    }
    .width('100%')
    .height('100%')
    .alignContent(Alignment.Bottom)
  }
}
