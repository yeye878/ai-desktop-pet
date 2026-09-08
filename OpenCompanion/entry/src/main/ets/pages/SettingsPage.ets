import { AppContext, SettingKey } from '../commons/AppContext';
import { LlmClient, LlmProfiles } from '../core/LlmClient';
import { LlmProfile } from '../core/Types';
import { AgentRepo } from '../core/AgentRepo';
import { DistributedSync } from '../commons/DistributedSync';
import { PetTheme, ThemeEngine } from '../pet/ThemeEngine';
import { AccentBridge, Ink, Radius, Shadow } from '../ui/DesignTokens';
import { AppIcon, IconButton, PillButton, SettingRow, FieldLabel } from '../ui/Widgets';

@Builder
function emptySection(): void {
}

/**
 * 设置分组卡片：标题 + 可选说明 + 内容（尾随闭包）。
 */
@Component
struct SettingsSection {
  @Prop title: string = '';
  @Prop caption: string = '';
  @BuilderParam content: () => void = emptySection;

  build() {
    Column({ space: 10 }) {
      Column({ space: 2 }) {
        Text(this.title)
          .fontSize(14)
          .fontWeight(FontWeight.Bold)
          .fontColor(Ink.text)
          .width('100%')
        if (this.caption.length > 0) {
          Text(this.caption)
            .fontSize(11)
            .fontColor(Ink.text3)
            .width('100%')
        }
      }
      .width('100%')
      this.content()
    }
    .width('100%')
    .padding(14)
    .backgroundColor(Ink.bg)
    .borderRadius(Radius.card)
    .border({ width: 0.5, color: Ink.border })
    .alignItems(HorizontalAlign.Start)
  }
}

/**
 * 设置面板（API 配置管理 + 外观主题 + 语音 + 行为隐私 + 关于）。
 * 对应桌面版 Settings.vue 的 API Profile 管理核心能力。
 * 容器：底部抽屉（手机）/ 居中卡片（平板），由 Index 提供纱罩。
 */
@Component
export struct SettingsPanel {
  onClose: () => void = () => {
  };
  @State profiles: LlmProfile[] = [];
  @State activeId: string = '';
  @State showEditor: boolean = false;
  @State edit: LlmProfile = new LlmProfile();
  @State testResult: string = '';
  @State testOk: boolean = false;
  @State autoSpeak: boolean = false;
  @State confirmTools: boolean = true;
  @State personality: string = 'gentle';
  @State hotWords: string = '';
  @State weatherCity: string = '北京';
  @State proxyHost: string = '';
  @State proxyPort: string = '';
  @State themeIdx: number = 0;
  @State dragY: number = 0;
  @State wide: boolean = AppContext.vpWidth >= 720;
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';

  aboutToAppear(): void {
    this.themeIdx = ThemeEngine.currentIndex();
    this.refresh();
  }

  refresh(): void {
    this.profiles = LlmProfiles.list();
    this.activeId = LlmProfiles.activeId();
    this.autoSpeak = AppContext.getBool(SettingKey.VOICE_AUTO_SPEAK, false);
    this.confirmTools = AppContext.getBool(SettingKey.TOOL_CONFIRM, true);
    this.personality = AppContext.getSetting(SettingKey.PERSONALITY, 'gentle');
    this.hotWords = AppContext.getSetting(SettingKey.HOT_WORDS, '');
    this.weatherCity = AppContext.getSetting(SettingKey.WEATHER_CITY, '北京');
    this.proxyHost = AppContext.getSetting(SettingKey.PROXY_HOST, '');
    this.proxyPort = AppContext.getSetting(SettingKey.PROXY_PORT, '');
  }

  newProfile(): void {
    const p = new LlmProfile();
    p.id = `pf_${Date.now()}`;
    p.name = '新配置';
    this.edit = p;
    this.showEditor = true;
  }

  editProfile(p: LlmProfile): void {
    LlmProfiles.resolveKey(p).then((key: string) => {
      const copy = new LlmProfile();
      copy.id = p.id;
      copy.name = p.name;
      copy.baseUrl = p.baseUrl;
      copy.model = p.model;
      copy.temperature = p.temperature;
      copy.useStream = p.useStream;
      copy.confirmEnabled = p.confirmEnabled;
      copy.apiKey = key;
      this.edit = copy;
      this.showEditor = true;
    });
  }

