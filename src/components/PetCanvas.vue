<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { usePetStore, resolveSkinId, type Theme, type PetState } from "../stores/pet";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, currentMonitor } from "@tauri-apps/api/window";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  DAIMAO_BATIAO_STILLS,
  DAIMAO_BATIAO_VIDEO_URL,
  PET_CHARACTER_SETTING_KEY,
  resolvePetCharacterId,
  type PetCharacterId,
} from "../services/petCharacters";
import { parsePixelPetManifest, type PixelPetManifest } from "../services/customPixelPet";
import { isPetCanvasPoint } from "../services/petHitTest";

const emit = defineEmits<{
  click: [];
  contextmenu: [event: MouseEvent];
  dragging: [dragging: boolean];
  /** 桌宠被点击/抚摸（预览舞台用来做本地反馈）。与 click 分开：click 在桌宠窗口里绑的是切换聊天面板。 */
  petted: [];
}>();
const pet = usePetStore();
const props = defineProps<{
  character?: PetCharacterId;
  /** 预览实例：禁止一切窗口级副作用（拖拽窗口、双击归位、拖放进食、改写用户设置） */
  preview?: boolean;
  /** 画布缩放倍数。内部画布固定 120x140，仅做视觉缩放，坐标按此反算 */
  scale?: number;
  /** 预览实例也参与状态同步与指针反馈（悬停视线、点击起跳）。窗口副作用仍由 preview 挡住 */
  live?: boolean;
}>();
const canvasRef = ref<HTMLCanvasElement | null>(null);
// 画布后备存储尺寸。桌宠窗口（App.vue）、命中检测（petHitTest.ts）与全部绘制坐标都以此为准
const CANVAS_W = 120;
const CANVAS_H = 140;
const displayScale = computed(() => Math.max(0.1, props.scale ?? 1));
const isLive = computed(() => !props.preview || props.live);
let frameCount = 0;
let mouseDownScreen = { x: 0, y: 0 };
let winPosAtDown = { x: 0, y: 0 };
let dragStarted = false;
let dragFromCanvas = false;
let previewPressed = false; // 预览实例：按下发生在桌宠身上，松开时才算一次"摸摸"
let tickTimer: ReturnType<typeof setInterval> | null = null;
// Interactivity state
let petMx = -999, petMy = -999; // mouse relative to canvas
let isHovering = false;
let bounceOffset = 0;    // y-offset for bounce animation
let bounceVy = 0;        // bounce velocity
let squashAmt = 1;       // squash scale
let longPressTimer: ReturnType<typeof setTimeout> | null = null;
let tiltAngle = 0;       // body tilt toward mouse
let unlistenDragDrop: UnlistenFn | null = null;

type DaimaoMedia = { type: "image"; image: HTMLImageElement } | { type: "video" };
const daimaoStillImages = DAIMAO_BATIAO_STILLS.map((src) => {
  const image = new Image();
  image.decoding = "async";
  image.src = src;
  return image;
});
const daimaoMedia: DaimaoMedia[] = [
  ...daimaoStillImages.map((image) => ({ type: "image" as const, image })),
  { type: "video" },
];
let daimaoMediaIndex = 0;
let daimaoNextSwitchFrame = 0;
let daimaoVideo: HTMLVideoElement | null = null;
let daimaoVideoCanvas: HTMLCanvasElement | null = null;
let customSpriteImage: HTMLImageElement | null = null;
let customSpriteAssetId = "";
let customSpriteReady = false;
let customSpriteFailed = false;

watch(() => pet.customPixelPetAsset?.id, () => {
  customSpriteImage = null;
  customSpriteAssetId = "";
  customSpriteReady = false;
  customSpriteFailed = false;
});

// ===== 粒子系统 =====
interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  maxLife: number;
  size: number;
  color: string;
  type: "heart" | "star" | "note" | "spark" | "zzz" | "question" | "drop";
  rotation: number;
  rotationSpeed: number;
}

const particles: Particle[] = [];
let lastState = "";

function spawnParticles(type: Particle["type"], count: number, cx: number, cy: number) {
  const theme = pet.visualTheme;
  for (let i = 0; i < count; i++) {
    const angle = Math.random() * Math.PI * 2;
    const speed = 0.5 + Math.random() * 1.5;
    particles.push({
      x: cx + (Math.random() - 0.5) * 20,
      y: cy + (Math.random() - 0.5) * 10,
      vx: Math.cos(angle) * speed,
      vy: -Math.abs(Math.sin(angle) * speed) - 0.5,
      life: 1,
      maxLife: 40 + Math.random() * 30,
      size: 4 + Math.random() * 6,
      color: theme.particleColors[Math.floor(Math.random() * theme.particleColors.length)],
      type,
      rotation: Math.random() * Math.PI * 2,
      rotationSpeed: (Math.random() - 0.5) * 0.15,
    });
  }
}

function updateParticles() {
  for (let i = particles.length - 1; i >= 0; i--) {
    const p = particles[i];
    p.x += p.vx;
    p.y += p.vy;
    p.vy += 0.02; // 微重力
    p.life -= 1 / p.maxLife;
    p.rotation += p.rotationSpeed;
    if (p.life <= 0) {
      particles.splice(i, 1);
    }
  }
}

function drawParticle(ctx: CanvasRenderingContext2D, p: Particle) {
  ctx.save();
  ctx.globalAlpha = Math.max(0, p.life);
  ctx.translate(p.x, p.y);
  ctx.rotate(p.rotation);

  switch (p.type) {
    case "heart":
      drawHeart(ctx, p.size, p.color);
      break;
    case "star":
      drawStar(ctx, p.size, p.color);
      break;
    case "note":
      drawNote(ctx, p.size, p.color);
      break;
    case "spark":
      drawSpark(ctx, p.size, p.color);
      break;
    case "question":
      drawQuestion(ctx, p.size, p.color);
      break;
    case "drop":
      drawWaterDrop(ctx, p.size, p.color);
      break;
    default:
      ctx.fillStyle = p.color;
      ctx.beginPath();
      ctx.arc(0, 0, p.size / 2, 0, Math.PI * 2);
      ctx.fill();
  }

  ctx.restore();
}

function drawHeart(ctx: CanvasRenderingContext2D, size: number, color: string) {
  const s = size / 10;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.moveTo(0, s * 3);
  ctx.bezierCurveTo(-s * 5, -s * 2, -s * 5, -s * 5, 0, -s * 2);
  ctx.bezierCurveTo(s * 5, -s * 5, s * 5, -s * 2, 0, s * 3);
  ctx.fill();
}

function drawStar(ctx: CanvasRenderingContext2D, size: number, color: string) {
  ctx.fillStyle = color;
  ctx.beginPath();
  for (let i = 0; i < 5; i++) {
    const angle = (i * 4 * Math.PI) / 5 - Math.PI / 2;
    const r = size / 2;
    const method = i === 0 ? "moveTo" : "lineTo";
    ctx[method](Math.cos(angle) * r, Math.sin(angle) * r);
  }
  ctx.closePath();
  ctx.fill();
}

function drawNote(ctx: CanvasRenderingContext2D, size: number, color: string) {
  ctx.fillStyle = color;
  ctx.font = `${size + 4}px sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("\u{266A}", 0, 0);
}

function drawSpark(ctx: CanvasRenderingContext2D, size: number, color: string) {
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  const r = size / 2;
  for (let i = 0; i < 4; i++) {
    const angle = (i * Math.PI) / 2;
    ctx.beginPath();
    ctx.moveTo(Math.cos(angle) * r * 0.3, Math.sin(angle) * r * 0.3);
    ctx.lineTo(Math.cos(angle) * r, Math.sin(angle) * r);
    ctx.stroke();
  }
}

function drawQuestion(ctx: CanvasRenderingContext2D, size: number, color: string) {
  ctx.fillStyle = color;
  ctx.font = `bold ${size + 2}px sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("?", 0, 0);
}

