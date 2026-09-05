import { bus, Ev, BusEvent, BusListener } from '../commons/EventBus';
import { KbService } from '../knowledge/KbService';
import { DocImporter } from '../knowledge/DocImporter';
import { KbHit } from '../knowledge/KbService';
import { AppContext } from '../commons/AppContext';
import { CancelToken } from '../core/Types';
import { IntentEngine } from '../core/IntentEngine';

interface DocRow {
  id: string;
  title: string;
  source: string;
  chars: number;
}

/**
 * 知识库页（对应命题 F2 端侧 RAG）：
 * 文档导入（文件/图片OCR）→ 检索问答（RAG 会话 / 端侧离线模式）→ 引用来源
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
      // 状态行
      Text(this.statusLine.replace(/\n/g, ' · '))
        .fontSize(12)
        .fontColor('#8A7A80')
        .width('100%')
        .padding({ left: 14, right: 14, top: 8 })
        .maxLines(2)

      // 操作按钮
      Row({ space: 8 }) {
        Button('📄 导入文档')
          .backgroundColor('#FFE3EA')
          .fontColor('#C2557E')
          .onClick(() => {
            DocImporter.importTextDocs().then((msg: string) => {
              this.answer = msg;
              // 知识库变更 → 动态热词表失效（ASR 引擎经 KB_CHANGED 事件延迟重建）
              IntentEngine.invalidateHotwords();
              this.refresh();
            });
          })
        Button('📷 拍照识字入库')
          .backgroundColor('#FFF3D6')
          .fontColor('#8A6A1F')
          .onClick(() => {
            DocImporter.importImagesOcr().then((msg: string) => {
              this.answer = msg;
              IntentEngine.invalidateHotwords();
              this.refresh();
            });
          })
        Toggle({ type: ToggleType.Checkbox, isOn: this.offlineMode })
          .onChange((on: boolean) => {
            AppContext.setBool('kb_offline_mode', on).then(() => {
              this.refresh();
            });
          })
        Text('离线模式(端侧模型)').fontSize(11).fontColor('#8A7A80')
      }
      .width('100%')
      .padding({ left: 12, right: 12 })

      // 文档列表
      Text(this.docs.length > 0 ? `已入库文档（${this.docs.length}）` : '还没有文档，先导入课程笔记/文献/会议纪要试试')
        .fontSize(12)
        .fontColor('#8A7A80')
        .width('100%')
        .padding({ left: 14 })
      List({ space: 6 }) {
        ForEach(this.docs, (d: DocRow) => {
          ListItem() {
            Row({ space: 8 }) {
              Text(d.source === 'ocr' ? '📷' : '📄').fontSize(18)
              Column({ space: 2 }) {
                Text(d.title).fontSize(14).fontColor('#3D2E33').maxLines(1)
                Text(`${d.chars} 字 · ${d.source}`)
                  .fontSize(11)
                  .fontColor('#B9A9AF')
              }
              .alignItems(HorizontalAlign.Start)
              .layoutWeight(1)
              Text('🗑')
                .fontSize(15)
                .onClick(() => {
                  KbService.deleteDoc(d.id).then(() => {
                    IntentEngine.invalidateHotwords();
                    this.refresh();
                  });
                })
            }
            .width('100%')
            .padding(10)
            .backgroundColor('#FFFFFF')
            .borderRadius(12)
          }
        }, (d: DocRow) => d.id)
      }
      .height('26%')
      .width('100%')
      .padding({ left: 12, right: 12 })

      // 问答区
      Column({ space: 8 }) {
        Row({ space: 8 }) {
          TextInput({ placeholder: '问知识库：如「这门课的重点是什么？」', text: this.question })
            .layoutWeight(1)
            .height(40)
            .backgroundColor('#FFFFFF')
            .onChange((v: string) => {
              this.question = v;
            })
            .onSubmit(() => {
              this.ask();
            })
          Button(this.answering ? '停止' : '提问')
            .backgroundColor(this.answering ? '#F0E4E8' : '#F76C8A')
            .onClick(() => {
              if (this.answering) {
                this.askToken?.cancel();
              } else {
                this.ask();
              }
            })
        }
        .width('100%')
        Scroll() {
          Column({ space: 8 }) {
            if (this.sources.length > 0) {
              Column({ space: 4 }) {
                Text('📌 引用来源').fontSize(12).fontColor('#C2557E')
                ForEach(this.sources, (h: KbHit) => {
                  Text(`《${h.docTitle}》 ${h.content.substring(0, 60)}…`)
                    .fontSize(11)
                    .fontColor('#8A7A80')
                    .maxLines(1)
                }, (h: KbHit) => `${h.docTitle}_${h.score}`)
              }
              .width('100%')
              .padding(8)
              .backgroundColor('#FFF7F4')
              .borderRadius(10)
            }
            Text(this.answer.length > 0 ? this.answer : (this.answering ? '思考中…' : ''))
              .fontSize(14)
              .fontColor('#3D2E33')
              .width('100%')
          }
          .width('100%')
          .padding(10)
        }
        .layoutWeight(1)
        .width('100%')
        .backgroundColor('#FFFFFF')
        .borderRadius(12)
        .scrollBar(BarState.Auto)
      }
      .layoutWeight(1)
      .width('100%')
      .padding({ left: 12, right: 12, bottom: 10 })
    }
    .width('100%')
    .height('100%')
  }
}
