import { bus, Ev, BusEvent } from '../commons/EventBus';
import { AgentRepo } from '../core/AgentRepo';
import { ToolRegistry } from '../core/ToolRegistry';
import { AgentProfile, ToolDef } from '../core/Types';

/**
 * 智能体工坊（对应 Dashboard agents 页 + 命题 F4 ≥3 子智能体）：
 * 内置三子智能体 + 自定义创建 + AI 生成规格 + 工具白名单
 */
@Component
export struct AgentsPage {
  @State agents: AgentProfile[] = [];
  @State showEditor: boolean = false;
  @State editId: string = '';
  @State editName: string = '';
  @State editAvatar: string = '🤖';
  @State editDesc: string = '';
  @State editPrompt: string = '';
  @State editTools: string[] = [];
  @State generating: boolean = false;
  private allTools: ToolDef[] = [];

  aboutToAppear(): void {
    this.refresh();
    bus.on(Ev.AGENTS_CHANGED, (_e: BusEvent) => {
      this.refresh();
    });
  }

  refresh(): void {
    AgentRepo.list().then((list: AgentProfile[]) => {
      this.agents = list;
    });
    this.allTools = ToolRegistry.all();
  }

  openEditor(a: AgentProfile | null): void {
    if (a === null) {
      this.editId = '';
      this.editName = '';
      this.editAvatar = '🤖';
      this.editDesc = '';
      this.editPrompt = '';
      this.editTools = [];
    } else {
      this.editId = a.id;
      this.editName = a.name;
      this.editAvatar = a.avatar;
      this.editDesc = a.description;
      this.editPrompt = a.systemPrompt;
      this.editTools = a.allowedTools;
    }
    this.showEditor = true;
  }

  save(): void {
    if (this.editName.trim().length === 0) {
      return;
    }
    const a = new AgentProfile();
    a.id = this.editId;
    a.name = this.editName.trim();
    a.avatar = this.editAvatar;
    a.description = this.editDesc;
    a.systemPrompt = this.editPrompt;
    a.allowedTools = this.editTools;
    AgentRepo.save(a).then(() => {
      this.showEditor = false;
      this.refresh();
    });
  }

