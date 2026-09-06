import { bus, Ev, BusEvent, BusListener } from '../commons/EventBus';
import { KbService } from '../knowledge/KbService';
import { DocImporter } from '../knowledge/DocImporter';
import { KbHit } from '../knowledge/KbService';
import { AppContext } from '../commons/AppContext';
import { CancelToken } from '../core/Types';
import { IntentEngine } from '../core/IntentEngine';
import { Ink, Radius } from '../ui/DesignTokens';
import { AppIcon, EmptyHint, IconButton, PillButton } from '../ui/Widgets';

interface DocRow {
  id: string;
  title: string;
  source: string;
  chars: number;
}

/**
 * 知识库页（对应命题 F2 端侧 RAG）：
 * 检索问答（RAG 会话 / 端侧离线模式）→ 引用来源；文档导入（文件 / 图片 OCR）。
 */
@Component
export struct KnowledgePage {
  @State docs: DocRow[] = [];
  @State question: string = '';
  @State answer: string = '';
  @State sources: KbHit[] = [];
  @State answering: boolean = false;
  @State statusLine: string = '';
  @State offlineMode: boolean = AppContext.getBool('kb_offline_mode', false);
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
  private askToken: CancelToken | null = null;
  private onKbChanged: BusListener = (_e: BusEvent) => {
    this.refresh();
  };

  aboutToAppear(): void {
    this.refresh();
    bus.on(Ev.KB_CHANGED, this.onKbChanged);
  }

  aboutToDisappear(): void {
    bus.off(Ev.KB_CHANGED, this.onKbChanged);
  }

  refresh(): void {
    this.offlineMode = AppContext.getBool('kb_offline_mode', false);
    KbService.statusText().then((s: string) => {
      this.statusLine = s;
    });
    KbService.listDocs().then((rows: Array<Map<string, Object>>) => {
      const out: DocRow[] = [];
      rows.forEach((r: Map<string, Object>) => {
        const d: DocRow = {
          id: String(r.get('id')),
          title: String(r.get('title')),
          source: String(r.get('source')),
          chars: Number(r.get('char_count'))
        };
        out.push(d);
      });
      this.docs = out;
    });
  }

  async ask(): Promise<void> {
    const q = this.question.trim();
    if (q.length === 0 || this.answering) {
      return;
    }
    this.answering = true;
    this.answer = '';
    this.sources = [];
    this.askToken = new CancelToken();
    try {
      await KbService.askStream(q, {
        onDelta: (t: string) => {
          this.answer += t;
        },
        onSources: (hits: KbHit[]) => {
          this.sources = hits;
        }
      }, this.askToken);
    } catch (e) {
      this.answer = `出错了: ${(e as Error).message}`;
    }
    this.askToken = null;
    this.answering = false;
  }

