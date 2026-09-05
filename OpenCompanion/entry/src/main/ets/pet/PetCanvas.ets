import { BehaviorEngine } from './BehaviorEngine';
import { ThemeEngine } from './ThemeEngine';

@Component
export struct PetCanvas {
  @Prop sizeVp: number = 220;
  @State offsetX: number = 0;
  @State offsetY: number = 0;
  @State bubble: string = '';
  @State bubbleShow: boolean = false;
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(new RenderingContextSettings(true));
  private frameTimer: number = -1;
  private frame: number = 0;
  private bubbleUntil: number = 0;

  aboutToAppear(): void {
    BehaviorEngine.start();
  }

  aboutToDisappear(): void {
    if (this.frameTimer >= 0) {
      clearInterval(this.frameTimer);
    }
  }

  build() {
    Stack() {
      Canvas(this.ctx)
        .width(this.sizeVp)
        .height(this.sizeVp)
        .onReady(() => {
          // onReady 可能因窗口尺寸变化重复触发，先清旧定时器避免多实例并发绘制
          if (this.frameTimer >= 0) {
            clearInterval(this.frameTimer);
          }
          this.frameTimer = setInterval(() => {
            this.frame++;
            this.draw();
          }, 90);
        })
      if (this.bubbleShow && this.bubble.length > 0) {
        Text(this.bubble)
          .fontSize(14)
          .fontColor('#5B4A4F')
          .backgroundColor('#FFFFFF')
          .borderRadius(12)
          .padding({ left: 10, right: 10, top: 5, bottom: 5 })
          .offset({ x: 0, y: -this.sizeVp / 2 - 14 })
      }
    }
    .width(this.sizeVp)
    .height(this.sizeVp)
    .translate({ x: this.offsetX, y: this.offsetY })
    .gesture(
      PanGesture()
        .onActionUpdate((event: GestureEvent) => {
          this.offsetX = this.offsetX + event.offsetX * 0.2;
          this.offsetY = this.offsetY + event.offsetY * 0.2;
        })
    )
    .onClick(() => {
      BehaviorEngine.pet();
      this.showBubble(BehaviorEngine.moodText);
    })
  }

  showBubble(text: string): void {
    this.bubble = text;
    this.bubbleShow = true;
    this.bubbleUntil = Date.now() + 2600;
    setTimeout(() => {
      if (Date.now() >= this.bubbleUntil) {
        this.bubbleShow = false;
      }
    }, 2700);
  }

  // ---------- 绘制 ----------

