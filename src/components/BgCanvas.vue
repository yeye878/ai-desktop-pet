<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from 'vue';
import { usePetStore } from '../stores/pet';
import { PHOTO_BACKGROUNDS, photoBackground } from '../services/bgPhotos';

const props = defineProps<{
  mode: string;
  customImage?: string;
  intensity?: number;
  /**
   * 多窗格「切片」信息。
   *
   * 多窗口模式下，如果每个格子各自 cover 一张图，看起来就是几张
   * 互不相干的壁纸拼在一起（视觉模型原话："separately cropped wallpaper
   * panels，no continuity across panes"）。
   *
   * 正确的做法是把整幅画按窗格布局切开，每格只画属于自己的那一条带：
   * 几格拼起来是**一幅连续的画**，接缝处的线条、明暗都能对上。
   *
   *   index  当前是第几格（0 起）
   *   cols   横向切几份
   *   rows   纵向切几份
   *   col    当前格在第几列
   *   row    当前格在第几行
   *   不传则退化为单格 cover 居中（单窗格视图）
   */
  slab?: { index: number; cols: number; rows: number; col: number; row: number };
}>();
const petStore = usePetStore();
const canvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let W = 0, H = 0, animId = 0;
let mx = -999, my = -999;
let particles: any[] = [];
let ripples: any[] = [];
let trail: { x: number; y: number; t: number }[] = [];
let customImg: HTMLImageElement | null = null;
/**
 * 照片背景缓存。
 * 每种模式一张 HTMLImageElement，加载完成后才能画；
 * 未加载完时先画底色，避免出现黑屏。
 */
const photoImages: Record<string, HTMLImageElement> = {};
let photoLoadStarted = false;
/** 漂移相位：让照片极缓地平移/缩放，静止的贴图感会消失 */

let ro: ResizeObserver | null = null;

/**
 * 背景强度系数（0.15 ~ 1.2）。
 * 控制台是米白纸面，背景一旦太实就会像在纸上贴了一块色板；
 * 默认 0.72 接近「纸面透出的一点光」，所有绘制都乘这个系数。
 */
const strength = () => {
  const raw = Number(props.intensity);
  return Number.isFinite(raw) ? Math.min(1.2, Math.max(0.15, raw)) : 0.72;
};

// Scifi state
let sfNodes: any[] = [];
let sfPackets: any[] = [];
let sfEMPs: any[] = [];
let sfScanY = 0;
let sfHexes: any[] = [];
let sfNebulae: any[] = [];
// Cute state
let ctPetals: any[] = [];
let ctBubbles: any[] = [];
// Minimal state
let mnDots: any[] = [];
let mnRays: any[] = [];
// Aurora state
let auCurtains: any[] = [];
let auStars: any[] = [];
// Ink state
let ikRidges: any[] = [];
let ikMist: any[] = [];
let ikBands: any[] = [];
let ikRipples: any[] = [];
let ikBirds: any[] = [];
let ikSun: { x: number; y: number; r: number; alpha: number } | null = null;
let ikBoat: { x: number; y: number; phase: number; flip: boolean } | null = null;
// Rain state（细雨）
let rnDrops: any[] = [];
let rnStreaks: any[] = [];
let rnRipples: any[] = [];
let rnSplashes: any[] = [];

const CUTE_C = ['#bb8fa2','#c99eb0','#dcc0cc','#d6ceba','#8aa6b8','#d9bcc8','#cf9e89','#a97f92'];
const SF_COLS = { cyan:'#79b1b3', purple:'#8aa6b8', green:'#8aa98f', blue:'#7e93a8', teal:'#97c8ca' };

function drawHeart(c: CanvasRenderingContext2D, x: number, y: number, s: number) {
  c.beginPath();
  c.moveTo(x, y + s * 0.25);
  c.bezierCurveTo(x - s * 0.5, y - s * 0.2, x - s * 0.5, y - s * 0.6, x, y - s * 0.35);
  c.bezierCurveTo(x + s * 0.5, y - s * 0.6, x + s * 0.5, y - s * 0.2, x, y + s * 0.25);
  c.closePath(); c.fill();
}

function drawStar(c: CanvasRenderingContext2D, x: number, y: number, r: number) {
  c.beginPath();
  for (let i = 0; i < 10; i++) {
    const rad = i % 2 === 0 ? r : r * 0.4;
    const a = (i * Math.PI) / 5 - Math.PI / 2;
    i === 0 ? c.moveTo(x + Math.cos(a) * rad, y + Math.sin(a) * rad)
            : c.lineTo(x + Math.cos(a) * rad, y + Math.sin(a) * rad);
  }
  c.closePath(); c.fill();
}

function drawSparkle(c: CanvasRenderingContext2D, x: number, y: number, s: number) {
  c.beginPath();
  c.moveTo(x, y - s); c.lineTo(x + s * 0.12, y - s * 0.12);
  c.lineTo(x + s, y); c.lineTo(x + s * 0.12, y + s * 0.12);
  c.lineTo(x, y + s); c.lineTo(x - s * 0.12, y + s * 0.12);
  c.lineTo(x - s, y); c.lineTo(x - s * 0.12, y - s * 0.12);
  c.closePath(); c.fill();
}



function rnd(a: number, b: number) { return a + Math.random() * (b - a); }
function pick<T>(arr: T[]): T { return arr[Math.floor(Math.random() * arr.length)]; }



function initCute() {
  ctPetals = []; ctBubbles = [];
  // Sakura petals
  // 花瓣换成更深的玫粉：原来 #d9bcc8 / #eddfe5 压在近白底上几乎看不见，
  // 这就是"看起来什么都没画"的原因。
  for (let i = 0; i < 52; i++) ctPetals.push({
    x: rnd(0, W), y: rnd(-H, H),
    vx: rnd(-0.5, 0.5), vy: rnd(0.4, 1.2),
    size: rnd(3.5, 9), angle: rnd(0, 6.28), spin: rnd(-0.04, 0.04),
    sway: rnd(0.5, 1.5), swayOff: rnd(0, 6.28),
    color: pick(['#c98fa4','#c98fa4','#b8768f','#d9a8b8','#a9647e','#cf9db0']),
    alpha: rnd(0.45, 0.85), temp: false, life: 1, decay: 0,
  });
  // Floating pastel bubbles
  for (let i = 0; i < 18; i++) ctBubbles.push({
    x: rnd(0, W), y: rnd(0, H),
    vx: rnd(-0.2, 0.2), vy: rnd(-0.3, -0.08),
    r: rnd(5, 16), phase: rnd(0, 6.28), speed: rnd(0.8, 1.8),
    color: pick(CUTE_C), alpha: rnd(0.22, 0.42),
  });
}

function initMinimal() {
  mnDots = []; mnRays = [];
  const sp = 18;
  for (let gx = sp; gx < W; gx += sp)
    for (let gy = sp; gy < H; gy += sp)
      mnDots.push({ x: gx, y: gy, glow: 0, r: 1.8 });
}

/**
 * 极光：几条竖直光带在深色夜空里缓慢摆动，叠加一层极细的星点。
 * 光带用二次曲线画出上下摆动的轮廓，按列逐像素算出亮度，避免整块渐变显得死板。
 */
function initAurora() {
  auCurtains = []; auStars = [];
  // 帘幕：真实极光是成排悬挂的「帘子」，所以数量少、每片宽、纵向贯穿大半个画面。
  // 上边界在画面顶部之外（淡入），下边界落在 55%~78% 高度（100km 处锐利截止）。
  const count = Math.max(2, Math.min(5, Math.round(W / 260)));
  for (let i = 0; i < count; i++) {
    auCurtains.push({
      x: (i + 0.5) / count * W + rnd(-W * 0.08, W * 0.08),
      width: rnd(W * 0.34, W * 0.62),
      sway: rnd(W * 0.03, W * 0.075),
      swaySpeed: rnd(0.08, 0.2),
      phase: rnd(0, 6.28),
      drift: rnd(-0.025, 0.025),
      top: rnd(-H * 0.55, -H * 0.15),      // 从画面顶部之外开始
      bottom: rnd(H * 0.55, H * 0.78),      // 帘幕底部
      alpha: rnd(0.85, 1.15),
    });
  }
  for (let i = 0; i < 110; i++) {
    auStars.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(0.4, 1.2), base: rnd(0.15, 0.5),
      twinkle: rnd(0.6, 1.8), phase: rnd(0, 6.28),
    });
  }
}

/**
 * 水墨山水：远峰 + 云海 + 近岭，按「墨分五色」由淡到浓叠四层。
 * 山脊用若干高斯峰叠加缓波生成，有主峰、有起伏，不是等幅折线；
 * 每层山脚压一条雾带，让下一层从云里长出来，形成纵深与留白。
 * 点缀一轮淡日与几只远鸟，点击处晕开一团墨。
 */
function makeInkRidge(baseline: number, amp: number, seed: number) {
  const points: { x: number; y: number }[] = [];
  const step = Math.max(14, W / 72);
  const peaks: { cx: number; h: number; w: number; skew: number }[] = [];
  const nPeaks = 2 + Math.floor(Math.random() * 3);
  for (let i = 0; i < nPeaks; i++) {
    const cx = rnd(-W * 0.05, W * 1.05);
    const h = rnd(0.5, 1) * amp;
    const w = rnd(W * 0.06, W * 0.16);
    peaks.push({ cx, h, w, skew: rnd(0.55, 1.5) });
    // 肩峰：主峰旁的小尖峰，轮廓有主次而不是圆包
    peaks.push({
      cx: cx + w * rnd(0.7, 1.3) * (Math.random() < 0.5 ? -1 : 1),
      h: h * rnd(0.25, 0.45), w: w * rnd(0.3, 0.5), skew: rnd(0.6, 1.4),
    });
  }
  for (let x = -step; x <= W + step; x += step) {
    let y = baseline - Math.sin(x * 0.004 + seed) * amp * 0.14 - Math.sin(x * 0.013 + seed * 1.7) * amp * 0.06;
    for (const p of peaks) {
      const d = (x - p.cx) / p.w;
      // 不对称尖峰：一侧陡一侧缓，1.5 次指数比高斯更峭
      const dd = d < 0 ? d * p.skew : d / p.skew;
      y -= p.h * Math.exp(-Math.pow(Math.abs(dd), 1.5));
    }
    y -= Math.abs(Math.sin(x * 0.045 + seed * 3.1)) * amp * 0.035;
    points.push({ x, y });
  }
  return points;
}

function initInk() {
  ikRidges = []; ikMist = []; ikBands = []; ikRipples = []; ikBirds = [];
  // 由远及近：远峰高而淡，近岭低而浓；墨分五色，色调取黛墨（冷青黑）
  const layers = [
    { base: 0.58, amp: 0.26, alpha: 0.22, tint: '164,176,192' },
    { base: 0.69, amp: 0.20, alpha: 0.30, tint: '134,148,166' },
    { base: 0.80, amp: 0.14, alpha: 0.42, tint: '100,114,134' },
    { base: 0.93, amp: 0.10, alpha: 0.55, tint: '56,68,86' },
  ];
  layers.forEach((L, i) => {
    const points = makeInkRidge(H * L.base, H * L.amp, rnd(0, 100));
    // 皴笔：预生成自山脊垂下的短竖笔（披麻皴），避免逐帧随机造成闪烁
    const strokes: { x: number; y: number; len: number; bow: number; a: number }[] = [];
    const nSt = 12 + i * 5;
    for (let s = 0; s < nSt; s++) {
      const p = points[Math.floor(rnd(1, points.length - 1))];
      strokes.push({
        x: p.x + rnd(-6, 6), y: p.y + rnd(2, 7),
        len: rnd(H * 0.02, H * 0.06), bow: rnd(-3.5, 3.5), a: rnd(0.3, 0.75),
      });
    }
    ikRidges.push({
      points,
      top: Math.min(...points.map((p) => p.y)),
      strokes,
      alpha: L.alpha,
      tint: L.tint,
      base: H * L.base,
      layer: i,
    });
    // 山脚雾带：横切山体下部，让下一层从雾里浮现
    ikBands.push({
      y: H * (L.base - 0.015),
      h: rnd(H * 0.055, H * 0.095),
      drift: rnd(0.05, 0.16) * (i % 2 ? 1 : -1),
      phase: rnd(0, 6.28),
      alpha: rnd(0.65, 0.9),
    });
  });
  // 漂浮的云雾团
  for (let i = 0; i < 6; i++) {
    ikMist.push({
      x: rnd(-W * 0.1, W), y: rnd(H * 0.4, H * 0.95),
      w: rnd(W * 0.4, W * 0.85), h: rnd(H * 0.05, H * 0.12),
      drift: rnd(0.06, 0.2), phase: rnd(0, 6.28), alpha: rnd(0.08, 0.14),
    });
  }
  // 淡日：一轮暖朱圆挂在远峰之上，是画面的视觉锚点
  ikSun = { x: rnd(W * 0.6, W * 0.82), y: rnd(H * 0.16, H * 0.26), r: rnd(26, 40), alpha: rnd(0.65, 0.85) };
  // 远鸟：一群「人」字墨，缓缓掠过天空
  const flockX = rnd(W * 0.12, W * 0.62), flockY = rnd(H * 0.15, H * 0.32);
  const nBirds = 4 + Math.floor(Math.random() * 3);
  for (let i = 0; i < nBirds; i++) {
    ikBirds.push({
      x: flockX + i * rnd(8, 16) + rnd(-W * 0.03, W * 0.03),
      y: flockY + i * rnd(2, 7) + rnd(-H * 0.03, H * 0.03),
      vx: rnd(0.22, 0.4), size: rnd(4.5, 8),
      flap: rnd(0, 6.28), flapSpd: rnd(0.07, 0.12), alpha: rnd(0.4, 0.62),
    });
  }
  // 孤舟：一叶泊在中间雾水里，是画面的画眼
  ikBoat = { x: rnd(W * 0.16, W * 0.42), y: H * 0.845, phase: rnd(0, 6.28), flip: Math.random() < 0.5 };
}

