import { AppContext, SettingKey } from '../commons/AppContext';
import { LlmClient, LlmProfiles } from '../core/LlmClient';
import { LlmProfile } from '../core/Types';
import { AgentRepo } from '../core/AgentRepo';
import { DistributedSync } from '../commons/DistributedSync';

/**
 * 设置面板（API 配置管理 + 语音 + 隐私 + 关于）。
 * 对应桌面版 Settings.vue 的 API Profile 管理核心能力。
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
  @State autoSpeak: boolean = false;
  @State confirmTools: boolean = true;
  @State personality: string = 'gentle';
  @State hotWords: string = '';
  @State weatherCity: string = '北京';

  aboutToAppear(): void {
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
    try {
      const reply = await LlmClient.testConnection(this.edit);
      this.testResult = `✅ 连接成功: ${reply}`;
    } catch (e) {
      this.testResult = `❌ ${(e as Error).message}`;
    }
  }

  build() {
    Stack() {
      Column({ space: 10 }) {
      // 标题栏
      Row() {
        Text('设置').fontSize(17).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
        Blank()
        Button('✖')
          .type(ButtonType.Circle)
          .width(30)
          .height(30)
          .backgroundColor('#F7F2F4')
          .fontColor('#8A7A80')
          .onClick(() => {
            this.onClose();
          })
      }
      .width('100%')

      Scroll() {
        Column({ space: 14 }) {
          // API 配置
          Column({ space: 8 }) {
            Row() {
              Text('LLM API 配置（OpenAI 兼容 / 云端或本地网关）')
                .fontSize(14).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
              Blank()
              Button('＋ 添加')
                .height(30)
                .backgroundColor('#F76C8A')
                .onClick(() => {
                  this.newProfile();
                })
            }
            .width('100%')
            if (this.profiles.length === 0) {
              Text('还没有配置。支持任意 OpenAI 兼容服务（OpenAI / 通义 / DeepSeek / 本地推理网关）')
                .fontSize(12).fontColor('#B0670A').width('100%')
            }
            ForEach(this.profiles, (p: LlmProfile) => {
              Row({ space: 8 }) {
                Radio({ value: p.id, group: 'profiles' })
                  .checked(p.id === this.activeId)
                  .onChange((on: boolean) => {
                    if (on) {
                      LlmProfiles.setActive(p.id).then(() => {
                        this.refresh();
                      });
                    }
                  })
                Column({ space: 2 }) {
                  Text(`${p.name}  ${p.model}`).fontSize(13).fontColor('#3D2E33')
                  Text(p.baseUrl).fontSize(10).fontColor('#B9A9AF').maxLines(1)
                }
                .alignItems(HorizontalAlign.Start)
                .layoutWeight(1)
                Button('编辑').height(28).backgroundColor('#F7F2F4').fontColor('#5B4A4F').onClick(() => {
                  this.editProfile(p);
                })
                Button('删').height(28).backgroundColor('#F7F2F4').fontColor('#B0670A').onClick(() => {
                  LlmProfiles.removeProfile(p.id).then(() => {
                    this.refresh();
                  });
                })
              }
              .width('100%')
              .padding(8)
              .backgroundColor('#FDF8F6')
              .borderRadius(10)
            }, (p: LlmProfile) => p.id)
          }
          .width('100%')
          .padding(12)
          .backgroundColor('#FFFFFF')
          .borderRadius(14)

          // 语音
          Column({ space: 8 }) {
            Text('语音').fontSize(14).fontWeight(FontWeight.Bold).fontColor('#5B4A4F').width('100%')
            Row() {
              Text('回复自动朗读（端侧 TTS）').fontSize(13).fontColor('#5B4A4F').layoutWeight(1)
              Toggle({ type: ToggleType.Switch, isOn: this.autoSpeak })
                .selectedColor('#F76C8A')
                .onChange((on: boolean) => {
                  AppContext.setBool(SettingKey.VOICE_AUTO_SPEAK, on).then(() => {
                    this.refresh();
                  });
                })
            }
            .width('100%')
            Column({ space: 4 }) {
              Text('识别热词（逗号分隔，提升语音指令识别率）').fontSize(12).fontColor('#8A7A80').width('100%')
              TextArea({ text: this.hotWords })
                .height(60)
                .onChange((v: string) => {
                  this.hotWords = v;
                  AppContext.setSetting(SettingKey.HOT_WORDS, v);
                })
            }
            .width('100%')
          }
          .width('100%')
          .padding(12)
          .backgroundColor('#FFFFFF')
          .borderRadius(14)

          // 行为与隐私
          Column({ space: 8 }) {
            Text('行为').fontSize(14).fontWeight(FontWeight.Bold).fontColor('#5B4A4F').width('100%')
            Row() {
              Text('高风险工具执行前需要确认').fontSize(13).fontColor('#5B4A4F').layoutWeight(1)
              Toggle({ type: ToggleType.Switch, isOn: this.confirmTools })
                .selectedColor('#F76C8A')
                .onChange((on: boolean) => {
                  AppContext.setBool(SettingKey.TOOL_CONFIRM, on).then(() => {
                    this.refresh();
                  });
                })
            }
            .width('100%')
            Column({ space: 4 }) {
              Text('性格').fontSize(12).fontColor('#8A7A80').width('100%')
              Row({ space: 6 }) {
                ForEach([['gentle', '温柔'], ['energetic', '元气'], ['calm', '沉稳'], ['mentor', '导师'], ['witty', '机灵']],
                  (item: Array<string>) => {
                    Text(item[1])
                      .fontSize(12)
                      .fontColor(this.personality === item[0] ? '#C2557E' : '#8A7A80')
                      .backgroundColor(this.personality === item[0] ? '#FFE3EA' : '#F7F2F4')
                      .borderRadius(10)
                      .padding({ left: 10, right: 10, top: 4, bottom: 4 })
                      .onClick(() => {
                        AppContext.setSetting(SettingKey.PERSONALITY, item[0]).then(() => {
                          this.refresh();
                        });
                      })
                  }, (item: Array<string>) => item[0])
              }
            }
            .width('100%')
            Column({ space: 4 }) {
              Text('天气城市（日程管家简报用）').fontSize(12).fontColor('#8A7A80').width('100%')
              TextInput({ text: this.weatherCity })
                .height(38)
                .onChange((v: string) => {
                  this.weatherCity = v;
                  AppContext.setSetting(SettingKey.WEATHER_CITY, v);
                })
            }
            .width('100%')
          }
          .width('100%')
          .padding(12)
          .backgroundColor('#FFFFFF')
          .borderRadius(14)

          // 跨设备协同
          Column({ space: 4 }) {
            Text('跨设备协同').fontSize(14).fontWeight(FontWeight.Bold).fontColor('#5B4A4F').width('100%')
            Text(DistributedSync.statusText())
              .fontSize(12)
              .fontColor('#8A7A80')
              .width('100%')
            Text('任务接续：多设备登录同一华为账号后，任务中心可一键把当前对话流转到手机/平板/2in1 继续，' +
              '接续自动携带最近对话上下文。')
              .fontSize(11)
              .fontColor('#B9A9AF')
              .width('100%')
          }
          .width('100%')
          .padding(12)
          .backgroundColor('#FFFFFF')
          .borderRadius(14)

          // 关于
          Column({ space: 4 }) {
            Text('关于').fontSize(14).fontWeight(FontWeight.Bold).fontColor('#5B4A4F').width('100%')
            Text('OpenCompanion 智能伙伴 —— OpenHarmony 原生智能多模态个人助理（ArkTS + ArkUI）。' +
              '端侧 RAG：@kit.DataAugmentationKit；语音：@kit.CoreSpeechKit；OCR：@kit.CoreVisionKit。' +
              '数据全部存储在设备本地，API 密钥使用系统资产加密存储。')
              .fontSize(11)
              .fontColor('#8A7A80')
              .width('100%')
          }
          .width('100%')
          .padding(12)
          .backgroundColor('#FFFFFF')
          .borderRadius(14)
        }
        .width('100%')
        .padding(14)
      }
      .layoutWeight(1)
      .width('100%')
      .scrollBar(BarState.Auto)
      }
      .width('100%')
      .height('100%')

      // API 配置编辑浮层
      if (this.showEditor) {
        this.ProfileEditor()
      }
    }
    .width('100%')
    .height('100%')
  }

  @Builder
  ProfileEditor() {
    Column() {
      Scroll() {
        Column({ space: 10 }) {
          Text(this.edit.name.length > 0 ? `编辑 ${this.edit.name}` : '新增 API 配置')
            .fontSize(15).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
          TextInput({ placeholder: '配置名称', text: this.edit.name })
            .height(40)
            .onChange((v: string) => {
              this.edit.name = v;
            })
          TextInput({ placeholder: 'Base URL（如 https://api.openai.com/v1）', text: this.edit.baseUrl })
            .height(40)
            .onChange((v: string) => {
              this.edit.baseUrl = v;
            })
          TextInput({ placeholder: '模型名（如 gpt-4o-mini / qwen-plus）', text: this.edit.model })
            .height(40)
            .onChange((v: string) => {
              this.edit.model = v;
            })
          TextInput({ placeholder: 'API Key（存储在系统安全区，不会明文落盘）', text: this.edit.apiKey })
            .type(InputType.Password)
            .height(40)
            .onChange((v: string) => {
              this.edit.apiKey = v;
            })
          Row({ space: 10 }) {
            Text('流式').fontSize(13)
            Toggle({ type: ToggleType.Switch, isOn: this.edit.useStream })
              .selectedColor('#F76C8A')
              .onChange((on: boolean) => {
                this.edit.useStream = on;
              })
            Text('工具需确认').fontSize(13)
            Toggle({ type: ToggleType.Switch, isOn: this.edit.confirmEnabled })
              .selectedColor('#F76C8A')
              .onChange((on: boolean) => {
                this.edit.confirmEnabled = on;
              })
          }
          if (this.testResult.length > 0) {
            Text(this.testResult).fontSize(12).fontColor(this.testResult.startsWith('✅') ? '#3E8E5A' : '#B0670A')
              .width('100%')
          }
          Row({ space: 10 }) {
            Button('测试连接')
              .layoutWeight(1)
              .backgroundColor('#9D7BFF')
              .onClick(() => {
                this.testConn();
              })
            Button('保存')
              .layoutWeight(1)
              .backgroundColor('#F76C8A')
              .onClick(() => {
                this.saveProfile();
              })
          }
          .width('100%')
        }
        .width('100%')
        .padding(16)
      }
      .width('100%')
      .height('100%')
    }
    .width('92%')
    .padding(6)
    .backgroundColor('#FFFFFF')
    .borderRadius(16)
  }
}
