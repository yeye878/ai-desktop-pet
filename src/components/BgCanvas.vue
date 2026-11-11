<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from 'vue';
import { usePetStore } from '../stores/pet';

const props = defineProps<{ mode: string; customImage?: string; intensity?: number }>();
const petStore = usePetStore();
const canvasRef = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let W = 0, H = 0, animId = 0;
let mx = -999, my = -999;
let particles: any[] = [];
let ripples: any[] = [];
let trail: { x: number; y: number; t: number }[] = [];
let customImg: HTMLImageElement | null = null;
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
let ikRipples: any[] = [];
// Mist state（晨雾）
let msBands: any[] = [];
let msMotes: any[] = [];
// Rain state（细雨）
let rnDrops: any[] = [];
let rnStreaks: any[] = [];
let rnRipples: any[] = [];
// Dusk state（黄昏）
let dkGlow: any[] = [];
let dkDust: any[] = [];
// Bloom state（花影）
let blBranches: any[] = [];
let blPetals: any[] = [];
let blShadow: any[] = [];

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
  for (let i = 0; i < 35; i++) ctPetals.push({
    x: rnd(0, W), y: rnd(-H, H),
    vx: rnd(-0.5, 0.5), vy: rnd(0.4, 1.2),
    size: rnd(3, 8), angle: rnd(0, 6.28), spin: rnd(-0.04, 0.04),
    sway: rnd(0.5, 1.5), swayOff: rnd(0, 6.28),
    color: pick(['#d9bcc8','#d9bcc8','#c29cad','#eddfe5','#e6d2da','#f2e8e6']),
    alpha: rnd(0.3, 0.65), temp: false, life: 1, decay: 0,
  });
  // Floating pastel bubbles
  for (let i = 0; i < 18; i++) ctBubbles.push({
    x: rnd(0, W), y: rnd(0, H),
    vx: rnd(-0.2, 0.2), vy: rnd(-0.3, -0.08),
    r: rnd(5, 16), phase: rnd(0, 6.28), speed: rnd(0.8, 1.8),
    color: pick(CUTE_C), alpha: rnd(0.1, 0.25),
  });
}

function initMinimal() {
  mnDots = []; mnRays = [];
  const sp = 24;
  for (let gx = sp; gx < W; gx += sp)
    for (let gy = sp; gy < H; gy += sp)
      mnDots.push({ x: gx, y: gy, glow: 0, r: 1.5 });
}

/**
 * 极光：几条竖直光带在深色夜空里缓慢摆动，叠加一层极细的星点。
 * 光带用二次曲线画出上下摆动的轮廓，按列逐像素算出亮度，避免整块渐变显得死板。
 */
function initAurora() {
  auCurtains = []; auStars = [];
  // 浅色纸面上用「极淡的冷色」做 multiply 叠色：饱和度压到很低，
  // 否则在米白底色上会立刻变成一块彩色板。
  const palette = [
    { core: [176, 226, 214], edge: [186, 220, 240] },   // 淡青绿 → 淡天蓝
    { core: [196, 196, 240], edge: [208, 200, 244] },   // 淡靛蓝 → 淡紫
    { core: [180, 232, 226], edge: [196, 238, 232] },   // 淡薄荷
  ];
  const count = Math.max(2, Math.min(4, Math.round(W / 320)));
  for (let i = 0; i < count; i++) {
    const base = palette[i % palette.length];
    auCurtains.push({
      x: (i + 0.5) / count * W + rnd(-W * 0.06, W * 0.06),
      width: rnd(W * 0.24, W * 0.44),
      sway: rnd(W * 0.02, W * 0.06),
      swaySpeed: rnd(0.1, 0.24),
      phase: rnd(0, 6.28),
      drift: rnd(-0.035, 0.035),
      top: rnd(-H * 0.2, H * 0.1),
      bottom: rnd(H * 0.55, H * 0.95),
      core: base.core,
      edge: base.edge,
      alpha: rnd(0.22, 0.34),
    });
  }
  for (let i = 0; i < 70; i++) {
    auStars.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(0.4, 1.2), base: rnd(0.15, 0.5),
      twinkle: rnd(0.6, 1.8), phase: rnd(0, 6.28),
    });
  }
}

/**
 * 水墨：远景山脊 + 缓慢流动的雾带 + 点击扩散的墨晕。
 * 山脊用几条折线叠出层次，越远越淡，保证文字区域依然干净。
 */
