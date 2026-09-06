import { Ink, Radius, Shadow } from './DesignTokens';

@Builder
function emptyBuilder(): void {
}

/**
 * OpenCompanion 通用组件库。
 * 所有页面共用这一套视觉原子，保证「一套圆角 / 一套墨阶 / 一个强调色」。
 */

// ---------- 图标 ----------

@Component
export struct AppIcon {
  @Prop icon: Resource = $r('sys.symbol.circle');
  @Prop sizeVp: number = 20;
  @Prop color: string = Ink.text2;
  @Prop weight: number = 500;

  build() {
    SymbolGlyph(this.icon)
      .fontSize(this.sizeVp)
      .fontColor([this.color])
      .fontWeight(this.weight)
  }
}

// ---------- 按钮 ----------

@Component
export struct PillButton {
  @Prop label: string = '';
  @Prop icon: Resource | null = null;
  /** ink | accent | soft | quiet | danger */
  @Prop variant: string = 'ink';
  @Prop isEnabled: boolean = true;
  @Prop heightVp: number = 44;
  @Prop fill: boolean = false;
  @Prop fontSizeVp: number = 15;
  onTap: () => void = () => {
  };
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';

  private bg(): string {
    if (!this.isEnabled) {
      return Ink.sunken;
    }
    if (this.variant === 'ink') {
      return Ink.text;
    }
    if (this.variant === 'accent') {
      return this.accentDeep;
    }
    if (this.variant === 'soft') {
      return this.accentTint;
    }
    if (this.variant === 'danger') {
      return Ink.dangerSoft;
    }
    return Ink.sunken;
  }

  private fg(): string {
    if (!this.isEnabled) {
      return Ink.text3;
    }
    if (this.variant === 'ink') {
      return Ink.onInk;
    }
    if (this.variant === 'accent') {
      return '#FFFFFF';
    }
    if (this.variant === 'soft') {
      return this.accentDeep;
    }
    if (this.variant === 'danger') {
      return Ink.danger;
    }
    return Ink.text2;
  }

  build() {
    Button() {
      Row({ space: 6 }) {
        if (this.icon !== null) {
          AppIcon({
            icon: this.icon as Resource,
            sizeVp: this.fontSizeVp + 2,
            color: this.fg(),
            weight: 600
          })
        }
        Text(this.label)
          .fontSize(this.fontSizeVp)
          .fontWeight(FontWeight.Medium)
          .fontColor(this.fg())
      }
    }
    .type(ButtonType.Capsule)
    .height(this.heightVp)
    .width(this.fill ? '100%' : undefined)
    .backgroundColor(this.bg())
    .enabled(this.isEnabled)
    .padding({ left: 18, right: 18 })
    .opacity(this.isEnabled ? 1 : 0.75)
    .onClick(() => {
      if (this.isEnabled) {
        this.onTap();
      }
    })
  }
}

@Component
export struct IconButton {
  @Prop icon: Resource = $r('sys.symbol.circle');
  @Prop bg: string = Ink.sunken;
  @Prop color: string = Ink.text2;
  @Prop sizeVp: number = 38;
  @Prop iconVp: number = 18;
  onTap: () => void = () => {
  };

  build() {
    Button() {
      AppIcon({ icon: this.icon, sizeVp: this.iconVp, color: this.color, weight: 600 })
    }
    .type(ButtonType.Circle)
    .width(this.sizeVp)
    .height(this.sizeVp)
    .backgroundColor(this.bg)
    .onClick(() => {
      this.onTap();
    })
  }
}

// ---------- 容器 ----------

@Component
export struct Card {
  @Prop padVp: number = 16;
  @Prop bg: string = Ink.card;
  @Prop radiusVp: number = Radius.card;
  @Prop bordered: boolean = true;
  @BuilderParam content: () => void = emptyBuilder;

  build() {
    Column() {
      this.content()
    }
    .width('100%')
    .padding(this.padVp)
    .backgroundColor(this.bg)
    .borderRadius(this.radiusVp)
    .border({
      width: this.bordered ? 0.5 : 0,
      color: Ink.border
    })
  }
}

@Component
export struct SectionHeader {
  @Prop title: string = '';
  @Prop caption: string = '';

  build() {
    Row() {
      Text(this.title)
        .fontSize(16)
        .fontWeight(FontWeight.Bold)
        .fontColor(Ink.text)
        .layoutWeight(1)
      if (this.caption.length > 0) {
        Text(this.caption)
          .fontSize(12)
          .fontColor(Ink.text3)
      }
    }
    .width('100%')
  }
}

