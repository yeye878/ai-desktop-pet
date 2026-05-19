<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from 'vue';
import { usePetStore } from '../stores/pet';

const props = defineProps<{ mode: string; customImage?: string }>();
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

const CUTE_C = ['#ff6b9d','#ff85b1','#ffaed7','#ffd700','#c084fc','#f9a8d4','#fdba74','#fb7185'];
const SF_COLS = { cyan:'#38bdf8', purple:'#a78bfa', green:'#34d399', blue:'#60a5fa', teal:'#2dd4bf' };

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
    color: pick(['#ffc0cb','#ffb7c5','#ff90a0','#ffd9e8','#ffc5d6','#ffe4e1']),
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
  // Floating space nebulas
  sfNebulae = [
    { x: W * 0.25, y: H * 0.25, vx: rnd(0.04, 0.12), vy: rnd(0.03, 0.08), r: Math.max(140, W * 0.35), color1: 'rgba(56, 189, 248, 0.075)', color2: 'rgba(56, 189, 248, 0)' },
    { x: W * 0.75, y: H * 0.7, vx: rnd(-0.12, -0.04), vy: rnd(-0.08, -0.03), r: Math.max(160, W * 0.42), color1: 'rgba(167, 139, 250, 0.07)', color2: 'rgba(167, 139, 250, 0)' },
    { x: W * 0.5, y: H * 0.45, vx: rnd(0.02, 0.08), vy: rnd(-0.06, -0.02), r: Math.max(110, W * 0.3), color1: 'rgba(45, 212, 191, 0.06)', color2: 'rgba(45, 212, 191, 0)' }
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
}