function initInk() {
  ikRidges = []; ikMist = []; ikRipples = [];
  const layers = 3;
  for (let layer = 0; layer < layers; layer++) {
    const points: { x: number; y: number }[] = [];
    const step = Math.max(48, W / 22);
    const baseline = H * (0.62 + layer * 0.11);
    const amp = H * (0.1 - layer * 0.02);
    let seed = rnd(0, 100);
    for (let x = -step; x <= W + step; x += step) {
      seed += rnd(0.6, 1.8);
      const y =
        baseline -
        Math.sin(seed) * amp -
        Math.sin(seed * 0.5 + layer) * amp * 0.5;
      points.push({ x, y });
    }
    ikRidges.push({
      points,
      // 远山用淡墨，alpha 比原先再压低一档，让纸色主导
      alpha: 0.075 - layer * 0.017,
      tint: layer === 0 ? "120,128,140" : layer === 1 ? "104,112,126" : "92,100,114",
    });
  }
  for (let i = 0; i < 5; i++) {
    ikMist.push({
      x: rnd(-W * 0.1, W),
      y: rnd(H * 0.45, H * 0.95),
      w: rnd(W * 0.5, W * 0.95),
      h: rnd(H * 0.06, H * 0.14),
      drift: rnd(0.06, 0.22),
      phase: rnd(0, 6.28),
      alpha: rnd(0.05, 0.11),
    });
  }
}

/**
 * 晨雾：几层横向雾带由下往上缓慢抬升，叠一点几乎看不见的浮尘。
 * 不画具体形状，只做「明度层次」，所以放在任何内容后面都不抢。
 */
function initMist() {
  msBands = []; msMotes = [];
  const layers = 4;
  for (let i = 0; i < layers; i++) {
    msBands.push({
      y: H * (0.42 + i * 0.14),
      h: rnd(H * 0.16, H * 0.3),
      drift: rnd(-0.05, 0.05),
      rise: rnd(0.01, 0.05),
      phase: rnd(0, 6.28),
      alpha: rnd(0.05, 0.1) * (1 - i * 0.12),
    });
  }
  for (let i = 0; i < 40; i++) {
    msMotes.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(0.5, 1.6), vx: rnd(-0.06, 0.06), vy: rnd(-0.12, -0.02),
      alpha: rnd(0.05, 0.16), phase: rnd(0, 6.28),
    });
  }
}

/**
 * 细雨：斜向雨丝 + 玻璃上的水珠 + 落点涟漪。
 * 雨丝用细长线段而不是点，才有「斜雨」的方向感。
 */
function initRain() {
  rnDrops = []; rnStreaks = []; rnRipples = [];
  for (let i = 0; i < 70; i++) {
    rnStreaks.push({
      x: rnd(-W * 0.2, W), y: rnd(-H, H),
      len: rnd(10, 26), vx: rnd(0.5, 1.1), vy: rnd(3.2, 6),
      alpha: rnd(0.05, 0.16),
    });
  }
  for (let i = 0; i < 26; i++) {
    rnDrops.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(1.2, 3.2), drift: rnd(0, 0.16), wiggle: rnd(0, 6.28),
      alpha: rnd(0.06, 0.18),
    });
  }
}

/**
 * 黄昏：角落的暖橙余晖 + 淡紫补光 + 空气里的浮尘。
 * 暖色只压在底部两角，不铺满，避免整页发黄。
 */
function initDusk() {
  dkGlow = []; dkDust = [];
  dkGlow = [
    { x: W * 0.16, y: H * 0.92, r: Math.max(180, W * 0.42), drift: 0.03, phase: 0, rgb: '232, 190, 160', alpha: 0.18 },
    { x: W * 0.84, y: H * 0.18, r: Math.max(160, W * 0.38), drift: -0.025, phase: 2.1, rgb: '198, 186, 216', alpha: 0.15 },
    { x: W * 0.5, y: H * 0.55, r: Math.max(140, W * 0.32), drift: 0.015, phase: 4.2, rgb: '246, 226, 210', alpha: 0.1 },
  ];
  for (let i = 0; i < 34; i++) {
    dkDust.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(0.6, 2), vx: rnd(-0.08, 0.08), vy: rnd(-0.16, -0.03),
      alpha: rnd(0.06, 0.18), phase: rnd(0, 6.28),
    });
  }
}