  async saveProfile(): Promise<void> {
    // 落盘前归一化 Base URL，避免缺协议/带尾斜杠的地址存入后请求失败
    this.edit.baseUrl = LlmClient.normalizeBaseUrl(this.edit.baseUrl);
    this.edit.name = this.edit.name.trim();
    this.edit.model = this.edit.model.trim();
    const list = LlmProfiles.list();
    let replaced = false;
    const out: LlmProfile[] = [];
    for (const p of list) {
      if (p.id === this.edit.id) {
        out.push(this.edit);
        replaced = true;
      } else {
        out.push(p);
      }
    }
    if (!replaced) {
      out.push(this.edit);
    }
    await LlmProfiles.saveAll(out);
    if (this.activeId.length === 0) {
      await LlmProfiles.setActive(this.edit.id);
    }
    this.showEditor = false;
    this.refresh();
  }

  async testConn(): Promise<void> {
    this.testResult = '测试中…';
    this.testOk = false;
    try {
      const reply = await LlmClient.testConnection(this.edit);
      this.testResult = `连接成功：${reply}`;
      this.testOk = true;
    } catch (e) {
      const msg = (e as Error).message;
      // 域名解析失败多半是地址不完整或设备 DNS 异常，补一句人话提示（代理可在下方“网络代理”配置）
      const hint = /resolve|2300006|2300007/i.test(msg) ?
        '（域名无法解析：请检查 Base URL 是否完整含 https://；若设备 DNS 异常，可在设置页“网络代理”指定代理绕过）' : '';
      this.testResult = `连接失败：${msg}${hint}`;
      this.testOk = false;
    }
  }

  async pickTheme(i: number): Promise<void> {
    this.themeIdx = i;
    await ThemeEngine.setTheme(i);
    AccentBridge.sync();
  }

  private themeName(i: number): string {
    const names: string[] = ['樱花', '海雾', '星云', '森林', '落日', '花见', '极光'];
    return names[i] ?? '';
  }

  build() {
    Stack() {
      Column() {
        // 拖拽把手 + 标题栏（下拉可关闭抽屉）
        Column() {
          if (!this.wide) {
            Row()
              .width(36)
              .height(4)
              .borderRadius(Radius.pill)
              .backgroundColor(Ink.border)
              .margin({ top: 10, bottom: 8 })
          }
          Row() {
            Text('设置')
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
                this.onClose();
              }
            })
          }
          .width('100%')
          .padding({ left: 16, right: 12, top: this.wide ? 12 : 2, bottom: 8 })
        }
        .width('100%')
        .gesture(
          PanGesture({ fingers: 1, direction: PanDirection.Vertical })
            .onActionUpdate((e: GestureEvent) => {
              this.dragY = Math.max(0, e.offsetY);
            })
            .onActionEnd((e: GestureEvent) => {
              if (e.offsetY > 90) {
                this.dragY = 0;
                this.onClose();
              } else {
                animateTo({ duration: 220, curve: Curve.EaseOut }, () => {
                  this.dragY = 0;
                });
              }
            })
        )