function drawWaterDrop(ctx: CanvasRenderingContext2D, size: number, color: string) {
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(0, size * 0.3, size / 2, 0, Math.PI);
  ctx.lineTo(0, -size / 2);
  ctx.closePath();
  ctx.fill();
}

// 状态变化时触发粒子
function checkStateParticles() {
  if (pet.state !== lastState) {
    const { x, y } = pet.position;
    switch (pet.state) {
      case "happy":
        spawnParticles("heart", 6, x, y - 30);
        break;
      case "speaking":
        spawnParticles("note", 3, x + 30, y - 30);
        break;
      case "listening":
        spawnParticles("note", 3, x + 25, y - 32);
        break;
      case "working":
        spawnParticles("spark", 5, x, y - 25);
        break;
      case "thinking":
        spawnParticles("star", 4, x + 25, y - 35);
        break;
      case "confused":
        spawnParticles("question", 4, x, y - 40);
        break;
      case "waving":
        spawnParticles("spark", 5, x + 35, y - 20);
        break;
      case "stuffed":
        spawnParticles("star", 5, x, y - 30);
        spawnParticles("note", 2, x, y - 20);
        break;
      case "refusing":
        spawnParticles("spark", 4, x, y - 20);
        break;
    }
    lastState = pet.state;
  }

  // 开心状态持续产生心形
  if (pet.state === "happy" && frameCount % 25 === 0) {
    spawnParticles("heart", 1, pet.position.x + (Math.random() - 0.5) * 40, pet.position.y - 35);
  }
  // 说话状态持续产生音符
  if (pet.state === "speaking" && frameCount % 30 === 0) {
    spawnParticles("note", 1, pet.position.x + 30, pet.position.y - 25);
  }
  // 思考状态持续产生星星
  if (pet.state === "thinking" && frameCount % 20 === 0) {
    spawnParticles("star", 1, pet.position.x + 30 + Math.random() * 15, pet.position.y - 40 - Math.random() * 10);
  }
  // 工作状态持续产生专注火花
  if (pet.state === "working" && frameCount % 24 === 0) {
    spawnParticles("spark", 1, pet.position.x + (Math.random() - 0.5) * 36, pet.position.y - 20);
  }
  // 馋嘴流口水
  if (pet.state === "hungry" && frameCount % 15 === 0) {
    spawnParticles("drop", 1, pet.position.x + 10, pet.position.y + 10);
  }
}

// 定时从后端同步状态
// tick 会推进后端行为状态机（如 waving 数秒后自动回 idle），只能由桌宠窗口这一个时钟驱动；
// 预览实例改用只读的 get_pet_state，否则状态机会被多个窗口加速推进。
async function syncState() {
  try {
    const state = await invoke<Record<string, unknown>>(props.preview ? "get_pet_state" : "tick");
    if (state.state && typeof state.state === "string") {
      pet.setState(state.state as any);
    }
    if (typeof state.happiness === "number") pet.happiness = state.happiness;
    if (typeof state.energy === "number") pet.energy = state.energy;
  } catch {
    // 后端未就绪时忽略
  }
}

async function loadSkin() {
  try {
    const skinId = await invoke<string>("get_skin");
    pet.skin = resolveSkinId(skinId);
  } catch {}
}

async function loadFontColor() {
  try {
    pet.fontColor = await invoke<string>("get_font_color");
  } catch {}
}

async function loadCharacter() {
  if (props.character) return;

  try {
    pet.character = resolvePetCharacterId(await invoke<string>("get_setting_value", {
      key: PET_CHARACTER_SETTING_KEY,
    }));
  } catch {}
}

function ensureDaimaoVideo() {
  if (daimaoVideo) return daimaoVideo;

  daimaoVideo = document.createElement("video");
  daimaoVideo.src = DAIMAO_BATIAO_VIDEO_URL;
  daimaoVideo.muted = true;
  daimaoVideo.loop = true;
  daimaoVideo.playsInline = true;
  daimaoVideo.preload = "auto";
  return daimaoVideo;
}

function pickDaimaoMedia() {
  if (daimaoMedia.length === 0) return null;

  if (frameCount >= daimaoNextSwitchFrame) {
    let nextIndex = Math.floor(Math.random() * daimaoMedia.length);
    if (daimaoMedia.length > 1 && nextIndex === daimaoMediaIndex) {
      nextIndex = (nextIndex + 1) % daimaoMedia.length;
    }
    daimaoMediaIndex = nextIndex;
    daimaoNextSwitchFrame = frameCount + 480 + Math.floor(Math.random() * 480);
  }

  return daimaoMedia[daimaoMediaIndex];
}

function drawDaimaoVideo(
  ctx: CanvasRenderingContext2D,
  video: HTMLVideoElement,
  x: number,
  y: number,
  width: number,
  height: number,
) {
  if (!daimaoVideoCanvas) daimaoVideoCanvas = document.createElement("canvas");
  const canvas = daimaoVideoCanvas;
  canvas.width = 160;
  canvas.height = 180;
  const videoCtx = canvas.getContext("2d");
  if (!videoCtx || video.readyState < 2 || !video.videoWidth || !video.videoHeight) return false;

  videoCtx.clearRect(0, 0, canvas.width, canvas.height);
  const scale = Math.min(canvas.width / video.videoWidth, canvas.height / video.videoHeight);
  const drawW = video.videoWidth * scale;
  const drawH = video.videoHeight * scale;
  videoCtx.drawImage(video, (canvas.width - drawW) / 2, canvas.height - drawH - 4, drawW, drawH);

  const frame = videoCtx.getImageData(0, 0, canvas.width, canvas.height);
  const data = frame.data;
  const imgW = canvas.width;
  const imgH = canvas.height;

  for (let i = 0; i < data.length; i += 4) {
    const pixelIndex = i / 4;
    const px = pixelIndex % imgW;
    const py = Math.floor(pixelIndex / imgW);

    // 只处理边缘区域（上下左右各20像素）
    const isEdge = px < 20 || px >= imgW - 20 || py < 20 || py >= imgH - 20;
    if (!isEdge) continue;

    const r = data[i];
    const g = data[i + 1];
    const b = data[i + 2];

    // 计算与纯白色的欧氏距离
    // 纯白色: RGB(255, 255, 255)
    const distance = Math.sqrt(
      Math.pow(255 - r, 2) +
      Math.pow(255 - g, 2) +
      Math.pow(255 - b, 2)
    );

    // 边缘区域的白色像素透明化
    // 距离越小，颜色越接近白色
    // 阈值设为15，只透明化非常接近白色的像素
    if (distance < 15) {
      data[i + 3] = 0;
    }
  }
  videoCtx.putImageData(frame, 0, 0);
  ctx.drawImage(canvas, x, y, width, height);
  return true;
}

function drawDaimaoPet(ctx: CanvasRenderingContext2D) {
  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  const media = pickDaimaoMedia();
  const bob = Math.sin(frameCount * 0.04) * 2;
  const drawW = 112;
  const drawH = 126;
  const drawX = (ctx.canvas.width - drawW) / 2;
  const drawY = ctx.canvas.height - drawH - 5 + bob;

  ctx.fillStyle = "rgba(15, 23, 42, 0.12)";
  ctx.beginPath();
  ctx.ellipse(ctx.canvas.width / 2, ctx.canvas.height - 12, 36, 5.5, 0, 0, Math.PI * 2);
  ctx.fill();

  if (media?.type === "video") {
    const video = ensureDaimaoVideo();
    if (video.paused) void video.play().catch(() => {});
    if (drawDaimaoVideo(ctx, video, drawX, drawY, drawW, drawH)) return;
  }

  const fallbackImage =
    media?.type === "image" ? media.image : daimaoStillImages[daimaoMediaIndex % daimaoStillImages.length];
  if (fallbackImage?.complete) {
    ctx.drawImage(fallbackImage, drawX, drawY, drawW, drawH);
  }
}

function activeCharacter() {
  return props.character || pet.character;
}