/**
 * 花影：窗外的花枝在纸上的投影 + 缓慢飘落的花瓣。
 * 影子用低对比的灰粉，花瓣用极淡的暖粉，整体压在 15% 以内。
 */
function initBloom() {
  blBranches = []; blPetals = []; blShadow = [];
  for (let i = 0; i < 3; i++) {
    const points: { x: number; y: number }[] = [];
    let x = W * (0.1 + i * 0.3) + rnd(-W * 0.06, W * 0.06);
    let y = -20;
    const stepY = H / 8;
    while (y < H * (0.55 + i * 0.1)) {
      points.push({ x, y });
      x += rnd(-W * 0.05, W * 0.05);
      y += stepY * rnd(0.7, 1.2);
    }
    blBranches.push({ points, width: rnd(1.2, 2.4), alpha: 0.05 + i * 0.012 });
  }
  for (let i = 0; i < 22; i++) {
    blShadow.push({
      x: rnd(0, W), y: rnd(0, H),
      r: rnd(3, 9), alpha: rnd(0.03, 0.08), phase: rnd(0, 6.28), sway: rnd(0.2, 0.6),
    });
  }
  for (let i = 0; i < 26; i++) {
    blPetals.push({
      x: rnd(0, W), y: rnd(-H, H),
      vx: rnd(-0.24, 0.24), vy: rnd(0.3, 0.85),
      size: rnd(2.5, 6), angle: rnd(0, 6.28), spin: rnd(-0.03, 0.03),
      sway: rnd(0.4, 1.2), swayOff: rnd(0, 6.28),
      alpha: rnd(0.12, 0.3),
    });
  }
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

function initParticles() {
  particles = []; ripples = []; trail = [];
  if (props.mode === 'cute') initCute();
  else if (props.mode === 'scifi') initScifi();
  else if (props.mode === 'minimal') initMinimal();
  else if (props.mode === 'aurora') initAurora();
  else if (props.mode === 'ink') initInk();
  else if (props.mode === 'mist') initMist();
  else if (props.mode === 'rain') initRain();
  else if (props.mode === 'dusk') initDusk();
  else if (props.mode === 'bloom') initBloom();
}

function addRipple(x: number, y: number, big = false) {
  const color =
    props.mode === 'scifi' ? 'rgba(56,189,248,' :
    props.mode === 'cute' ? 'rgba(255,107,157,' :
    props.mode === 'aurora' ? 'rgba(94,234,212,' :
    props.mode === 'ink' ? 'rgba(54,62,74,' :
    props.mode === 'rain' ? 'rgba(150,180,192,' :
    props.mode === 'bloom' ? 'rgba(211,182,189,' :
    props.mode === 'dusk' ? 'rgba(212,178,155,' :
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

  if (props.mode === 'cute') {
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
    }
    for (let i = ikRipples.length - 1; i >= 0; i--) {
      const r = ikRipples[i];
      r.r += 1.1; r.alpha *= 0.972;
      if (r.alpha < 0.006) ikRipples.splice(i, 1);
    }
    for (const p of particles) { p.life -= p.decay; p.x += p.vx; p.y += p.vy; p.vx *= 0.97; p.vy *= 0.97; }
    particles = particles.filter(p => p.life > 0);
  } else if (props.mode === 'mist') {
    for (const b of msBands) {
      b.phase += 0.004;
      b.y -= b.rise;
      b.x = (b.x ?? 0) + b.drift;
      if (b.y + b.h < 0) b.y = H + rnd(0, H * 0.1);
    }
    for (const m of msMotes) {
      m.phase += 0.01;
      m.x += m.vx; m.y += m.vy;
      if (m.y < -4) { m.y = H + 4; m.x = rnd(0, W); }
      if (m.x < -4) m.x = W + 4; if (m.x > W + 4) m.x = -4;
    }
  } else if (props.mode === 'rain') {
    for (const s of rnStreaks) {
      s.x += s.vx; s.y += s.vy;
      if (s.y > H + s.len) { s.y = rnd(-H * 0.4, -10); s.x = rnd(-W * 0.2, W); }
      if (s.x > W + 20) s.x = -20;
    }
    for (const d of rnDrops) {
      d.wiggle += 0.02;
      d.y += d.drift;
      d.x += Math.sin(d.wiggle) * 0.06;
      if (d.y > H + 6) { d.y = -6; d.x = rnd(0, W); }
    }
    for (let i = rnRipples.length - 1; i >= 0; i--) {
      const r = rnRipples[i];
      r.r += 0.5; r.alpha *= 0.96;
      if (r.alpha < 0.01) rnRipples.splice(i, 1);
    }
  } else if (props.mode === 'dusk') {
    for (const g of dkGlow) {
      g.phase += 0.006;
      g.x += Math.sin(g.phase) * g.drift;
      // 点击产生的余晖会慢慢淡掉并回收，不然越点越亮
      if (g.burst) g.alpha *= 0.992;
    }
    dkGlow = dkGlow.filter((g) => !g.burst || g.alpha > 0.01);
    for (const d of dkDust) {
      d.phase += 0.012;
      d.x += d.vx; d.y += d.vy;
      if (d.y < -4) { d.y = H + 4; d.x = rnd(0, W); }
      if (d.x < -4) d.x = W + 4; if (d.x > W + 4) d.x = -4;
    }
  } else if (props.mode === 'bloom') {
    for (const p of blPetals) {
      p.x += p.vx + Math.sin(p.phase += 0.01) * p.sway * 0.4;
      p.y += p.vy; p.angle += p.spin;
      if (p.y > H + 12) { p.y = -12; p.x = rnd(0, W); }
      if (p.x < -12) p.x = W + 12; if (p.x > W + 12) p.x = -12;
    }
    for (const s of blShadow) s.phase += 0.008;
  }
  particles = particles.filter(p => !p.temp || p.life > 0);
}