@Component
export struct Hairline {
  @Prop insetVp: number = 0;

  build() {
    Row()
      .width('100%')
      .height(0.5)
      .backgroundColor(Ink.hairline)
      .margin({ left: this.insetVp })
  }
}

// ---------- 小件 ----------

@Component
export struct Chip {
  @Prop label: string = '';
  @Prop icon: Resource | null = null;
  /** accent | sunken | ok | warn | danger */
  @Prop variant: string = 'sunken';
  @StorageProp('uiAccentDeep') accentDeep: string = '#F76C8A';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';

  private fg(): string {
    if (this.variant === 'accent') {
      return this.accentDeep;
    }
    if (this.variant === 'ok') {
      return Ink.ok;
    }
    if (this.variant === 'warn') {
      return Ink.warn;
    }
    if (this.variant === 'danger') {
      return Ink.danger;
    }
    return Ink.text2;
  }

  private bg(): string {
    if (this.variant === 'accent') {
      return this.accentTint;
    }
    if (this.variant === 'ok') {
      return Ink.okSoft;
    }
    if (this.variant === 'warn') {
      return Ink.warnSoft;
    }
    if (this.variant === 'danger') {
      return Ink.dangerSoft;
    }
    return Ink.sunken;
  }

  build() {
    Row({ space: 4 }) {
      if (this.icon !== null) {
        AppIcon({ icon: this.icon as Resource, sizeVp: 12, color: this.fg(), weight: 600 })
      }
      Text(this.label)
        .fontSize(11.5)
        .fontWeight(FontWeight.Medium)
        .fontColor(this.fg())
    }
    .padding({ left: 9, right: 9, top: 4.5, bottom: 4.5 })
    .borderRadius(Radius.pill)
    .backgroundColor(this.bg())
  }
}

@Component
export struct AvatarTile {
  /** emoji 字符作为头像属于用户数据，不算 UI 装饰 */
  @Prop avatar: string = '🐹';
  @Prop sizeVp: number = 40;
  @Prop fontSizeVp: number = 20;
  @Prop bg: string = '';
  @StorageProp('uiAccentTint') accentTint: string = 'rgba(247,108,138,0.12)';

  build() {
    Column() {
      Text(this.avatar)
        .fontSize(this.fontSizeVp)
    }
    .width(this.sizeVp)
    .height(this.sizeVp)
    .borderRadius(this.sizeVp / 2)
    .backgroundColor(this.bg.length > 0 ? this.bg : this.accentTint)
    .justifyContent(FlexAlign.Center)
    .alignItems(HorizontalAlign.Center)
  }
}

@Component
export struct EmptyHint {
  @Prop icon: Resource = $r('sys.symbol.doc_text');
  @Prop title: string = '';
  @Prop sub: string = '';

  build() {
    Column({ space: 8 }) {
      AppIcon({ icon: this.icon, sizeVp: 30, color: Ink.text3, weight: 400 })
      Text(this.title)
        .fontSize(14)
        .fontWeight(FontWeight.Medium)
        .fontColor(Ink.text2)
      if (this.sub.length > 0) {
        Text(this.sub)
          .fontSize(12)
          .fontColor(Ink.text3)
          .textAlign(TextAlign.Center)
          .lineHeight(18)
      }
    }
    .width('100%')
    .padding({ top: 36, bottom: 36, left: 24, right: 24 })
    .alignItems(HorizontalAlign.Center)
  }
}

/** 状态行：图标 + 标签 + 右侧值（mono 数值 / 状态字） */
@Component
export struct StatusLine {
  @Prop icon: Resource = $r('sys.symbol.circle');
  @Prop label: string = '';
  @Prop value: string = '';
  /** ok | warn | muted */
  @Prop tone: string = 'muted';

  private dotColor(): string {
    if (this.tone === 'ok') {
      return Ink.ok;
    }
    if (this.tone === 'warn') {
      return Ink.warn;
    }
    return Ink.text3;
  }

  private valueColor(): string {
    if (this.tone === 'ok') {
      return Ink.ok;
    }
    if (this.tone === 'warn') {
      return Ink.warn;
    }
    return Ink.text2;
  }

  build() {
    Row({ space: 10 }) {
      AppIcon({ icon: this.icon, sizeVp: 16, color: Ink.text3, weight: 500 })
      Text(this.label)
        .fontSize(13)
        .fontColor(Ink.text2)
        .layoutWeight(1)
      Text(this.value)
        .fontSize(12)
        .fontWeight(FontWeight.Medium)
        .fontColor(this.valueColor())
      Circle({ width: 6, height: 6 }).fill(this.dotColor())
    }
    .width('100%')
    .height(36)
  }
}

