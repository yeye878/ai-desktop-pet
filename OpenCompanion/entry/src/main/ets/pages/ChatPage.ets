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
import { Ink, Radius, Shadow } from '../ui/DesignTokens';
import { AppIcon, AvatarTile, IconButton, PillButton, Chip, ThinkingDots } from '../ui/Widgets';

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
 * 流式对话、@子智能体、/命令、图片多模态、语音指令、工具确认、ask_user。
 * 视觉：助手消息为无底框编辑排版，用户消息为强调色气泡，输入区为浮动胶囊条。
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
  @State pulseScale: number = 1;
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentLight') accentLight: string = '#FFE3EA';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';
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
      ChatService.pushLocalAssistantMessage(`出错啦：${d['message'] ?? '未知错误'}`);
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
    // 语音浮层的脉冲环
    setTimeout(() => {
      animateTo({
        duration: 1500,
        iterations: -1,
        playMode: PlayMode.Alternate,
        curve: Curve.EaseInOut
      }, () => {
        this.pulseScale = 1.35;
      });
    }, 200);
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
        List({ scroller: this.scroller, space: 14 }) {
          if (this.msgs.length === 0) {
            ListItem() {
              this.ChatEmpty()
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
        .padding({ left: 16, right: 16, top: 8 })
        .scrollBar(BarState.Off)
        .edgeEffect(EdgeEffect.Spring)

        // 图片预览条
        if (this.pickedImages.length > 0) {
          Row({ space: 8 }) {
            ForEach(this.pickedImages, (uri: string) => {
              Stack({ alignContent: Alignment.TopEnd }) {
                Image(uri)
                  .width(52)
                  .height(52)
                  .borderRadius(Radius.inner)
                  .objectFit(ImageFit.Cover)
                Circle({ width: 16, height: 16 })
                  .fill(Ink.text)
                  .margin({ right: -4, top: -4 })
                  .onClick(() => {
                    this.pickedImages = this.pickedImages.filter((u: string) => u !== uri);
                  })
              }
            }, (uri: string) => uri)
            Blank()
            Text(`${this.pickedImages.length} 张图片`)
              .fontSize(11.5)
              .fontColor(Ink.text3)
          }
          .width('100%')
          .padding({ left: 16, right: 16, top: 4 })
        }

        // @提及提示
        if (this.agents.length > 0) {
          Scroll() {
            Row({ space: 6 }) {
              ForEach(this.agents, (a: AgentProfile) => {
                Chip({
                  label: `@${a.name}`,
                  variant: 'accent',
                })
                  .onClick(() => {
                    this.input = `${this.input}@${a.name} `;
                  })
              }, (a: AgentProfile) => a.id)
            }
            .padding({ left: 16, right: 16 })
          }
          .scrollable(ScrollDirection.Horizontal)
          .scrollBar(BarState.Off)
          .width('100%')
        }

        // 浮动输入条
        Column() {
          Row({ space: 8 }) {
            IconButton({
              icon: this.busy
                ? $r('sys.symbol.stop_circle_fill')
                : (TtsController.isSpeaking() ? $r('sys.symbol.speaker_wave_3_fill') : $r('sys.symbol.mic_fill')),
              sizeVp: 40,
              iconVp: 18,
              bg: this.busy ? Ink.sunken : this.accentTint,
              color: this.busy ? Ink.text3 : this.accentDeep,
              onTap: () => {
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
              }
            })
            IconButton({
              icon: $r('sys.symbol.camera'),
              sizeVp: 40,
              iconVp: 18,
              bg: Ink.sunken,
              color: Ink.text2,
              onTap: () => {
                this.pickImage();
              }
            })
            TextArea({
              placeholder: '说点什么…',
              text: this.input
            })
              .layoutWeight(1)
              .constraintSize({ minHeight: 40, maxHeight: 96 })
              .backgroundColor(Color.Transparent)
              .placeholderColor(Ink.text3)
              .placeholderFont({ size: 14 })
              .caretColor(this.accentDeep)
              .padding({ left: 4, right: 4 })
              .onChange((v: string) => {
                this.input = v;
              })
              .onSubmit(() => {
                if (this.input.trim().length > 0 && !this.busy) {
                  this.sendText(this.input.trim());
                }
              })
            IconButton({
              icon: this.busy ? $r('sys.symbol.hourglass') : $r('sys.symbol.paperplane_fill'),
              sizeVp: 40,
              iconVp: 17,
              bg: this.busy ? Ink.sunken : this.accentDeep,
              color: this.busy ? Ink.text3 : '#FFFFFF',
              onTap: () => {
                if (this.busy) {
                  ChatService.abort();
                } else if (this.input.trim().length > 0) {
                  this.sendText(this.input.trim());
                }
              }
            })
          }
          .width('100%')
          .padding({ left: 10, right: 10, top: 8, bottom: 12 })
          .backgroundColor(Ink.card)
          .borderRadius(28)
          .border({ width: 0.5, color: Ink.border })
          .shadow(Shadow.soft)
        }
        .width('100%')
        .padding({ left: 12, right: 12, bottom: 2 })
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

  // ---------- 空态 ----------

  @Builder
  ChatEmpty() {
    Column({ space: 14 }) {
      Column({ space: 10 }) {
        AvatarTile({ avatar: '🐹', sizeVp: 64, fontSizeVp: 32 })
        Text('嗨，我是小伴')
          .fontSize(18)
          .fontWeight(FontWeight.Bold)
          .fontColor(Ink.text)
        Text('可以对话、语音、发图片，还能召唤智能体小队')
          .fontSize(13)
          .fontColor(Ink.text3)
          .textAlign(TextAlign.Center)
      }
      .alignItems(HorizontalAlign.Center)
      .padding({ top: 48 })

      Column({ space: 8 }) {
        this.SuggestRow('@学习助手', '帮我复习一下重点概念')
        this.SuggestRow('/team', '准备明天的答辩')
        this.SuggestRow('语音说', '今天有什么日程')
      }
      .width('100%')
      .padding({ top: 8 })
    }
    .width('100%')
    .padding({ left: 8, right: 8 })
  }

  @Builder
  SuggestRow(kicker: string, text: string) {
    Row({ space: 10 }) {
      Text(kicker)
        .fontSize(12)
        .fontWeight(FontWeight.Medium)
        .fontColor(this.accentDeep)
      Text(text)
        .fontSize(13)
        .fontColor(Ink.text2)
        .layoutWeight(1)
      AppIcon({ icon: $r('sys.symbol.arrow_right'), sizeVp: 13, color: Ink.text3, weight: 600 })
    }
    .width('100%')
    .padding({ left: 12, right: 12, top: 11, bottom: 11 })
    .borderRadius(Radius.inner)
    .backgroundColor(Ink.card)
    .border({ width: 0.5, color: Ink.border })
    .onClick(() => {
      const full = kicker.startsWith('@') || kicker.startsWith('/') ? `${kicker} ${text}` : text;
      this.sendText(full);
    })
  }

  // ---------- 浮层 ----------

  @Builder
  VoiceOverlay() {
    Column({ space: 20 }) {
      Blank()
      if (this.voiceConfirmHit !== null) {
        // 低置信度复述确认：识别疑似专有词误识时，先复述再执行（命题 R5 六层防线的确认兜底）
        Column({ space: 12 }) {
          AppIcon({ icon: $r('sys.symbol.questionmark_circle'), sizeVp: 26, color: Ink.text2, weight: 500 })
          Text('你是想说')
            .fontSize(14)
            .fontColor(Ink.text2)
          Text(`「${this.voiceConfirmHit.phrase}」`)
            .fontSize(26)
            .fontWeight(FontWeight.Bold)
            .fontColor(this.accentDeep)
          Text(`识别原文：${this.voiceConfirmRaw}`)
            .fontSize(12)
            .fontColor(Ink.text3)
        }
        .alignItems(HorizontalAlign.Center)
        Column({ space: 10 }) {
          PillButton({
            label: '对，执行',
            variant: 'ink',
            heightVp: 46,
            onTap: (): void => {
              const phrase = this.voiceConfirmHit !== null ? this.voiceConfirmHit.phrase : '';
              this.voiceConfirmHit = null;
              this.showVoice = false;
              if (phrase.length > 0) {
                this.sendText(phrase);
              }
            }
          })
          PillButton({
            label: '按原文发送',
            variant: 'soft',
            heightVp: 42,
            fontSizeVp: 13.5,
            onTap: (): void => {
              const raw = this.voiceConfirmRaw;
              this.voiceConfirmHit = null;
              this.showVoice = false;
              if (raw.length > 0) {
                this.sendText(raw);
              }
            }
          })
          Text('重新说')
            .fontSize(13)
            .fontColor(Ink.text3)
            .padding(8)
            .onClick(() => {
              this.voiceConfirmHit = null;
              this.voicePartial = '';
              AsrController.start();
            })
        }
        .width(240)
      } else {
        // 聆听态：脉冲环 + 麦克风
        Stack() {
          Circle({ width: 120, height: 120 })
            .fill(this.accentTint)
            .scale({ x: this.pulseScale, y: this.pulseScale })
          Circle({ width: 96, height: 96 })
            .fill('#FFFFFF')
            .shadow(Shadow.soft)
          AppIcon({
            icon: $r('sys.symbol.mic_fill'),
            sizeVp: 34,
            color: this.voiceState === 'listening' ? this.accentDeep : Ink.text3,
            weight: 600
          })
        }
        .width(140)
        .height(140)
        Column({ space: 8 }) {
          Text(this.voiceState === 'listening' ? '在听，说完停顿一下即可' : '语音引擎启动中')
            .fontSize(13)
            .fontColor(Ink.text3)
          Text(this.voicePartial.length > 0 ? this.voicePartial : '…')
            .fontSize(21)
            .fontWeight(FontWeight.Bold)
            .fontColor(Ink.text)
            .textAlign(TextAlign.Center)
            .lineHeight(30)
            .padding({ left: 28, right: 28 })
            .maxLines(4)
        }
        .alignItems(HorizontalAlign.Center)
        Row({ space: 14 }) {
          PillButton({
            label: '完成',
            icon: $r('sys.symbol.checkmark'),
            variant: 'ink',
            heightVp: 44,
            onTap: (): void => {
              const finalText = AsrController.stop();
              this.showVoice = false;
              if (finalText.trim().length > 0) {
                this.sendText(finalText.trim());
              }
            }
          })
          PillButton({
            label: '取消',
            variant: 'quiet',
            heightVp: 44,
            onTap: (): void => {
              AsrController.cancel();
              this.showVoice = false;
            }
          })
        }
        .width(260)
      }
      Blank()
    }
    .width('100%')
    .height('100%')
    .backgroundColor(this.accentLight)
    .opacity(0.985)
  }

  @Builder
  ConfirmOverlay() {
    Column() {
      Column({ space: 12 }) {
        Row({ space: 10 }) {
          Column() {
            AppIcon({ icon: $r('sys.symbol.lock_shield'), sizeVp: 18, color: this.accentDeep, weight: 600 })
          }
          .width(38)
          .height(38)
          .borderRadius(12)
          .backgroundColor(this.accentTint)
          .justifyContent(FlexAlign.Center)
          Column({ space: 1 }) {
            Text('工具执行确认')
              .fontSize(16)
              .fontWeight(FontWeight.Bold)
              .fontColor(Ink.text)
            Text('请求执行下方工具，请确认')
              .fontSize(11.5)
              .fontColor(Ink.text3)
          }
          .alignItems(HorizontalAlign.Start)
          .layoutWeight(1)
        }
        .width('100%')

        Chip({ label: this.confirmPayload?.tool ?? '', variant: 'sunken' })

        Text(this.confirmPayload?.args ?? '')
          .fontSize(12)
          .fontColor(Ink.text2)
          .fontFamily('monospace')
          .backgroundColor(Ink.sunken)
          .borderRadius(Radius.inner)
          .padding(10)
          .width('100%')
          .maxLines(6)

        Row({ space: 10 }) {
          PillButton({
            label: '拒绝',
            variant: 'quiet',
            heightVp: 44,
            fill: true,
            onTap: (): void => {
              if (this.confirmPayload !== null) {
                Gates.resolveToolConfirm(this.confirmPayload.id, false);
              }
              this.showConfirm = false;
            }
          })
            .layoutWeight(1)
          PillButton({
            label: '批准执行',
            icon: $r('sys.symbol.checkmark'),
            variant: 'accent',
            heightVp: 44,
            fill: true,
            onTap: (): void => {
              if (this.confirmPayload !== null) {
                Gates.resolveToolConfirm(this.confirmPayload.id, true);
              }
              this.showConfirm = false;
            }
          })
            .layoutWeight(1)
        }
        .width('100%')
        Text('批准后本会话同类工具将不再重复确认')
          .fontSize(11)
          .fontColor(Ink.text3)
          .width('100%')
          .textAlign(TextAlign.Center)
      }
      .padding(18)
      .backgroundColor(Ink.card)
      .borderRadius(Radius.card)
      .shadow(Shadow.lift)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#52201914')
    .justifyContent(FlexAlign.Center)
    .padding(20)
  }

  @Builder
  AskOverlay() {
    Column() {
      Column({ space: 12 }) {
        Row({ space: 10 }) {
          Column() {
            AppIcon({ icon: $r('sys.symbol.questionmark_circle'), sizeVp: 18, color: this.accentDeep, weight: 600 })
          }
          .width(38)
          .height(38)
          .borderRadius(12)
          .backgroundColor(this.accentTint)
          .justifyContent(FlexAlign.Center)
          Text('需要你确认')
            .fontSize(16)
            .fontWeight(FontWeight.Bold)
            .fontColor(Ink.text)
            .layoutWeight(1)
        }
        .width('100%')
        Text(this.askPayload?.question ?? '')
          .fontSize(14)
          .fontColor(Ink.text2)
          .lineHeight(21)
          .width('100%')
        if (this.askPayload !== null && this.askPayload.options.length > 0) {
          Column({ space: 8 }) {
            ForEach(this.askPayload.options, (o: Record<string, string>, idx: number) => {
              Button(o['label'] ?? `选项${idx + 1}`)
                .width('100%')
                .height(44)
                .type(ButtonType.Capsule)
                .fontSize(14)
                .fontWeight(FontWeight.Medium)
                .fontColor(Ink.text)
                .backgroundColor(Ink.sunken)
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
            .height(44)
            .backgroundColor(Ink.sunken)
            .borderRadius(Radius.inner)
            .caretColor(this.accentDeep)
            .onChange((v: string) => {
              this.askReply = v;
            })
          PillButton({
            label: '回答',
            variant: 'ink',
            heightVp: 44,
            fill: true,
            onTap: (): void => {
              if (this.askPayload !== null) {
                Gates.resolveAskUser(this.askPayload.id, this.askReply.length > 0 ? this.askReply : '(空回答)');
              }
              this.showAsk = false;
            }
          })
        }
        Text('跳过')
          .fontSize(13)
          .fontColor(Ink.text3)
          .padding(6)
          .onClick(() => {
            if (this.askPayload !== null) {
              Gates.resolveAskUser(this.askPayload.id, '(用户跳过了这个问题)');
            }
            this.showAsk = false;
          })
      }
      .alignItems(HorizontalAlign.Start)
      .padding(18)
      .backgroundColor(Ink.card)
      .borderRadius(Radius.card)
      .shadow(Shadow.lift)
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#52201914')
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
      }
    }
    .width('100%')
    .alignItems(VerticalAlign.Top)
  }

  @Builder
  UserBubble(m: ChatMessage) {
    Column({ space: 6 }) {
      if (m.quote.length > 0) {
        Text(`回复：${m.quote}`)
          .fontSize(11)
          .fontColor('rgba(255,255,255,0.75)')
          .backgroundColor('rgba(255,255,255,0.16)')
          .borderRadius(8)
          .padding(6)
          .width('100%')
      }
      if (m.imageUris.length > 0) {
        Row({ space: 4 }) {
          ForEach(m.imageUris, (uri: string) => {
            Image(uri.startsWith('file://') ? uri : `file://${uri}`)
              .width(96)
              .height(96)
              .borderRadius(Radius.inner)
              .objectFit(ImageFit.Cover)
          }, (uri: string) => uri)
        }
        .justifyContent(FlexAlign.End)
        .width('100%')
      }
      if (m.content.length > 0) {
        Text(m.content)
          .fontSize(15)
          .fontColor('#FFFFFF')
          .lineHeight(22)
      }
    }
    .constraintSize({ maxWidth: '80%' })
    .padding(12)
    .backgroundColor(this.accentDeep)
    .borderRadius({
      topLeft: Radius.card,
      topRight: Radius.card,
      bottomLeft: Radius.card,
      bottomRight: 5
    })
    .shadow(Shadow.hair)
    .alignItems(HorizontalAlign.End)
  }

  @Builder
  AssistantBubble(m: ChatMessage) {
    Row({ space: 9 }) {
      AvatarTile({
        avatar: m.agentAvatar.length > 0 ? m.agentAvatar : '🐹',
        sizeVp: 30,
        fontSizeVp: 15
      })
      Column({ space: 6 }) {
        // layoutWeight(1) 保证文本在列表内边距内换行
        if (m.agentName.length > 0) {
          Text(m.agentName)
            .fontSize(11)
            .fontWeight(FontWeight.Medium)
            .fontColor(this.accentDeep)
        }
        if (m.thinking.length > 0) {
          Text(m.thinking)
            .fontSize(11.5)
            .fontColor(Ink.text3)
            .backgroundColor(Ink.sunken)
            .borderRadius(Radius.inner)
            .padding(8)
            .width('100%')
            .maxLines(6)
        }
        if (m.toolEvents.length > 0) {
          Column({ space: 4 }) {
            ForEach(m.toolEvents, (t: string) => {
              Text(t)
                .fontSize(11)
                .fontColor(Ink.text3)
                .fontFamily('monospace')
                .maxLines(2)
            }, (t: string, i: number) => `${i}_${t.substring(0, 40)}`)
          }
          .width('100%')
          .padding({ left: 10, right: 10, top: 7, bottom: 7 })
          .backgroundColor(Ink.sunken)
          .borderRadius(Radius.inner)
        }
        if (m.content.length > 0) {
          Text(m.content)
            .fontSize(15)
            .fontColor(Ink.text)
            .lineHeight(23)
            .width('100%')
        } else if (this.busy) {
          // 生成中：三点波动，替代静态省略号
          ThinkingDots({ color: this.accentDeep })
        } else {
          Text('(无内容)')
            .fontSize(13)
            .fontColor(Ink.text3)
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
    }
    .width('100%')
    .alignItems(VerticalAlign.Top)
  }
}
