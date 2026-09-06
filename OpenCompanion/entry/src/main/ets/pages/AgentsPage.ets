import { bus, Ev, BusEvent, BusListener } from '../commons/EventBus';
import { AgentRepo } from '../core/AgentRepo';
import { ToolRegistry } from '../core/ToolRegistry';
import { AgentProfile, ToolDef } from '../core/Types';
import { Ink, Radius, Shadow } from '../ui/DesignTokens';
import { AppIcon, AvatarTile, Chip, IconButton, PillButton } from '../ui/Widgets';

/**
 * 智能体工坊（对应 Dashboard agents 页 + 命题 F4 ≥3 子智能体）：
 * 内置三子智能体 + 自定义创建 + AI 生成规格 + 工具白名单。
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
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
  private allTools: ToolDef[] = [];
  private onAgentsChanged: BusListener = (_e: BusEvent) => {
    this.refresh();
  };

  aboutToAppear(): void {
    this.refresh();
    bus.on(Ev.AGENTS_CHANGED, this.onAgentsChanged);
  }

  aboutToDisappear(): void {
    bus.off(Ev.AGENTS_CHANGED, this.onAgentsChanged);
  }

  refresh(): void {
    AgentRepo.list().then((list: AgentProfile[]) => {
      this.agents = list;
    }).catch((_e: Error) => {
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
          Text('内置 3 个子智能体，可组队协作')
            .fontSize(12)
            .fontColor(Ink.text3)
            .layoutWeight(1)
          PillButton({
            label: '新建',
            icon: $r('sys.symbol.plus'),
            variant: 'ink',
            heightVp: 34,
            fontSizeVp: 13,
            onTap: (): void => {
              this.openEditor(null);
            }
          })
        }
        .width('100%')
        .padding({ left: 16, right: 16, top: 10 })

        List({ space: 10 }) {
          ForEach(this.agents, (a: AgentProfile) => {
            ListItem() {
              Column({ space: 10 }) {
                Row({ space: 11 }) {
                  AvatarTile({ avatar: a.avatar, sizeVp: 42, fontSizeVp: 21 })
                  Column({ space: 3 }) {
                    Row({ space: 6 }) {
                      Text(a.name)
                        .fontSize(15)
                        .fontWeight(FontWeight.Bold)
                        .fontColor(Ink.text)
                      if (a.isBuiltin) {
                        Chip({ label: '内置', variant: 'accent' })
                      }
                    }
                    Text(a.description)
                      .fontSize(12)
                      .fontColor(Ink.text2)
                      .maxLines(2)
                    Text(a.allowedTools.length > 0 ? a.allowedTools.join(' · ') : '全部工具可用')
                      .fontSize(10.5)
                      .fontColor(Ink.text3)
                      .fontFamily('monospace')
                      .maxLines(1)
                  }
                  .alignItems(HorizontalAlign.Start)
                  .layoutWeight(1)
                }
                .width('100%')
                .alignItems(VerticalAlign.Top)

                Row({ space: 8 }) {
                  Button('@ 对话')
                    .height(32)
                    .type(ButtonType.Capsule)
                    .fontSize(12.5)
                    .fontWeight(FontWeight.Medium)
                    .fontColor(this.accentDeep)
                    .backgroundColor(this.accentTint)
                    .onClick(() => {
                      // 跳转对话页并预填 @提及（draft 由 ChatPage 的 NAVIGATE 监听消费）
                      bus.emit(Ev.NAVIGATE, { 'page': 'chat', 'draft': `@${a.name} ` } as Record<string, string>);
                    })
                  Button('编辑')
                    .height(32)
                    .type(ButtonType.Capsule)
                    .fontSize(12.5)
                    .fontColor(Ink.text2)
                    .backgroundColor(Ink.sunken)
                    .onClick(() => {
                      this.openEditor(a);
                    })
                  Blank()
                  if (!a.isBuiltin) {
                    IconButton({
                      icon: $r('sys.symbol.trash'),
                      sizeVp: 32,
                      iconVp: 15,
                      bg: '#00000000',
                      color: Ink.text3,
                      onTap: () => {
                        AgentRepo.delete(a.id).then((msg: string) => {
                          this.refresh();
                        });
                      }
                    })
                  }
                }
                .width('100%')
              }
              .padding(14)
              .backgroundColor(Ink.card)
              .borderRadius(Radius.card)
              .border({ width: 0.5, color: Ink.border })
            }
          }, (a: AgentProfile) => a.id)
        }
        .layoutWeight(1)
        .width('100%')
        .padding({ left: 16, right: 16 })
        .scrollBar(BarState.Off)
      }
      .width('100%')
      .height('100%')

      // 编辑浮层
      if (this.showEditor) {
        this.EditorOverlay()
      }
    }
    .width('100%')
    .height('100%')
  }

  @Builder
  EditorOverlay() {
    Column() {
      Scroll() {
        Column({ space: 12 }) {
          Row() {
            Text(this.editId.length > 0 ? '编辑智能体' : '新建智能体')
              .fontSize(17)
              .fontWeight(FontWeight.Bold)
              .fontColor(Ink.text)
            Blank()
            IconButton({
              icon: $r('sys.symbol.xmark'),
              sizeVp: 30,
              iconVp: 14,
              bg: Ink.sunken,
              color: Ink.text2,
              onTap: () => {
                this.showEditor = false;
              }
            })
          }
          .width('100%')

          Row({ space: 8 }) {
            TextInput({ text: this.editAvatar })
              .width(56)
              .height(44)
              .textAlign(TextAlign.Center)
              .fontSize(18)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .onChange((v: string) => {
                this.editAvatar = v;
              })
            TextInput({ placeholder: '名称', text: this.editName })
              .layoutWeight(1)
              .height(44)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .caretColor(this.accentDeep)
              .onChange((v: string) => {
                this.editName = v;
              })
          }
          .width('100%')

          TextInput({ placeholder: '一句话职责描述', text: this.editDesc })
            .height(44)
            .backgroundColor(Ink.sunken)
            .borderRadius(Radius.inner)
            .caretColor(this.accentDeep)
            .onChange((v: string) => {
              this.editDesc = v;
            })
          TextArea({ placeholder: '系统提示词（人设与工作方式）', text: this.editPrompt })
            .height(92)
            .backgroundColor(Ink.sunken)
            .borderRadius(Radius.inner)
            .caretColor(this.accentDeep)
            .onChange((v: string) => {
              this.editPrompt = v;
            })

          Row({ space: 8 }) {
            Text('工具白名单（不选 = 全部可用）')
              .fontSize(12)
              .fontColor(Ink.text3)
              .layoutWeight(1)
            PillButton({
              label: this.generating ? '生成中…' : 'AI 生成',
              icon: $r('sys.symbol.wand_and_stars'),
              variant: 'soft',
              heightVp: 32,
              fontSizeVp: 12.5,
              isEnabled: !this.generating,
              onTap: (): void => {
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
              }
            })
          }
          .width('100%')

          Flex({ wrap: FlexWrap.Wrap, justifyContent: FlexAlign.Start }) {
            ForEach(this.allTools, (t: ToolDef) => {
              Row({ space: 6 }) {
                // select 绑定初始白名单：编辑已有智能体时正确回显，避免误清配置
                Checkbox({ name: t.name, group: 'agentTools' })
                  .select(this.editTools.includes(t.name))
                  .selectedColor(this.accentDeep)
                  .width(16)
                  .height(16)
                  .onChange((on: boolean) => {
                    if (on) {
                      this.editTools = this.editTools.concat([t.name]);
                    } else {
                      this.editTools = this.editTools.filter((x: string) => x !== t.name);
                    }
                  })
                Text(t.name)
                  .fontSize(12)
                  .fontColor(Ink.text2)
              }
              .padding({ left: 4, right: 8, top: 5, bottom: 5 })
              .borderRadius(Radius.inner)
              .backgroundColor(Ink.sunken)
              .margin({ right: 8, bottom: 8 })
            }, (t: ToolDef) => t.name)
          }
          .width('100%')

          PillButton({
            label: '保存',
            variant: 'ink',
            heightVp: 46,
            onTap: (): void => {
              this.save();
            }
          })
            .width('100%')
        }
        .width('100%')
        .padding({ left: 18, right: 18, top: 16, bottom: 24 })
      }
      .width('100%')
      .height('100%')
      .align(Alignment.Top)
      .scrollBar(BarState.Off)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#F8F5F1FA')
    .shadow(Shadow.lift)
  }
}