function initScifi() {
  sfNodes = []; sfPackets = []; sfEMPs = []; sfHexes = []; sfScanY = 0;
  // Neural nodes
  const count = Math.floor(W * H / 5500);
  for (let i = 0; i < Math.max(12, count); i++) {
    sfNodes.push({
      x: rnd(W * 0.05, W * 0.95), y: rnd(H * 0.05, H * 0.95),
      vx: rnd(-0.18, 0.18), vy: rnd(-0.18, 0.18),
      r: rnd(2.5, 5.5), pulse: rnd(0, 6.28), pulseSpd: rnd(0.8, 2.2),
      color: pick(Object.values(SF_COLS)), energy: 0, activatedAt: 0,
      type: Math.random() < 0.2 ? 'hub' : 'node',
    });
  }
  // Hex grid cells
  const hs = 24;
  for (let col = -1; col < Math.ceil(W / (hs * 1.73)) + 1; col++) {
    for (let row = -1; row < Math.ceil(H / (hs * 1.5)) + 1; row++) {
      const hx = col * hs * 1.73 + (row % 2) * hs * 0.865;
      const hy = row * hs * 1.5;
      sfHexes.push({ x: hx, y: hy, s: hs, glow: 0, glowDecay: rnd(0.005, 0.015) });
    }
  }
  // Floating space nebulas：浅色纸面上用极淡的冷色雾，靠 multiply 压出层次
  sfNebulae = [
    { x: W * 0.25, y: H * 0.25, vx: rnd(0.04, 0.12), vy: rnd(0.03, 0.08), r: Math.max(140, W * 0.35), color1: 'rgba(196, 224, 248, 0.30)', color2: 'rgba(196, 224, 248, 0)' },
    { x: W * 0.75, y: H * 0.7, vx: rnd(-0.12, -0.04), vy: rnd(-0.08, -0.03), r: Math.max(160, W * 0.42), color1: 'rgba(216, 214, 250, 0.26)', color2: 'rgba(216, 214, 250, 0)' },
    { x: W * 0.5, y: H * 0.45, vx: rnd(0.02, 0.08), vy: rnd(-0.06, -0.02), r: Math.max(110, W * 0.3), color1: 'rgba(202, 236, 232, 0.24)', color2: 'rgba(202, 236, 232, 0)' }
  ];
}

function spawnPacket(fromNode: any, toNode: any) {
  sfPackets.push({
    x: fromNode.x, y: fromNode.y, tx: toNode.x, ty: toNode.y,
    ox: fromNode.x, oy: fromNode.y,
    t: 0, color: fromNode.color, r: rnd(1.5, 3),
  });
}

/**
 * 预加载全部照片背景。
 * 只在首次需要时触发一次，加载失败的项保持未定义，绘制时兜底成底色。
 */
function ensurePhotoLoaded() {
  if (photoLoadStarted) return;
  photoLoadStarted = true;
  for (const cfg of Object.values(PHOTO_BACKGROUNDS)) {
    if (photoImages[cfg.id]) continue;
    const img = new Image();
    img.decoding = "async";
    img.onload = () => { photoImages[cfg.id] = img; };
    img.src = cfg.src;
  }
}

function initParticles() {
  particles = []; ripples = []; trail = [];
  if (props.mode === 'sakura') initCute();
  else if (props.mode === 'scifi') initScifi();
  else if (props.mode === 'minimal') initMinimal();
  else if (props.mode === 'aurora') initAurora();
  else if (props.mode === 'ink') initInk();
  else if (props.mode === 'rain') initRain();
}

function addRipple(x: number, y: number, big = false) {
  const color =
    props.mode === 'scifi' ? 'rgba(56,189,248,' :
    props.mode === 'sakura' ? 'rgba(201,168,180,' :
    props.mode === 'aurora' ? 'rgba(94,234,212,' :
    props.mode === 'ink' ? 'rgba(54,62,74,' :
    props.mode === 'rain' ? 'rgba(150,180,192,' :
    'rgba(148,163,184,';
  if (big) {
    for (let i = 0; i < 3; i++)
      ripples.push({ x, y, r: i * 8, maxR: 120 + i * 30, alpha: 0.4 - i * 0.08, lw: 2.5 - i * 0.5, speed: 2.5 - i * 0.3, color });
  } else {
    ripples.push({ x, y, r: 0, maxR: 50, alpha: 0.2, lw: 1.5, speed: 1.5, color });
  }
}

function update() {
  const now = performance.now() / 1000;
  // Ripples
  for (let i = ripples.length - 1; i >= 0; i--) {
    const r = ripples[i]; r.r += r.speed; r.alpha *= 0.965;
    if (r.alpha < 0.005 || r.r > r.maxR) ripples.splice(i, 1);
  }
  // Trail fade
  for (let i = trail.length - 1; i >= 0; i--) {
    trail[i].t -= 0.02;
    if (trail[i].t <= 0) trail.splice(i, 1);
  }

  if (props.mode === 'sakura') {
    const t = now;
    // Petals
    for (const p of ctPetals) {
      p.x += p.vx + Math.sin(t * p.sway + p.swayOff) * 0.4;
      p.y += p.vy; p.angle += p.spin;
      if (p.temp) { p.life -= p.decay; p.vy += 0.03; }
      if (p.y > H + 15) { p.y = -15; p.x = rnd(0, W); }
      if (p.x < -15) p.x = W + 15; if (p.x > W + 15) p.x = -15;
    }
    ctPetals = ctPetals.filter(p => !p.temp || p.life > 0);
    // Bubbles
    for (const b of ctBubbles) {
      const dx = mx - b.x, dy = my - b.y, d = Math.sqrt(dx*dx+dy*dy);
      if (d < 90 && d > 1) { b.x += (dx/d)*0.5; b.y += (dy/d)*0.5; }
      b.x += b.vx + Math.sin(t * b.speed + b.phase) * 0.3;
      b.y += b.vy;
      if (b.y < -b.r*2) { b.y = H + b.r; b.x = rnd(0, W); }
    }
    // Temp particles
    for (const p of particles) {
      p.x += p.vx; p.y += p.vy; p.angle += p.spin;
      p.life -= p.decay; p.vy += 0.03;
    }
    particles = particles.filter(p => p.life > 0);
  } else if (props.mode === 'minimal') {
    // Dots respond to cursor
    for (const d of mnDots) {
      const dx = d.x - mx, dy = d.y - my, dist = Math.sqrt(dx*dx+dy*dy);
      const target = dist < 70 ? Math.min(1, (70 - dist) / 70) : 0;
      d.glow += (target - d.glow) * 0.1;
    }
    // Rays decay
    for (let i = mnRays.length - 1; i >= 0; i--) {
      mnRays[i].alpha -= 0.012;
      if (mnRays[i].alpha <= 0) mnRays.splice(i, 1);
    }
    // Temp burst particles
    for (const p of particles) {
      p.x += p.vx; p.y += p.vy; p.vx *= 0.97; p.vy *= 0.97;
      p.life -= p.decay;
    }
    particles = particles.filter(p => p.life > 0);
  } else if (props.mode === 'scifi') {
    const now2 = performance.now();
    const petState = petStore.state;

    // Define speed and frequency multipliers based on pet state
    let speedMult = 1.0;
    let packetSpawnRate = 0.003;
    let nodeJitter = 0.0;
    let scanlineSpeed = 0.6;

    if (petState === 'thinking') {
      speedMult = 1.8;
      nodeJitter = 0.4;
      packetSpawnRate = 0.018; // active thought processing
      scanlineSpeed = 1.2;
    } else if (petState === 'speaking') {
      speedMult = 1.35;
      nodeJitter = 0.15;
      packetSpawnRate = 0.025; // massive communication activity
      scanlineSpeed = 1.8;
    } else if (petState === 'listening') {
      speedMult = 0.55;
      packetSpawnRate = 0.001;
      scanlineSpeed = 0.45;

      // Sonar scan pulse in listening state
      if (Math.random() < 0.016) {
        const sx = mx > -900 ? mx : W / 2;
        const sy = my > -900 ? my : H / 2;
        ripples.push({
          x: sx,
          y: sy,
          r: 0,
          maxR: Math.max(W, H) * 0.75,
          alpha: 0.38,
          lw: 1.2,
          speed: 2.2,
          color: 'rgba(56,189,248,'
        });
      }
    }

    // Nebulae drift
    for (const neb of sfNebulae) {
      neb.x += neb.vx * speedMult;
      neb.y += neb.vy * speedMult;
      if (neb.x - neb.r > W) neb.x = -neb.r;
      if (neb.x + neb.r < 0) neb.x = W + neb.r;
      if (neb.y - neb.r > H) neb.y = -neb.r;
      if (neb.y + neb.r < 0) neb.y = H + neb.r;
    }

    // Scan line
    sfScanY = (sfScanY + scanlineSpeed) % H;

    // Nodes
    for (const n of sfNodes) {
      n.x += n.vx * speedMult;
      n.y += n.vy * speedMult;

      if (nodeJitter > 0) {
        n.x += rnd(-nodeJitter, nodeJitter);
        n.y += rnd(-nodeJitter, nodeJitter);
      }

      if (n.x < 0 || n.x > W) { n.vx *= -1; n.x = Math.max(0, Math.min(W, n.x)); }
      if (n.y < 0 || n.y > H) { n.vy *= -1; n.y = Math.max(0, Math.min(H, n.y)); }
      n.pulse += n.pulseSpd * 0.016 * speedMult;

      // Mouse attraction
      const dx = mx - n.x, dy = my - n.y, dist = Math.sqrt(dx*dx+dy*dy);
      if (dist < 110 && dist > 2) {
        n.energy = Math.min(1, n.energy + 0.045);
        // Soft pull
        n.x += (dx / dist) * 0.22 * speedMult;
        n.y += (dy / dist) * 0.22 * speedMult;
      } else {
        n.energy *= 0.94;
      }

      // EMPs
      for (const e of sfEMPs) {
        const ed = Math.sqrt((n.x-e.x)**2+(n.y-e.y)**2);
        if (Math.abs(ed - e.r) < 22) { n.energy = 1.0; n.activatedAt = now2; }
      }

      // Randomly spawn data packets
      if (Math.random() < packetSpawnRate && sfNodes.length > 1) {
        const targets = sfNodes.filter(o => o !== n);
        const t = targets[Math.floor(Math.random() * targets.length)];
        const d2 = Math.sqrt((n.x-t.x)**2+(n.y-t.y)**2);
        if (d2 < 240) spawnPacket(n, t);
      }
    }

    // Hex glows
    for (const h of sfHexes) { h.glow = Math.max(0, h.glow - h.glowDecay); }
    // Move cursor hex
    for (const h of sfHexes) {
      const d = Math.sqrt((h.x-mx)**2+(h.y-my)**2);
      if (d < h.s * 2.8) {
        h.glow = Math.min(1.0, h.glow + 0.14);
      }
    }

    // Data packets
    for (let i = sfPackets.length-1; i >= 0; i--) {
      const p = sfPackets[i];
      p.t += 0.024 * speedMult;
      if (p.t >= 1) { sfPackets.splice(i, 1); continue; }
    }

    // Recompute packet pos from stored origin
    for (const pk of sfPackets) {
      pk.x = pk.ox + (pk.tx - pk.ox) * pk.t;
      pk.y = pk.oy + (pk.ty - pk.oy) * pk.t;
    }

    // EMPs expand
    for (let i = sfEMPs.length-1; i >= 0; i--) {
      const e = sfEMPs[i]; e.r += e.spd; e.alpha *= 0.97;
      if (e.alpha < 0.01) { sfEMPs.splice(i, 1); }
    }

    // Update scifi burst particles
    for (const p of particles) {
      if (p.shape === 'scifi-spark') {
        p.x += p.vx * speedMult;
        p.y += p.vy * speedMult;
        p.vx *= 0.94;
        p.vy *= 0.94;
        p.life -= p.decay * speedMult;
      }
    }
  } else if (props.mode === 'minimal') {
    for (const p of particles) {
      if (p.shape !== 'grid') { p.life -= p.decay; p.x += p.vx; p.y += p.vy; p.vx *= 0.98; p.vy *= 0.98; continue; }
      const dx = p.baseX - mx, dy = p.baseY - my, dist = Math.sqrt(dx * dx + dy * dy);
      if (dist < 55) {
        const s = 1 + (55 - dist) / 55 * 3.5; p.size = p.baseSize * s; p.alpha = 0.2 + (55 - dist) / 55 * 0.6;
      } else { p.size += (p.baseSize - p.size) * 0.08; p.alpha += (0.2 - p.alpha) * 0.08; }
    }
  } else if (props.mode === 'aurora') {
    const petState = petStore.state;
    // 桌宠思考/说话时极光更亮更活跃，静默时几乎不动，避免抢注意力。
    const energy = petState === 'thinking' ? 1.55 : petState === 'speaking' ? 1.28 : 1;
    for (const c of auCurtains) {
      c.phase += 0.004 * c.swaySpeed * energy;
      c.x += c.drift * energy;
      if (c.x - c.width > W) c.x = -c.width;
      if (c.x + c.width < 0) c.x = W + c.width;
    }
    for (const s of auStars) s.phase += 0.02 * s.twinkle;
    // 鼠标附近的光带被“推开”，产生轻微的极光扰动
    for (const p of particles) { p.life -= p.decay; p.y -= 0.35; }
    particles = particles.filter(p => p.life > 0);
  } else if (props.mode === 'ink') {
    for (const m of ikMist) {
      m.x += m.drift;
      m.phase += 0.008;
      if (m.x - m.w > W) m.x = -m.w;
      if (m.x + m.w < 0) m.x = W + m.w;
    }
    for (const b of ikBands) { b.phase += 0.006; b.y += Math.sin(b.phase) * 0.02; }
    for (const bird of ikBirds) {
      bird.x += bird.vx; bird.flap += bird.flapSpd;
      bird.y += Math.sin(bird.flap * 0.4) * 0.08;
      if (bird.x - bird.size * 2 > W) { bird.x = -bird.size * 2; bird.y = rnd(H * 0.14, H * 0.4); }
    }
    if (ikBoat) ikBoat.phase += 0.012;
    for (let i = ikRipples.length - 1; i >= 0; i--) {
      const r = ikRipples[i];
      r.r += r.speed; r.alpha *= 0.968;
      if (r.alpha < 0.006) ikRipples.splice(i, 1);
    }
    for (const p of particles) { p.life -= p.decay; p.x += p.vx; p.y += p.vy; p.vx *= 0.97; p.vy *= 0.97; }
    particles = particles.filter(p => p.life > 0);
  } else if (props.mode === 'rain') {
    // 雨丝：出屏后从顶部落回，x 也跟着重新随机，避免形成固定斜纹
    for (const s of rnStreaks) {
      s.x += s.vx; s.y += s.vy;
      if (s.y - s.len > H) { s.y = rnd(-H * 0.5, -s.len); s.x = rnd(-W * 0.25, W * 1.05); }
      if (s.x > W + 30) { s.x = -30; s.y = rnd(-H * 0.5, H); }
    }
    // 玻璃水珠：缓缓下滑、左右微摆，落到底部重置；同时积累拖尾长度
    for (const d of rnDrops) {
      d.wiggle += 0.022;
      d.y += d.drift;
      d.x += Math.sin(d.wiggle) * 0.05;
      d.trail = Math.min(d.r * 9, d.trail + d.drift * 2.4);
      if (d.y - d.trail > H + 8) {
        d.y = rnd(-20, -4);
        d.x = rnd(0, W);
        d.trail = 0;
      }
    }
    // 涟漪：扩散 + 淡出
    for (let i = rnRipples.length - 1; i >= 0; i--) {
      const r = rnRipples[i];
      r.r += 0.55; r.alpha *= 0.955;
      if (r.alpha < 0.01) rnRipples.splice(i, 1);
    }
    // 水花：向上弹起后落下
    for (let i = rnSplashes.length - 1; i >= 0; i--) {
      const sp = rnSplashes[i];
      sp.vy += 0.06;
      sp.h += sp.vy;
      sp.alpha *= 0.96;
      if (sp.h < 0 || sp.alpha < 0.01) rnSplashes.splice(i, 1);
    }
    // 偶尔在随机位置自动落一滴，让画面在你不动的时候也有雨
    if (Math.random() < 0.09) {
      const rx = rnd(W * 0.05, W * 0.95);
      const ry = rnd(H * 0.3, H * 0.95);
      rnRipples.push({ x: rx, y: ry, r: 0.5, alpha: 0.32 });
      for (let i = 0; i < 3; i++) {
        rnSplashes.push({
          x: rx + rnd(-6, 6), y: ry,
          r: rnd(0.8, 1.8), h: 0, vy: rnd(-1.1, -0.4), alpha: rnd(0.3, 0.5),
        });
      }
    }
  }
  particles = particles.filter(p => !p.temp || p.life > 0);
}