function drawAurora(ctx: CanvasRenderingContext2D) {
  const k = strength();
  // 1. 纸面底色：不再是夜空，而是比控制台纸面略偏冷的浅色，边界自然消失
  const bg = ctx.createLinearGradient(0, 0, W * 0.4, H);
  bg.addColorStop(0, '#f6fbfa');
  bg.addColorStop(0.5, '#f3f6fb');
  bg.addColorStop(1, '#f8f5fb');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  // 2. 极光光晕：用柔和的径向光斑代替硬边光带，边缘到 0 收尾，避免出现“色块边”
  ctx.save();
  ctx.globalCompositeOperation = 'multiply';
  for (const c of auCurtains) {
    const sway = Math.sin(c.phase) * c.sway;
    const topY = c.top + Math.sin(c.phase * 0.7) * H * 0.06;
    const botY = c.bottom + Math.cos(c.phase * 0.6) * H * 0.05;
    const css = `${c.core[0]},${c.core[1]},${c.core[2]}`;
    const cssEdge = `${c.edge[0]},${c.edge[1]},${c.edge[2]}`;
    // 一条竖向的柔光柱：横向用三段径向叠出中轴亮、两侧散开
    const steps = 7;
    for (let i = 0; i < steps; i++) {
      const t = i / (steps - 1);
      const lateral = 1 - Math.abs(t - 0.5) * 2;
      const curvature = Math.sin(t * Math.PI) * sway;
      const x = c.x + (t - 0.5) * c.width + curvature;
      const rx = c.width / steps * 1.8;
      const peak = c.alpha * lateral * lateral * k * 1.05;
      if (peak <= 0.002) continue;
      const grad = ctx.createRadialGradient(x, (topY + botY) / 2, 0, x, (topY + botY) / 2, Math.max(rx, (botY - topY) * 0.7));
      grad.addColorStop(0, `rgba(${css},${peak.toFixed(3)})`);
      grad.addColorStop(0.55, `rgba(${cssEdge},${(peak * 0.42).toFixed(3)})`);
      grad.addColorStop(1, `rgba(${cssEdge},0)`);
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.ellipse(x, (topY + botY) / 2, rx, (botY - topY) * 0.62, 0, 0, Math.PI * 2);
      ctx.fill();
    }
  }
  ctx.restore();

  // 3. 星点：改成纸面上的细闪，亮度很低，不抢文字
  for (const s of auStars) {
    const twinkle = 0.55 + Math.sin(s.phase) * 0.45;
    ctx.save();
    ctx.globalAlpha = Math.min(0.32, Math.max(0.02, s.base * twinkle * k * 0.5));
    ctx.fillStyle = '#8fb8c9';
    ctx.beginPath();
    ctx.arc(s.x, s.y, s.r * 0.9, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 4. 点击产生的光点
  for (const p of particles) {
    ctx.save();
    ctx.globalAlpha = p.alpha * Math.max(0, p.life) * k;
    ctx.fillStyle = p.color;
    ctx.shadowColor = p.color;
    ctx.shadowBlur = 6;
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

function drawInk(ctx: CanvasRenderingContext2D) {
  const k = strength();
  // 1. 宣纸底：与聊天区原本的纸色一致，衔接不突兀
  const paper = ctx.createLinearGradient(0, 0, W * 0.2, H);
  paper.addColorStop(0, '#faf7f0');
  paper.addColorStop(0.55, '#f6f2e9');
  paper.addColorStop(1, '#f1ece2');
  ctx.fillStyle = paper;
  ctx.fillRect(0, 0, W, H);

  // 2. 雾带（先画，压在山脊下面，营造纵深）
  ctx.save();
  for (const m of ikMist) {
    const y = m.y + Math.sin(m.phase) * H * 0.012;
    const grad = ctx.createRadialGradient(m.x, y, 0, m.x, y, m.w * 0.55);
    grad.addColorStop(0, `rgba(255,255,255,${(m.alpha * 1.5 * k).toFixed(3)})`);
    grad.addColorStop(0.6, `rgba(252,249,242,${(m.alpha * 0.65 * k).toFixed(3)})`);
    grad.addColorStop(1, 'rgba(255,255,255,0)');
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.ellipse(m.x, y, m.w * 0.55, m.h, 0, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();

  // 3. 远山：三层折线，越远越淡，山脚用渐变收进纸色（浅墨，不压文字）
  for (let i = ikRidges.length - 1; i >= 0; i--) {
    const ridge = ikRidges[i];
    const pts = ridge.points;
    if (!pts.length) continue;
    ctx.beginPath();
    ctx.moveTo(pts[0].x, pts[0].y);
    for (let k2 = 1; k2 < pts.length - 1; k2++) {
      const midX = (pts[k2].x + pts[k2 + 1].x) / 2;
      const midY = (pts[k2].y + pts[k2 + 1].y) / 2;
      ctx.quadraticCurveTo(pts[k2].x, pts[k2].y, midX, midY);
    }
    ctx.lineTo(pts[pts.length - 1].x, pts[pts.length - 1].y);
    ctx.lineTo(pts[pts.length - 1].x, H);
    ctx.lineTo(pts[0].x, H);
    ctx.closePath();
    const fill = ctx.createLinearGradient(0, H * 0.4, 0, H);
    fill.addColorStop(0, `rgba(${ridge.tint},${(ridge.alpha * k * 1.1).toFixed(3)})`);
    fill.addColorStop(0.7, `rgba(${ridge.tint},${(ridge.alpha * k * 0.62).toFixed(3)})`);
    fill.addColorStop(1, `rgba(${ridge.tint},0)`);
    ctx.fillStyle = fill;
    ctx.fill();
  }

  // 4. 墨晕（点击）
  for (const r of ikRipples) {
    ctx.save();
    ctx.globalAlpha = r.alpha * k;
    ctx.strokeStyle = `rgba(96,104,116,${(r.alpha * 0.7).toFixed(2)})`;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(r.x, r.y, r.r, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }

  // 5. 墨点飞溅
  for (const p of particles) {
    ctx.save();
    ctx.globalAlpha = p.alpha * Math.max(0, p.life) * k * 0.8;
    ctx.fillStyle = 'rgba(84,92,104,0.7)';
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size * 0.85, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 6. 共享涟漪（鼠标经过）
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

/** 晨雾：底部厚、顶部薄的一层层雾带，配极细浮尘 */
function drawMist(ctx: CanvasRenderingContext2D) {
  const k = strength();
  const bg = ctx.createLinearGradient(0, 0, 0, H);
  bg.addColorStop(0, '#fbfdfc');
  bg.addColorStop(0.42, '#eef5f4');
  bg.addColorStop(0.72, '#e3edec');
  bg.addColorStop(1, '#d8e5e4');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  for (const b of msBands) {
    const y = b.y + Math.sin(b.phase) * H * 0.02;
    const grad = ctx.createLinearGradient(0, y, 0, y + b.h);
    grad.addColorStop(0, `rgba(255,255,255,0)`);
    grad.addColorStop(0.4, `rgba(212,228,227,${(b.alpha * k * 3.2).toFixed(3)})`);
    grad.addColorStop(0.75, `rgba(184,206,205,${(b.alpha * k * 3.6).toFixed(3)})`);
    grad.addColorStop(1, `rgba(166,192,192,${(b.alpha * k * 2.4).toFixed(3)})`);
    ctx.fillStyle = grad;
    ctx.fillRect(0, y, W, b.h);
  }

  for (const m of msMotes) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.5, m.alpha * k * 2.4 * (0.6 + Math.sin(m.phase) * 0.4));
    ctx.fillStyle = '#ffffff';
    ctx.beginPath();
    ctx.arc(m.x, m.y, m.r, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

/** 细雨：斜雨丝 + 玻璃水珠 + 落点涟漪 */
function drawRain(ctx: CanvasRenderingContext2D) {
  const k = strength();
  const bg = ctx.createLinearGradient(0, 0, W * 0.3, H);
  bg.addColorStop(0, '#f9fcfd');
  bg.addColorStop(0.5, '#eaf2f5');
  bg.addColorStop(1, '#dbe7ec');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  ctx.lineCap = 'round';
  for (const s of rnStreaks) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.45, s.alpha * k * 2.6);
    ctx.strokeStyle = '#93b0bd';
    ctx.lineWidth = 1.1;
    ctx.beginPath();
    ctx.moveTo(s.x, s.y);
    ctx.lineTo(s.x - s.vx * 2.2, s.y - s.len);
    ctx.stroke();
    ctx.restore();
  }

  for (const d of rnDrops) {
    ctx.save();
    ctx.globalAlpha = d.alpha * k;
    const g = ctx.createRadialGradient(d.x - d.r * 0.3, d.y - d.r * 0.3, 0, d.x, d.y, d.r);
    g.addColorStop(0, 'rgba(255,255,255,0.95)');
    g.addColorStop(0.7, 'rgba(186,210,220,0.55)');
    g.addColorStop(1, 'rgba(150,180,196,0.45)');
    ctx.fillStyle = g;
    ctx.beginPath();
    ctx.ellipse(d.x, d.y, d.r, d.r * 1.25, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  for (const r of rnRipples) {
    ctx.save();
    ctx.globalAlpha = r.alpha * k;
    ctx.strokeStyle = 'rgba(150,180,192,0.7)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.ellipse(r.x, r.y, r.r, r.r * 0.36, 0, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }
}

/** 黄昏：底部两角暖橙 + 顶部一角淡紫，中间保持纸色 */
function drawDusk(ctx: CanvasRenderingContext2D) {
  const k = strength();
  const bg = ctx.createLinearGradient(0, 0, W * 0.2, H);
  bg.addColorStop(0, '#fdf9f4');
  bg.addColorStop(0.4, '#f9f0e8');
  bg.addColorStop(0.72, '#f4e7dd');
  bg.addColorStop(1, '#eee0d6');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  ctx.save();
  ctx.globalCompositeOperation = 'multiply';
  for (const g of dkGlow) {
    const grad = ctx.createRadialGradient(g.x, g.y, 0, g.x, g.y, g.r);
    grad.addColorStop(0, `rgba(${g.rgb},${Math.min(0.62, g.alpha * k * 2.6).toFixed(3)})`);
    grad.addColorStop(0.42, `rgba(${g.rgb},${Math.min(0.4, g.alpha * k * 1.5).toFixed(3)})`);
    grad.addColorStop(0.72, `rgba(${g.rgb},${Math.min(0.18, g.alpha * k * 0.7).toFixed(3)})`);
    grad.addColorStop(1, `rgba(${g.rgb},0)`);
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(g.x, g.y, g.r, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();

  for (const d of dkDust) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.55, d.alpha * k * 2.6 * (0.6 + Math.sin(d.phase) * 0.4));
    ctx.fillStyle = '#fffdf8';
    ctx.beginPath();
    ctx.arc(d.x, d.y, d.r, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

/** 花影：花枝投影 + 缓慢飘落的花瓣 */
function drawBloom(ctx: CanvasRenderingContext2D) {
  const k = strength();
  const bg = ctx.createLinearGradient(0, 0, W * 0.25, H);
  bg.addColorStop(0, '#fdfaf9');
  bg.addColorStop(0.45, '#f9f1f1');
  bg.addColorStop(0.78, '#f4e9e9');
  bg.addColorStop(1, '#efe2e2');
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, W, H);

  // 花枝投影
  for (const b of blBranches) {
    if (b.points.length < 2) continue;
    ctx.save();
    ctx.globalAlpha = Math.min(0.34, b.alpha * k * 3.4);
    ctx.strokeStyle = '#7d6a68';
    ctx.lineWidth = b.width * 1.3;
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(b.points[0].x, b.points[0].y);
    for (let i = 1; i < b.points.length - 1; i++) {
      const mx = (b.points[i].x + b.points[i + 1].x) / 2;
      const my = (b.points[i].y + b.points[i + 1].y) / 2;
      ctx.quadraticCurveTo(b.points[i].x, b.points[i].y, mx, my);
    }
    ctx.stroke();
    ctx.restore();
  }

  // 花影
  for (const s of blShadow) {
    ctx.save();
    ctx.globalAlpha = s.alpha * k * (0.7 + Math.sin(s.phase) * 0.3);
    const grad = ctx.createRadialGradient(s.x, s.y, 0, s.x, s.y, s.r);
    grad.addColorStop(0, 'rgba(198,160,170,0.42)');
    grad.addColorStop(0.55, 'rgba(211,182,189,0.2)');
    grad.addColorStop(1, 'rgba(211,182,189,0)');
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.ellipse(s.x + Math.sin(s.phase) * 6, s.y, s.r * 1.4, s.r, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 飘落花瓣
  for (const p of blPetals) {
    ctx.save();
    ctx.globalAlpha = Math.min(0.62, p.alpha * k * 2.2);
    ctx.translate(p.x, p.y);
    ctx.rotate(p.angle);
    ctx.fillStyle = '#d9b3bc';
    ctx.beginPath();
    ctx.ellipse(0, 0, p.size, p.size * 0.52, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

function draw() {
  if (!ctx) return;
  ctx.clearRect(0, 0, W, H);

  // Background fills
  if (props.mode === 'cute') {
    // Background gradient
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0,'rgba(255,220,230,0.18)'); g.addColorStop(0.5,'rgba(255,200,215,0.12)'); g.addColorStop(1,'rgba(250,230,248,0.16)');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // Bubbles
    for (const b of ctBubbles) {
      ctx.save(); ctx.globalAlpha = b.alpha;
      const bg = ctx.createRadialGradient(b.x - b.r*0.3, b.y - b.r*0.3, 0, b.x, b.y, b.r);
      bg.addColorStop(0,'rgba(255,255,255,0.6)'); bg.addColorStop(1, b.color + '22');
      ctx.fillStyle = bg;
      ctx.beginPath(); ctx.arc(b.x, b.y, b.r, 0, Math.PI*2); ctx.fill();
      ctx.strokeStyle = b.color + '55'; ctx.lineWidth = 0.8; ctx.stroke();
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
      ctx.strokeStyle = 'rgba(255,255,255,0.45)'; ctx.lineWidth = 0.5;
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
    g.addColorStop(0,'rgba(248,250,252,0.2)'); g.addColorStop(1,'rgba(226,232,240,0.12)');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    // Draw connection lines between nearby glowing dots
    ctx.lineWidth = 0.8;
    for (let i = 0; i < mnDots.length; i++) {
      const a = mnDots[i]; if (a.glow < 0.05) continue;
      for (let j = i + 1; j < mnDots.length; j++) {
        const b = mnDots[j]; if (b.glow < 0.05) continue;
        const dx = a.x - b.x, dy = a.y - b.y, d = Math.sqrt(dx*dx+dy*dy);
        if (d < 50) {
          ctx.strokeStyle = `rgba(148,163,184,${Math.min(a.glow,b.glow)*0.5})`;
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
      if (h.glow > 0.01) {
        ctx.strokeStyle = `rgba(120,168,214,${(h.glow * 0.22 * k).toFixed(3)})`;
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
  } else if (props.mode === 'minimal') {
    const g = ctx.createLinearGradient(0, 0, W, H);
    g.addColorStop(0, 'rgba(241,245,249,0.12)'); g.addColorStop(1, 'rgba(226,232,240,0.08)');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
  } else if (props.mode === 'aurora') {
    drawAurora(ctx);
  } else if (props.mode === 'ink') {
    drawInk(ctx);
  } else if (props.mode === 'mist') {
    drawMist(ctx);
  } else if (props.mode === 'rain') {
    drawRain(ctx);
  } else if (props.mode === 'dusk') {
    drawDusk(ctx);
  } else if (props.mode === 'bloom') {
    drawBloom(ctx);
  } else if (props.mode === 'custom' && customImg) {
    ctx.globalAlpha = 0.18;
    const ir = customImg.width / customImg.height, cr = W / H;
    let dw: number, dh: number, dx: number, dy: number;
    if (ir > cr) { dh = H; dw = H * ir; dx = (W - dw) / 2; dy = 0; }
    else { dw = W; dh = W / ir; dx = 0; dy = (H - dh) / 2; }
    ctx.drawImage(customImg, dx, dy, dw, dh); ctx.globalAlpha = 1;
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

function animate() { update(); draw(); animId = requestAnimationFrame(animate); }

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
  if (props.mode === 'cute') trail.push({ x, y, t: 1 });
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
  if (props.mode === 'dusk' && Math.random() < 0.1) {
    particles.push({
      x: x + rnd(-8, 8), y: y + rnd(-5, 5),
      vx: rnd(-0.1, 0.1), vy: rnd(-0.2, -0.05),
      size: rnd(0.8, 1.8), alpha: rnd(0.2, 0.45),
      life: 1, decay: rnd(0.015, 0.03), temp: true,
    });
  }
  if (props.mode === 'bloom' && Math.random() < 0.07) {
    blShadow.forEach((s) => {
      const dx = s.x - x, dy = s.y - y;
      if (dx * dx + dy * dy < 12000) s.phase += 0.4;
    });
  }
}

function clearPointer() {
  mx = -999;
  my = -999;
  trail = [];
}

function onClick(x: number, y: number) {
  const mode = props.mode;
  if (mode === 'cute') {
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
    ikRipples.push({ x, y, r: 2, alpha: 0.5 });
    ikRipples.push({ x, y, r: 0, alpha: 0.34 });
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
        if (dx * dx + dy * dy < 9000) ridge.alpha = Math.min(0.24, ridge.alpha + 0.012);
      }
    }
  }
  else if (mode === 'mist') {
    // 点击处雾被推开：把附近的浮尘往外推
    for (const m of msMotes) {
      const dx = m.x - x, dy = m.y - y, d = Math.sqrt(dx * dx + dy * dy);
      if (d < 120 && d > 1) { m.vx += (dx / d) * 0.5; m.vy += (dy / d) * 0.5; }
    }
    addRipple(x, y, true);
  }
  else if (mode === 'rain') {
    // 点击处溅起一圈水花
    for (let i = 0; i < 3; i++) rnRipples.push({ x, y, r: i * 6, alpha: 0.5 - i * 0.12 });
    for (let i = 0; i < 10; i++) {
      const angle = rnd(0, Math.PI * 2);
      const speed = rnd(0.6, 2.2);
      particles.push({
        x, y,
        vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed - 0.6,
        size: rnd(0.8, 1.8), alpha: rnd(0.3, 0.6),
        life: 1, decay: rnd(0.02, 0.04), temp: true,
      });
    }
  }
  else if (mode === 'dusk') {
    // 点击处余晖亮一下
    dkGlow.push({ x, y, r: 90, drift: 0, phase: 0, rgb: '232, 190, 160', alpha: 0.26, burst: true });
    for (let i = 0; i < 12; i++) {
      const angle = rnd(0, Math.PI * 2);
      const speed = rnd(0.4, 1.8);
      particles.push({
        x, y,
        vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed - 0.3,
        size: rnd(0.8, 2), alpha: rnd(0.25, 0.55),
        life: 1, decay: rnd(0.012, 0.026), temp: true,
      });
    }
  }
  else if (mode === 'bloom') {
    // 点击处花瓣散开
    for (let i = 0; i < 10; i++) {
      blPetals.push({
        x, y,
        vx: rnd(-1.6, 1.6), vy: rnd(-1.8, -0.2),
        size: rnd(2.5, 5.5), angle: rnd(0, 6.28), spin: rnd(-0.06, 0.06),
        sway: 1, swayOff: rnd(0, 6.28),
        alpha: rnd(0.3, 0.6),
      });
    }
    addRipple(x, y, true);
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