  build() {
    Stack() {
      Column({ space: 10 }) {
        Row({ space: 8 }) {
          Text('内置 3 个子智能体，可 @点名 或 /team 组队协作')
            .fontSize(12)
            .fontColor('#8A7A80')
          Blank()
          Button('＋ 新建')
            .height(32)
            .backgroundColor('#F76C8A')
            .onClick(() => {
              this.openEditor(null);
            })
        }
        .width('100%')
        .padding({ left: 12, right: 12, top: 8 })

        List({ space: 8 }) {
          ForEach(this.agents, (a: AgentProfile) => {
            ListItem() {
              Column({ space: 6 }) {
                Row({ space: 10 }) {
                  Text(a.avatar).fontSize(30)
                  Column({ space: 2 }) {
                    Row({ space: 6 }) {
                      Text(a.name).fontSize(16).fontWeight(FontWeight.Bold).fontColor('#3D2E33')
                      if (a.isBuiltin) {
                        Text('内置').fontSize(10).fontColor('#C2557E').backgroundColor('#FFE3EA').borderRadius(6).padding({ left: 5, right: 5, top: 1, bottom: 1 })
                      }
                    }
                    Text(a.description).fontSize(12).fontColor('#8A7A80').maxLines(2)
                    Text(`工具: ${a.allowedTools.length > 0 ? a.allowedTools.join(' / ') : '全部'}`)
                      .fontSize(10)
                      .fontColor('#B9A9AF')
                      .maxLines(1)
                  }
                  .alignItems(HorizontalAlign.Start)
                  .layoutWeight(1)
                }
                .width('100%')
                Row({ space: 8 }) {
                  Button('@对话')
                    .height(30)
                    .backgroundColor('#FFE3EA')
                    .fontColor('#C2557E')
                    .onClick(() => {
                      bus.emit(Ev.NAVIGATE, { 'page': 'chat' } as Record<string, string>);
                      setTimeout(() => {
                        // 预填输入：由 ChatPage 输入框接收（简化为提示）
                      }, 50);
                    })
                  Button('编辑')
                    .height(30)
                    .backgroundColor('#F7F2F4')
                    .fontColor('#5B4A4F')
                    .onClick(() => {
                      this.openEditor(a);
                    })
                  if (!a.isBuiltin) {
                    Button('删除')
                      .height(30)
                      .backgroundColor('#F7F2F4')
                      .fontColor('#B0670A')
                      .onClick(() => {
                        AgentRepo.delete(a.id).then((msg: string) => {
                          this.refresh();
                        });
                      })
                  }
                }
                .width('100%')
              }
              .padding(12)
              .backgroundColor('#FFFFFF')
              .borderRadius(14)
            }
          }, (a: AgentProfile) => a.id)
        }
        .layoutWeight(1)
        .width('100%')
        .padding({ left: 12, right: 12 })
      }
      .width('100%')
      .height('100%')

      // 编辑浮层
      if (this.showEditor) {
        Column() {
          Scroll() {
            Column({ space: 10 }) {
              Row() {
                Text(this.editId.length > 0 ? '编辑智能体' : '新建智能体')
                  .fontSize(16).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
                Blank()
                Text('✖').fontSize(16).onClick(() => {
                  this.showEditor = false;
                })
              }
              .width('100%')
              Row({ space: 8 }) {
                TextInput({ text: this.editAvatar })
                  .width(60)
                  .height(40)
                  .onChange((v: string) => {
                    this.editAvatar = v;
                  })
                TextInput({ placeholder: '名称', text: this.editName })
                  .layoutWeight(1)
                  .height(40)
                  .onChange((v: string) => {
                    this.editName = v;
                  })
              }
              .width('100%')
              TextInput({ placeholder: '一句话职责描述', text: this.editDesc })
                .height(40)
                .onChange((v: string) => {
                  this.editDesc = v;
                })
              TextArea({ placeholder: '系统提示词（人设与工作方式）', text: this.editPrompt })
                .height(90)
                .onChange((v: string) => {
                  this.editPrompt = v;
                })
              Row({ space: 8 }) {
                Text('工具白名单（不选=全部可用）').fontSize(12).fontColor('#8A7A80')
                Blank()
                Button('✨ AI 生成')
                  .height(30)
                  .backgroundColor('#9D7BFF')
                  .enabled(!this.generating)
                  .onClick(() => {
                    this.generating = true;
                    AgentRepo.generateSpec(this.editDesc.length > 0 ? this.editDesc : this.editPrompt)
                      .then((a: AgentProfile | null) => {
                        this.generating = false;
                        if (a !== null) {
                          this.editName = a.name;
                          this.editAvatar = a.avatar;
                          this.editDesc = a.description;
                          this.editPrompt = a.systemPrompt;
                          this.editTools = a.allowedTools;
                        }
                      }).catch((_e: Error) => {
                        this.generating = false;
                      });
                  })
              }
              .width('100%')
              Flex({ wrap: FlexWrap.Wrap, justifyContent: FlexAlign.Start }) {
                ForEach(this.allTools, (t: ToolDef) => {
                  Row({ space: 4 }) {
                    Checkbox()
                      .selectedColor('#F76C8A')
                      .width(16)
                      .height(16)
                      .onChange((on: boolean) => {
                        if (on) {
                          this.editTools = this.editTools.concat([t.name]);
                        } else {
                          this.editTools = this.editTools.filter((x: string) => x !== t.name);
                        }
                      })
                    Text(t.name).fontSize(11).fontColor('#5B4A4F')
                  }
                  .padding(4)
                }, (t: ToolDef) => t.name)
              }
              .width('100%')
              Button('保存')
                .width('100%')
                .backgroundColor('#F76C8A')
                .onClick(() => {
                  this.save();
                })
            }
            .width('100%')
            .padding(16)
          }
          .width('100%')
          .height('100%')
        }
        .width('100%')
        .height('100%')
        .backgroundColor('rgba(253,248,246,0.99)')
      }
    }
    .width('100%')
    .height('100%')
  }
}