/**
 * 极光（写实版）。
 *
 * 依据真实极光的光谱与形态：
 *   · 557.7nm 氧原子绿（100~150km）是主体，也是最常见的颜色
 *   · 630.0nm 氧原子红出现在更高的 200km 以上，所以红的在帘幕顶部
 *   · 427.8nm 氮分子蓝紫出现在最低的边缘
 *   · 形态是「帘幕」：成排的竖直光柱，会折叠成褶，底部约 100km 处突然截止
 *
 * 底色用「黄昏」而不是「深夜」：这样既写实（真实极光照片里天空往往是
 * 深蓝到墨绿），又能保持整体明亮 —— 深色天空只压在上半部三分之一。
 */
/**
 * 极光（写实重建，第二版）。
 *
 * 第一版被指"像磨砂玻璃的均匀竖条纹"。核心错误是：把极光当成了
 * 「一排等宽光柱」，而真实极光是**折叠的飘带（folded ribbon）**：
 *   · 下缘是一条连续、起伏、会折叠的亮线（对应 100km 处的沉降边界）
 *   · 从这条线向上，细密射线逐渐变淡、变宽、偏红（630nm 氧红在更高处）
 *   · 主体是 557.7nm 氧绿，底部偶有 427.8nm 氮蓝紫
 *
 * 所以画法是：先算出这条「折叠曲线」，再把垂直渐变裁进曲线形成的带子里，
 * 然后在带子里撒疏密不一的细射线。这样形状是连续的，不会有矩形硬边。
 */
/**
 * 极光（第三版 · 写实）。
 *
 * 技术路线：不用 clip 裁剪，也不用 fillRect 画射线 —— 那两者必然产生硬边和
 * 扫描线感。照片里的极光本质是「海量柔和光斑的叠加」，所以这里就照这个物理
 * 事实来画：沿一条会折叠的曲线撒椭圆柔光斑，用 lighter 叠加，重叠处自然变亮。
 *
 * 颜色按高度插值（对应真实发射高度）：
 *   下缘 ~100km  557.7nm 氧绿，最亮，偶有氮蓝紫
 *   中部 ~150km  正绿 → 青
 *   上部 ~200km+ 630.0nm 氧红，被大气吸收后呈暗紫红
 *
 * 亮度用两层不同频率的正弦叠加调制，保证相邻光斑的节奏不一样。
 */
/**
 * 极光（第四版 · 写实）。
 *
 * 结构上只有一个决定：画「极少数几条宽阔飘带」，而不是「许多细光柱」。
 * 真实极光的形态是 2~4 条连续的帘幕，内部亮度平滑过渡，
 * 靠下缘的折叠与向上的扩散形成层次。任何把光切成细条的做法都会立刻
 * 变成扫描线/霓虹管的观感 —— 这是前三版失败的原因。
 *
 * 柔边的做法：同一条路径重复画 3 遍，逐次加宽并降低 alpha。
 * 比 shadowBlur 可控，也不会把颜色洗成白色。
 */
/**
 * 极光（第五版 · 写实）。
 *
 * 结构：2~3 条宽阔飘带，下缘带高频锯齿（不是光滑搁板），
 * 亮度集中在下缘并向上快速透明，飘带内部有稀疏的竖向射线。
 *
 * 关键参数纪律（前四版失败的经验）：
 *   · 峰值 alpha <= 0.3 —— 超过就过曝成白色霓虹
 *   · 只画一遍飘带本体，不叠加宽层 —— 叠了会出现"同心轮廓"像塑料模具
 *   · 竖向射线必须是 clip 内的窄条，不能是独立 fillRect —— 否则变扫描线
 *   · 天空要够深 —— 灰亮天空立刻失去夜间曝光的真实感
 */
