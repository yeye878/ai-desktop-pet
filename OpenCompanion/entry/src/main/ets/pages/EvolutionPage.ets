import { EvolutionManager } from '../core/EvolutionManager';
import { bus, Ev, BusEvent } from '../commons/EventBus';

/**
 * 进化中心（对应桌面版 EvolutionCenter：画像准则 / 进化提案 / 编年史 + 一键复盘）
 */
@Component
export struct EvolutionPage {
  @State level: number = 1;
  @State levelTitle: string = '初识';
  @State insightCount: number = 0;
  @State logCount: number = 0;
  @State reflecting: boolean = false;
  @State reflectMsg: string = '';
  @State tab: number = 0;   // 0 提案 1 准则 2 编年史
  @State proposals: Array<Map<string, Object>> = [];
  @State insights: Array<Map<string, Object>> = [];
  @State logs: Array<Map<string, Object>> = [];
  @State autoOn: boolean = true;

  aboutToAppear(): void {
    this.refresh();
    bus.on(Ev.EVOLUTION_CHANGED, (_e: BusEvent) => {
      this.refresh();
    });
  }

  refresh(): void {
    EvolutionManager.summary().then((s: Record<string, Object>) => {
      this.level = typeof s['level'] === 'number' ? s['level'] as number : 1;
      this.levelTitle = typeof s['title'] === 'string' ? s['title'] as string : '初识';
      this.insightCount = typeof s['insights'] === 'number' ? s['insights'] as number : 0;
      this.logCount = typeof s['appliedLogs'] === 'number' ? s['appliedLogs'] as number : 0;
    });
    EvolutionManager.listProposals().then((rows: Array<Map<string, Object>>) => {
      this.proposals = rows;
    });
    EvolutionManager.listInsights().then((rows: Array<Map<string, Object>>) => {
      this.insights = rows;
    });
    EvolutionManager.listLogs().then((rows: Array<Map<string, Object>>) => {
      this.logs = rows;
    });
    this.autoOn = EvolutionManager.isAutoEnabled();
  }

  reflect(): void {
    if (this.reflecting) {
      return;
    }
    this.reflecting = true;
    this.reflectMsg = '正在回顾我们的对话…';
    EvolutionManager.reflect().then((msg: string) => {
      this.reflecting = false;
      this.reflectMsg = msg;
      this.refresh();
    }).catch((e: Error) => {
      this.reflecting = false;
      this.reflectMsg = `复盘失败: ${e.message}`;
    });
  }