function addRipple(x: number, y: number, big = false) {
  const color = props.mode === 'scifi' ? 'rgba(56,189,248,' : props.mode === 'cute' ? 'rgba(255,107,157,' : 'rgba(148,163,184,';
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
  }
  particles = particles.filter(p => !p.temp || p.life > 0);
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
      ctx.save(); ctx.globalAlpha = t.t * 0.5; ctx.fillStyle = '#ffd700';
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
      ctx.strokeStyle = '#94a3b8'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.moveTo(ray.x, ray.y); ctx.lineTo(ray.tx, ray.ty); ctx.stroke();
      ctx.restore();
    }
    // Dots
    for (const d of mnDots) {
      const alpha = 0.12 + d.glow * 0.7;
      const size = 1.5 + d.glow * 3;
      ctx.save(); ctx.globalAlpha = alpha;
      ctx.fillStyle = d.glow > 0.3 ? '#818cf8' : '#94a3b8';
      if (d.glow > 0.2) { ctx.shadowColor = '#818cf8'; ctx.shadowBlur = 6 * d.glow; }
      ctx.beginPath(); ctx.arc(d.x, d.y, size, 0, Math.PI*2); ctx.fill();
      ctx.shadowBlur = 0; ctx.restore();
    }
    // Burst particles (click)
    for (const p of particles) {
      ctx.save(); ctx.globalAlpha = p.alpha * p.life;
      ctx.strokeStyle = '#94a3b8'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.arc(p.x, p.y, 1.5, 0, Math.PI*2); ctx.fill();
      ctx.restore();
    }
  } else if (props.mode === 'scifi') {
    const t = performance.now() / 1000;
    const petState = petStore.state;

    // 1. Deep space backdrop
    const g = ctx.createLinearGradient(0, 0, 0, H);
    g.addColorStop(0, '#040618'); g.addColorStop(0.5, '#070a24'); g.addColorStop(1, '#030514');
    ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
    
    // 2. Glowing Dynamic space nebulas
    ctx.save();
    ctx.globalCompositeOperation = 'screen';
    for (const neb of sfNebulae) {
      const ng = ctx.createRadialGradient(neb.x, neb.y, 0, neb.x, neb.y, neb.r);
      ng.addColorStop(0, neb.color1);
      ng.addColorStop(1, neb.color2);
      ctx.fillStyle = ng;
      ctx.beginPath(); ctx.arc(neb.x, neb.y, neb.r, 0, Math.PI*2); ctx.fill();
    }
    ctx.restore();

    // 3. Hex Grid
    ctx.lineWidth = 1;
    for (const h of sfHexes) {
      if (h.glow > 0.01) {
        ctx.strokeStyle = `rgba(56,189,248,${h.glow * 0.16})`;
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
    
    // 4. Scanline
    const sg = ctx.createLinearGradient(0, sfScanY - 50, 0, sfScanY);
    sg.addColorStop(0, 'rgba(56,189,248,0)'); sg.addColorStop(1, 'rgba(56,189,248,0.065)');
    ctx.fillStyle = sg; ctx.fillRect(0, sfScanY - 50, W, 50);
    ctx.fillStyle = 'rgba(56,189,248,0.22)'; ctx.fillRect(0, sfScanY, W, 1);

    // 5. Neural Connections (glitchy/pulsing when thinking)
    ctx.lineWidth = 1.2;
    for (let i = 0; i < sfNodes.length; i++) {
      for (let j = i + 1; j < sfNodes.length; j++) {
        const a = sfNodes[i], b = sfNodes[j], dx = a.x - b.x, dy = a.y - b.y, d = Math.sqrt(dx*dx+dy*dy);
        if (d < 240) {
          const e = Math.max(a.energy, b.energy);
          let alpha = (1 - d/240) * (0.12 + e * 0.45);
          
          if (petState === 'thinking') {
            // High-frequency transmission pulse
            alpha *= (0.68 + Math.sin(t * 15.0 + d * 0.05) * 0.32);
          } else if (petState === 'speaking') {
            // Wave propagation pulse
            alpha *= (0.8 + Math.cos(t * 6.0 - d * 0.02) * 0.2);
          }
          
          ctx.strokeStyle = `rgba(167,139,250,${alpha})`;
          ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
        }
      }
    }

    // Quantum capture threads from cursor to nearby nodes
    if (mx > -900) {
      for (const n of sfNodes) {
        const dx = n.x - mx, dy = n.y - my, dist = Math.sqrt(dx*dx+dy*dy);
        if (dist < 110) {
          const strength = (110 - dist) / 110;
          ctx.strokeStyle = `rgba(56, 189, 248, ${strength * 0.45})`;
          ctx.lineWidth = 0.6 + strength * 0.8;
          ctx.beginPath();
          ctx.moveTo(mx, my);
          ctx.lineTo(n.x, n.y);
          ctx.stroke();

          // Technical micro-distance tag at midpoint of thread
          if (strength > 0.6) {
            ctx.fillStyle = `rgba(56, 189, 248, ${strength * 0.6})`;
            ctx.font = '6px monospace';
            const midX = mx + dx * 0.5;
            const midY = my + dy * 0.5;
            ctx.fillText(`${Math.round(dist)}pm`, midX + 5, midY - 2);
          }
        }
      }
    }

    // 6. Nodes (Hubs & standard nodes)
    for (const n of sfNodes) {
      ctx.fillStyle = n.color;
      ctx.shadowColor = n.color; 
      ctx.shadowBlur = 8 + n.energy * 18;
      ctx.beginPath(); ctx.arc(n.x, n.y, n.r + n.energy * 2.2, 0, Math.PI*2); ctx.fill();
      ctx.shadowBlur = 0;
      
      if (n.type === 'hub') {
        ctx.strokeStyle = n.color; ctx.lineWidth = 1;
        ctx.beginPath(); ctx.arc(n.x, n.y, n.r + 6.5 + Math.sin(n.pulse)*2.5, 0, Math.PI*2); ctx.stroke();
      }
    }

    // 7. Packets
    for (const p of sfPackets) {
      ctx.fillStyle = p.color; ctx.shadowColor = p.color; ctx.shadowBlur = 8;
      ctx.beginPath(); ctx.arc(p.x, p.y, p.r, 0, Math.PI*2); ctx.fill();
      ctx.shadowBlur = 0;
    }

    // 8. EMPs
    for (const e of sfEMPs) {
      ctx.strokeStyle = e.color; ctx.lineWidth = 1.5 + e.alpha * 3;
      ctx.globalAlpha = e.alpha;
      ctx.beginPath(); ctx.arc(e.x, e.y, e.r, 0, Math.PI*2); ctx.stroke();
      ctx.globalAlpha = 1.0;
    }

    // 9. Holographic reticle at cursor
    if (mx > -900) {
      ctx.save(); ctx.translate(mx, my);
      
      // Outer rotating tick ring
      ctx.rotate(t * 0.7);
      ctx.strokeStyle = 'rgba(56,189,248,0.55)'; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.arc(0, 0, 22, 0, Math.PI * 2); ctx.stroke();
      for (let i = 0; i < 12; i++) {
        const a = (i / 12) * Math.PI * 2;
        const len = i % 3 === 0 ? 5.5 : 2.5;
        ctx.beginPath();
        ctx.moveTo(Math.cos(a) * 17, Math.sin(a) * 17);
        ctx.lineTo(Math.cos(a) * (17 + len), Math.sin(a) * (17 + len));
        ctx.stroke();
      }
      
      // Inner rotating dashed ring
      ctx.rotate(-t * 1.5); 
      ctx.strokeStyle = 'rgba(167,139,250,0.45)'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.arc(0, 0, 14, 0.4, Math.PI * 2 - 0.4); ctx.stroke();
      
      ctx.restore();
      
      // Crosshair lines
      ctx.save(); ctx.translate(mx, my);
      ctx.strokeStyle = 'rgba(56,189,248,0.38)'; ctx.lineWidth = 0.8;
      ctx.beginPath(); ctx.moveTo(-28, 0); ctx.lineTo(-6, 0); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(6, 0); ctx.lineTo(28, 0); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(0, -28); ctx.lineTo(0, -6); ctx.stroke();
      ctx.beginPath(); ctx.moveTo(0, 6); ctx.lineTo(0, 28); ctx.stroke();
      
      // Cyberpunk Tech Data Labels
      ctx.fillStyle = 'rgba(56,189,248,0.6)';
      ctx.font = '8px monospace';
      const stateStr = petState.toUpperCase();
      ctx.fillText(`SYS_STATE: ${stateStr}`, 32, -12);
      ctx.fillText(`LOC: [${Math.round(mx)},${Math.round(my)}]`, 32, -2);
      ctx.fillText(`NET_ACTIVE: TRUE`, 32, 8);
      ctx.restore();
    }

    // 10. Quantum Particle Burst drawing
    for (const p of particles) {
      if (p.shape === 'scifi-spark') {
        ctx.save();
        ctx.globalAlpha = p.alpha * p.life;
        ctx.fillStyle = p.color;
        ctx.shadowColor = p.color;
        ctx.shadowBlur = 6;
        
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
      color: pick(['#ffc0cb','#ff90a0','#ffb7c5','#ffd9e8']),
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
}

defineExpose({ onMouseMove, onClick, resize });

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
