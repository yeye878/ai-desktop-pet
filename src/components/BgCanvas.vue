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
  else if (props.mode === 'rain') initRain();
}

function addRipple(x: number, y: number, big = false) {
  const color =
    props.mode === 'scifi' ? 'rgba(56,189,248,' :
    props.mode === 'cute' ? 'rgba(255,107,157,' :
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