function resolveSpritePath(path: string) {
  if (!path || path.startsWith("data:") || path.startsWith("http:") || path.startsWith("https:")) {
    return path;
  }
  return convertFileSrc(path);
}

function ensureCustomSprite() {
  const asset = pet.customPixelPetAsset;
  if (!asset?.sprite_path) {
    customSpriteImage = null;
    customSpriteAssetId = "";
    customSpriteReady = false;
    customSpriteFailed = false;
    return;
  }

  if (asset.id === customSpriteAssetId && customSpriteImage) return;

  customSpriteAssetId = asset.id;
  customSpriteReady = false;
  customSpriteFailed = false;
  customSpriteImage = new Image();
  customSpriteImage.decoding = "async";
  customSpriteImage.onload = () => {
    customSpriteReady = true;
    customSpriteFailed = false;
  };
  customSpriteImage.onerror = () => {
    customSpriteReady = false;
    customSpriteFailed = true;
  };
  customSpriteImage.src = resolveSpritePath(asset.sprite_path);
}

function customAnimationName() {
  if (pet.state === "speaking") return "speaking";
  if (pet.state === "thinking") return "thinking";
  if (pet.state === "working") return "working";
  if (pet.state === "sleeping") return "sleeping";
  if (pet.state === "hungry") return "hungry";
  if (pet.state === "stuffed") return "stuffed";
  if (pet.state === "refusing") return "refusing";
  if (pet.state === "dragging") return "dragging";
  if (pet.state === "happy" || pet.state === "waving") return "happy";
  return "idle";
}