  private draw(): void {
    const c = this.ctx;
    const s = this.sizeVp;
    const theme = ThemeEngine.get();
    const state = BehaviorEngine.state;
    const t = this.frame;
    c.clearRect(0, 0, s, s);

    const cx = s / 2;
    const breathe = state === 'sleeping' ? Math.sin(t * 0.03) * 2 : Math.sin(t * 0.09) * 2.5;
    const squash = state === 'happy' ? Math.abs(Math.sin(t * 0.25)) * 6 : 0;
    const bodyW = s * 0.62 + breathe;
    const bodyH = s * 0.55 - breathe + squash;
    const bodyY = s * 0.58;

    // 影子
    c.beginPath();
    c.ellipse(cx, bodyY + bodyH / 2 + 8, bodyW * 0.42, 7, 0, 0, Math.PI * 2);
    c.fillStyle = 'rgba(0,0,0,0.06)';
    c.fill();

    // 耳朵
    c.fillStyle = theme.primary;
    const earDx = bodyW * 0.30;
    const earY = bodyY - bodyH / 2 + 2;
    c.beginPath();
    c.arc(cx - earDx, earY, s * 0.075, 0, Math.PI * 2);
    c.fill();
    c.beginPath();
    c.arc(cx + earDx, earY, s * 0.075, 0, Math.PI * 2);
    c.fill();
    c.fillStyle = theme.light;
    c.beginPath();
    c.arc(cx - earDx, earY + 1, s * 0.038, 0, Math.PI * 2);
    c.fill();
    c.beginPath();
    c.arc(cx + earDx, earY + 1, s * 0.038, 0, Math.PI * 2);
    c.fill();

    // 身体（圆润团子）
    c.fillStyle = theme.primary;
    c.beginPath();
    c.ellipse(cx, bodyY, bodyW / 2, bodyH / 2, 0, 0, Math.PI * 2);
    c.fill();
    // 肚皮
    c.fillStyle = theme.light;
    c.beginPath();
    c.ellipse(cx, bodyY + bodyH * 0.16, bodyW * 0.32, bodyH * 0.3, 0, 0, Math.PI * 2);
    c.fill();

    // 眼睛（睡觉画线）
    const eyeY = bodyY - bodyH * 0.08;
    const eyeDx = bodyW * 0.14;
    c.strokeStyle = '#4A3B40';
    c.fillStyle = '#4A3B40';
    c.lineWidth = 2.5;
    if (state === 'sleeping') {
      c.beginPath();
      c.arc(cx - eyeDx, eyeY, 5, 0.15 * Math.PI, 0.85 * Math.PI);
      c.stroke();
      c.beginPath();
      c.arc(cx + eyeDx, eyeY, 5, 0.15 * Math.PI, 0.85 * Math.PI);
      c.stroke();
    } else {
      const blink = (t % 90) < 5;
      const eyeH = blink ? 1.5 : 6 + (state === 'happy' ? -2 : 0);
      c.beginPath();
      c.ellipse(cx - eyeDx, eyeY, 4, Math.max(1.5, eyeH), 0, 0, Math.PI * 2);
      c.fill();
      c.beginPath();
      c.ellipse(cx + eyeDx, eyeY, 4, Math.max(1.5, eyeH), 0, 0, Math.PI * 2);
      c.fill();
      // 高光
      c.fillStyle = '#FFFFFF';
      c.beginPath();
      c.arc(cx - eyeDx + 1.5, eyeY - 2, 1.4, 0, Math.PI * 2);
      c.fill();
      c.beginPath();
      c.arc(cx + eyeDx + 1.5, eyeY - 2, 1.4, 0, Math.PI * 2);
      c.fill();
    }

    // 腮红
    c.fillStyle = 'rgba(255,120,140,0.35)';
    c.beginPath();
    c.ellipse(cx - eyeDx * 1.9, eyeY + 8, 6, 3.6, 0, 0, Math.PI * 2);
    c.fill();
    c.beginPath();
    c.ellipse(cx + eyeDx * 1.9, eyeY + 8, 6, 3.6, 0, 0, Math.PI * 2);
    c.fill();

    // 嘴
    c.strokeStyle = '#4A3B40';
    c.lineWidth = 2;
    c.beginPath();
    if (state === 'happy' || state === 'speaking') {
      const open = state === 'speaking' ? 4 + Math.abs(Math.sin(t * 0.5)) * 4 : 4;
      c.arc(cx, eyeY + 9, open, 0.1 * Math.PI, 0.9 * Math.PI);
    } else if (state === 'sad' || state === 'confused') {
      c.arc(cx, eyeY + 14, 4, 1.15 * Math.PI, 1.85 * Math.PI);
    } else {
      c.arc(cx, eyeY + 8, 4, 0.15 * Math.PI, 0.85 * Math.PI);
    }
    c.stroke();

    // 挥手
    if (state === 'waving' || state === 'happy') {
      const waveAngle = Math.sin(t * 0.5) * 0.6;
      c.save();
      c.translate(cx + bodyW / 2 - 4, bodyY);
      c.rotate(-0.6 + waveAngle);
      c.fillStyle = theme.primary;
      c.beginPath();
      c.ellipse(0, -12, 5, 12, 0, 0, Math.PI * 2);
      c.fill();
      c.restore();
    }

    // 状态符号
    c.fillStyle = theme.deep;
    c.font = '16px sans-serif';
    if (state === 'sleeping') {
      const zz = t % 60;
      c.globalAlpha = 0.5 + 0.5 * Math.sin(zz * 0.2);
      c.fillText('z', cx + bodyW / 2 - 6, bodyY - bodyH / 2 - 6 - (zz % 30) * 0.4);
      c.fillText('Z', cx + bodyW / 2 + 8, bodyY - bodyH / 2 - 14 - (zz % 30) * 0.4);
      c.globalAlpha = 1;
    } else if (state === 'thinking') {
      const dots = '.'.repeat(1 + (Math.floor(t / 8) % 3));
      c.fillText(dots, cx + bodyW / 2 - 2, bodyY - bodyH / 2 - 4);
    } else if (state === 'confused') {
      c.fillText('?', cx + bodyW / 2 - 2, bodyY - bodyH / 2 - 4);
    }
  }
}