function drawAurora(ctx: CanvasRenderingContext2D) {
  const k = strength();

  // ---------- 1. 夜空 ----------
  const sky = ctx.createLinearGradient(0, 0, 0, H);
  sky.addColorStop(0, '#060c16');
  sky.addColorStop(0.24, '#0a131f');
  sky.addColorStop(0.5, '#101d2b');
  sky.addColorStop(0.72, '#182a38');
  sky.addColorStop(0.88, '#243946');
  sky.addColorStop(1, '#33505c');
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, W, H);

  // ---------- 2. 星点 ----------
  for (const s of auStars) {
    const fade = Math.max(0, 1 - s.y / (H * 0.72));
    if (fade <= 0.05) continue;
    const twinkle = 0.65 + Math.sin(s.phase) * 0.35;
    ctx.save();
    ctx.globalAlpha = Math.min(0.7, s.base * twinkle * k * 1.25 * fade);
    ctx.fillStyle = '#dce8ff';
    ctx.beginPath();
    ctx.arc(s.x, s.y, Math.max(0.3, s.r * 0.5), 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // ---------- 3. 飘带 ----------
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';

  const ribbons = auCurtains.slice(0, 3);


  for (let ri = 0; ri < ribbons.length; ri++) {
    const c = ribbons[ri];
    const phase = c.phase;
    const SEG = 150;

    // 下缘：低频大褶 + 中频起伏 + 两层高频锯齿。
    // 高频那两层是修掉"光滑搁板"感的关键 —— 真实下缘是毛糙的。
    const hemAt = (t: number) => {
      const fold1 = Math.sin(t * Math.PI * 1.7 + phase * 1.1) * c.sway * 0.95;
      const fold2 = Math.sin(t * Math.PI * 3.3 + phase * 0.5) * c.sway * 0.42;
      const x = c.x - c.width / 2 + t * c.width + fold1 + fold2;
      const y = c.bottom
        + Math.sin(t * Math.PI * 2.4 + phase * 2.1) * H * 0.03
        + Math.sin(t * Math.PI * 5.1 - phase) * H * 0.014
        + Math.sin(t * Math.PI * 11.7 + phase * 3.3) * H * 0.007
        + Math.sin(t * Math.PI * 23.1 - phase * 1.7) * H * 0.003;
      return { x, y };
    };

    // 上缘高度：低频调制，有高有矮
    const topAt = (t: number) => {
      const h1 = Math.sin(t * Math.PI * 1.2 + phase * 0.9) * 0.5 + 0.5;
      const h2 = Math.sin(t * Math.PI * 2.9 - phase * 1.5) * 0.5 + 0.5;
      return c.bottom - H * ((0.16 + h1 * 0.46) * (0.7 + h2 * 0.3));
    };

    // 横向亮暗调制：让飘带内部不是均匀一片
    const brightAt = (t: number) => {
      const b1 = Math.sin(t * Math.PI * 4.3 + phase * 2.2) * 0.5 + 0.5;
      const b2 = Math.sin(t * Math.PI * 9.7 - phase * 1.3) * 0.5 + 0.5;
      return 0.35 + b1 * b2 * 0.65;
    };

    // 3a. 飘带路径：上缘 → 下缘（两端收窄到 0 高度，避免硬切边）
    ctx.beginPath();
    for (let i = 0; i <= SEG; i++) {
      const t = i / SEG;
      const hem = hemAt(t);
      const taper = Math.sin(t * Math.PI);
      const yTop = hem.y - (hem.y - topAt(t)) * Math.max(0, taper);
      if (i === 0) ctx.moveTo(hem.x, yTop);
      else ctx.lineTo(hem.x, yTop);
    }
    for (let i = SEG; i >= 0; i--) {
      const t = i / SEG;
      const hem = hemAt(t);
      ctx.lineTo(hem.x, hem.y);
    }
    ctx.closePath();

    ctx.save();
    ctx.clip();

    // 3b. 底色：很淡的绿，只用来托住上层，让天空仍能透出来
    const baseA = Math.min(0.1, 0.07 * c.alpha * k);
    ctx.fillStyle = `rgba(60, 190, 120, ${baseA.toFixed(3)})`;
    ctx.fillRect(0, 0, W, H);

    // 3c. 竖向射线：clip 内的窄条，稀疏、融入飘带。
    //     试过"三段锥形 + 半透明叠加"来加深厚度，但实测反而更像人工光柱，
    //     所以这里保持最简单的单条渐变 —— 简单的东西更不容易露馅。
    const rayCount = Math.round(c.width / 6.5);
    for (let i = 0; i < rayCount; i++) {
      const t = i / rayCount;
      const hem = hemAt(t);
      const top = topAt(t);
      const n1 = Math.sin(i * 5.71 + phase * 4.1) * 0.5 + 0.5;
      const n2 = Math.sin(i * 1.93 - phase * 2.3) * 0.5 + 0.5;
      const n3 = Math.sin(i * 11.37 + phase) * 0.5 + 0.5;
      const strengthN = n1 * n2 * n3;
      if (strengthN < 0.2) continue;
      const rayW = Math.max(0.7, (1.2 + n2 * 2.8) * Math.max(0.55, k));
      const rayH = (hem.y - top) * (0.42 + n3 * 0.58);
      const ra = Math.min(0.13, strengthN * 0.12 * brightAt(t) * c.alpha * k);
      if (ra < 0.006) continue;
      const rg = ctx.createLinearGradient(0, hem.y, 0, hem.y - rayH);
      rg.addColorStop(0, `rgba(185, 255, 195, ${(ra * 1.9).toFixed(3)})`);
      rg.addColorStop(0.32, `rgba(88, 242, 140, ${ra.toFixed(3)})`);
      rg.addColorStop(0.72, `rgba(68, 198, 158, ${(ra * 0.38).toFixed(3)})`);
      rg.addColorStop(1, 'rgba(68, 178, 168, 0)');
      ctx.fillStyle = rg;
      ctx.fillRect(hem.x - rayW / 2, hem.y - rayH, rayW, rayH);
    }

    // 3d. 下缘强光：整条飘带最亮的地方，向上快速衰减（指数式）
    //     这是"光从一条边界向上扩散"的物理样子
    const rimH = H * 0.3;
    for (let i = 0; i < SEG; i++) {
      const t = i / SEG;
      const t2 = (i + 1) / SEG;
      const h1 = hemAt(t);
      const h2 = hemAt(t2);
      const bm = brightAt(t);
      const ra = Math.min(0.3, 0.24 * bm * c.alpha * k);
      const rg = ctx.createLinearGradient(0, h1.y, 0, h1.y - rimH);
      rg.addColorStop(0, `rgba(220, 255, 205, ${ra.toFixed(3)})`);
      rg.addColorStop(0.06, `rgba(140, 250, 150, ${(ra * 0.8).toFixed(3)})`);
      rg.addColorStop(0.22, `rgba(66, 225, 120, ${(ra * 0.34).toFixed(3)})`);
      rg.addColorStop(0.5, `rgba(56, 185, 145, ${(ra * 0.1).toFixed(3)})`);
      rg.addColorStop(1, 'rgba(60, 160, 170, 0)');
      ctx.fillStyle = rg;
      ctx.beginPath();
      ctx.moveTo(h1.x, h1.y);
      ctx.lineTo(h2.x, h2.y);
      ctx.lineTo(h2.x, h2.y - rimH * 0.9);
      ctx.lineTo(h1.x, h1.y - rimH);
      ctx.closePath();
      ctx.fill();
    }

    // 3e. 上部的紫红（630nm 氧红只出现在更高处，很淡）
    const topA = Math.min(0.05, 0.035 * c.alpha * k);
    const tg = ctx.createLinearGradient(0, c.bottom - H * 0.42, 0, c.bottom - H * 0.85);
    tg.addColorStop(0, 'rgba(150, 90, 170, 0)');
    tg.addColorStop(0.5, `rgba(170, 100, 180, ${topA.toFixed(3)})`);
    tg.addColorStop(1, 'rgba(190, 110, 190, 0)');
    ctx.fillStyle = tg;
    ctx.fillRect(0, c.bottom - H * 0.85, W, H * 0.45);

    ctx.restore();

  }
  ctx.restore();

  // ---------- 3g. 山脊溢出光：极光照亮山脊上缘 ----------
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  const ridgeBaseY = H * 0.9;
  const spill = ctx.createLinearGradient(0, ridgeBaseY - H * 0.14, 0, ridgeBaseY + H * 0.02);
  spill.addColorStop(0, 'rgba(60, 180, 125, 0)');
  spill.addColorStop(0.45, `rgba(56, 170, 120, ${(0.055 * k).toFixed(3)})`);
  spill.addColorStop(0.8, `rgba(48, 150, 112, ${(0.09 * k).toFixed(3)})`);
  spill.addColorStop(1, 'rgba(40, 120, 96, 0)');
  ctx.fillStyle = spill;
  ctx.fillRect(0, ridgeBaseY - H * 0.14, W, H * 0.16);
  ctx.restore();

  // ---------- 4. 大气泛光（很淡） ----------
  ctx.save();
  ctx.globalCompositeOperation = 'lighter';
  const air = ctx.createLinearGradient(0, H * 0.35, 0, H * 0.9);
  air.addColorStop(0, 'rgba(40, 140, 95, 0)');
  air.addColorStop(0.55, `rgba(40, 150, 100, ${(0.028 * k).toFixed(3)})`);
  air.addColorStop(1, 'rgba(50, 130, 105, 0)');
  ctx.fillStyle = air;
  ctx.fillRect(0, H * 0.35, W, H * 0.55);
  ctx.restore();

  // ---------- 5. 地形剪影 ----------
  ctx.save();
  const ridgeBase = H * 0.9;
  ctx.fillStyle = 'rgba(4, 9, 14, 0.94)';
  ctx.beginPath();
  ctx.moveTo(0, H);
  for (let x = 0; x <= W; x += Math.max(3, W / 200)) {
    const y = ridgeBase
      - Math.sin(x * 0.0041) * H * 0.065
      - Math.sin(x * 0.0109 + 1.9) * H * 0.042
      - Math.sin(x * 0.0253 + 0.4) * H * 0.02
      - Math.sin(x * 0.0587 + 2.7) * H * 0.008
      - Math.sin(x * 0.131 + 1.1) * H * 0.003;
    ctx.lineTo(x, y);
  }
  ctx.lineTo(W, H);
  ctx.closePath();
  ctx.fill();

  ctx.globalCompositeOperation = 'lighter';
  const rim = ctx.createLinearGradient(0, ridgeBase - H * 0.1, 0, ridgeBase + H * 0.02);
  rim.addColorStop(0, 'rgba(60, 170, 120, 0)');
  rim.addColorStop(0.6, `rgba(56, 165, 116, ${(0.075 * k).toFixed(3)})`);
  rim.addColorStop(1, 'rgba(44, 110, 92, 0)');
  ctx.fillStyle = rim;
  ctx.fillRect(0, ridgeBase - H * 0.1, W, H * 0.12);
  ctx.restore();

  // ---------- 6. 镜头暗角 ----------
  const vig = ctx.createRadialGradient(
    W * 0.5, H * 0.42, Math.min(W, H) * 0.3,
    W * 0.5, H * 0.42, Math.max(W, H) * 0.82,
  );
  vig.addColorStop(0, 'rgba(0, 0, 0, 0)');
  vig.addColorStop(1, 'rgba(0, 0, 0, 0.38)');
  ctx.fillStyle = vig;
  ctx.fillRect(0, 0, W, H);

  // ---------- 7. 点击光点 ----------
  for (const p of particles) {
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.globalAlpha = p.alpha * Math.max(0, p.life) * k * 0.7;
    const pg = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, p.size * 4);
    pg.addColorStop(0, 'rgba(190, 255, 210, 0.6)');
    pg.addColorStop(0.45, 'rgba(80, 225, 145, 0.24)');
    pg.addColorStop(1, 'rgba(55, 180, 125, 0)');
    ctx.fillStyle = pg;
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size * 4, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

function drawInk(ctx: CanvasRenderingContext2D) {
  const k = strength();
  // 1. 宣纸底：明亮的冷白，顶部一丝暖意，整体通透不发黄，贴合浅色 UI
  const paper = ctx.createLinearGradient(0, 0, W * 0.25, H);
  // 宣纸底：参考图是纯白纸面，这里留一丝冷调把纸"托"起来
  paper.addColorStop(0, '#ffffff');
  paper.addColorStop(0.42, '#fdfefe');
  paper.addColorStop(0.76, '#f8fbfc');
  paper.addColorStop(1, '#f2f7f9');
  ctx.fillStyle = paper;
  ctx.fillRect(0, 0, W, H);

  // 2. 淡日：远峰之上一轮极淡的暖圆，画面的视觉锚点
  if (ikSun) {
    const s = ikSun;
    const halo = ctx.createRadialGradient(s.x, s.y, 0, s.x, s.y, s.r * 2.6);
    halo.addColorStop(0, `rgba(228,150,116,${(0.2 * s.alpha * k).toFixed(3)})`);
    halo.addColorStop(0.4, `rgba(232,170,140,${(0.09 * s.alpha * k).toFixed(3)})`);
    halo.addColorStop(1, 'rgba(232,170,140,0)');
    ctx.fillStyle = halo;
    ctx.beginPath(); ctx.arc(s.x, s.y, s.r * 2.6, 0, Math.PI * 2); ctx.fill();
    const disc = ctx.createRadialGradient(s.x - s.r * 0.2, s.y - s.r * 0.2, 0, s.x, s.y, s.r);
    disc.addColorStop(0, `rgba(214,120,90,${(0.36 * s.alpha * k).toFixed(3)})`);
    disc.addColorStop(0.8, `rgba(208,114,84,${(0.27 * s.alpha * k).toFixed(3)})`);
    disc.addColorStop(1, 'rgba(208,114,84,0)');
    ctx.fillStyle = disc;
    ctx.beginPath(); ctx.arc(s.x, s.y, s.r, 0, Math.PI * 2); ctx.fill();
  }

  // 3. 远鸟：几笔「人」字墨，轻轻掠过
  for (const b of ikBirds) {
    const wing = Math.sin(b.flap) * b.size * 0.32;
    ctx.save();
    ctx.globalAlpha = b.alpha * k;
    ctx.strokeStyle = 'rgba(70,82,98,0.8)';
    ctx.lineWidth = 1.1;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(b.x - b.size, b.y - wing);
    ctx.quadraticCurveTo(b.x - b.size * 0.3, b.y + b.size * 0.18, b.x, b.y);
    ctx.quadraticCurveTo(b.x + b.size * 0.3, b.y + b.size * 0.18, b.x + b.size, b.y - wing);
    ctx.stroke();
    ctx.restore();
  }

  // 4. 群山：由远及近，每层山脚压一条雾带，下一层从云里浮现
  const traceRidge = (pts: { x: number; y: number }[]) => {
    ctx.moveTo(pts[0].x, pts[0].y);
    for (let i = 1; i < pts.length - 1; i++) {
      const midX = (pts[i].x + pts[i + 1].x) / 2;
      const midY = (pts[i].y + pts[i + 1].y) / 2;
      ctx.quadraticCurveTo(pts[i].x, pts[i].y, midX, midY);
    }
    ctx.lineTo(pts[pts.length - 1].x, pts[pts.length - 1].y);
  };
  for (let i = 0; i < ikRidges.length; i++) {
    const ridge = ikRidges[i];
    const pts = ridge.points;
    if (!pts.length) continue;
    // 山体：峰顶积墨、山脚淡出收进雾里；渐变锚在真实峰顶高度
    ctx.beginPath();
    traceRidge(pts);
    ctx.lineTo(pts[pts.length - 1].x, H);
    ctx.lineTo(pts[0].x, H);
    ctx.closePath();
    const fill = ctx.createLinearGradient(0, ridge.top, 0, ridge.base + H * 0.04);
    fill.addColorStop(0, `rgba(${ridge.tint},${(ridge.alpha * k * 1.2).toFixed(3)})`);
    fill.addColorStop(0.45, `rgba(${ridge.tint},${(ridge.alpha * k * 0.62).toFixed(3)})`);
    fill.addColorStop(1, `rgba(${ridge.tint},0)`);
    ctx.fillStyle = fill;
    ctx.fill();
    // 勾勒：沿山脊一道略浓的墨线，带柔边，像毛笔勾出的轮廓
    ctx.save();
    ctx.beginPath();
    traceRidge(pts);
    ctx.strokeStyle = `rgba(${ridge.tint},${Math.min(0.62, ridge.alpha * 1.8 * k).toFixed(3)})`;
    ctx.lineWidth = 1.15 + ridge.layer * 0.3;
    ctx.lineJoin = 'round';
    ctx.shadowColor = `rgba(${ridge.tint},0.3)`;
    ctx.shadowBlur = 2.5;
    ctx.stroke();
    ctx.restore();
    // 皴笔：自山脊垂下的短竖笔（披麻皴），init 时预生成避免闪烁
    ctx.save();
    ctx.lineCap = 'round';
    ctx.lineWidth = 0.7;
    for (const st of ridge.strokes) {
      ctx.strokeStyle = `rgba(${ridge.tint},${(ridge.alpha * 0.55 * st.a * k).toFixed(3)})`;
      ctx.beginPath();
      ctx.moveTo(st.x, st.y);
      ctx.quadraticCurveTo(st.x + st.bow, st.y + st.len * 0.6, st.x + st.bow * 0.4, st.y + st.len);
      ctx.stroke();
    }
    ctx.restore();
    // 山脚雾带
    const band = ikBands[i];
    if (band) {
      const by = band.y + Math.sin(band.phase) * H * 0.008;
      const bg = ctx.createLinearGradient(0, by - band.h, 0, by + band.h);
      bg.addColorStop(0, 'rgba(250,251,252,0)');
      bg.addColorStop(0.5, `rgba(250,251,252,${(0.72 * band.alpha * k).toFixed(3)})`);
      bg.addColorStop(1, 'rgba(250,251,252,0)');
      ctx.fillStyle = bg;
      ctx.fillRect(0, by - band.h, W, band.h * 2);
    }
    // 孤舟泊在中景雾水：画在近岭之前，近岸自然遮住船底，形成纵深
    if (i === 2 && ikBoat) {
      const b = ikBoat;
      const boatY = b.y + Math.sin(b.phase) * 1.6;
      ctx.save();
      ctx.translate(b.x, boatY);
      if (b.flip) ctx.scale(-1, 1);
      ctx.lineCap = 'round';
      ctx.globalAlpha = 0.6 * k;
      ctx.fillStyle = 'rgba(52,64,80,0.8)';
      ctx.strokeStyle = 'rgba(52,64,80,0.9)';
      ctx.lineWidth = 1.1;
      ctx.beginPath();
      ctx.moveTo(-16, 0);
      ctx.quadraticCurveTo(0, 5.5, 16, -1);
      ctx.quadraticCurveTo(6, 2.2, -16, 0);
      ctx.closePath();
      ctx.fill();
      ctx.beginPath();
      ctx.moveTo(-5, -0.5);
      ctx.quadraticCurveTo(0, -6.5, 6, -0.8);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(9.5, -1);
      ctx.lineTo(10.5, -4.5);
      ctx.stroke();
      ctx.beginPath(); ctx.arc(10.8, -5.6, 1.2, 0, Math.PI * 2); ctx.fill();
      ctx.globalAlpha = 0.2 * k;
      ctx.beginPath();
      ctx.moveTo(-12, 4.5);
      ctx.quadraticCurveTo(0, 6.5, 12, 4);
      ctx.stroke();
      ctx.restore();
    }
  }

  // 5. 漂浮云雾：前景几团缓缓流动的淡雾
  for (const m of ikMist) {
    const y = m.y + Math.sin(m.phase) * H * 0.012;
    const grad = ctx.createRadialGradient(m.x, y, 0, m.x, y, m.w * 0.55);
    grad.addColorStop(0, `rgba(255,255,255,${(m.alpha * 1.5 * k).toFixed(3)})`);
    grad.addColorStop(0.6, `rgba(250,251,252,${(m.alpha * 0.6 * k).toFixed(3)})`);
    grad.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.ellipse(m.x, y, m.w * 0.55, m.h, 0, 0, Math.PI * 2);
    ctx.fill();
  }

  // 6. 朱印：右下角一枚落款小印，水墨的点睛之笔
  ctx.save();
  ctx.translate(W - 40, H - 44);
  ctx.rotate(-0.03);
  ctx.globalAlpha = Math.min(0.85, 0.8 * k);
  ctx.fillStyle = 'rgba(178,58,44,0.92)';
  ctx.beginPath();
  ctx.roundRect(-10.5, -10.5, 21, 21, 3);
  ctx.fill();
  ctx.strokeStyle = 'rgba(252,248,244,0.92)';
  ctx.lineWidth = 1.3;
  ctx.beginPath();
  ctx.moveTo(-4.5, -6); ctx.lineTo(-4.5, 6);
  ctx.moveTo(-7, -2.5); ctx.lineTo(-2, -2.5);
  ctx.moveTo(-7, 2.5); ctx.lineTo(-2, 2.5);
  ctx.moveTo(4.5, -6); ctx.lineTo(4.5, 6);
  ctx.moveTo(2, -4); ctx.lineTo(7, -4);
  ctx.moveTo(2, 0); ctx.lineTo(7, 0);
  ctx.moveTo(2, 4); ctx.lineTo(7, 4);
  ctx.stroke();
  ctx.restore();

  // 7. 墨晕（点击）：一团墨在水中晕开，中心浓、边缘淡，带不规则扩散
  for (const r of ikRipples) {
    ctx.save();
    const bloom = ctx.createRadialGradient(r.x, r.y, 0, r.x, r.y, r.r);
    bloom.addColorStop(0, `rgba(70,82,98,${(r.alpha * 0.5 * k).toFixed(3)})`);
    bloom.addColorStop(0.6, `rgba(86,98,114,${(r.alpha * 0.28 * k).toFixed(3)})`);
    bloom.addColorStop(0.85, `rgba(96,108,124,${(r.alpha * 0.12 * k).toFixed(3)})`);
    bloom.addColorStop(1, 'rgba(96,108,124,0)');
    ctx.fillStyle = bloom;
    ctx.beginPath(); ctx.arc(r.x, r.y, r.r, 0, Math.PI * 2); ctx.fill();
    ctx.globalAlpha = r.alpha * k;
    ctx.strokeStyle = `rgba(70,82,98,${(r.alpha * 0.6).toFixed(3)})`;
    ctx.lineWidth = 1;
    ctx.beginPath(); ctx.arc(r.x, r.y, r.r * 0.92, 0, Math.PI * 2); ctx.stroke();
    ctx.restore();
  }

  // 7. 墨点飞溅
  for (const p of particles) {
    ctx.save();
    ctx.globalAlpha = p.alpha * Math.max(0, p.life) * k * 0.8;
    ctx.fillStyle = 'rgba(70,82,98,0.7)';
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size * 0.85, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 8. 共享涟漪（鼠标经过）
  for (const r of ripples) {
    ctx.save();
    ctx.globalAlpha = r.alpha;
    ctx.strokeStyle = r.color + r.alpha.toFixed(2) + ')';
    ctx.lineWidth = r.lw * 0.8;
    ctx.beginPath();
    ctx.arc(r.x, r.y, r.r, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }
}

/** 细雨：斜雨丝 + 玻璃水珠 + 落点涟漪 */
/**
 * 细雨（重写版）。
 *
 * 旧版只有「一排等长的斜线 + 几个圆圈」，问题在于：
 *   1. 雨丝没有前后层次，看起来像贴了一层斜纹纸
 *   2. 水珠是个规则椭圆，像浮在画面上的贴纸
 *   3. 缺「雨落在某个面上」的反馈，所以没有空间感
 *
 * 新版按真实雨景的三层结构来做：
 *   - 远景雨：细、慢、极淡，只提供整体的"雨幕"质感
 *   - 中景雨：主雨丝，两端渐隐（真实雨丝不会是一根等粗的棍子）
 *   - 近景雨：粗、快、更深，偶尔划过镜头，制造纵深
 *   再加玻璃水珠（带高光、带拖尾、会慢慢下滑）与落地涟漪，把「雨」落到一个面上。
 */
function initRain() {
  rnStreaks = []; rnDrops = []; rnRipples = []; rnSplashes = [];

  // 三层雨幕：远 / 中 / 近
  const layers = [
    { count: 58, lenMin: 7, lenMax: 14, vxMin: 0.25, vxMax: 0.5, vyMin: 1.6, vyMax: 2.6, w: 0.55, aMin: 0.06, aMax: 0.12 },
    { count: 44, lenMin: 14, lenMax: 28, vxMin: 0.5, vxMax: 0.95, vyMin: 3.4, vyMax: 5.4, w: 0.9, aMin: 0.1, aMax: 0.18 },
    { count: 18, lenMin: 28, lenMax: 52, vxMin: 0.9, vxMax: 1.6, vyMin: 6.2, vyMax: 9.5, w: 1.3, aMin: 0.12, aMax: 0.22 },
  ];
  for (const layer of layers) {
    for (let i = 0; i < layer.count; i++) {
      rnStreaks.push({
        x: rnd(-W * 0.25, W * 1.05),
        y: rnd(-H, H),
        len: rnd(layer.lenMin, layer.lenMax),
        vx: rnd(layer.vxMin, layer.vxMax),
        vy: rnd(layer.vyMin, layer.vyMax),
        width: layer.w,
        alpha: rnd(layer.aMin, layer.aMax),
      });
    }
  }

  // 玻璃水珠：大小分两档，大的慢、小的快
  for (let i = 0; i < 20; i++) {
    const big = i < 7;
    rnDrops.push({
      x: rnd(0, W), y: rnd(0, H),
      r: big ? rnd(2.6, 4.6) : rnd(1.1, 2.2),
      drift: big ? rnd(0.02, 0.1) : rnd(0.05, 0.22),
      wiggle: rnd(0, 6.28),
      trail: 0,
      alpha: big ? rnd(0.16, 0.3) : rnd(0.1, 0.2),
    });
  }
}

/**
 * 细雨绘制。
 * 顺序很重要：底色 → 远景雨 → 中景雨 → 玻璃水珠 → 近景雨（压在珠子前，制造纵深）→ 涟漪
 */
/**
 * 细雨（写实版）：隔着窗玻璃看外面的雨。
 *
 * 真实摄影的层次（这是「写实」与「卡通」的分界）：
 *   1. 窗外是失焦的：灰白天光、模糊的树影/建筑轮廓、雨丝在外面落
 *   2. 玻璃上有一层凝结雾
 *   3. 玻璃表面有大小不一的水珠 —— 对焦在这里，所以水珠是清晰的
 *   4. 水珠有：倒映窗外的折射色、左上高光点、下缘暗边、内反射亮弧
 *   5. 大珠会下滑，拖出一道湿痕
 *
 * 关键：焦点关系不能反 —— 窗外模糊、玻璃清晰。反了就立刻变成贴图。
 */
/**
 * 细雨（第二版）：隔着湿玻璃看外面的阴雨天。
 *
 * 第一版被指"像磨砂塑料上的装饰气泡"，根因是窗外没有可辨认的景物，
 * 水珠也没有折射任何东西。这一版先建窗外，再在玻璃上做水系。
 *
 * 焦点策略：玻璃清晰（微珠/中珠/大珠边缘锐利），窗外模糊（用 blur 滤镜）。
 * 这样景深才有结构，而不是"全部一样软"。
 */
/**
 * 烟雨江南。
 *
 * 这里画的不是"城市雨夜"（那是上一版：阴天天空 + 楼房轮廓 + 窗格灯光），
 * 而是江南水墨的意境：远山、静水、垂柳、薄雾、毛毛雨。
 *
 * 层次由远及近：
 *   1. 灰绿天空 —— 接近宣纸底色，对比压得很低
 *   2. 三层远山 —— 越远越淡，边缘晕开（水墨远山就是这么画的）
 *   3. 静水倒影 —— 山的模糊镜像，越往下越散
 *   4. 雾带       —— 横向淡白条，随时间缓慢横移
 *   5. 垂柳       —— 细长下垂的枝条，是江南最标志的元素
 *   6. 毛毛雨     —— 极细极密的短斜线，不是暴雨
 */
/**
 * 细雨：纯下雨，带动态。
 *
 * 这里画的是「下雨」本身，不画窗户、不画楼房、不画玻璃水珠。
 *
 * 动态来自 update() 里一直在跑的那几套状态：
 *   rnStreaks   三层雨幕 —— 远层密而淡、中层中等、近层粗而快
 *   rnWind      缓慢变化的阵风 —— 整片雨丝方向会轻轻摆动，雨才「活」
 *   rnRipples   地面涟漪 —— 持续生成、扩散、淡出
 *   rnSplashes  溅起的水花 —— 向上弹起后落回
 *
 * drawRain 只负责把这些状态画出来。之前那版"烟雨江南"用位置哈希算毛毛雨，
 * 每帧完全一样，所以看起来是死的。
 */
function drawRain(ctx: CanvasRenderingContext2D) {
  const k = strength();

  // 1. 底色：左上角亮、右下角略沉，模拟阴天窗边的光
  const bg = ctx.createLinearGradient(0, 0, W * 0.35, H);
  bg.addColorStop(0, '#fbfdfe');
  bg.addColorStop(0.42, '#eef4f7');
  bg.addColorStop(0.78, '#e2ecf1');
  bg.addColorStop(1, '#d7e4ea');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  // 2. 窗玻璃上的一层薄雾：让远处的雨丝显得「在玻璃后面」
  const haze = ctx.createLinearGradient(0, 0, 0, H);
  haze.addColorStop(0, `rgba(255,255,255,${(0.2 * k).toFixed(3)})`);
  haze.addColorStop(0.5, `rgba(255,255,255,${(0.06 * k).toFixed(3)})`);
  haze.addColorStop(1, `rgba(226,238,243,${(0.14 * k).toFixed(3)})`);
  ctx.fillStyle = haze;
  ctx.fillRect(0, 0, W, H);

  ctx.lineCap = 'round';

  // 3. 远景 + 中景雨丝（按宽度排序，细的先画）
  const farAndMid = rnStreaks.filter((s) => s.width < 1);
  const near = rnStreaks.filter((s) => s.width >= 1);

  for (const s of farAndMid) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.58, s.alpha * k * 3.4);
    // 两端渐隐：雨丝不是等粗的棍子
    const tailX = s.x - s.vx * 2.6;
    const tailY = s.y - s.len;
    const grad = ctx.createLinearGradient(s.x, s.y, tailX, tailY);
    grad.addColorStop(0, 'rgba(118,152,172,1)');
    grad.addColorStop(0.4, 'rgba(134,166,184,0.72)');
    grad.addColorStop(0.78, 'rgba(158,186,200,0.28)');
    grad.addColorStop(1, 'rgba(176,198,210,0)');
    ctx.strokeStyle = grad;
    ctx.lineWidth = s.width;
    ctx.beginPath();
    ctx.moveTo(s.x, s.y);
    ctx.lineTo(tailX, tailY);
    ctx.stroke();
    ctx.restore();
  }

  // 4. 玻璃水珠：高光 + 阴影 + 拖尾
  for (const d of rnDrops) {
    const rx = d.r;
    const ry = d.r * rnd(1, 1.02) * 1.16;

    // 拖尾：水珠下滑留下的湿痕
    if (d.trail > 2) {
      const trailGrad = ctx.createLinearGradient(d.x, d.y - d.trail, d.x, d.y);
      trailGrad.addColorStop(0, 'rgba(198,220,230,0)');
      trailGrad.addColorStop(1, `rgba(198,220,230,${(0.28 * k).toFixed(3)})`);
      ctx.save();
      ctx.strokeStyle = trailGrad;
      ctx.lineWidth = rx * 0.9;
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(d.x, d.y - d.trail);
      ctx.lineTo(d.x, d.y);
      ctx.stroke();
      ctx.restore();
    }

    ctx.save();
    ctx.globalAlpha = d.alpha * k;

    // 珠子本体：上暗下亮，像镜头里的真实水珠
    const body = ctx.createRadialGradient(
      d.x - rx * 0.35, d.y - ry * 0.4, rx * 0.1,
      d.x, d.y, ry,
    );
    body.addColorStop(0, 'rgba(255,255,255,0.95)');
    body.addColorStop(0.45, 'rgba(232,244,248,0.7)');
    body.addColorStop(0.82, 'rgba(178,204,216,0.55)');
    body.addColorStop(1, 'rgba(150,180,196,0.28)');
    ctx.fillStyle = body;
    ctx.beginPath();
    ctx.ellipse(d.x, d.y, rx, ry, 0, 0, Math.PI * 2);
    ctx.fill();

    // 下缘的亮边（全内反射），水珠才有体积
    ctx.strokeStyle = 'rgba(255,255,255,0.65)';
    ctx.lineWidth = 0.7;
    ctx.beginPath();
    ctx.ellipse(d.x, d.y + ry * 0.12, rx * 0.82, ry * 0.8, 0, 0.15 * Math.PI, 0.85 * Math.PI);
    ctx.stroke();

    // 左上小高光点
    ctx.globalAlpha = Math.min(0.62, d.alpha * k * 2.2);
    const spark = ctx.createRadialGradient(
      d.x - rx * 0.32, d.y - ry * 0.38, 0,
      d.x - rx * 0.32, d.y - ry * 0.38, rx * 0.42,
    );
    spark.addColorStop(0, 'rgba(255,255,255,0.9)');
    spark.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = spark;
    ctx.beginPath();
    ctx.arc(d.x - rx * 0.32, d.y - ry * 0.38, rx * 0.42, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 5. 近景雨：压在珠子之上，制造「雨从眼前划过」的纵深
  for (const s of near) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.46, s.alpha * k * 2.8);
    const tailX = s.x - s.vx * 3.4;
    const tailY = s.y - s.len;
    const grad = ctx.createLinearGradient(s.x, s.y, tailX, tailY);
    grad.addColorStop(0, 'rgba(96,132,152,0.98)');
    grad.addColorStop(0.45, 'rgba(120,152,170,0.58)');
    grad.addColorStop(1, 'rgba(150,180,196,0)');
    ctx.strokeStyle = grad;
    ctx.lineWidth = s.width;
    ctx.beginPath();
    ctx.moveTo(s.x, s.y);
    ctx.lineTo(tailX, tailY);
    ctx.stroke();
    ctx.restore();
  }

  // 6. 落地涟漪 + 溅起的水花
  for (const r of rnRipples) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.5, r.alpha * k * 1.4);
    ctx.strokeStyle = 'rgba(146,178,192,0.85)';
    ctx.lineWidth = 0.9;
    ctx.beginPath();
    ctx.ellipse(r.x, r.y, r.r, r.r * 0.3, 0, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }

  for (const sp of rnSplashes) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.55, sp.alpha * k * 1.6);
    ctx.fillStyle = 'rgba(198,220,230,0.9)';
    ctx.beginPath();
    ctx.ellipse(sp.x, sp.y - sp.h, sp.r, sp.r * 0.72, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

/**
 * 画真实照片背景。
 *
 * 要点：
 *  · cover 铺满，绝不拉伸（拉伸一眼假）
 *  · ken-burns 极缓漂移：缩放 1.04→1.10 + 平移，周期约 40 秒，肉眼几乎察觉不到但画面是活的
 *  · 叠一层色调把照片统一到应用配色，避免每张图色彩打架
 *  · 暗角压住四角，让中间气泡区域保持视觉中心
 *  · 底部再压一层渐变，保证输入框区域的文字可读
 */
/**
 * 照片模式下保留的交互层：鼠标涟漪 + 点击光点。
 * 这些是「桌面宠物应用」该有的活气，照片本身是静的。
 */
function drawPhotoBackground(ctx: CanvasRenderingContext2D, k: number) {
  const cfg = photoBackground(props.mode);
  if (!cfg) return false;
  ensurePhotoLoaded();

  // 底色兜底：照片未加载完时先铺一层同色调，避免闪白
  ctx.fillStyle = "rgba(44, 50, 58, 1)";
  ctx.fillRect(0, 0, W, H);

  const img = photoImages[cfg.id];
  if (!img) return true; // 已画兜底色，等图加载

  /*
   * 完全静态的 cover 铺满。
   *
   * 这里删掉了 ken-burns 缩放、鼠标视差、光标光斑、涟漪、粒子。
   * 用户明确要求「所有的动态效果都删了」—— 照片本身是完整画面，
   * 加任何运动或叠加都会干扰阅读，静止反而是最好的效果。
   */
  /*
   * 铺图：单格用 cover；多窗格用「整幅画按格切分」。
   *
   * 单格 cover 时，图片短边铺满，长边溢出居中裁 —— 这是标准做法。
   *
   * 多窗格时不能用 cover，否则每格都是同一张图的中心，四格完全一样。
   * 改成：先算出整幅画铺满**整个窗格区域**时的尺寸和位置，
   * 再按当前格的行列号截取对应的那一条带。
   * 这样跨格子的线条是连续的，拼起来是一整幅画。
   */
  const ir = img.width / img.height;
  const cr = W / H;
  const slab = props.slab;

  if (slab && slab.cols > 0 && slab.rows > 0) {
    /*
     * 全景模式：把图片按整个网格的比例铺满，再取自己那一块。
     *
     * 关键点：铺满的是「整个网格的总尺寸」，不是单格尺寸。
     * 总尺寸按 cols:rows 的比例放大到覆盖单格所需的倍数。
     */
    const gw = W * slab.cols;
    const gh = H * slab.rows;
    const gr = gw / gh;
    const gscale = ir > gr ? gh / img.height : gw / img.width;
    const gdw = img.width * gscale;
    const gdh = img.height * gscale;
    const gox = (gw - gdw) / 2;
    const goy = (gh - gdh) / 2;

    // 本格在整幅画里的左上角
    const ox = gox - slab.col * W;
    const oy = goy - slab.row * H;

    ctx.save();
    ctx.globalAlpha = cfg.opacity;
    ctx.drawImage(img, ox, oy, gdw, gdh);
    ctx.restore();
  } else {
    const scale = ir > cr ? H / img.height : W / img.width;
    const dw = img.width * scale;
    const dh = img.height * scale;
    ctx.save();
    ctx.globalAlpha = cfg.opacity;
    ctx.drawImage(img, (W - dw) / 2, (H - dh) / 2, dw, dh);
    ctx.restore();
  }
  ctx.restore();

  /*
   * 提亮。
   *
   * 之前这块是"压暗"（暗角 + 上下黑渐变），用户反馈画面太暗。
   * 改成 screen 提亮：把整张照片往亮部推，做出通透感，
   * 亮度随背景浓度档位线性变化（浓度越高提亮越多）。
   */
  ctx.save();
  ctx.globalCompositeOperation = "screen";
  const lift = ctx.createLinearGradient(0, 0, W * 0.4, H);
  lift.addColorStop(0, `rgba(255, 253, 247, ${(0.10 * k).toFixed(3)})`);
  lift.addColorStop(0.55, `rgba(252, 253, 255, ${(0.06 * k).toFixed(3)})`);
  lift.addColorStop(1, `rgba(246, 250, 255, ${(0.03 * k).toFixed(3)})`);
  ctx.fillStyle = lift;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();

  // ---------- 色调统一（很淡，只把照片和界面拉到一个色温） ----------
  ctx.save();
  ctx.fillStyle = cfg.tint;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();

  /*
   * 上下只留极轻的过渡。
   *
   * 原来是顶部 0.3 / 底部 0.28 的黑渐变，为了让标题栏和输入框"压得住"，
   * 但那正是"画面闷"的主因（codex: "the large dark header reduces the
   * visible scene"）。现在压到 0.06/0.09，只留一丝层次，不再吃掉亮部。
   */
  const top = ctx.createLinearGradient(0, 0, 0, H * 0.15);
  top.addColorStop(0, "rgba(24, 28, 34, 0.06)");
  top.addColorStop(1, "rgba(24, 28, 34, 0)");
  ctx.fillStyle = top;
  ctx.fillRect(0, 0, W, H * 0.15);

  const bottom = ctx.createLinearGradient(0, H * 0.84, 0, H);
  bottom.addColorStop(0, "rgba(24, 28, 34, 0)");
  bottom.addColorStop(1, "rgba(24, 28, 34, 0.09)");
  ctx.fillStyle = bottom;
  ctx.fillRect(0, H * 0.84, W, H * 0.16);

  return true;
}

function drawLightField(ctx: CanvasRenderingContext2D, k: number, warm = '255, 250, 240') {
  const diag = Math.sqrt(W * W + H * H);

  /*
   * 这一层负责"贵气"而不是"阴影"。
   *
   * 之前的做法是 multiply 压暗做纵深，实测把画面底部压到 L≈0.65，
   * 大片灰区域看起来像蒙了层土 —— 那是"廉价"的真正来源，不是缺少对比。
   *
   * 现在反过来：先用 screen 把整体提亮到接近纯纸白，再用极轻的明暗塑造
   * 一点点起伏，最后加一道柔光带和中心光斑。亮而通透才是纸面该有的样子。
   */

  // 1. 整体提亮：一层很淡的暖白，把纸面提到"被光照着"的亮度
  ctx.save();
  ctx.globalCompositeOperation = 'screen';
  const lift = ctx.createLinearGradient(0, 0, W * 0.6, H);
  lift.addColorStop(0, `rgba(${warm},${Math.min(0.16, 0.13 * k).toFixed(3)})`);
  lift.addColorStop(0.55, `rgba(255, 254, 252,${(0.07 * k).toFixed(3)})`);
  lift.addColorStop(1, `rgba(252, 253, 255,${(0.04 * k).toFixed(3)})`);
  ctx.fillStyle = lift;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();

  // 2. 极轻的斜向明暗：只留 8% 的落差，够形成"光的方向"，不会发灰
  ctx.save();
  ctx.globalCompositeOperation = 'multiply';
  const sweep = ctx.createLinearGradient(0, 0, W * 0.95, H);
  sweep.addColorStop(0, 'rgba(255, 255, 255, 1)');
  sweep.addColorStop(0.4, `rgba(252, 252, 253,${(1 - 0.012 * k).toFixed(3)})`);
  sweep.addColorStop(0.75, `rgba(244, 246, 248,${(1 - 0.038 * k).toFixed(3)})`);
  sweep.addColorStop(1, `rgba(236, 240, 244,${(1 - 0.062 * k).toFixed(3)})`);
  ctx.fillStyle = sweep;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();

  // 3. 角落收边：12% 上限，只负责"画面不外溢"，不负责压暗
  ctx.save();
  ctx.globalCompositeOperation = 'multiply';
  const vignette = ctx.createRadialGradient(
    W * 0.38, H * 0.3, diag * 0.1,
    W * 0.38, H * 0.3, diag * 0.95,
  );
  vignette.addColorStop(0, 'rgba(255,255,255,0)');
  vignette.addColorStop(0.55, `rgba(248, 249, 251,${(0.03 * k).toFixed(3)})`);
  vignette.addColorStop(0.82, `rgba(238, 241, 245,${(0.07 * k).toFixed(3)})`);
  vignette.addColorStop(1, `rgba(226, 231, 238,${(0.12 * k).toFixed(3)})`);
  ctx.fillStyle = vignette;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();

  // 4. 顶部柔光带：像窗光打在墙顶，是"亮"的主要观感来源
  ctx.save();
  ctx.globalCompositeOperation = 'screen';
  const top = ctx.createLinearGradient(0, 0, 0, H * 0.46);
  top.addColorStop(0, `rgba(255, 255, 255,${(0.3 * k).toFixed(3)})`);
  top.addColorStop(0.5, `rgba(255, 254, 250,${(0.08 * k).toFixed(3)})`);
  top.addColorStop(1, 'rgba(255, 255, 255, 0)');
  ctx.fillStyle = top;
  ctx.fillRect(0, 0, W, H * 0.46);
  ctx.restore();

  // 5. 左上光斑：一处柔和的高光，画面立刻"有光"而不是"有灰"
  ctx.save();
  ctx.globalCompositeOperation = 'screen';
  const bloom = ctx.createRadialGradient(
    W * 0.24, H * 0.12, 0,
    W * 0.24, H * 0.12, Math.max(W, H) * 0.46,
  );
  bloom.addColorStop(0, `rgba(${warm},${Math.min(0.26, 0.18 * k).toFixed(3)})`);
  bloom.addColorStop(0.45, `rgba(255, 252, 246,${(0.06 * k).toFixed(3)})`);
  bloom.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = bloom;
  ctx.fillRect(0, 0, W, H);
  ctx.restore();
}

/**
 * 纸纹噪声：用极细的随机点打散大面积纯色。
 * 只画一次到离屏 canvas 再复用，避免每帧几千次 fillRect。
 */
let grainCanvas: HTMLCanvasElement | null = null;
function grainLayer(): HTMLCanvasElement {
  if (grainCanvas) return grainCanvas;
  const c = document.createElement('canvas');
  c.width = 128;
  c.height = 128;
  const g = c.getContext('2d');
  if (g) {
    const img = g.createImageData(128, 128);
    for (let i = 0; i < img.data.length; i += 4) {
      const v = 120 + Math.random() * 135;
      img.data[i] = v; img.data[i + 1] = v; img.data[i + 2] = v;
      img.data[i + 3] = Math.random() < 0.35 ? 22 : 0;
    }
    g.putImageData(img, 0, 0);
  }
  grainCanvas = c;
  return c;
}

function drawGrain(ctx: CanvasRenderingContext2D, k: number) {
  const tile = grainLayer();
  ctx.save();
  ctx.globalAlpha = Math.min(0.5, 0.3 * k);
  ctx.globalCompositeOperation = 'overlay';
  const pattern = ctx.createPattern(tile, 'repeat');
  if (pattern) {
    ctx.fillStyle = pattern;
    ctx.fillRect(0, 0, W, H);
  }
  ctx.restore();
}

function draw() {
  if (!ctx) return;
  ctx.clearRect(0, 0, W, H);

  // 照片背景优先：极光/水墨/细雨/科幻/樱花 现在都是真实摄影
  if (drawPhotoBackground(ctx, strength())) {
    /*
     * 照片模式完全静态。
     *
     * 这里曾经画过涟漪 + 光点 + 视差 + 光标光斑，用户明确要求
     * 「所有的动态效果都删了」。照片本身就是完整画面，
     * 加任何叠加层都只是干扰；静止反而更像一张高级壁纸。
     * 程序化背景（水墨/极光的手绘分支）保留它们自己的动画。
     */
    return;
  }

  // Background fills
  if (props.mode === 'sakura') {
    // Background gradient
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0, '#fffbfc'); g.addColorStop(0.34, '#fdf2f6'); g.addColorStop(0.68, '#fbeaf2'); g.addColorStop(1, '#f7e6f0');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // Bubbles
    for (const b of ctBubbles) {
      ctx.save(); ctx.globalAlpha = b.alpha;
      const bg = ctx.createRadialGradient(b.x - b.r*0.3, b.y - b.r*0.3, 0, b.x, b.y, b.r);
      bg.addColorStop(0,'rgba(255,255,255,0.85)'); bg.addColorStop(1, b.color + '55');
      ctx.fillStyle = bg;
      ctx.beginPath(); ctx.arc(b.x, b.y, b.r, 0, Math.PI*2); ctx.fill();
      ctx.strokeStyle = b.color + '99'; ctx.lineWidth = 1; ctx.stroke();
      ctx.restore();
    }
    // Sakura petals
    for (const p of ctPetals) {
      ctx.save(); ctx.globalAlpha = p.alpha * (p.temp ? p.life : 1);
      ctx.translate(p.x, p.y); ctx.rotate(p.angle);
      ctx.fillStyle = p.color;
      // Petal shape
      ctx.beginPath(); ctx.ellipse(0, 0, p.size, p.size * 0.55, 0, 0, Math.PI*2); ctx.fill();
      // Inner crease
      ctx.strokeStyle = 'rgba(255,255,255,0.7)'; ctx.lineWidth = 0.6;
      ctx.beginPath(); ctx.moveTo(-p.size * 0.5, 0); ctx.lineTo(p.size * 0.5, 0); ctx.stroke();
      ctx.restore();
    }
    // Trail
    for (const t of trail) {
      ctx.save(); ctx.globalAlpha = t.t * 0.5; ctx.fillStyle = '#d6ceba';
      drawSparkle(ctx, t.x, t.y, 2.5 + t.t * 5); ctx.restore();
    }
    // Burst particles
    for (const p of particles) {
      ctx.save(); ctx.globalAlpha = p.alpha * p.life;
      ctx.fillStyle = p.color;
      ctx.translate(p.x, p.y); ctx.rotate(p.angle);
      if (p.shape === 'heart') drawHeart(ctx, 0, 0, p.size);
      else if (p.shape === 'sparkle') drawSparkle(ctx, 0, 0, p.size);
      else if (p.shape === 'star') drawStar(ctx, 0, 0, p.size);
      else { ctx.beginPath(); ctx.arc(0, 0, p.size, 0, Math.PI*2); ctx.fill(); }
      ctx.restore();
    }
  } else if (props.mode === 'minimal') {
    // Background
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0, '#fdfefe'); g.addColorStop(0.4, '#f7fafc'); g.addColorStop(0.72, '#eef5f9'); g.addColorStop(1, '#e4eef5');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // Draw connection lines between nearby glowing dots
    ctx.lineWidth = 0.8;
    for (let i = 0; i < mnDots.length; i++) {
      const a = mnDots[i]; if (a.glow < 0.05) continue;
      for (let j = i + 1; j < mnDots.length; j++) {
        const b = mnDots[j]; if (b.glow < 0.05 && (a.glow < 0.05)) continue;
        const dx = a.x - b.x, dy = a.y - b.y, d = Math.sqrt(dx*dx+dy*dy);
        if (d < 50) {
          ctx.strokeStyle = `rgba(120,140,166,${Math.min(a.glow,b.glow)*0.95})`;
          ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
        }
      }
    }
    // Constellation rays (click)
    for (const ray of mnRays) {
      ctx.save(); ctx.globalAlpha = ray.alpha;
      ctx.strokeStyle = '#adb2bb'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.moveTo(ray.x, ray.y); ctx.lineTo(ray.tx, ray.ty); ctx.stroke();
      ctx.restore();
    }
    // Dots
    for (const d of mnDots) {
      const alpha = 0.12 + d.glow * 0.7;
      const size = 1.5 + d.glow * 3;
      ctx.save(); ctx.globalAlpha = alpha;
      ctx.fillStyle = d.glow > 0.3 ? '#8aa6b8' : '#adb2bb';
      if (d.glow > 0.2) { ctx.shadowColor = '#8aa6b8'; ctx.shadowBlur = 6 * d.glow; }
      ctx.beginPath(); ctx.arc(d.x, d.y, size, 0, Math.PI*2); ctx.fill();
      ctx.shadowBlur = 0; ctx.restore();
    }
    // Burst particles (click)
    for (const p of particles) {
      ctx.save(); ctx.globalAlpha = p.alpha * p.life;
      ctx.strokeStyle = '#adb2bb'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.arc(p.x, p.y, 1.5, 0, Math.PI*2); ctx.fill();
      ctx.restore();
    }
  } else if (props.mode === 'scifi') {
    const t = performance.now() / 1000;
    const petState = petStore.state;
    const k = strength();

    // 1. 纸面底色：浅冷灰蓝，紧贴控制台纸色，避免深色块与浅色 UI 撞边
    const g = ctx.createLinearGradient(0, 0, W * 0.3, H);
    g.addColorStop(0, '#f7fafd');
    g.addColorStop(0.5, '#f4f7fc');
    g.addColorStop(1, '#f7f5fd');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);

    // 2. 星云：浅色纸上用 multiply 叠一层极淡的冷色雾
    ctx.save();
    ctx.globalCompositeOperation = 'multiply';
    for (const neb of sfNebulae) {
      const ng = ctx.createRadialGradient(neb.x, neb.y, 0, neb.x, neb.y, neb.r);
      ng.addColorStop(0, neb.color1);
      ng.addColorStop(1, neb.color2);
      ctx.fillStyle = ng;
      ctx.beginPath(); ctx.arc(neb.x, neb.y, neb.r, 0, Math.PI*2); ctx.fill();
    }
    ctx.restore();

    // 3. Hex Grid：纸面上的淡网格，只在鼠标附近浮现
    ctx.lineWidth = 1;
    for (const h of sfHexes) {
      // 常驻一层极淡的网格（base 0.18），鼠标靠近再叠加到 1.0。
      // 之前只有 glow>0.01 才画，而 glow 初始为 0，导致不碰鼠标时整片空白。
      const vis = 0.18 + h.glow * 0.82;
      if (vis > 0.01) {
        ctx.strokeStyle = `rgba(110,158,206,${(vis * 0.3 * k).toFixed(3)})`;
        ctx.beginPath();
        for (let i = 0; i < 6; i++) {
          const a = i * Math.PI / 3;
          let vx = h.x + Math.cos(a) * h.s;
          let vy = h.y + Math.sin(a) * h.s;
          
          // Warp mesh vertices away from cursor
          if (mx > -900) {
            const dx = vx - mx;
            const dy = vy - my;
            const dist = Math.sqrt(dx*dx+dy*dy);
            if (dist < 90) {
              const force = (90 - dist) / 90;
              const push = force * force * 14.0; // push vertices away by up to 14px
              vx += (dx / (dist || 1)) * push;
              vy += (dy / (dist || 1)) * push;
            }
          }
          
          i === 0 ? ctx.moveTo(vx, vy) : ctx.lineTo(vx, vy);
        }
        ctx.closePath(); ctx.stroke();
      }
    }
    
    // 4. Scanline：改成纸面上缓慢掠过的一道极淡光带
    const sg = ctx.createLinearGradient(0, sfScanY - 60, 0, sfScanY);
    sg.addColorStop(0, 'rgba(150,190,230,0)');
    sg.addColorStop(1, `rgba(150,190,230,${(0.055 * k).toFixed(3)})`);
    ctx.fillStyle = sg; ctx.fillRect(0, sfScanY - 60, W, 60);
    ctx.fillStyle = `rgba(150,190,230,${(0.10 * k).toFixed(3)})`; ctx.fillRect(0, sfScanY, W, 1);

    // 5. Neural Connections (glitchy/pulsing when thinking)
    ctx.lineWidth = 1;
    for (let i = 0; i < sfNodes.length; i++) {
      for (let j = i + 1; j < sfNodes.length; j++) {
        const a = sfNodes[i], b = sfNodes[j], dx = a.x - b.x, dy = a.y - b.y, d = Math.sqrt(dx*dx+dy*dy);
        if (d < 240) {
          const e = Math.max(a.energy, b.energy);
          let alpha = (1 - d/240) * (0.08 + e * 0.30) * k;

          if (petState === 'thinking') {
            // High-frequency transmission pulse
            alpha *= (0.68 + Math.sin(t * 15.0 + d * 0.05) * 0.32);
          } else if (petState === 'speaking') {
            // Wave propagation pulse
            alpha *= (0.8 + Math.cos(t * 6.0 - d * 0.02) * 0.2);
          }

          ctx.strokeStyle = `rgba(140,150,220,${alpha.toFixed(3)})`;
          ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
        }
      }
    }

    // Quantum capture threads from cursor to nearby nodes
    if (mx > -900) {
      for (const n of sfNodes) {
        const dx = n.x - mx, dy = n.y - my, dist = Math.sqrt(dx*dx+dy*dy);
        if (dist < 110) {
          const near = (110 - dist) / 110;
          ctx.strokeStyle = `rgba(120,168,220,${(near * 0.34 * k).toFixed(3)})`;
          ctx.lineWidth = 0.6 + near * 0.7;
          ctx.beginPath();
          ctx.moveTo(mx, my);
          ctx.lineTo(n.x, n.y);
          ctx.stroke();

          // Technical micro-distance tag at midpoint of thread
          if (near > 0.6) {
            ctx.fillStyle = `rgba(120,168,220,${(near * 0.5 * k).toFixed(3)})`;
            ctx.font = '6px monospace';
            const midX = mx + dx * 0.5;
            const midY = my + dy * 0.5;
            ctx.fillText(`${Math.round(dist)}pm`, midX + 5, midY - 2);
          }
        }
      }
    }

    // 6. Nodes：小球改成纸面上的彩色圆点，去掉大范围光晕
    for (const n of sfNodes) {
      ctx.save();
      ctx.globalAlpha = (0.5 + n.energy * 0.5) * k;
      ctx.fillStyle = n.color;
      ctx.shadowColor = n.color;
      ctx.shadowBlur = 4 + n.energy * 8;
      ctx.beginPath(); ctx.arc(n.x, n.y, n.r * 0.7 + n.energy * 1.4, 0, Math.PI*2); ctx.fill();
      ctx.restore();

      if (n.type === 'hub') {
        ctx.save();
        ctx.globalAlpha = 0.32 * k;
        ctx.strokeStyle = n.color; ctx.lineWidth = 0.9;
        ctx.beginPath(); ctx.arc(n.x, n.y, n.r + 5.5 + Math.sin(n.pulse)*2, 0, Math.PI*2); ctx.stroke();
        ctx.restore();
      }
    }

    // 7. Packets
    for (const p of sfPackets) {
      ctx.save();
      ctx.globalAlpha = 0.75 * k;
      ctx.fillStyle = p.color; ctx.shadowColor = p.color; ctx.shadowBlur = 4;
      ctx.beginPath(); ctx.arc(p.x, p.y, p.r * 0.8, 0, Math.PI*2); ctx.fill();
      ctx.restore();
    }

    // 8. EMPs
    for (const e of sfEMPs) {
      ctx.save();
      ctx.strokeStyle = e.color; ctx.lineWidth = 1 + e.alpha * 2;
      ctx.globalAlpha = e.alpha * k;
      ctx.beginPath(); ctx.arc(e.x, e.y, e.r, 0, Math.PI*2); ctx.stroke();
      ctx.restore();
    }

    // 9. Holographic reticle at cursor（弱化：只留淡环，不铺满文字区）
    if (mx > -900) {
      ctx.save(); ctx.translate(mx, my);
      ctx.globalAlpha = 0.34 * k;

      // Outer rotating tick ring
      ctx.rotate(t * 0.7);
      ctx.strokeStyle = 'rgba(120,168,220,0.55)'; ctx.lineWidth = 0.9;
      ctx.beginPath(); ctx.arc(0, 0, 20, 0, Math.PI * 2); ctx.stroke();
      for (let i = 0; i < 12; i++) {
        const a = (i / 12) * Math.PI * 2;
        const len = i % 3 === 0 ? 4.5 : 2;
        ctx.beginPath();
        ctx.moveTo(Math.cos(a) * 15, Math.sin(a) * 15);
        ctx.lineTo(Math.cos(a) * (15 + len), Math.sin(a) * (15 + len));
        ctx.stroke();
      }

      // Inner rotating dashed ring
      ctx.rotate(-t * 1.5);
      ctx.strokeStyle = 'rgba(150,150,220,0.45)'; ctx.lineWidth = 0.7;
      ctx.beginPath(); ctx.arc(0, 0, 12, 0.4, Math.PI * 2 - 0.4); ctx.stroke();
      ctx.restore();
    }

    // 10. Quantum Particle Burst drawing
    for (const p of particles) {
      if (p.shape === 'scifi-spark') {
        ctx.save();
        ctx.globalAlpha = p.alpha * p.life * k;
        ctx.fillStyle = p.color;
        ctx.shadowColor = p.color;
        ctx.shadowBlur = 4;

        ctx.translate(p.x, p.y);
        ctx.rotate(t * 2.0);
        ctx.fillRect(-p.size/2, -p.size/2, p.size, p.size);
        ctx.restore();
      }
    }
  } else if (props.mode === 'aurora') {
    drawAurora(ctx);
  } else if (props.mode === 'ink') {
    drawInk(ctx);
  } else if (props.mode === 'rain') {
    drawRain(ctx);
  } else if (props.mode === 'custom' && customImg) {
    ctx.globalAlpha = 0.18;
    const ir = customImg.width / customImg.height, cr = W / H;
    let dw: number, dh: number, dx: number, dy: number;
    if (ir > cr) { dh = H; dw = H * ir; dx = (W - dw) / 2; dy = 0; }
    else { dw = W; dh = W / ir; dx = 0; dy = (H - dh) / 2; }
    ctx.drawImage(customImg, dx, dy, dw, dh); ctx.globalAlpha = 1;
  }

  // 统一光场：所有预设背景共用，给画面明暗纵深（custom 图片不加，避免压暗用户照片）
  if (props.mode !== 'custom') {
    const k = strength();
    // 水墨要保住墨色深度：共享光场的 screen 提亮减半，否则黛墨被洗成脏灰
    drawLightField(ctx, props.mode === 'ink' ? k * 0.55 : k);
    drawGrain(ctx, k);
  }

  // Ripples (shared)
  for (const r of ripples) {
    ctx.save(); ctx.globalAlpha = r.alpha;
    ctx.strokeStyle = r.color + r.alpha.toFixed(2) + ')';
    ctx.lineWidth = r.lw;
    ctx.beginPath(); ctx.arc(r.x, r.y, r.r, 0, Math.PI*2); ctx.stroke();
    ctx.restore();
  }
}