  build() {
    Column({ space: 10 }) {
      Text(this.statusLine.replace(/\n/g, ' · '))
        .fontSize(11)
        .fontColor(Ink.text3)
        .width('100%')
        .padding({ left: 16, right: 16, top: 8 })
        .maxLines(2)

      // 问答区（主操作）：有答案时答案区占据剩余空间，无答案时整块收紧
      Column({ space: 8 }) {
        Row({ space: 8 }) {
          Row({ space: 8 }) {
            AppIcon({ icon: $r('sys.symbol.questionmark_circle'), sizeVp: 15, color: Ink.text3, weight: 500 })
            TextInput({ placeholder: '问知识库，如「这门课的重点是什么？」', text: this.question })
              .layoutWeight(1)
              .height(40)
              .backgroundColor(Color.Transparent)
              .placeholderColor(Ink.text3)
              .placeholderFont({ size: 13.5 })
              .caretColor(this.accentDeep)
              .padding({ left: 0, right: 0 })
              .onChange((v: string) => {
                this.question = v;
              })
              .onSubmit(() => {
                this.ask();
              })
          }
          .layoutWeight(1)
          .height(44)
          .padding({ left: 14, right: 14 })
          .borderRadius(Radius.pill)
          .backgroundColor(Ink.sunken)
          PillButton({
            label: this.answering ? '停止' : '提问',
            variant: this.answering ? 'quiet' : 'accent',
            heightVp: 44,
            onTap: (): void => {
              if (this.answering) {
                this.askToken?.cancel();
              } else {
                this.ask();
              }
            }
          })
        }
        .width('100%')
        .padding({ left: 12, right: 12, top: 4 })

        if (this.answer.length > 0 || this.sources.length > 0 || this.answering) {
          Scroll() {
            Column({ space: 10 }) {
              if (this.sources.length > 0) {
                Column({ space: 6 }) {
                  Row({ space: 5 }) {
                    AppIcon({ icon: $r('sys.symbol.bookmark_fill'), sizeVp: 12, color: this.accentDeep, weight: 600 })
                    Text('引用来源')
                      .fontSize(11.5)
                      .fontWeight(FontWeight.Medium)
                      .fontColor(this.accentDeep)
                  }
                  ForEach(this.sources, (h: KbHit) => {
                    Text(`《${h.docTitle}》 ${h.content.substring(0, 60)}…`)
                      .fontSize(11)
                      .fontColor(Ink.text3)
                      .maxLines(1)
                      .width('100%')
                  }, (h: KbHit) => `${h.docTitle}_${h.score}`)
                }
                .width('100%')
                .padding(10)
                .borderRadius(Radius.inner)
                .backgroundColor(this.accentTint)
              }
              if (this.answer.length > 0 || this.answering) {
                Text(this.answer.length > 0 ? this.answer : '思考中…')
                  .fontSize(14)
                  .fontColor(Ink.text)
                  .lineHeight(23)
                  .width('100%')
              }
            }
            .width('100%')
            .padding(12)
          }
          .layoutWeight(1)
          .width('100%')
          .align(Alignment.Top)
          .scrollBar(BarState.Auto)
        }
      }
      .width('100%')

      // 导入与文档
      Column({ space: 8 }) {
        Row({ space: 8 }) {
          this.ImportTile($r('sys.symbol.doc_text'), '导入文档', 'txt / md / pdf 等', (): void => {
            DocImporter.importTextDocs().then((msg: string) => {
              this.answer = msg;
              // 知识库变更 → 动态热词表失效（ASR 引擎经 KB_CHANGED 事件延迟重建）
              IntentEngine.invalidateHotwords();
              this.refresh();
            });
          })
          this.ImportTile($r('sys.symbol.camera'), '拍照识字', '截图 / 讲义拍照入库', (): void => {
            DocImporter.importImagesOcr().then((msg: string) => {
              this.answer = msg;
              IntentEngine.invalidateHotwords();
              this.refresh();
            });
          })
        }
        .width('100%')
        .padding({ left: 12, right: 12 })

        Row({ space: 8 }) {
          Text(this.docs.length > 0 ? `已入库 ${this.docs.length} 篇` : '还没有文档')
            .fontSize(12)
            .fontWeight(FontWeight.Medium)
            .fontColor(Ink.text2)
            .layoutWeight(1)
          Text('离线模式')
            .fontSize(12)
            .fontColor(Ink.text3)
          Toggle({ type: ToggleType.Switch, isOn: this.offlineMode })
            .selectedColor(this.accentDeep)
            .scale({ x: 0.85, y: 0.85 })
            .onChange((on: boolean) => {
              AppContext.setBool('kb_offline_mode', on).then(() => {
                this.refresh();
              });
            })
        }
        .width('100%')
        .padding({ left: 16, right: 16 })

        if (this.docs.length === 0) {
          EmptyHint({
            icon: $r('sys.symbol.folder'),
            title: '先导入课程笔记 / 文献 / 会议纪要',
            sub: '导入后即可在对话与提问中引用'
          })
            .padding({ bottom: 8 })
        } else {
          List({ space: 0 }) {
            ForEach(this.docs, (d: DocRow, idx: number) => {
              ListItem() {
                Row({ space: 10 }) {
                  Column() {
                    AppIcon({
                      icon: d.source === 'ocr' ? $r('sys.symbol.camera') : $r('sys.symbol.doc_text'),
                      sizeVp: 15,
                      color: Ink.text2,
                      weight: 500
                    })
                  }
                  .width(32)
                  .height(32)
                  .borderRadius(10)
                  .backgroundColor(Ink.sunken)
                  .justifyContent(FlexAlign.Center)
                  Column({ space: 2 }) {
                    Text(d.title)
                      .fontSize(13.5)
                      .fontColor(Ink.text)
                      .maxLines(1)
                    Text(`${d.chars} 字 · ${d.source === 'ocr' ? '拍照识字' : '文档'}`)
                      .fontSize(11)
                      .fontColor(Ink.text3)
                  }
                  .alignItems(HorizontalAlign.Start)
                  .layoutWeight(1)
                  IconButton({
                    icon: $r('sys.symbol.trash'),
                    sizeVp: 30,
                    iconVp: 14,
                    bg: '#00000000',
                    color: Ink.text3,
                    onTap: () => {
                      KbService.deleteDoc(d.id).then(() => {
                        IntentEngine.invalidateHotwords();
                        this.refresh();
                      });
                    }
                  })
                }
                .width('100%')
                .padding({ left: 14, right: 8, top: 9, bottom: 9 })
              }
            }, (d: DocRow) => d.id)
          }
          .height('26%')
          .width('100%')
          .padding({ left: 12, right: 12 })
          .scrollBar(BarState.Off)
        }
      }
      .width('100%')
      .padding({ bottom: 12 })
    }
    .width('100%')
    .height('100%')
  }

  @Builder
  ImportTile(icon: Resource, label: string, sub: string, action: () => void) {
    Row({ space: 10 }) {
      Column() {
        AppIcon({ icon: icon, sizeVp: 17, color: this.accentDeep, weight: 600 })
      }
      .width(36)
      .height(36)
      .borderRadius(12)
      .backgroundColor(this.accentTint)
      .justifyContent(FlexAlign.Center)
      Column({ space: 2 }) {
        Text(label)
          .fontSize(13.5)
          .fontWeight(FontWeight.Medium)
          .fontColor(Ink.text)
        Text(sub)
          .fontSize(10.5)
          .fontColor(Ink.text3)
          .maxLines(1)
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
    }
    .layoutWeight(1)
    .padding(10)
    .borderRadius(Radius.card)
    .backgroundColor(Ink.card)
    .border({ width: 0.5, color: Ink.border })
    .onClick(() => {
      action();
    })
  }
}