  build() {
    Column({ space: 10 }) {
      // 等级卡
      Row({ space: 12 }) {
        Text('🌱').fontSize(36)
        Column({ space: 2 }) {
          Text(`Lv.${this.level} ${this.levelTitle}`)
            .fontSize(18)
            .fontWeight(FontWeight.Bold)
            .fontColor('#3D2E33')
          Text(`画像准则 ${this.insightCount} 条 · 进化操作 ${this.logCount} 次`)
            .fontSize(12)
            .fontColor('#8A7A80')
        }
        .alignItems(HorizontalAlign.Start)
        .layoutWeight(1)
        Button(this.reflecting ? '复盘中…' : '立即复盘')
          .backgroundColor('#6FCF97')
          .onClick(() => {
            this.reflect();
          })
      }
      .width('100%')
      .padding(14)
      .backgroundColor('#FFFFFF')
      .borderRadius(16)
      .margin({ top: 8, left: 12, right: 12 })

      if (this.reflectMsg.length > 0) {
        Text(this.reflectMsg)
          .fontSize(12)
          .fontColor('#3E8E5A')
          .width('100%')
          .padding({ left: 16, right: 16 })
      }

      Row({ space: 8 }) {
        Text('自动复盘').fontSize(13).fontColor('#5B4A4F')
        Toggle({ type: ToggleType.Switch, isOn: this.autoOn })
          .selectedColor('#F76C8A')
          .onChange((on: boolean) => {
            EvolutionManager.setAutoEnabled(on).then(() => {
              this.refresh();
            });
          })
        Text('（检测到纠错/新偏好时自动反思）').fontSize(11).fontColor('#B9A9AF')
      }
      .width('100%')
      .padding({ left: 16 })

      // Tab
      Row({ space: 8 }) {
        ForEach(['进化提案', '画像准则', '编年史'], (label: string, idx: number) => {
          Text(label)
            .fontSize(14)
            .fontColor(this.tab === idx ? '#C2557E' : '#8A7A80')
            .fontWeight(this.tab === idx ? FontWeight.Bold : FontWeight.Normal)
            .backgroundColor(this.tab === idx ? '#FFE3EA' : Color.Transparent)
            .borderRadius(12)
            .padding({ left: 12, right: 12, top: 5, bottom: 5 })
            .onClick(() => {
              this.tab = idx;
            })
        }, (label: string, idx: number) => `${idx}_${label}`)
      }
      .width('100%')
      .padding({ left: 12 })

      Scroll() {
        Column({ space: 8 }) {
          if (this.tab === 0) {
            if (this.proposals.length === 0) {
              Text('暂无待处理的进化提案。多聊聊，或点「立即复盘」。').fontSize(12).fontColor('#B9A9AF')
            }
            ForEach(this.proposals, (p: Map<string, Object>) => {
              Column({ space: 6 }) {
                Row({ space: 6 }) {
                  Text(`${String(p.get('ptype'))}`).fontSize(12).fontColor('#C2557E').backgroundColor('#FFE3EA').borderRadius(6).padding({ left: 6, right: 6, top: 2, bottom: 2 })
                  Text(String(p.get('name'))).fontSize(14).fontWeight(FontWeight.Bold).fontColor('#3D2E33')
                  Blank()
                  Button('采纳').height(28).backgroundColor('#6FCF97').onClick(() => {
                    EvolutionManager.applyProposal(String(p.get('id'))).then((msg: string) => {
                      this.reflectMsg = msg;
                      this.refresh();
                    });
                  })
                  Button('拒绝').height(28).backgroundColor('#F0E4E8').fontColor('#8A7A80').onClick(() => {
                    EvolutionManager.rejectProposal(String(p.get('id'))).then(() => {
                      this.refresh();
                    });
                  })
                }
                .width('100%')
                Text(String(p.get('payload'))).fontSize(11).fontColor('#8A7A80').maxLines(3)
              }
              .width('100%')
              .padding(10)
              .backgroundColor('#FFFFFF')
              .borderRadius(12)
            }, (p: Map<string, Object>) => String(p.get('id')))
          } else if (this.tab === 1) {
            if (this.insights.length === 0) {
              Text('还没有画像准则。复盘后会在这里沉淀「它对你的理解」。').fontSize(12).fontColor('#B9A9AF')
            }
            ForEach(this.insights, (ins: Map<string, Object>) => {
              Row({ space: 8 }) {
                Text(`(${String(ins.get('category'))})`).fontSize(10).fontColor('#C2557E')
                Text(String(ins.get('content'))).fontSize(13).fontColor('#3D2E33').layoutWeight(1)
                Text(`置信 ${Number(ins.get('confidence')).toFixed(2)} ×${Number(ins.get('hit_count'))}`)
                  .fontSize(10)
                  .fontColor('#B9A9AF')
              }
              .width('100%')
              .padding(10)
              .backgroundColor('#FFFFFF')
              .borderRadius(12)
            }, (ins: Map<string, Object>) => String(ins.get('id')))
          } else {
            if (this.logs.length === 0) {
              Text('暂无进化记录。').fontSize(12).fontColor('#B9A9AF')
            }
            ForEach(this.logs, (log: Map<string, Object>) => {
              Row({ space: 8 }) {
                Text(String(log.get('action'))).fontSize(11).fontColor('#6FCF97')
                Text(new Date(Number(log.get('created_at'))).toLocaleString())
                  .fontSize(11)
                  .fontColor('#8A7A80')
                  .layoutWeight(1)
                Text('回滚').fontSize(12).fontColor('#B0670A').onClick(() => {
                  EvolutionManager.rollbackLog(String(log.get('id'))).then((msg: string) => {
                    this.reflectMsg = msg;
                    this.refresh();
                  });
                })
              }
              .width('100%')
              .padding(10)
              .backgroundColor('#FFFFFF')
              .borderRadius(12)
            }, (log: Map<string, Object>) => String(log.get('id')))
          }
        }
        .width('100%')
        .padding({ left: 12, right: 12, bottom: 12 })
      }
      .layoutWeight(1)
      .width('100%')
      .scrollBar(BarState.Off)
    }
    .width('100%')
    .height('100%')
  }
}