function animate() {
  update();
  draw();
  animId = requestAnimationFrame(animate);
}

function resize() {
  const el = canvasRef.value?.parentElement;
  if (!el || !canvasRef.value) return;
  const dpr = window.devicePixelRatio || 1;
  W = el.clientWidth; H = el.clientHeight;
  canvasRef.value.width = W * dpr; canvasRef.value.height = H * dpr;
  canvasRef.value.style.width = W + 'px'; canvasRef.value.style.height = H + 'px';
  ctx = canvasRef.value.getContext('2d'); if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  initParticles();
}

function onMouseMove(x: number, y: number) {
  mx = x; my = y;
  if (props.mode === 'sakura') trail.push({ x, y, t: 1 });
  if (props.mode === 'custom') { if (Math.random() < 0.15) addRipple(x, y); }
  if (props.mode === 'aurora' && Math.random() < 0.12) {
    particles.push({
      x: x + rnd(-10, 10), y: y + rnd(-6, 6),
      vx: rnd(-0.15, 0.15), vy: rnd(-0.5, -0.15),
      size: rnd(1.2, 2.6), alpha: rnd(0.35, 0.7),
      life: 1, decay: rnd(0.012, 0.024), temp: true,
      color: Math.random() < 0.5 ? 'rgba(94,234,212,0.9)' : 'rgba(167,139,250,0.85)',
    });
  }
  if (props.mode === 'ink' && Math.random() < 0.08) { addRipple(x, y); }

}