function drawCustomPixelFallback(ctx: CanvasRenderingContext2D) {
  const theme = pet.visualTheme;
  ctx.fillStyle = "rgba(15, 23, 42, 0.12)";
  ctx.beginPath();
  ctx.ellipse(ctx.canvas.width / 2, ctx.canvas.height - 15, 36, 6, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = theme.primary;
  ctx.fillRect(36, 42, 48, 48);
  ctx.fillStyle = theme.accent;
  ctx.fillRect(42, 34, 36, 12);
  ctx.fillStyle = "#2f2633";
  ctx.fillRect(46, 58, 8, 8);
  ctx.fillRect(66, 58, 8, 8);
  ctx.fillStyle = "rgba(255,255,255,0.75)";
  ctx.fillRect(48, 58, 3, 3);
  ctx.fillRect(68, 58, 3, 3);
}

function drawCustomPixelPet(ctx: CanvasRenderingContext2D) {
  ensureCustomSprite();
  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  const asset = pet.customPixelPetAsset;
  const manifest = parsePixelPetManifest(asset?.manifest) as PixelPetManifest | null;

  if (bounceOffset !== 0 || bounceVy !== 0) {
    bounceVy += 1.0;
    bounceOffset += bounceVy;
    if (bounceOffset >= 0) {
      bounceOffset = 0;
      bounceVy = 0;
    }
  }
  squashAmt += (1 - squashAmt) * 0.1;

  if (!manifest || !customSpriteImage || customSpriteFailed || !customSpriteReady) {
    drawCustomPixelFallback(ctx);
    if (!props.preview && (!asset || !manifest || customSpriteFailed)) {
      pet.character = "classic";
    }
    finishFrame(ctx);
    return;
  }

  const animation = manifest.animations[customAnimationName()] || manifest.animations.idle;
  const frames = animation.frames.length > 0 ? animation.frames : manifest.animations.idle.frames;
  const fps = Math.max(1, animation.fps || 8);
  const ticksPerFrame = Math.max(1, Math.round(60 / fps));
  const sourceFrame = frames[Math.floor(frameCount / ticksPerFrame) % frames.length] || 0;
  const frameW = manifest.frameSize.width;
  const frameH = manifest.frameSize.height;
  const columns = Math.max(1, manifest.sheet.columns || 1);
  const sx = (sourceFrame % columns) * frameW;
  const sy = Math.floor(sourceFrame / columns) * frameH;
  const scale = manifest.displayScale || 1.65;
  const drawW = Math.min(112, Math.round(frameW * scale * (2 - squashAmt)));
  const drawH = Math.min(116, Math.round(frameH * scale * squashAmt));
  const bob = pet.state === "sleeping" ? 2 : Math.sin(frameCount * 0.06) * 2;
  const drawX = Math.round((ctx.canvas.width - drawW) / 2);
  const drawY = Math.round(24 + bob + bounceOffset + (116 - drawH) / 2);

  ctx.fillStyle = "rgba(15, 23, 42, 0.14)";
  ctx.beginPath();
  ctx.ellipse(ctx.canvas.width / 2, ctx.canvas.height - 15, 34, 5.5, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.save();
  ctx.imageSmoothingEnabled = false;
  if (pet.state === "refusing") {
    ctx.translate(Math.sin(frameCount * 1.5) * 4, 0);
  }
  ctx.drawImage(customSpriteImage, sx, sy, frameW, frameH, drawX, drawY, drawW, drawH);
  ctx.restore();
  finishFrame(ctx);
}

// ===== 绘制桌宠 (Bongo Cat 经典敲键盘萌猫) =====
function draw(ctx: CanvasRenderingContext2D) {
  if (activeCharacter() === "custom-pixel") {
    drawCustomPixelPet(ctx);
    return;
  }

  if (activeCharacter() === "daimao-batiao") {
    drawDaimaoPet(ctx);
    return;
  }

  const { x, y } = pet.position;
  const size = 80;

  // 弹跳物理计算 (Spring & gravity)
  if (bounceOffset !== 0 || bounceVy !== 0) {
    bounceVy += 1.0;
    bounceOffset += bounceVy;
    if (bounceOffset >= 0) {
      bounceOffset = 0;
      bounceVy = 0;
    }
  }

  // 挤压复原 (Squash & stretch recovery)
  squashAmt += (1 - squashAmt) * 0.12;

  // 视线与身体倾斜 (Tilt toward cursor)
  const targetTilt = petMx > -900 ? Math.max(-0.12, Math.min(0.12, ((petMx - x) / x) * 0.16)) : 0;
  tiltAngle += (targetTilt - tiltAngle) * 0.09;

  const drawY = y + bounceOffset;

  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  let shakeX = 0;
  if (pet.state === "refusing") {
    shakeX = Math.sin(frameCount * 1.5) * 6;
  }

  // 呼吸与身体伸缩
  const breatheFreq = pet.state === "sleeping" ? 0.03 : 0.055;
  const breatheAmp = pet.state === "sleeping" ? 0.035 : 0.022;
  const breathe = 1 + Math.sin(frameCount * breatheFreq) * breatheAmp;
  const squishState = pet.state === "happy" ? 1 + Math.sin(frameCount * 0.15) * 0.04 : 1;
  const stuffedExpand = pet.state === "stuffed" ? 1.15 : 1;
  const bw = (size - 2) * breathe * squishState * stuffedExpand * (2 - squashAmt);
  const bh = (size - 6) * (breathe / squishState) * squashAmt;

  const theme = pet.visualTheme;

  // 1. 地面柔和环境投影
  drawBongoGroundShadow(ctx, x, y + 46, bw, bounceOffset, frameCount);

  // 应用桌宠中心倾斜变换
  ctx.save();
  ctx.translate(x + shakeX, drawY);
  ctx.rotate(tiltAngle);
  ctx.translate(-x, -drawY);

  // 身体柔和彩色光晕 (跟随主题色，悬停时增亮)
  const glowAlpha = (pet.isAnimatedSkin ? 0.22 : 0.12) + Math.sin(frameCount * 0.03) * 0.04;
  ctx.fillStyle = hexToRgba(theme.accent, isHovering ? glowAlpha + 0.14 : glowAlpha);
  ctx.beginPath();
  ctx.ellipse(x, drawY + 2, bw / 2 + 10, bh / 2 + 8, 0, 0, Math.PI * 2);
  ctx.fill();

  // 2. 摇摆白猫尾巴 (层级在身体后方)
  drawBongoTail(ctx, x, drawY, pet.state, frameCount);

  // 3. 萌萌猫耳朵 (外耳 + 柔粉内耳 + 欢快微抖)
  drawBongoEars(ctx, x, drawY, bw, bh, pet.state, frameCount, isHovering);

  // 4. Bongo Cat 纯白棉花糖圆润身躯与小铃铛项圈
  drawBongoBody(ctx, x, drawY, bw, bh, theme, frameCount);

  // 5. 灵动五官 (水灵大眼、视线追踪、自然眨眼、:3猫唇、软萌腮红)
  const isBlink = frameCount % 165 < 5;
  drawBongoFace(ctx, x, drawY, bw, bh, pet.state, frameCount, isBlink, petMx, petMy, theme);

  // 6. 木质小书桌与机械键盘 (Bongo Cat 灵魂核心)
  drawBongoDeskAndKeyboard(ctx, x, drawY, frameCount, theme, pet.state);

  // 7. Bongo 双爪 (敲键盘交替拍击、双手高举欢呼、作揖、托腮)
  drawBongoPaws(ctx, x, drawY, bw, bh, pet.state, frameCount, theme);

  // 8. 状态专属小道具与微动特效 (思考小鱼干泡泡、按键光芒、沉睡泡泡与Zzz)
  drawBongoStateEffects(ctx, x, drawY, bw, bh, pet.state, frameCount, theme);

  finishFrame(ctx);
}

function finishFrame(ctx: CanvasRenderingContext2D) {
  ctx.restore(); // 恢复倾斜变换
  checkStateParticles();
  updateParticles();
  for (const p of particles) {
    drawParticle(ctx, p);
  }
}

// 1. 地面柔和阴影
function drawBongoGroundShadow(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number, bw: number,
  bounceOff: number, frame: number,
) {
  const jumpHeight = Math.max(0, -bounceOff);
  const shadowScale = Math.max(0.35, 1 - jumpHeight / 48);
  const shadowAlpha = Math.max(0.04, 0.15 * shadowScale);
  const breath = 1 + Math.sin(frame * 0.05) * 0.03;
  const sw = (bw * 0.52) * shadowScale * breath;
  const sh = 7 * shadowScale;

  const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, sw);
  grad.addColorStop(0, `rgba(30, 41, 59, ${shadowAlpha * 1.4})`);
  grad.addColorStop(0.55, `rgba(30, 41, 59, ${shadowAlpha * 0.6})`);
  grad.addColorStop(1, "rgba(30, 41, 59, 0)");
  ctx.fillStyle = grad;
  ctx.beginPath();
  ctx.ellipse(cx, cy, sw, sh, 0, 0, Math.PI * 2);
  ctx.fill();
}

// 2. 摇摆猫尾巴
function drawBongoTail(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  state: PetState, frame: number,
) {
  ctx.save();
  const wagFreq = state === "happy" ? 0.28 : state === "waving" ? 0.22 : 0.075;
  const wagAmp = state === "happy" ? 12 : state === "waving" ? 9 : 5.5;
  const wag = state === "sleeping" ? -2 : Math.sin(frame * wagFreq) * wagAmp;

  const startX = cx + 24;
  const startY = cy + 22;
  const cp1X = cx + 36 + wag * 0.4;
  const cp1Y = cy + 20 - Math.abs(wag) * 0.2;
  const cp2X = cx + 46 + wag * 0.8;
  const cp2Y = cy + 6;
  const endX = cx + 38 + wag;
  const endY = cy - 6 + Math.abs(wag) * 0.3;

  // 尾巴外描边
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 6.2;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(startX, startY);
  ctx.bezierCurveTo(cp1X, cp1Y, cp2X, cp2Y, endX, endY);
  ctx.stroke();

  // 尾巴主体雪白
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = 4.2;
  ctx.beginPath();
  ctx.moveTo(startX, startY);
  ctx.bezierCurveTo(cp1X, cp1Y, cp2X, cp2Y, endX, endY);
  ctx.stroke();

  ctx.restore();
}

// 3. 萌萌猫耳朵
function drawBongoEars(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  bw: number, bh: number,
  state: PetState, frame: number,
  hover: boolean,
) {
  ctx.save();
  const isTwitching = hover || state === "happy";
  const twitchL = isTwitching && frame % 120 < 14 ? Math.sin(frame * 0.6) * 3 : 0;
  const twitchR = isTwitching && frame % 135 < 14 ? -Math.sin(frame * 0.6) * 3 : 0;

  const earBaseY = cy - bh * 0.28;
  const earTipY = cy - bh * 0.52;

  // --- 左耳 ---
  const leftTipX = cx - bw * 0.32 + twitchL;
  const leftTipY = earTipY + twitchL * 0.5;
  ctx.beginPath();
  ctx.moveTo(cx - bw * 0.42, earBaseY + 4);
  ctx.quadraticCurveTo(cx - bw * 0.38, earBaseY - 14, leftTipX, leftTipY);
  ctx.quadraticCurveTo(cx - bw * 0.18, earBaseY - 12, cx - bw * 0.12, earBaseY);
  ctx.closePath();

  ctx.fillStyle = "#ffffff";
  ctx.fill();
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.4;
  ctx.stroke();

  // 左耳粉嫩耳蜗
  ctx.beginPath();
  ctx.moveTo(cx - bw * 0.36, earBaseY + 1);
  ctx.quadraticCurveTo(cx - bw * 0.34, earBaseY - 10, leftTipX + 2, leftTipY + 4);
  ctx.quadraticCurveTo(cx - bw * 0.22, earBaseY - 8, cx - bw * 0.18, earBaseY);
  ctx.closePath();
  ctx.fillStyle = "#fbcfe8";
  ctx.fill();

  // --- 右耳 ---
  const rightTipX = cx + bw * 0.32 + twitchR;
  const rightTipY = earTipY + twitchR * 0.5;
  ctx.beginPath();
  ctx.moveTo(cx + bw * 0.12, earBaseY);
  ctx.quadraticCurveTo(cx + bw * 0.18, earBaseY - 12, rightTipX, rightTipY);
  ctx.quadraticCurveTo(cx + bw * 0.38, earBaseY - 14, cx + bw * 0.42, earBaseY + 4);
  ctx.closePath();

  ctx.fillStyle = "#ffffff";
  ctx.fill();
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.4;
  ctx.stroke();

  // 右耳粉嫩耳蜗
  ctx.beginPath();
  ctx.moveTo(cx + bw * 0.18, earBaseY);
  ctx.quadraticCurveTo(cx + bw * 0.22, earBaseY - 8, rightTipX - 2, rightTipY + 4);
  ctx.quadraticCurveTo(cx + bw * 0.34, earBaseY - 10, cx + bw * 0.36, earBaseY + 1);
  ctx.closePath();
  ctx.fillStyle = "#fbcfe8";
  ctx.fill();

  ctx.restore();
}

// 4. Bongo Cat 纯白棉花糖身躯与项圈
function drawBongoBody(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  bw: number, bh: number,
  theme: Theme, frame: number,
) {
  ctx.save();
  const halfW = bw * 0.44;
  const halfH = bh * 0.44;

  // 极简圆润猫咪轮廓
  ctx.beginPath();
  ctx.moveTo(cx - halfW * 0.75, cy - halfH * 0.65);
  ctx.bezierCurveTo(
    cx - halfW * 0.45, cy - halfH * 1.05,
    cx + halfW * 0.45, cy - halfH * 1.05,
    cx + halfW * 0.75, cy - halfH * 0.65,
  );
  ctx.bezierCurveTo(
    cx + halfW * 1.08, cy - halfH * 0.15,
    cx + halfW * 1.06, cy + halfH * 0.65,
    cx + halfW * 0.72, cy + halfH * 0.96,
  );
  ctx.bezierCurveTo(
    cx + halfW * 0.38, cy + halfH * 1.05,
    cx - halfW * 0.38, cy + halfH * 1.05,
    cx - halfW * 0.72, cy + halfH * 0.96,
  );
  ctx.bezierCurveTo(
    cx - halfW * 1.06, cy + halfH * 0.65,
    cx - halfW * 1.08, cy - halfH * 0.15,
    cx - halfW * 0.75, cy - halfH * 0.65,
  );
  ctx.closePath();

  // 渐变填充 (柔和洁白棉花糖)
  const bodyGrad = ctx.createLinearGradient(cx, cy - halfH, cx, cy + halfH);
  bodyGrad.addColorStop(0, "#ffffff");
  bodyGrad.addColorStop(0.75, "#ffffff");
  bodyGrad.addColorStop(1, "#f1f5f9");
  ctx.fillStyle = bodyGrad;
  ctx.fill();

  // 饱满精致深色描边
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.4;
  ctx.stroke();

  // 可爱小红项圈与金铃铛 (随身体呼吸轻微晃动)
  const collarY = cy + halfH * 0.32;
  const collarW = halfW * 0.85;
  ctx.beginPath();
  ctx.ellipse(cx, collarY, collarW, 7, 0, 0.15 * Math.PI, 0.85 * Math.PI);
  ctx.strokeStyle = theme.primary || "#ef4444";
  ctx.lineWidth = 4.2;
  ctx.stroke();

  // 迷你金色小铃铛
  const bellSway = Math.sin(frame * 0.08) * 1.2;
  const bellX = cx + bellSway;
  const bellY = collarY + 6.5;
  ctx.fillStyle = "#fbbf24";
  ctx.beginPath();
  ctx.arc(bellX, bellY, 3.8, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = "#d97706";
  ctx.lineWidth = 1;
  ctx.stroke();

  // 铃铛小孔
  ctx.fillStyle = "#92400e";
  ctx.beginPath();
  ctx.arc(bellX, bellY + 1.2, 1, 0, Math.PI * 2);
  ctx.fill();

  ctx.restore();
}

// 5. 灵动五官 (水灵大眼、视线追踪、自然眨眼、:3猫唇、软萌腮红)
function drawBongoFace(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  _bw: number, _bh: number,
  state: PetState, frame: number,
  isBlink: boolean,
  mx: number, my: number,
  _theme: Theme,
) {
  ctx.save();
  const faceY = cy - 2;
  const eyeDist = 13.5;
  const leftEyeX = cx - eyeDist;
  const rightEyeX = cx + eyeDist;
  const eyeY = faceY - 2;

  // 软萌腮红 (草莓牛奶粉)
  const blushAlpha = state === "happy" ? 0.72 : 0.52;
  ctx.fillStyle = `rgba(251, 113, 133, ${blushAlpha})`;
  ctx.beginPath();
  ctx.ellipse(cx - 21.5, faceY + 5.5, 5.5, 3.2, -0.05, 0, Math.PI * 2);
  ctx.fill();
  ctx.beginPath();
  ctx.ellipse(cx + 21.5, faceY + 5.5, 5.5, 3.2, 0.05, 0, Math.PI * 2);
  ctx.fill();

  // 两侧萌系猫须 (细腻干净)
  ctx.strokeStyle = "#94a3b8";
  ctx.lineWidth = 1.3;
  // 左侧胡须
  ctx.beginPath();
  ctx.moveTo(cx - 22, faceY + 3);
  ctx.lineTo(cx - 35, faceY + 1);
  ctx.moveTo(cx - 22, faceY + 7);
  ctx.lineTo(cx - 34, faceY + 8);
  // 右侧胡须
  ctx.moveTo(cx + 22, faceY + 3);
  ctx.lineTo(cx + 35, faceY + 1);
  ctx.moveTo(cx + 22, faceY + 7);
  ctx.lineTo(cx + 34, faceY + 8);
  ctx.stroke();

  // --- 眼睛表情分支 ---
  if (state === "sleeping") {
    // 沉睡弯弯睫毛眼 (-- --)
    ctx.strokeStyle = "#334155";
    ctx.lineWidth = 2.2;
    ctx.beginPath();
    ctx.arc(leftEyeX, eyeY + 1, 4.2, 0.15 * Math.PI, 0.85 * Math.PI);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rightEyeX, eyeY + 1, 4.2, 0.15 * Math.PI, 0.85 * Math.PI);
    ctx.stroke();
  } else if (state === "happy" || state === "waving") {
    // 开心/抚摸/招手：笑眯眯弯弯月牙眼 (^ ^)
    ctx.strokeStyle = "#334155";
    ctx.lineWidth = 2.5;
    ctx.beginPath();
    ctx.arc(leftEyeX, eyeY + 2, 4.6, 1.15 * Math.PI, 1.85 * Math.PI);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(rightEyeX, eyeY + 2, 4.6, 1.15 * Math.PI, 1.85 * Math.PI);
    ctx.stroke();
  } else if (isBlink) {
    // 自然眨眼线
    ctx.strokeStyle = "#334155";
    ctx.lineWidth = 2.4;
    ctx.beginPath();
    ctx.moveTo(leftEyeX - 4, eyeY);
    ctx.lineTo(leftEyeX + 4, eyeY);
    ctx.moveTo(rightEyeX - 4, eyeY);
    ctx.lineTo(rightEyeX + 4, eyeY);
    ctx.stroke();
  } else {
    // 经典水灵大眼 (支持光标视线追踪)
    let lookDx = 0, lookDy = 0;
    if (mx > -900) {
      lookDx = Math.max(-1.8, Math.min(1.8, (mx - cx) / 25));
      lookDy = Math.max(-1.8, Math.min(1.8, (my - eyeY) / 25));
    }

    // 左眼瞳
    ctx.fillStyle = "#1e293b";
    ctx.beginPath();
    ctx.ellipse(leftEyeX + lookDx, eyeY + lookDy, 4.5, 5.8, 0, 0, Math.PI * 2);
    ctx.fill();

    // 右眼瞳
    ctx.beginPath();
    ctx.ellipse(rightEyeX + lookDx, eyeY + lookDy, 4.5, 5.8, 0, 0, Math.PI * 2);
    ctx.fill();

    // 灵动大高光 (右上角)
    ctx.fillStyle = "#ffffff";
    ctx.beginPath();
    ctx.arc(leftEyeX + lookDx + 1.6, eyeY + lookDy - 2.0, 1.9, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.arc(rightEyeX + lookDx + 1.6, eyeY + lookDy - 2.0, 1.9, 0, Math.PI * 2);
    ctx.fill();

    // 细微小高光 (左下角)
    ctx.beginPath();
    ctx.arc(leftEyeX + lookDx - 1.5, eyeY + lookDy + 1.8, 1.0, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.arc(rightEyeX + lookDx - 1.5, eyeY + lookDy + 1.8, 1.0, 0, Math.PI * 2);
    ctx.fill();
  }

  // --- 萌系小鼻子与 :3 猫唇 ---
  const noseY = faceY + 2.6;
  ctx.fillStyle = "#f472b6";
  ctx.beginPath();
  ctx.ellipse(cx, noseY, 1.4, 1.0, 0, 0, Math.PI * 2);
  ctx.fill();

  const mouthY = noseY + 1.8;
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 1.9;

  if (state === "speaking" || (state === "happy" && frame % 40 < 20)) {
    // 说话或欢笑时张开小嘴 (含萌萌小粉舌)
    ctx.beginPath();
    ctx.arc(cx, mouthY + 1.5, 3.8, 0.1 * Math.PI, 0.9 * Math.PI);
    ctx.fillStyle = "#f43f5e";
    ctx.fill();
    ctx.stroke();

    // 舌尖
    ctx.fillStyle = "#fbcfe8";
    ctx.beginPath();
    ctx.arc(cx, mouthY + 3.2, 2.2, 0.1 * Math.PI, 0.9 * Math.PI);
    ctx.fill();
  } else {
    // 标志性 :3 傲娇猫唇
    ctx.beginPath();
    ctx.arc(cx - 2.6, mouthY + 1.8, 2.7, 0.15 * Math.PI, 0.95 * Math.PI);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx + 2.6, mouthY + 1.8, 2.7, 0.05 * Math.PI, 0.85 * Math.PI);
    ctx.stroke();
  }

  ctx.restore();
}

// 6. 木质小书桌与机械键盘 (Bongo Cat 灵魂核心)
function drawBongoDeskAndKeyboard(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  frame: number,
  theme: Theme,
  state: PetState,
) {
  ctx.save();
  const deskX = cx - 46;
  const deskY = cy + 19;
  const deskW = 92;
  const deskH = 26;
  const deskR = 7;

  // 1. 木质桌面
  const woodGrad = ctx.createLinearGradient(deskX, deskY, deskX, deskY + deskH);
  woodGrad.addColorStop(0, "#fef3c7");
  woodGrad.addColorStop(0.35, "#fde68a");
  woodGrad.addColorStop(1, "#f59e0b");
  ctx.fillStyle = woodGrad;

  ctx.beginPath();
  roundRect(ctx, deskX, deskY, deskW, deskH, deskR);
  ctx.fill();
  ctx.strokeStyle = "#d97706";
  ctx.lineWidth = 2.0;
  ctx.stroke();

  // 桌面边沿高光
  ctx.strokeStyle = "rgba(255, 255, 255, 0.65)";
  ctx.lineWidth = 1.2;
  ctx.beginPath();
  ctx.moveTo(deskX + deskR, deskY + 1.5);
  ctx.lineTo(deskX + deskW - deskR, deskY + 1.5);
  ctx.stroke();

  // 2. 机械键盘底座
  const kbX = cx - 35;
  const kbY = deskY + 3;
  const kbW = 58;
  const kbH = 16;
  const kbR = 3.5;

  ctx.fillStyle = "#f1f5f9";
  ctx.beginPath();
  roundRect(ctx, kbX, kbY, kbW, kbH, kbR);
  ctx.fill();
  ctx.strokeStyle = "#94a3b8";
  ctx.lineWidth = 1.2;
  ctx.stroke();

  // 3. 两排彩色马卡龙键帽
  const keyColors = [
    "#bae6fd", "#bbf7d0", "#fef08a", "#fbcfe8", "#e9d5ff",
    "#a7f3d0", "#fed7aa", "#fecdd3", "#c7d2fe", "#f5d0fe",
  ];

  // 敲键盘按压相位 (根据敲击状态计算当前被按下的键)
  const typingActive = state === "working" || state === "speaking" || (state === "idle" && frame % 160 < 40);
  const typePhase = Math.sin(frame * (state === "working" ? 0.38 : 0.18));
  const leftPress = typingActive && typePhase > 0.2;
  const rightPress = typingActive && typePhase < -0.2;

  // 上排 5 颗键帽
  for (let i = 0; i < 5; i++) {
    const kx = kbX + 4 + i * 10;
    const isPressed = (i === 1 && leftPress) || (i === 3 && rightPress);
    const ky = kbY + 2.5 + (isPressed ? 1.4 : 0);
    ctx.fillStyle = isPressed ? hexToRgba(theme.accent, 0.9) : keyColors[i];
    ctx.beginPath();
    roundRect(ctx, kx, ky, 8.5, 4.5, 1.2);
    ctx.fill();
  }

  // 下排 4 颗键帽 + 空格键
  for (let i = 0; i < 4; i++) {
    const kx = kbX + 5 + i * 12;
    const isPressed = (i === 0 && leftPress) || (i === 3 && rightPress);
    const ky = kbY + 8.5 + (isPressed ? 1.4 : 0);
    ctx.fillStyle = isPressed ? hexToRgba(theme.accent, 0.9) : keyColors[i + 5];
    ctx.beginPath();
    roundRect(ctx, kx, ky, 9.5, 4.5, 1.2);
    ctx.fill();
  }

  // 4. 桌面右侧萌萌小马克杯 (含小猫爪图标与微弱热气)
  const cupX = cx + 33;
  const cupY = deskY + 4;
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  roundRect(ctx, cupX, cupY, 9, 11, 2);
  ctx.fill();
  ctx.strokeStyle = "#cbd5e1";
  ctx.lineWidth = 1;
  ctx.stroke();

  // 杯把手
  ctx.beginPath();
  ctx.arc(cupX + 9, cupY + 5.5, 2.5, -0.4 * Math.PI, 0.4 * Math.PI);
  ctx.strokeStyle = "#cbd5e1";
  ctx.stroke();

  // 杯身小粉爪印
  ctx.fillStyle = "#f472b6";
  ctx.beginPath();
  ctx.arc(cupX + 4.5, cupY + 6.5, 1.4, 0, Math.PI * 2);
  ctx.fill();

  // 飘动的治愈热气
  const steamOff = (frame * 0.05) % (Math.PI * 2);
  ctx.strokeStyle = "rgba(148, 163, 184, 0.45)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(cupX + 4.5, cupY - 1);
  ctx.quadraticCurveTo(cupX + 2.5 + Math.sin(steamOff) * 1.5, cupY - 4, cupX + 4.5, cupY - 7);
  ctx.stroke();

  ctx.restore();
}

// 7. Bongo 双爪 (敲键盘交替拍击、高举招手、作揖求喂食、托腮)
function drawBongoPaws(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  _bw: number, _bh: number,
  state: PetState, frame: number,
  theme: Theme,
) {
  ctx.save();
  const leftPawHomeX = cx - 18;
  const rightPawHomeX = cx + 8;
  const pawDeskY = cy + 24;

  if (state === "happy" || state === "waving") {
    // 双爪高举欢呼开怀舞动 (\( =①ω①= )/) 露出粉嫩肉垫
    const waveL = Math.sin(frame * 0.22) * 5;
    const waveR = Math.cos(frame * 0.22) * 5;
    drawCuteRaisedPaw(ctx, cx - 26, cy + 4 + waveL, true);
    drawCuteRaisedPaw(ctx, cx + 26, cy + 4 + waveR, false);
  } else if (state === "thinking") {
    // 思考中：左爪搭在键盘上，右爪可爱地托着下巴
    drawTypingPaw(ctx, leftPawHomeX, pawDeskY, 0);
    // 右爪托腮
    drawChinPaw(ctx, cx + 16, cy + 6);
  } else if (state === "hungry") {
    // 饥饿中：双爪并拢在桌前作揖求小鱼干
    const begOff = Math.sin(frame * 0.25) * 3;
    drawTypingPaw(ctx, cx - 7, pawDeskY - 2 + begOff, 0);
    drawTypingPaw(ctx, cx + 7, pawDeskY - 2 + begOff, 0);
  } else if (state === "sleeping") {
    // 沉睡中：双爪交叉平趴在桌上，小脑袋枕着双爪
    drawSleepingPaws(ctx, cx, pawDeskY);
  } else {
    // 正常 / 工作 / 说话：经典 Bongo Cat 左右爪交替拍打键盘！
    const isTyping = state === "working" || state === "speaking" || (state === "idle" && frame % 160 < 40);
    const speed = state === "working" ? 0.38 : state === "speaking" ? 0.22 : 0.12;
    const cycle = isTyping ? Math.sin(frame * speed) : 0;

    // 左爪抬起/拍下 (cycle > 0 拍下，cycle < 0 抬起)
    const leftLift = isTyping ? (cycle < 0 ? Math.abs(cycle) * 7.5 : 0) : 0;
    // 右爪抬起/拍下 (与左爪相反)
    const rightLift = isTyping ? (cycle > 0 ? Math.abs(cycle) * 7.5 : 0) : 0;

    drawTypingPaw(ctx, leftPawHomeX, pawDeskY - leftLift, leftLift > 0.5 ? -0.15 : 0);
    drawTypingPaw(ctx, rightPawHomeX, pawDeskY - rightLift, rightLift > 0.5 ? 0.15 : 0);

    // 拍击按键时弹出微型打击波
    if (isTyping && Math.abs(cycle) < 0.15) {
      const tapX = cycle >= 0 ? rightPawHomeX + 4 : leftPawHomeX + 4;
      ctx.strokeStyle = hexToRgba(theme.accent, 0.65);
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.ellipse(tapX, pawDeskY + 4, 4.5, 2, 0, 0, Math.PI * 2);
      ctx.stroke();
    }
  }

  ctx.restore();
}

// 敲键盘棉花糖爪子
function drawTypingPaw(
  ctx: CanvasRenderingContext2D,
  px: number, py: number,
  rotation: number,
) {
  ctx.save();
  ctx.translate(px, py);
  ctx.rotate(rotation);

  // 爪爪主体 (纯白棉花糖)
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.ellipse(0, 0, 8.5, 6.2, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.2;
  ctx.stroke();

  // 爪指微细分线
  ctx.strokeStyle = "#cbd5e1";
  ctx.lineWidth = 1.2;
  ctx.beginPath();
  ctx.moveTo(-2, -3);
  ctx.lineTo(-2, 3);
  ctx.moveTo(2, -3);
  ctx.lineTo(2, 3);
  ctx.stroke();

  ctx.restore();
}

// 高举挥舞的小肉垫爪爪
function drawCuteRaisedPaw(
  ctx: CanvasRenderingContext2D,
  px: number, py: number,
  isLeft: boolean,
) {
  ctx.save();
  ctx.translate(px, py);
  ctx.rotate(isLeft ? -0.25 : 0.25);

  // 爪手掌
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.ellipse(0, 0, 8.8, 7.5, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.2;
  ctx.stroke();

  // 粉嫩中心主肉垫
  ctx.fillStyle = "#f472b6";
  ctx.beginPath();
  ctx.ellipse(0, 0.5, 3.6, 2.8, 0, 0, Math.PI * 2);
  ctx.fill();

  // 3颗可爱指尖小肉垫
  const dotR = 1.2;
  ctx.beginPath();
  ctx.arc(-4.2, -3.2, dotR, 0, Math.PI * 2);
  ctx.arc(0, -4.5, dotR, 0, Math.PI * 2);
  ctx.arc(4.2, -3.2, dotR, 0, Math.PI * 2);
  ctx.fill();

  ctx.restore();
}

// 思考时可爱的托腮小爪
function drawChinPaw(ctx: CanvasRenderingContext2D, px: number, py: number) {
  ctx.save();
  ctx.translate(px, py);
  ctx.rotate(0.35);

  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.ellipse(0, 0, 8.2, 6.5, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.2;
  ctx.stroke();

  ctx.restore();
}

// 睡眠时趴在桌上的双爪
function drawSleepingPaws(ctx: CanvasRenderingContext2D, cx: number, py: number) {
  ctx.save();
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.ellipse(cx - 10, py, 9, 5.5, -0.15, 0, Math.PI * 2);
  ctx.ellipse(cx + 10, py, 9, 5.5, 0.15, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = "#334155";
  ctx.lineWidth = 2.2;
  ctx.stroke();
  ctx.restore();
}

// 8. 状态专属小道具与微动特效 (思考小鱼干泡泡、按键光芒、沉睡泡泡与Zzz)
function drawBongoStateEffects(
  ctx: CanvasRenderingContext2D,
  cx: number, cy: number,
  _bw: number, bh: number,
  state: PetState, frame: number,
  theme: Theme,
) {
  ctx.save();

  // 思考中：头顶冒出想着“香喷喷小鱼干”的卡通泡泡
  if (state === "thinking") {
    const bobbleY = cy - bh * 0.45;
    ctx.fillStyle = "rgba(255, 255, 255, 0.85)";
    ctx.strokeStyle = "#94a3b8";
    ctx.lineWidth = 1.2;

    // 导向小气泡
    ctx.beginPath();
    ctx.arc(cx + 22, bobbleY - 6, 2.5, 0, Math.PI * 2);
    ctx.arc(cx + 28, bobbleY - 14, 4.0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();

    // 大思考气泡
    const bubbleX = cx + 36;
    const bubbleY = bobbleY - 26;
    ctx.fillStyle = "#ffffff";
    ctx.beginPath();
    ctx.ellipse(bubbleX, bubbleY, 15, 11, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();

    // 泡泡内的小鱼干
    ctx.save();
    ctx.translate(bubbleX, bubbleY);
    ctx.fillStyle = "#fbbf24";
    ctx.beginPath();
    ctx.ellipse(0, 0, 7.5, 4.0, 0, 0, Math.PI * 2);
    // 鱼尾
    ctx.moveTo(6, 0);
    ctx.lineTo(11, -3.5);
    ctx.lineTo(9, 0);
    ctx.lineTo(11, 3.5);
    ctx.closePath();
    ctx.fill();

    // 鱼眼
    ctx.fillStyle = "#78350f";
    ctx.beginPath();
    ctx.arc(-4, -1, 0.9, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  // 沉睡中：Zzz 气泡与呼吸微气泡
  if (state === "sleeping") {
    const zOff = Math.sin(frame * 0.05) * 4;
    const zAlpha = 0.55 + Math.sin(frame * 0.07) * 0.25;
    ctx.fillStyle = hexToRgba(theme.accent || "#6366f1", zAlpha);
    ctx.font = "bold 14px sans-serif";
    ctx.textAlign = "center";
    ctx.fillText("Z", cx + 26, cy - 20 + zOff);
    ctx.font = "bold 10px sans-serif";
    ctx.fillText("z", cx + 35, cy - 30 + zOff * 0.8);
    ctx.font = "bold 7px sans-serif";
    ctx.fillText("z", cx + 42, cy - 38 + zOff * 0.5);

    // 睡眠小呼噜气泡
    const bubbleBreath = 1 + Math.sin(frame * 0.06) * 0.35;
    ctx.fillStyle = "rgba(255, 255, 255, 0.65)";
    ctx.strokeStyle = "rgba(148, 163, 184, 0.75)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(cx + 8, cy + 3, 3.2 * bubbleBreath, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();
  }

  // 工作打字：键盘四周飞溅专注火花
  if (state === "working") {
    if (frame % 16 < 8) {
      const sparkX = cx - 20 + (frame % 3) * 16;
      ctx.fillStyle = hexToRgba(theme.accent, 0.75);
      ctx.beginPath();
      ctx.arc(sparkX, cy + 22, 1.6, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  ctx.restore();
}

// ===== 工具函数 =====

function hexToRgba(hex: string, alpha: number): string {
  const num = parseInt(hex.replace("#", ""), 16);
  const r = (num >> 16) & 0xff;
  const g = (num >> 8) & 0xff;
  const b = num & 0xff;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function roundRect(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number, r: number,
) {
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function animate() {
  const canvas = canvasRef.value;
  if (!canvas) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  frameCount++;
  draw(ctx);
  requestAnimationFrame(animate);
}

// 把指针位置换算回画布单位。getBoundingClientRect 在 CSS transform 缩放下返回的是视觉尺寸，
// 所以命中检测与视线跟随都必须先除以 scale，否则舞台里的桌宠会"摸不到"。
function pointerToCanvas(e: MouseEvent) {
  const canvas = canvasRef.value;
  if (!canvas) return null;
  const rect = canvas.getBoundingClientRect();
  const s = displayScale.value;
  return { x: (e.clientX - rect.left) / s, y: (e.clientY - rect.top) / s };
}

function isPetHit(e: MouseEvent) {
  const point = pointerToCanvas(e);
  if (!point) return false;
  return isPetCanvasPoint(point.x, point.y, activeCharacter(), pet.state, pet.position);
}

async function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  if (!isPetHit(e)) return;
  // 预览实例不能拖窗口：getCurrentWindow() 在这里会解析成宿主窗口（仪表盘），
  // 放开就会把控制台自己拖走。只保留按下反馈。
  if (props.preview) {
    dragFromCanvas = false;
    previewPressed = true;
    longPressTimer = setTimeout(() => { squashAmt = 0.75; }, 500);
    return;
  }
  dragFromCanvas = true;
  emit("dragging", true);
  mouseDownScreen = { x: e.screenX, y: e.screenY };
  dragStarted = false;
  // Long press squash
  longPressTimer = setTimeout(() => {
    if (dragFromCanvas && !dragStarted) squashAmt = 0.75;
  }, 500);
  const win = getCurrentWindow();
  const scale = window.devicePixelRatio || 1;
  const pos = await win.outerPosition();
  winPosAtDown = { x: pos.x / scale, y: pos.y / scale };
}

function onMouseMove(e: MouseEvent) {
  // Update pet-relative mouse position
  const point = pointerToCanvas(e);
  if (point) {
    petMx = point.x;
    petMy = point.y;
    const wasHovering = isHovering;
    isHovering = isPetHit(e);
    if (!wasHovering && isHovering) spawnParticles('spark', 2, pet.position.x, pet.position.y - 30);
  }
  // Drag
  if (!dragFromCanvas || e.buttons !== 1) return;
  const dx = e.screenX - mouseDownScreen.x;
  const dy = e.screenY - mouseDownScreen.y;
  if (!dragStarted && (Math.abs(dx) > 3 || Math.abs(dy) > 3)) {
    dragStarted = true;
  }
  if (dragStarted) {
    getCurrentWindow().setPosition(
      new LogicalPosition(winPosAtDown.x + dx, winPosAtDown.y + dy),
    );
  }
}

function onMouseUp(e: MouseEvent) {
  // 预览实例：只做本地反馈（起跳 + 爱心），不触发桌宠窗口的 click 语义
  if (props.preview) {
    const wasPressed = previewPressed;
    previewPressed = false;
    if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
    squashAmt = 1;
    if (!wasPressed || e.button !== 0 || !isPetHit(e)) return;
    bounceVy = -7.5;
    bounceOffset = 0;
    squashAmt = 0.78;
    spawnParticles('heart', 5, pet.position.x, pet.position.y - 25);
    spawnParticles('spark', 3, pet.position.x, pet.position.y - 20);
    emit("petted");
    return;
  }
  if (!dragFromCanvas || e.button !== 0) return;
  dragFromCanvas = false;
  emit("dragging", false);
  if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
  squashAmt = 1;
  if (!dragStarted) {
    // Q弹起跳 + 粒子爆发
    bounceVy = -7.5;
    bounceOffset = 0;
    squashAmt = 0.78;
    spawnParticles('heart', 5, pet.position.x, pet.position.y - 25);
    spawnParticles('spark', 3, pet.position.x, pet.position.y - 20);
    emit("click");
  }
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  if (!isPetHit(e)) return;
  // 预览实例不弹右键菜单窗口（那会在光标处开一个真实的 context-menu 窗口）
  if (props.preview) return;
  emit("contextmenu", e);
}

async function resetPosition() {
  try {
    const win = getCurrentWindow();
    const scale = window.devicePixelRatio || 1;
    const monitor = await currentMonitor();
    if (monitor) {
      const mw = monitor.size.width / scale;
      const mh = monitor.size.height / scale;
      const ww = (await win.outerSize()).width / scale;
      const wh = (await win.outerSize()).height / scale;
      await win.setPosition(new LogicalPosition(mw / 2 - ww / 2, mh / 2 - wh / 2));
    }
  } catch {}
}

async function eatFiles(paths: string[]) {
  try {
    await invoke("eat_files", { paths });
    pet.setState("stuffed");
    invoke("set_pet_state", { newState: "stuffed" });
  } catch (err) {
    pet.setState("refusing");
    invoke("set_pet_state", { newState: "refusing" });
  }
}

function onPreviewMove(e: MouseEvent) {
  const point = pointerToCanvas(e);
  if (!point) return;
  petMx = point.x;
  petMy = point.y;
  const wasHovering = isHovering;
  isHovering = isPetHit(e);
  if (!wasHovering && isHovering) spawnParticles('spark', 2, pet.position.x, pet.position.y - 30);
}

function onCanvasLeave() {
  petMx = -999;
  petMy = -999;
  isHovering = false;
  if (props.preview) {
    previewPressed = false;
    if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
  }
}

onMounted(async () => {
  const canvas = canvasRef.value;
  if (canvas) {
    canvas.width = CANVAS_W;
    canvas.height = CANVAS_H;
    animate();
    if (isLive.value) {
      // 预览实例不接管 window 级别的 mousemove/mouseup：那会和宿主页面抢事件。
      // 预览改用 canvas 自身的 @mousemove / @mouseup（见模板）。
      if (!props.preview) {
        window.addEventListener("mousemove", onMouseMove);
        window.addEventListener("mouseup", onMouseUp);
      }
      // 双击归位会调用 setPosition，把宿主窗口传到显示器中心，严禁在预览里绑定
      if (!props.preview) canvas.addEventListener("dblclick", resetPosition);
      canvas.addEventListener("mouseleave", onCanvasLeave);
      tickTimer = setInterval(syncState, 1000);
      loadSkin();
      loadFontColor();
      loadCharacter();
    }
  }

  if (props.preview) return;

  unlistenDragDrop = await getCurrentWindow().onDragDropEvent((e) => {
    if (e.payload.type === "enter") {
      // .lnk 文件由 ChatBubble 处理注册，不“吃”
      const hasLnk = e.payload.paths.some((p: string) => p.toLowerCase().endsWith(".lnk"));
      if (hasLnk) return;
      pet.setState("hungry");
      invoke("set_pet_state", { newState: "hungry" });
    } else if (e.payload.type === "over") {
      // over 事件不携带 paths，保持当前状态
    } else if (e.payload.type === "leave") {
      pet.setState("idle");
      invoke("set_pet_state", { newState: "idle" });
    } else if (e.payload.type === "drop") {
      const paths = e.payload.paths;
      // .lnk 文件由 ChatBubble 处理注册，不“吃”
      const hasLnk = paths.some((p: string) => p.toLowerCase().endsWith(".lnk"));
      if (hasLnk) return;
      eatFiles(paths);
    }
  });
});

onUnmounted(() => {
  window.removeEventListener("mousemove", onMouseMove);
  window.removeEventListener("mouseup", onMouseUp);
  if (unlistenDragDrop) unlistenDragDrop();
  emit("dragging", false);
  if (tickTimer) clearInterval(tickTimer);
});
</script>

<template>
  <div
    class="pet-canvas-stage"
    :style="{
      width: CANVAS_W * displayScale + 'px',
      height: CANVAS_H * displayScale + 'px',
    }"
  >
    <canvas
      ref="canvasRef"
      :class="['pet-canvas', { 'is-preview': props.preview, 'is-inert': props.preview && !props.live }]"
      :style="{
        width: CANVAS_W + 'px',
        height: CANVAS_H + 'px',
        transform: displayScale === 1 ? undefined : `scale(${displayScale})`,
      }"
      @mousedown="onMouseDown"
      @mouseup="props.preview ? onMouseUp($event) : undefined"
      @mousemove="props.preview ? onPreviewMove($event) : undefined"
      @contextmenu="onContextMenu"
    />
  </div>
</template>

<style scoped>
/* 外层负责占位（画布是绝对定位的，不参与文档流），内层保持 120x140 并被缩放 */
.pet-canvas-stage {
  position: relative;
  flex-shrink: 0;
}

.pet-canvas {
  position: absolute;
  top: 0;
  left: 0;
  transform-origin: top left;
  cursor: grab;
  pointer-events: auto;
}
.pet-canvas:active {
  cursor: grabbing;
}
/* 预览实例不能拖动窗口，用手型提示"可以摸" */
.pet-canvas.is-preview,
.pet-canvas.is-preview:active {
  cursor: pointer;
}
/* 非 live 的预览（换装选择器里的小缩略图）完全不接收指针，交互安全不依赖调用方 */
.pet-canvas.is-inert {
  pointer-events: none;
}
</style>