        Scroll() {
          Column({ space: 12 }) {
            // ---------- 外观 ----------
            SettingsSection({ title: '宠物主题', caption: '界面强调色跟随桌宠配色' }) {
              Row({ space: 4 }) {
                ForEach(ThemeEngine.THEMES, (t: PetTheme, i: number) => {
                  this.ThemeSwatch(t, i)
                }, (t: PetTheme, i: number) => `${i}_${t.name}`)
              }
              .width('100%')
            }

            // ---------- API 配置 ----------
            SettingsSection({ title: 'LLM API', caption: 'OpenAI 兼容 · 云端或本地网关' }) {
              Column({ space: 8 }) {
                if (this.profiles.length === 0) {
                  Text('还没有配置，支持任意 OpenAI 兼容服务（OpenAI / 通义 / DeepSeek / 本地推理网关）')
                    .fontSize(12)
                    .fontColor(Ink.warn)
                    .width('100%')
                }
                ForEach(this.profiles, (p: LlmProfile) => {
                  Row({ space: 10 }) {
                    Circle({ width: 17, height: 17 })
                      .fill(p.id === this.activeId ? this.accentDeep : Color.Transparent)
                      .stroke(p.id === this.activeId ? this.accentDeep : Ink.border)
                      .strokeWidth(1.5)
                    Column({ space: 2 }) {
                      Text(`${p.name} · ${p.model}`)
                        .fontSize(13)
                        .fontColor(Ink.text)
                        .maxLines(1)
                      Text(p.baseUrl)
                        .fontSize(10.5)
                        .fontColor(Ink.text3)
                        .maxLines(1)
                    }
                    .alignItems(HorizontalAlign.Start)
                    .layoutWeight(1)
                    .onClick(() => {
                      LlmProfiles.setActive(p.id).then(() => {
                        this.refresh();
                      });
                    })
                    IconButton({
                      icon: $r('sys.symbol.square_and_pencil'),
                      sizeVp: 30,
                      iconVp: 13,
                      bg: Ink.sunken,
                      color: Ink.text2,
                      onTap: () => {
                        this.editProfile(p);
                      }
                    })
                    IconButton({
                      icon: $r('sys.symbol.trash'),
                      sizeVp: 30,
                      iconVp: 13,
                      bg: '#00000000',
                      color: Ink.text3,
                      onTap: () => {
                        // 删除 API 配置不可逆，先确认
                        AlertDialog.show({
                          message: `删除配置「${p.name}」？`,
                          autoCancel: true,
                          alignment: DialogAlignment.Center,
                          primaryButton: {
                            value: '取消',
                            action: () => {
                            }
                          },
                          secondaryButton: {
                            value: '删除',
                            fontColor: Ink.danger,
                            action: () => {
                              LlmProfiles.removeProfile(p.id).then(() => {
                                this.refresh();
                              });
                            }
                          }
                        });
                      }
                    })
                  }
                  .width('100%')
                  .padding(10)
                  .borderRadius(Radius.inner)
                  .backgroundColor(p.id === this.activeId ? this.accentTint : Ink.bg)
                  .animation({ duration: 200, curve: Curve.EaseOut })
                }, (p: LlmProfile) => p.id)
                PillButton({
                  label: '添加配置',
                  icon: $r('sys.symbol.plus'),
                  variant: 'soft',
                  heightVp: 36,
                  fontSizeVp: 13,
                  fill: true,
                  onTap: (): void => {
                    this.newProfile();
                  }
                })
              }
              .width('100%')
            }

            // ---------- 网络代理 ----------
            SettingsSection({
              title: '网络代理（可选）',
              caption: '设备域名解析异常时（如部分模拟器），指定 HTTP 代理中转，域名解析改在代理端完成'
            }) {
              Column({ space: 6 }) {
                FieldLabel({ text: '代理主机（留空则直连）' })
                TextInput({ placeholder: '如 10.0.2.2 或 127.0.0.1', text: this.proxyHost })
                  .height(40)
                  .backgroundColor(Ink.sunken)
                  .borderRadius(Radius.inner)
                  .fontSize(12.5)
                  .onChange((v: string) => {
                    this.proxyHost = v;
                    AppContext.setSetting(SettingKey.PROXY_HOST, v.trim());
                  })
                FieldLabel({ text: '代理端口' })
                TextInput({ placeholder: '如 8888', text: this.proxyPort })
                  .type(InputType.Number)
                  .height(40)
                  .backgroundColor(Ink.sunken)
                  .borderRadius(Radius.inner)
                  .fontSize(12.5)
                  .onChange((v: string) => {
                    this.proxyPort = v;
                    AppContext.setSetting(SettingKey.PROXY_PORT, v.trim());
                  })
              }
              .width('100%')
            }

            // ---------- 语音 ----------
            SettingsSection({ title: '语音' }) {
              Column({ space: 10 }) {
                SettingRow({ label: '回复自动朗读', desc: '端侧 TTS' }) {
                  Toggle({ type: ToggleType.Switch, isOn: this.autoSpeak })
                    .selectedColor(this.accentDeep)
                    .scale({ x: 0.85, y: 0.85 })
                    .onChange((on: boolean) => {
                      AppContext.setBool(SettingKey.VOICE_AUTO_SPEAK, on).then(() => {
                        this.refresh();
                      });
                    })
                }
                Column({ space: 6 }) {
                  FieldLabel({ text: '识别热词（逗号分隔，提升语音指令识别率）' })
                  TextArea({ text: this.hotWords })
                    .height(58)
                    .backgroundColor(Ink.sunken)
                    .borderRadius(Radius.inner)
                    .fontSize(12.5)
                    .onChange((v: string) => {
                      this.hotWords = v;
                      AppContext.setSetting(SettingKey.HOT_WORDS, v);
                    })
                }
                .width('100%')
              }
              .width('100%')
            }

            // ---------- 行为 ----------
            SettingsSection({ title: '行为' }) {
              Column({ space: 10 }) {
                SettingRow({ label: '高风险工具执行前需要确认' }) {
                  Toggle({ type: ToggleType.Switch, isOn: this.confirmTools })
                    .selectedColor(this.accentDeep)
                    .scale({ x: 0.85, y: 0.85 })
                    .onChange((on: boolean) => {
                      AppContext.setBool(SettingKey.TOOL_CONFIRM, on).then(() => {
                        this.refresh();
                      });
                    })
                }
                Column({ space: 6 }) {
                  FieldLabel({ text: '性格' })
                  Flex({ wrap: FlexWrap.Wrap }) {
                    ForEach([['gentle', '温柔'], ['energetic', '元气'], ['calm', '沉稳'], ['mentor', '导师'], ['witty', '机灵']],
                      (item: Array<string>) => {
                        Text(item[1])
                          .fontSize(12.5)
                          .fontWeight(FontWeight.Medium)
                          .fontColor(this.personality === item[0] ? this.accentDeep : Ink.text2)
                          .backgroundColor(this.personality === item[0] ? this.accentTint : Ink.sunken)
                          .borderRadius(Radius.pill)
                          .padding({ left: 13, right: 13, top: 6, bottom: 6 })
                          .margin({ right: 8, bottom: 8 })
                          .animation({ duration: 200, curve: Curve.EaseOut })
                          .onClick(() => {
                            AppContext.setSetting(SettingKey.PERSONALITY, item[0]).then(() => {
                              this.refresh();
                            });
                          })
                      }, (item: Array<string>) => item[0])
                  }
                  .width('100%')
                }
                .width('100%')
                Column({ space: 6 }) {
                  FieldLabel({ text: '天气城市（日程管家简报用）' })
                  TextInput({ text: this.weatherCity })
                    .height(40)
                    .backgroundColor(Ink.sunken)
                    .borderRadius(Radius.inner)
                    .onChange((v: string) => {
                      this.weatherCity = v;
                      AppContext.setSetting(SettingKey.WEATHER_CITY, v);
                    })
                }
                .width('100%')
              }
              .width('100%')
            }

            // ---------- 跨设备协同 ----------
            SettingsSection({ title: '跨设备协同' }) {
              Column({ space: 6 }) {
                Text(DistributedSync.statusText())
                  .fontSize(12)
                  .fontColor(Ink.text2)
                  .width('100%')
                Text('多设备登录同一华为账号后，任务中心可一键把当前对话流转到手机 / 平板 / 2in1 继续，接续自动携带最近对话上下文。')
                  .fontSize(11)
                  .fontColor(Ink.text3)
                  .lineHeight(17)
                  .width('100%')
              }
              .width('100%')
            }

            // ---------- 关于 ----------
            SettingsSection({ title: '关于' }) {
              Text('OpenCompanion 智能伙伴，OpenHarmony 原生智能多模态个人助理（ArkTS + ArkUI）。端侧 RAG：@kit.DataAugmentationKit；语音：@kit.CoreSpeechKit；OCR：@kit.CoreVisionKit。数据全部存储在设备本地，API 密钥使用系统资产加密存储。')
                .fontSize(11)
                .fontColor(Ink.text3)
                .lineHeight(17)
                .width('100%')
            }
          }
          .width('100%')
          .padding({ top: 14, bottom: 20, left: 16, right: 16 })
        }
        .layoutWeight(1)
        .width('100%')
        .align(Alignment.Top)
        .scrollBar(BarState.Off)
      }
      .width(this.wide ? '72%' : '100%')
      .height(this.wide ? '90%' : '96%')
      .backgroundColor(Ink.card)
      .borderRadius(this.wide ? Radius.sheet : { topLeft: Radius.sheet, topRight: Radius.sheet })
      .shadow(Shadow.lift)
      .translate({ y: this.dragY })
      // 拦截点击冒泡：点面板本身不关闭（纱罩点击才关闭）
      .onClick(() => {
      })

      // API 配置编辑浮层
      if (this.showEditor) {
        this.ProfileEditor()
      }
    }
    .width('100%')
    .height('100%')
    .alignContent(this.wide ? Alignment.Center : Alignment.Bottom)
    // 点卡片以外区域关闭（卡片自身拦截点击，内层组件各自消费）
    .onClick(() => {
      this.onClose();
    })
  }

  @Builder
  ThemeSwatch(t: PetTheme, i: number) {
    Column({ space: 5 }) {
      Stack() {
        Circle({ width: 36, height: 36 })
          .fill(t.primary)
        if (this.themeIdx === i) {
          AppIcon({ icon: $r('sys.symbol.checkmark'), sizeVp: 15, color: '#FFFFFF', weight: 700 })
        }
      }
      .width(36)
      .height(36)
      Text(this.themeName(i))
        .fontSize(10)
        .fontColor(this.themeIdx === i ? Ink.text : Ink.text3)
        .fontWeight(this.themeIdx === i ? FontWeight.Medium : FontWeight.Normal)
    }
    .layoutWeight(1)
    .alignItems(HorizontalAlign.Center)
    .onClick(() => {
      this.pickTheme(i);
    })
  }

  @Builder
  ProfileEditor() {
    // 自带纱罩的全屏编辑层
    Column() {
      Column() {
        Scroll() {
          Column({ space: 10 }) {
            Text(this.edit.name.length > 0 ? `编辑 ${this.edit.name}` : '新增 API 配置')
              .fontSize(15)
              .fontWeight(FontWeight.Bold)
              .fontColor(Ink.text)
              .width('100%')
            TextInput({ placeholder: '配置名称', text: this.edit.name })
              .height(44)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .onChange((v: string) => {
                this.edit.name = v;
              })
            FieldLabel({ text: 'Base URL（需含 https:// 协议头）' })
            TextInput({ placeholder: 'https://api.openai.com/v1', text: this.edit.baseUrl })
              .height(44)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .onChange((v: string) => {
                this.edit.baseUrl = v;
              })
            FieldLabel({ text: '模型名' })
            TextInput({ placeholder: 'gpt-4o-mini / qwen-plus', text: this.edit.model })
              .height(44)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .onChange((v: string) => {
                this.edit.model = v;
              })
            FieldLabel({ text: 'API Key（存储在系统安全区，不会明文落盘）' })
            TextInput({ placeholder: 'sk-...', text: this.edit.apiKey })
              .type(InputType.Password)
              .height(44)
              .backgroundColor(Ink.sunken)
              .borderRadius(Radius.inner)
              .onChange((v: string) => {
                this.edit.apiKey = v;
              })
            // 两个开关行各占满宽，横排必溢出裁切，改纵排
            Column() {
              SettingRow({ label: '流式响应' }) {
                Toggle({ type: ToggleType.Switch, isOn: this.edit.useStream })
                  .selectedColor(this.accentDeep)
                  .scale({ x: 0.85, y: 0.85 })
                  .onChange((on: boolean) => {
                    this.edit.useStream = on;
                  })
              }
              SettingRow({ label: '工具需确认' }) {
                Toggle({ type: ToggleType.Switch, isOn: this.edit.confirmEnabled })
                  .selectedColor(this.accentDeep)
                  .scale({ x: 0.85, y: 0.85 })
                  .onChange((on: boolean) => {
                    this.edit.confirmEnabled = on;
                  })
              }
            }
            .width('100%')
            if (this.testResult.length > 0) {
              Row({ space: 6 }) {
                AppIcon({
                  icon: this.testOk ? $r('sys.symbol.checkmark_circle_fill') : $r('sys.symbol.exclamationmark_triangle'),
                  sizeVp: 14,
                  color: this.testOk ? Ink.ok : Ink.warn,
                  weight: 600
                })
                Text(this.testResult)
                  .fontSize(12)
                  .fontColor(this.testOk ? Ink.ok : Ink.warn)
                  .layoutWeight(1)
              }
              .width('100%')
              .padding(10)
              .borderRadius(Radius.inner)
              .backgroundColor(this.testOk ? Ink.okSoft : Ink.warnSoft)
            }
            Row({ space: 10 }) {
              PillButton({
                label: '测试连接',
                variant: 'soft',
                heightVp: 44,
                fill: true,
                onTap: (): void => {
                  this.testConn();
                }
              })
                .layoutWeight(1)
              PillButton({
                label: '保存',
                variant: 'ink',
                heightVp: 44,
                fill: true,
                onTap: (): void => {
                  this.saveProfile();
                }
              })
                .layoutWeight(1)
            }
            .width('100%')
          }
          .width('100%')
          .padding(16)
        }
        .width('100%')
        .align(Alignment.Top)
        .scrollBar(BarState.Off)
      }
      .width('94%')
      // 高度跟随内容，最高不超过 88%，避免短表单留下一大片空白
      .constraintSize({ maxHeight: '88%' })
      .backgroundColor(Ink.card)
      .borderRadius(Radius.sheet)
      .shadow(Shadow.lift)
      .onClick(() => {
      })
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#52201914')
    .borderRadius(this.wide ? Radius.sheet : { topLeft: Radius.sheet, topRight: Radius.sheet })
    .clip(true)
    .justifyContent(FlexAlign.Center)
    .alignItems(HorizontalAlign.Center)
  }
}
