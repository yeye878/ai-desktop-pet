import { EvolutionManager } from '../core/EvolutionManager';
import { bus, Ev, BusEvent, BusListener } from '../commons/EventBus';
import { Ink, Radius } from '../ui/DesignTokens';
import { Chip, PillButton, SegmentedTabs, EmptyHint } from '../ui/Widgets';

/**
 * 进化中心（对应桌面版 EvolutionCenter：画像准则 / 进化提案 / 编年史 + 一键复盘）。
 * 视觉：等级徽章（双环）+ 分段标签 + 行式列表。
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
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentLight') accentLight: string = '#FFE3EA';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
  private onEvoChanged: BusListener = (_e: BusEvent) => {
    this.refresh();
  };

  aboutToAppear(): void {
    this.refresh();
    bus.on(Ev.EVOLUTION_CHANGED, this.onEvoChanged);
  }

  aboutToDisappear(): void {
    bus.off(Ev.EVOLUTION_CHANGED, this.onEvoChanged);
  }

  refresh(): void {
    EvolutionManager.summary().then((s: Record<string, Object>) => {
      this.level = typeof s['level'] === 'number' ? s['level'] as number : 1;
      this.levelTitle = typeof s['title'] === 'string' ? s['title'] as string : '初识';
      this.insightCount = typeof s['insights'] === 'number' ? s['insights'] as number : 0;
      this.logCount = typeof s['appliedLogs'] === 'number' ? s['appliedLogs'] as number : 0;
    }).catch((_e: Error) => {
    });
    EvolutionManager.listProposals().then((rows: Array<Map<string, Object>>) => {
      this.proposals = rows;
    }).catch((_e: Error) => {
    });
    EvolutionManager.listInsights().then((rows: Array<Map<string, Object>>) => {
      this.insights = rows;
    }).catch((_e: Error) => {
    });
    EvolutionManager.listLogs().then((rows: Array<Map<string, Object>>) => {
      this.logs = rows;
    }).catch((_e: Error) => {
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
    Column({ space: 12 }) {
      // 等级徽章卡（水平内缩由父级 padding 承担：width 100% + 自身 margin 会溢出）
      Column() {
      Column({ space: 10 }) {
      Row({ space: 14 }) {
        // 双环徽章
        Stack() {
          Circle({ width: 62, height: 62 })
            .fill(Color.Transparent)
            .stroke(this.accentTint)
            .strokeWidth(5)
          Circle({ width: 50, height: 50 })
            .fill(this.accentLight)
          Column({ space: 0 }) {
            Text(`Lv.${this.level}`)
              .fontSize(15)
              .fontWeight(FontWeight.Bold)
              .fontColor(this.accentDeep)
          }
          .justifyContent(FlexAlign.Center)
        }
        .width(62)
        .height(62)

        Column({ space: 3 }) {
          Text(this.levelTitle)
            .fontSize(18)
            .fontWeight(FontWeight.Bold)
            .fontColor(Ink.text)
        }
        .alignItems(HorizontalAlign.Start)
        .layoutWeight(1)

        PillButton({
          label: this.reflecting ? '复盘中…' : '立即复盘',
          icon: $r('sys.symbol.lightbulb'),
          variant: 'ink',
          heightVp: 38,
          fontSizeVp: 13,
          isEnabled: !this.reflecting,
          onTap: (): void => {
            this.reflect();
          }
        })
      }
      .width('100%')
      // 统计行：整行展示，避免在徽章与按钮之间的窄列内换行
      Text(`画像准则 ${this.insightCount} 条 · 进化操作 ${this.logCount} 次`)
        .fontSize(11.5)
        .fontColor(Ink.text3)
      }
      .width('100%')
      .padding(14)
      .backgroundColor(Ink.card)
      .borderRadius(Radius.card)
      .border({ width: 0.5, color: Ink.border })
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 10 })


      if (this.reflectMsg.length > 0) {
        Text(this.reflectMsg)
          .fontSize(12)
          .fontColor(this.reflectMsg.startsWith('复盘失败') || this.reflectMsg.startsWith('采纳失败') || this.reflectMsg.startsWith('拒绝失败') ? Ink.danger : Ink.ok)
          .width('100%')
          .padding({ left: 20, right: 20 })
      }

      Row({ space: 8 }) {
        Text('自动复盘')
          .fontSize(13)
          .fontColor(Ink.text)
        Text('检测到纠错或新偏好时自动反思')
          .fontSize(11)
          .fontColor(Ink.text3)
          .layoutWeight(1)
        Toggle({ type: ToggleType.Switch, isOn: this.autoOn })
          .selectedColor(this.accentDeep)
          .scale({ x: 0.85, y: 0.85 })
          .onChange((on: boolean) => {
            EvolutionManager.setAutoEnabled(on).then(() => {
              this.refresh();
            });
          })
      }
      .width('100%')
      .padding({ left: 20, right: 20 })

      // 分段标签
      Column() {
        SegmentedTabs({
          tabs: ['进化提案', '画像准则', '编年史'],
          active: this.tab,
          onPick: (idx: number): void => {
            this.tab = idx;
          }
        })
      }
      .padding({ left: 16, right: 16 })

      Scroll() {
        Column({ space: 8 }) {
          if (this.tab === 0) {
            if (this.proposals.length === 0) {
              EmptyHint({
                icon: $r('sys.symbol.wand_and_stars'),
                title: '暂无待处理的进化提案',
                sub: '多聊聊，或点「立即复盘」生成提案'
              })
            }
            ForEach(this.proposals, (p: Map<string, Object>) => {
              Column({ space: 8 }) {
                Row({ space: 8 }) {
                  Chip({ label: `${String(p.get('ptype'))}`, variant: 'accent' })
                  Text(String(p.get('name')))
                    .fontSize(14)
                    .fontWeight(FontWeight.Bold)
                    .fontColor(Ink.text)
                    .layoutWeight(1)
                }
                .width('100%')
                Text(String(p.get('payload')))
                  .fontSize(11.5)
                  .fontColor(Ink.text3)
                  .maxLines(3)
                  .width('100%')
                Row({ space: 8 }) {
                  Blank()
                  PillButton({
                    label: '采纳',
                    variant: 'soft',
                    heightVp: 30,
                    fontSizeVp: 12,
                    onTap: (): void => {
                      EvolutionManager.applyProposal(String(p.get('id'))).then((msg: string) => {
                        this.reflectMsg = msg;
                        this.refresh();
                      }).catch((e: Error) => {
                        this.reflectMsg = `采纳失败: ${e.message}`;
                      });
                    }
                  })
                  PillButton({
                    label: '拒绝',
                    variant: 'quiet',
                    heightVp: 30,
                    fontSizeVp: 12,
                    onTap: (): void => {
                      EvolutionManager.rejectProposal(String(p.get('id'))).then(() => {
                        this.refresh();
                      }).catch((e: Error) => {
                        this.reflectMsg = `拒绝失败: ${e.message}`;
                      });
                    }
                  })
                }
                .width('100%')
              }
              .width('100%')
              .padding(12)
              .backgroundColor(Ink.card)
              .borderRadius(Radius.card)
              .border({ width: 0.5, color: Ink.border })
            }, (p: Map<string, Object>) => String(p.get('id')))
          } else if (this.tab === 1) {
            if (this.insights.length === 0) {
              EmptyHint({
                icon: $r('sys.symbol.leaf'),
                title: '还没有画像准则',
                sub: '复盘后会在这里沉淀「它对你的理解」'
              })
            }
            ForEach(this.insights, (ins: Map<string, Object>) => {
              Row({ space: 8 }) {
                Chip({ label: `${String(ins.get('category'))}`, variant: 'sunken' })
                Text(String(ins.get('content')))
                  .fontSize(13)
                  .fontColor(Ink.text)
                  .layoutWeight(1)
                  .lineHeight(19)
                Text(`置信 ${Number(ins.get('confidence')).toFixed(2)} ×${Number(ins.get('hit_count'))}`)
                  .fontSize(10)
                  .fontColor(Ink.text3)
                  .fontFamily('monospace')
              }
              .width('100%')
              .alignItems(VerticalAlign.Top)
              .padding(12)
              .backgroundColor(Ink.card)
              .borderRadius(Radius.card)
              .border({ width: 0.5, color: Ink.border })
            }, (ins: Map<string, Object>) => String(ins.get('id')))
          } else {
            if (this.logs.length === 0) {
              EmptyHint({
                icon: $r('sys.symbol.clock'),
                title: '暂无进化记录'
              })
            }
            ForEach(this.logs, (log: Map<string, Object>) => {
              Row({ space: 8 }) {
                Text(String(log.get('action')))
                  .fontSize(11)
                  .fontWeight(FontWeight.Medium)
                  .fontColor(this.accentDeep)
                Text(new Date(Number(log.get('created_at'))).toLocaleString())
                  .fontSize(11)
                  .fontColor(Ink.text3)
                  .layoutWeight(1)
                  .textAlign(TextAlign.End)
                Text('回滚')
                  .fontSize(12)
                  .fontWeight(FontWeight.Medium)
                  .fontColor(Ink.warn)
                  .padding({ left: 8, right: 4, top: 4, bottom: 4 })
                  .onClick(() => {
                    // 回滚会撤销已沉淀的进化操作，先确认
                    AlertDialog.show({
                      message: `回滚这条进化记录（${String(log.get('action'))}）？`,
                      autoCancel: true,
                      alignment: DialogAlignment.Center,
                      primaryButton: {
                        value: '取消',
                        action: () => {
                        }
                      },
                      secondaryButton: {
                        value: '回滚',
                        fontColor: Ink.warn,
                        action: () => {
                          EvolutionManager.rollbackLog(String(log.get('id'))).then((msg: string) => {
                            this.reflectMsg = msg;
                            this.refresh();
                          }).catch((e: Error) => {
                            this.reflectMsg = `回滚失败: ${e.message}`;
                          });
                        }
                      }
                    });
                  })
              }
              .width('100%')
              .padding(12)
              .backgroundColor(Ink.card)
              .borderRadius(Radius.card)
              .border({ width: 0.5, color: Ink.border })
            }, (log: Map<string, Object>) => String(log.get('id')))
          }
        }
        .width('100%')
        .padding({ left: 16, right: 16, bottom: 16 })
      }
      .layoutWeight(1)
      .width('100%')
      .align(Alignment.Top)
      .scrollBar(BarState.Off)
      .edgeEffect(EdgeEffect.Spring)
    }
    .width('100%')
    .height('100%')
  }
}