// ---------- 分段控件 ----------

@Component
export struct SegmentedTabs {
  @Prop tabs: string[] = [];
  @Prop active: number = 0;
  onPick: (idx: number) => void = (_idx: number) => {
  };

  build() {
    Row({ space: 4 }) {
      ForEach(this.tabs, (label: string, idx: number) => {
        Text(label)
          .fontSize(13)
          .fontWeight(this.active === idx ? FontWeight.Bold : FontWeight.Normal)
          .fontColor(this.active === idx ? Ink.text : Ink.text2)
          .textAlign(TextAlign.Center)
          .layoutWeight(1)
          .height(34)
          .borderRadius(Radius.pill)
          .backgroundColor(this.active === idx ? Ink.card : Color.Transparent)
          .shadow(this.active === idx ? Shadow.hair : { radius: 0, color: Color.Transparent, offsetY: 0 })
          .animation({ duration: 220, curve: Curve.EaseOut })
          .onClick(() => {
            this.onPick(idx);
          })
      }, (label: string, idx: number) => `${idx}_${label}`)
    }
    .width('100%')
    .padding(3)
    .borderRadius(Radius.pill)
    .backgroundColor(Ink.sunken)
  }
}

// ---------- 设置行 ----------

@Component
export struct SettingRow {
  @Prop label: string = '';
  @Prop desc: string = '';
  @BuilderParam trailing: () => void = emptyBuilder;

  build() {
    Row({ space: 12 }) {
      Column({ space: 2 }) {
        Text(this.label)
          .fontSize(14)
          .fontColor(Ink.text)
        if (this.desc.length > 0) {
          Text(this.desc)
            .fontSize(11.5)
            .fontColor(Ink.text3)
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)

      this.trailing()
    }
    .width('100%')
    .padding({ top: 12, bottom: 12 })
  }
}

/** 输入框上方的小标签 */
@Component
export struct FieldLabel {
  @Prop text: string = '';

  build() {
    Text(this.text)
      .fontSize(12)
      .fontColor(Ink.text3)
      .width('100%')
  }
}

// ---------- 浮层 ----------

/**
 * 全屏浮层：墨色轻纱 + 底部圆角卡片（narrow）/ 居中卡片（wide）。
 * scrim 点击收起浮层；卡片本身拦截点击。
 */
@Component
export struct SheetOverlay {
  @Prop wide: boolean = false;
  @Prop title: string = '';
  @BuilderParam content: () => void = emptyBuilder;
  onClose: () => void = () => {
  };
  @State shown: boolean = false;

  aboutToAppear(): void {
    // 延迟一帧触发进入动效（淡入 + 上滑）
    setTimeout(() => {
      animateTo({ duration: 260, curve: Curve.EaseOut }, () => {
        this.shown = true;
      });
    }, 16);
  }

  private dismiss(): void {
    animateTo({ duration: 200, curve: Curve.EaseIn }, () => {
      this.shown = false;
    });
    setTimeout(() => {
      this.onClose();
    }, 200);
  }

  build() {
    Column() {
      Column() {
        // 把手
        if (!this.wide) {
          Row()
            .width(36)
            .height(4)
            .borderRadius(Radius.pill)
            .backgroundColor(Ink.hairline)
            .margin({ top: 10, bottom: 6 })
        }
        Row() {
          Text(this.title)
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
              this.dismiss();
            }
          })
        }
        .width('100%')

        Scroll() {
          Column() {
            this.content()
          }
          .width('100%')
          .padding({ top: 14, bottom: 24 })
        }
        .layoutWeight(1)
        .width('100%')
        .align(Alignment.Top)
        .scrollBar(BarState.Off)
      }
      .width(this.wide ? '66%' : '100%')
      .constraintSize({ maxHeight: this.wide ? '88%' : '94%' })
      .backgroundColor(Ink.card)
      .borderRadius(this.wide ? Radius.sheet : { topLeft: Radius.sheet, topRight: Radius.sheet })
      .padding({ left: 18, right: 18, top: this.wide ? 18 : 4, bottom: 8 })
      .shadow(Shadow.lift)
      .translate({ y: this.shown ? 0 : 480 })
      .opacity(this.shown ? 1 : 0)
    }
    .width('100%')
    .height('100%')
    .justifyContent(this.wide ? FlexAlign.Center : FlexAlign.End)
    .backgroundColor('#52201914')
    .onClick(() => {
      this.dismiss();
    })
  }
}