function clearPointer() {
  mx = -999;
  my = -999;
  trail = [];
}

function onClick(x: number, y: number) {
  const mode = props.mode;
  if (mode === 'sakura') {
    addRipple(x, y, true);
    // Burst petals + hearts
    const shapes = ['heart','sparkle','star','heart','heart'];
    for (let i = 0; i < 22; i++) ctPetals.push({
      x, y, vx: rnd(-3, 3), vy: rnd(-4, -0.5),
      size: rnd(4, 10), angle: rnd(0, 6.28), spin: rnd(-0.08, 0.08),
      sway: 1, swayOff: rnd(0, 6.28),
      color: pick(['#d9bcc8','#c29cad','#d9bcc8','#eddfe5']),
      alpha: rnd(0.5, 0.8), temp: true, life: 1, decay: rnd(0.01, 0.02),
    });
    for (let i = 0; i < 12; i++) particles.push({
      x, y, vx: rnd(-3.5, 3.5), vy: rnd(-3.5, -0.5),
      size: rnd(4, 9), alpha: rnd(0.5, 0.8), color: pick(CUTE_C),
      shape: pick(shapes), angle: rnd(0, 6.28), spin: rnd(-0.08, 0.08),
      life: 1, decay: rnd(0.012, 0.022),
    });
  } else if (mode === 'scifi') {
    // EMP shockwaves - 3 rings
    for (let i = 0; i < 3; i++) sfEMPs.push({ x, y, r: i*10, spd: 3.5 - i*0.5, alpha: 0.8 - i*0.15, color: SF_COLS.cyan, ring: i });
    // Blast all nearby nodes
    for (const n of sfNodes) {
      const dx = n.x - x, dy = n.y - y, d = Math.sqrt(dx*dx+dy*dy);
      if (d < 160 && d > 0) { const f = (160-d)/160 * 5; n.vx += (dx/d)*f; n.vy += (dy/d)*f; n.energy = 1; }
    }
    // Flash nearby hexes
    for (const h of sfHexes) {
      const d = Math.sqrt((h.x-x)**2+(h.y-y)**2);
      if (d < 80) h.glow = 1;
    }
    // Quantum particle burst!
    for (let i = 0; i < 18; i++) {
      const angle = rnd(0, Math.PI * 2);
      const speed = rnd(2.0, 5.5);
      particles.push({
        x, y,
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed,
        size: rnd(1.5, 3.5),
        alpha: rnd(0.6, 0.9),
        color: pick(Object.values(SF_COLS)),
        shape: 'scifi-spark',
        life: 1.0,
        decay: rnd(0.015, 0.035),
        temp: true
      });
    }
  }
  else if (mode === 'minimal') {
    // Constellation ray burst: shoot lines to nearby dots
    for (const d of mnDots) {
      const dx = d.x - x, dy = d.y - y, dist = Math.sqrt(dx*dx+dy*dy);
      if (dist < 120 && dist > 5) {
        mnRays.push({ x, y, tx: d.x, ty: d.y, alpha: 0.7 });
        d.glow = 1;
      }
    }
  }
  else if (mode === 'custom') { addRipple(x, y, true); }
  else if (mode === 'aurora') {
    addRipple(x, y, true);
    // 被点亮的光带向四周溅出光点
    for (let i = 0; i < 16; i++) {
      const angle = rnd(0, Math.PI * 2);
      const speed = rnd(0.8, 3.2);
      particles.push({
        x, y,
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed - 0.4,
        size: rnd(1.2, 3),
        alpha: rnd(0.5, 0.9),
        life: 1, decay: rnd(0.014, 0.03), temp: true,
        color: Math.random() < 0.5 ? 'rgba(94,234,212,0.95)' : 'rgba(167,139,250,0.9)',
      });
    }
  }
  else if (mode === 'ink') {
    // 一滴墨落下：三层晕圈以不同速度扩散，像墨在宣纸上洇开
    ikRipples.push({ x, y, r: 2, alpha: 0.5, speed: 1.5 });
    ikRipples.push({ x, y, r: 0, alpha: 0.36, speed: 1.0 });
    ikRipples.push({ x, y, r: 0, alpha: 0.24, speed: 0.6 });
    // 墨点飞溅
    for (let i = 0; i < 14; i++) {
      const angle = rnd(0, Math.PI * 2);
      const speed = rnd(0.6, 2.6);
      particles.push({
        x, y,
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed,
        size: rnd(0.8, 2.4),
        alpha: rnd(0.3, 0.7),
        life: 1, decay: rnd(0.01, 0.022), temp: true,
      });
    }
    // 近处山脊被“点亮”一点墨色
    for (const ridge of ikRidges) {
      for (const p of ridge.points) {
        const dx = p.x - x, dy = p.y - y;
        if (dx * dx + dy * dy < 9000) ridge.alpha = Math.min(0.4, ridge.alpha + 0.012);
      }
    }
  }
  else if (mode === 'rain') {
    // 点击处溅起水花：润漪 + 向上弹起的小水珠
    for (let i = 0; i < 3; i++) {
      rnRipples.push({ x, y, r: i * 5, alpha: 0.46 - i * 0.1 });
    }
    for (let i = 0; i < 14; i++) {
      rnSplashes.push({
        x: x + rnd(-10, 10),
        y,
        r: rnd(1, 2.2),
        h: 0,
        vy: rnd(-2.2, -0.8),
        alpha: rnd(0.4, 0.7),
      });
    }
  }
}

defineExpose({ onMouseMove, onClick, clearPointer, resize });

watch(() => props.mode, () => { if (W > 0) initParticles(); });
watch(() => props.customImage, (src) => {
  if (!src) { customImg = null; return; }
  const img = new Image(); img.onload = () => { customImg = img; }; img.src = src;
});

onMounted(() => {
  resize();
  if (props.customImage) {
    const img = new Image(); img.onload = () => { customImg = img; }; img.src = props.customImage;
  }
  animId = requestAnimationFrame(animate);
  ro = new ResizeObserver(resize);
  if (canvasRef.value?.parentElement) ro.observe(canvasRef.value.parentElement);
});

onBeforeUnmount(() => {
  if (animId) cancelAnimationFrame(animId);
  ro?.disconnect();
});
</script>

<template>
  <canvas ref="canvasRef" class="bg-canvas" />
</template>

<style scoped>
.bg-canvas {
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 0;
}
</style>
