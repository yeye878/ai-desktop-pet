import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { bus, Ev, BusEvent, BusListener } from '../commons/EventBus';
import { ChatMessage } from '../core/Types';
import { ChatService } from '../core/ChatService';
import { AgentRepo } from '../core/AgentRepo';
import { Gates } from '../core/Gates';
import { IntentEngine, IntentHit } from '../core/IntentEngine';
import { AsrController } from '../voice/AsrController';
import { TtsController } from '../voice/TtsController';
import { AgentProfile } from '../core/Types';

interface ConfirmPayload {
  id: string;
  tool: string;
  args: string;
}

interface AskPayload {
  id: string;
  question: string;
  options: Array<Record<string, string>>;
}

/**
 * 对话页（对应桌面版 ChatBubble + VoicePanel + ToolConfirmPanel 的合并重构）：
 * 流式对话、@子智能体、/命令、图片多模态、语音指令、工具确认、ask_user
 */
@Component
export struct ChatPage {
  @State msgs: ChatMessage[] = [];
  @State busy: boolean = false;
  @State input: string = '';
  @State pickedImages: string[] = [];
  @State showVoice: boolean = false;
  @State voicePartial: string = '';
  @State voiceState: string = 'idle';
  @State voiceConfirmHit: IntentHit | null = null;
  @State voiceConfirmRaw: string = '';
  @State showConfirm: boolean = false;
  @State confirmPayload: ConfirmPayload | null = null;
  @State showAsk: boolean = false;
  @State askPayload: AskPayload | null = null;
  @State askReply: string = '';
  @State agents: AgentProfile[] = [];
  private scroller: Scroller = new Scroller();
  // 监听器持有成员引用，aboutToDisappear 统一注销——Index 条件渲染切页会销毁组件，
  // 不注销则旧闭包累积（静音断句时同一句话被重复发送 N 次）
  private onChatChanged: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, Object> | null;
    if (d !== null && d !== undefined) {
      const m = d['messages'];
      const b = d['busy'];
      this.msgs = m as ChatMessage[];
      this.busy = b === true;
    }
    setTimeout(() => {
      this.scroller.scrollEdge(Edge.Bottom);
    }, 30);
  };
  private onVoicePartial: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, string> | null;
    if (d !== null && d !== undefined) {
      this.voicePartial = d['text'] ?? '';
    }
  };
  private onVoiceState: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, string> | null;
    if (d !== null && d !== undefined) {
      this.voiceState = d['state'] ?? 'idle';
      if (this.voiceState === 'idle' || this.voiceState === 'error') {
        // 静音自动断句 → 低置信度复述确认或直接发送（命题 R5）
        if (this.showVoice) {
          const finalText = AsrController.stop();
          const trimmed = finalText.trim();
          if (trimmed.length >= 2) {
            const hit = IntentEngine.route(trimmed);
            if (hit !== null && hit.corrected) {
              // 纠偏命中：复述确认，杜绝错误指令静默执行
              this.voiceConfirmHit = hit;
              this.voiceConfirmRaw = trimmed;
            } else {
              this.sendText(trimmed);
              this.showVoice = false;
            }
          } else if (trimmed.length === 0) {
            this.showVoice = false;
          } else {
            this.sendText(trimmed);
            this.showVoice = false;
          }
        }
      }
    }
  };
  private onRequestImage: BusListener = (_e: BusEvent) => {
    // 复合指令"拍一下这个文档"缺照片时：自动拉起相册选择
    this.pickImage();
  };
  private onToolConfirm: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, string> | null;
    if (d !== null && d !== undefined) {
      this.confirmPayload = { id: d['id'] ?? '', tool: d['tool'] ?? '', args: d['args'] ?? '' };
      this.showConfirm = true;
    }
  };
  private onAskUser: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, Object> | null;
    if (d !== null && d !== undefined) {
      const opts: Array<Record<string, string>> = [];
      const raw = d['options'] as Array<Object> | undefined;
      if (raw !== undefined) {
        raw.forEach((o: Object) => {
          const m = o as Record<string, string>;
          opts.push(m);
        });
      }
      this.askPayload = {
        id: typeof d['id'] === 'string' ? d['id'] as string : '',
        question: typeof d['question'] === 'string' ? d['question'] as string : '',
        options: opts
      };
      this.showAsk = true;
    }
  };
  private onAiError: BusListener = (e: BusEvent) => {
    const d = e.data as Record<string, string> | null;
    if (d !== null && d !== undefined) {
      ChatService.pushLocalAssistantMessage(`⚠️ ${d['message'] ?? '未知错误'}`);
    }
  };
  private onNavigate: BusListener = (e: BusEvent) => {
    // 消费 AgentsPage/EntryAbility 携带的 draft（@提及预填/接续草稿）与 continuation（跨设备上下文）
    const d = e.data as Record<string, string> | null;
    if (d !== null && d !== undefined) {
      const draft = d['draft'];
      if (typeof draft === 'string' && draft.length > 0) {
        this.input = this.input.length > 0 ? `${this.input} ${draft}` : draft;
      }
      const continuation = d['continuation'];
      if (typeof continuation === 'string' && continuation.length > 0) {
        ChatService.restoreContinuation(continuation);
      }
    }
  };

  aboutToAppear(): void {
    const snap = ChatService.snapshot();
    this.msgs = snap.messages;
    this.busy = snap.busy;
    AgentRepo.list().then((list: AgentProfile[]) => {
      this.agents = list;
    }).catch((_e: Error) => {
    });
    bus.on(Ev.CHAT_CHANGED, this.onChatChanged);
    bus.on(Ev.VOICE_PARTIAL, this.onVoicePartial);
    bus.on(Ev.VOICE_STATE, this.onVoiceState);
    bus.on(Ev.REQUEST_IMAGE, this.onRequestImage);
    bus.on(Ev.TOOL_CONFIRM, this.onToolConfirm);
    bus.on(Ev.ASK_USER, this.onAskUser);
    bus.on(Ev.AI_ERROR, this.onAiError);
    bus.on(Ev.NAVIGATE, this.onNavigate);
  }

  aboutToDisappear(): void {
    bus.off(Ev.CHAT_CHANGED, this.onChatChanged);
    bus.off(Ev.VOICE_PARTIAL, this.onVoicePartial);
    bus.off(Ev.VOICE_STATE, this.onVoiceState);
    bus.off(Ev.REQUEST_IMAGE, this.onRequestImage);
    bus.off(Ev.TOOL_CONFIRM, this.onToolConfirm);
    bus.off(Ev.ASK_USER, this.onAskUser);
    bus.off(Ev.AI_ERROR, this.onAiError);
    bus.off(Ev.NAVIGATE, this.onNavigate);
  }

  sendText(text: string): void {
    const images = this.pickedImages;
    this.pickedImages = [];
    this.input = '';
    ChatService.send(text, images);
  }

  async pickImage(): Promise<void> {
    const photoPicker = new photoAccessHelper.PhotoViewPicker();
    const opts: photoAccessHelper.PhotoSelectOptions = {
      MIMEType: photoAccessHelper.PhotoViewMIMETypes.IMAGE_TYPE,
      maxSelectNumber: 3
    };
    const result = await photoPicker.select(opts);
    if (result.photoUris && result.photoUris.length > 0) {
      // 复合指令缺省文案：拍照/选图后自动带出总结指令（对应命题示例场景）
      this.pickedImages = this.pickedImages.concat(result.photoUris);
      if (this.input.trim().length === 0) {
        this.input = '看一下这个文档，帮我总结重点';
      }
    }
  }

  build() {
    Stack() {
      Column() {
        // 消息列表
        List({ scroller: this.scroller, space: 8 }) {
          if (this.msgs.length === 0) {
            ListItem() {
              Column({ space: 8 }) {
                Text('🐹').fontSize(40)
                Text('嗨，我是小伴！')
                  .fontSize(16)
                  .fontWeight(FontWeight.Bold)
                  .fontColor('#5B4A4F')
                Text('试试：\n· @学习助手 帮我复习一下概念\n· /team 准备明天的答辩\n· 点 🎤 语音说「今天日程」\n· 发图片说「帮我总结重点」')
                  .fontSize(13)
                  .fontColor('#8A7A80')
                  .textAlign(TextAlign.Center)
              }
              .width('100%')
              .padding({ top: 60 })
            }
          }
          ForEach(this.msgs, (m: ChatMessage) => {
            ListItem() {
              this.Bubble(m)
            }
          }, (m: ChatMessage) => m.id)
        }
        .layoutWeight(1)
        .width('100%')
        .padding({ left: 12, right: 12, top: 6 })

        // 图片预览条
        if (this.pickedImages.length > 0) {
          Row({ space: 6 }) {
            ForEach(this.pickedImages, (uri: string) => {
              Image(uri)
                .width(52)
                .height(52)
                .borderRadius(8)
                .objectFit(ImageFit.Cover)
            }, (uri: string) => uri)
            Text('点 ✖ 移除').fontSize(11).fontColor('#8A7A80')
            Button('✖')
              .type(ButtonType.Circle)
              .width(26)
              .height(26)
              .backgroundColor('#F0E4E8')
              .fontColor('#8A7A80')
              .onClick(() => {
                this.pickedImages = [];
              })
          }
          .width('100%')
          .padding({ left: 12, right: 12, top: 4 })
        }

        // @提及提示
        if (this.agents.length > 0) {
          Row({ space: 6 }) {
            Text('@').fontSize(13).fontColor('#8A7A80')
            ForEach(this.agents, (a: AgentProfile) => {
              Text(`${a.avatar} ${a.name}`)
                .fontSize(12)
                .fontColor('#C2557E')
                .backgroundColor('#FFE3EA')
                .borderRadius(10)
                .padding({ left: 8, right: 8, top: 3, bottom: 3 })
                .onClick(() => {
                  this.input = `${this.input}@${a.name} `;
                })
            }, (a: AgentProfile) => a.id)
          }
          .width('100%')
          .padding({ left: 12, right: 12 })
        }

        // 输入区
        Row({ space: 8 }) {
          Button(this.busy ? '⏹' : (TtsController.isSpeaking() ? '🔊' : '🎤'))
            .type(ButtonType.Circle)
            .width(40)
            .height(40)
            .backgroundColor('#FFE3EA')
            .fontColor('#C2557E')
            .onClick(() => {
              if (this.busy) {
                ChatService.abort();
                return;
              }
              if (TtsController.isSpeaking()) {
                // 播报中点击 = 打断：立即停止合成并进入聆听（用户可随时插话）
                TtsController.stop();
              }
              this.voicePartial = '';
              this.voiceConfirmHit = null;
              this.showVoice = true;
              AsrController.start();
            })
          Button('📷')
            .type(ButtonType.Circle)
            .width(40)
            .height(40)
            .backgroundColor('#FFF3D6')
            .fontColor('#8A6A1F')
            .onClick(() => {
              this.pickImage();
            })
          TextArea({ placeholder: '说点什么…（@智能体 /命令 /team 组队）', text: this.input })
            .layoutWeight(1)
            .height(42)
            .borderRadius(21)
            .backgroundColor('#FFFFFF')
            .onChange((v: string) => {
              this.input = v;
            })
            .onSubmit(() => {
              if (this.input.trim().length > 0 && !this.busy) {
                this.sendText(this.input.trim());
              }
            })
          Button(this.busy ? '…' : '➤')
            .type(ButtonType.Circle)
            .width(40)
            .height(40)
            .backgroundColor(this.busy ? '#F0E4E8' : '#F76C8A')
            .fontColor(this.busy ? '#8A7A80' : Color.White)
            .onClick(() => {
              if (this.busy) {
                ChatService.abort();
              } else if (this.input.trim().length > 0) {
                this.sendText(this.input.trim());
              }
            })
        }
        .width('100%')
        .padding({ left: 10, right: 10, top: 6, bottom: 10 })
      }
      .width('100%')
      .height('100%')

      // 语音浮层
      if (this.showVoice) {
        this.VoiceOverlay()
      }
      // 工具确认浮层
      if (this.showConfirm && this.confirmPayload !== null) {
        this.ConfirmOverlay()
      }
      // ask_user 浮层
      if (this.showAsk && this.askPayload !== null) {
        this.AskOverlay()
      }
    }
    .width('100%')
    .height('100%')
  }

  // ---------- 浮层 ----------

  @Builder
  VoiceOverlay() {
    Column({ space: 16 }) {
      Blank()
      if (this.voiceConfirmHit !== null) {
        // 低置信度复述确认：识别疑似专有词误识时，先复述再执行（命题 R5 六层防线的确认兜底）
        Text('🤔 你是想说——').fontSize(15).fontColor('#8A7A80')
        Text(`「${this.voiceConfirmHit.phrase}」`)
          .fontSize(24)
          .fontWeight(FontWeight.Bold)
          .fontColor('#C2557E')
        Text(`（识别原文：${this.voiceConfirmRaw}）`)
          .fontSize(12)
          .fontColor('#B9A9AF')
        Button('对，执行')
          .width(200)
          .backgroundColor('#F76C8A')
          .onClick(() => {
            const phrase = this.voiceConfirmHit !== null ? this.voiceConfirmHit.phrase : '';
            this.voiceConfirmHit = null;
            this.showVoice = false;
            if (phrase.length > 0) {
              this.sendText(phrase);
            }
          })
        Button('按原文发送')
          .width(200)
          .backgroundColor('#FFE3EA')
          .fontColor('#C2557E')
          .onClick(() => {
            const raw = this.voiceConfirmRaw;
            this.voiceConfirmHit = null;
            this.showVoice = false;
            if (raw.length > 0) {
              this.sendText(raw);
            }
          })
        Text('重新说')
          .fontSize(13)
          .fontColor('#8A7A80')
          .onClick(() => {
            this.voiceConfirmHit = null;
            this.voicePartial = '';
            AsrController.start();
          })
      } else {
        Text('🎙')
          .fontSize(64)
          .opacity(this.voiceState === 'listening' ? 1 : 0.5)
        Text(this.voiceState === 'listening' ? '正在听…说完停顿一下即可发送' : '语音引擎启动中…')
          .fontSize(14)
          .fontColor('#8A7A80')
        Text(this.voicePartial.length > 0 ? this.voicePartial : '…')
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
          .fontColor('#5B4A4F')
          .textAlign(TextAlign.Center)
          .padding({ left: 24, right: 24 })
          .maxLines(4)
        Button('完成')
          .width(160)
          .backgroundColor('#F76C8A')
          .onClick(() => {
            const finalText = AsrController.stop();
            this.showVoice = false;
            if (finalText.trim().length > 0) {
              this.sendText(finalText.trim());
            }
          })
        Text('取消')
          .fontSize(13)
          .fontColor('#8A7A80')
          .onClick(() => {
            AsrController.cancel();
            this.showVoice = false;
          })
      }
      Blank()
    }
    .width('100%')
    .height('100%')
    .backgroundColor('rgba(255,247,244,0.98)')
  }

  @Builder
  ConfirmOverlay() {
    Column() {
      Column({ space: 10 }) {
        Row({ space: 8 }) {
          Text('🔐').fontSize(22)
          Text('工具执行确认').fontSize(16).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
        }
        Text(`我请求执行工具：${this.confirmPayload?.tool}`)
          .fontSize(14)
          .fontColor('#5B4A4F')
          .width('100%')
        Text(this.confirmPayload?.args ?? '')
          .fontSize(12)
          .fontColor('#8A7A80')
          .backgroundColor('#F7F2F4')
          .borderRadius(8)
          .padding(8)
          .width('100%')
          .maxLines(6)
        Row({ space: 12 }) {
          Button('拒绝')
            .layoutWeight(1)
            .backgroundColor('#F0E4E8')
            .fontColor('#8A7A80')
            .onClick(() => {
              if (this.confirmPayload !== null) {
                Gates.resolveToolConfirm(this.confirmPayload.id, false);
              }
              this.showConfirm = false;
            })
          Button('批准执行')
            .layoutWeight(1)
            .backgroundColor('#F76C8A')
            .onClick(() => {
              if (this.confirmPayload !== null) {
                Gates.resolveToolConfirm(this.confirmPayload.id, true);
              }
              this.showConfirm = false;
            })
        }
        .width('100%')
        Text('批准后本会话同类工具将不再重复确认').fontSize(11).fontColor('#B9A9AF')
      }
      .padding(16)
      .backgroundColor('#FFFFFF')
      .borderRadius(16)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('rgba(60,40,45,0.35)')
    .justifyContent(FlexAlign.Center)
    .padding(20)
  }

  @Builder
  AskOverlay() {
    Column() {
      Column({ space: 10 }) {
        Text('❓ 需要你确认').fontSize(16).fontWeight(FontWeight.Bold).fontColor('#5B4A4F')
        Text(this.askPayload?.question ?? '')
          .fontSize(14)
          .fontColor('#5B4A4F')
          .width('100%')
        if (this.askPayload !== null && this.askPayload.options.length > 0) {
          Column({ space: 8 }) {
            ForEach(this.askPayload.options, (o: Record<string, string>, idx: number) => {
              Button(o['label'] ?? `选项${idx + 1}`)
                .width('100%')
                .backgroundColor('#FFE3EA')
                .fontColor('#C2557E')
                .onClick(() => {
                  if (this.askPayload !== null) {
                    Gates.resolveAskUser(this.askPayload.id, o['label'] ?? '');
                  }
                  this.showAsk = false;
                })
            }, (o: Record<string, string>, idx: number) => `${idx}_${o['label']}`)
          }
          .width('100%')
        } else {
          TextInput({ placeholder: '输入你的回答…', text: this.askReply })
            .width('100%')
            .onChange((v: string) => {
              this.askReply = v;
            })
          Button('回答')
            .width('100%')
            .backgroundColor('#F76C8A')
            .onClick(() => {
              if (this.askPayload !== null) {
                Gates.resolveAskUser(this.askPayload.id, this.askReply.length > 0 ? this.askReply : '(空回答)');
              }
              this.showAsk = false;
            })
        }
        Text('跳过')
          .fontSize(13)
          .fontColor('#8A7A80')
          .onClick(() => {
            if (this.askPayload !== null) {
              Gates.resolveAskUser(this.askPayload.id, '(用户跳过了这个问题)');
            }
            this.showAsk = false;
          })
      }
      .padding(16)
      .backgroundColor('#FFFFFF')
      .borderRadius(16)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('rgba(60,40,45,0.35)')
    .justifyContent(FlexAlign.Center)
    .padding(20)
  }

  // ---------- 消息气泡 ----------

  @Builder
  Bubble(m: ChatMessage) {
    Row() {
      if (m.role === 'user') {
        Blank()
        this.UserBubble(m)
      } else {
        this.AssistantBubble(m)
        Blank()
      }
    }
    .width('100%')
  }

  @Builder
  UserBubble(m: ChatMessage) {
    Column({ space: 6 }) {
      if (m.quote.length > 0) {
        Text(`回复：${m.quote}`)
          .fontSize(11)
          .fontColor('#8A7A80')
          .backgroundColor('#F7F2F4')
          .borderRadius(6)
          .padding(6)
          .width('100%')
      }
      if (m.imageUris.length > 0) {
        Row({ space: 4 }) {
          ForEach(m.imageUris, (uri: string) => {
            Image(uri.startsWith('file://') ? uri : `file://${uri}`)
              .width(96)
              .height(96)
              .borderRadius(10)
              .objectFit(ImageFit.Cover)
          }, (uri: string) => uri)
        }
      }
      if (m.content.length > 0) {
        Text(m.content)
          .fontSize(15)
          .fontColor(Color.White)
      }
    }
    .constraintSize({ maxWidth: '82%' })
    .padding(10)
    .backgroundColor('#F76C8A')
    .borderRadius(14)
    .alignItems(HorizontalAlign.End)
  }

  @Builder
  AssistantBubble(m: ChatMessage) {
    Row({ space: 6 }) {
      Text(m.agentAvatar.length > 0 ? m.agentAvatar : '🐹')
        .fontSize(20)
      Column({ space: 4 }) {
        if (m.agentName.length > 0) {
          Text(m.agentName).fontSize(11).fontColor('#C2557E')
        }
        if (m.thinking.length > 0) {
          Text(`💭 ${m.thinking}`)
            .fontSize(11)
            .fontColor('#B9A9AF')
            .backgroundColor('#F7F2F4')
            .borderRadius(8)
            .padding(6)
            .width('100%')
            .maxLines(6)
        }
        if (m.toolEvents.length > 0) {
          Column({ space: 2 }) {
            ForEach(m.toolEvents, (t: string) => {
              Text(t).fontSize(11).fontColor('#8A7A80')
            }, (t: string, i: number) => `${i}_${t.substring(0, 40)}`)
          }
          .width('100%')
          .padding(6)
          .backgroundColor('#F7F2F4')
          .borderRadius(8)
        }
        Text(m.content.length > 0 ? m.content : (this.busy ? '…' : '(无内容)'))
          .fontSize(15)
          .fontColor('#3D2E33')
          .width('100%')
      }
      .alignItems(HorizontalAlign.Start)
      .constraintSize({ maxWidth: '78%' })
    }
    .width('100%')
    .alignItems(VerticalAlign.Top)
  }
}
