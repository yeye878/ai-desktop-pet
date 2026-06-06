<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from "vue";
import { usePetStore, resolveSkinId, type Theme } from "../stores/pet";
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
}>();
const pet = usePetStore();
const props = defineProps<{ character?: PetCharacterId; preview?: boolean }>();
const canvasRef = ref<HTMLCanvasElement | null>(null);
let frameCount = 0;
let mouseDownScreen = { x: 0, y: 0 };
let winPosAtDown = { x: 0, y: 0 };
let dragStarted = false;
let dragFromCanvas = false;
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
  // 馋嘴流口水
  if (pet.state === "hungry" && frameCount % 15 === 0) {
    spawnParticles("drop", 1, pet.position.x + 10, pet.position.y + 10);
  }
}

// 定时从后端同步状态
async function syncState() {
  try {
    const state = await invoke<Record<string, unknown>>("tick");
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

// ===== 绘制桌宠 =====
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
  // Update bounce physics
  if (bounceOffset !== 0 || bounceVy !== 0) {
    bounceVy += 1.0; // snappy gravity
    bounceOffset += bounceVy;
    if (bounceOffset >= 0) { bounceOffset = 0; bounceVy = 0; }
  }
  // Squash recovery
  squashAmt += (1 - squashAmt) * 0.1;
  // Tilt toward mouse
  const targetTilt = petMx > -900 ? Math.max(-0.12, Math.min(0.12, (petMx - x) / x * 0.15)) : 0;
  tiltAngle += (targetTilt - tiltAngle) * 0.08;

  const drawY = y + bounceOffset;

  ctx.clearRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";

  let shakeX = 0;
  if (pet.state === "refusing") {
    shakeX = Math.sin(frameCount * 1.5) * 6;
  }

  // Apply tilt transform around pet center
  ctx.save();
  ctx.translate(x + shakeX, drawY);
  ctx.rotate(tiltAngle);
  ctx.translate(-x, -drawY);


  // 阴影（带呼吸缩放）
  const shadowBreath = 1 + Math.sin(frameCount * 0.05) * 0.05;
  ctx.fillStyle = "rgba(15, 23, 42, 0.1)";
  ctx.beginPath();
  ctx.ellipse(x, drawY + size / 2 + 13, (size / 2.35) * shadowBreath, 6, 0, 0, Math.PI * 2);
  ctx.fill();

  // Body breathing
  const breathe = 1 + Math.sin(frameCount * 0.05) * 0.025;
  const squishState = pet.state === "happy" ? 1 + Math.sin(frameCount * 0.15) * 0.04 : 1;
  const stuffedExpand = pet.state === "stuffed" ? 1.15 : 1;
  const bw = size * breathe * squishState * stuffedExpand * (2 - squashAmt);
  const bh = size * breathe / squishState * squashAmt;

  // 身体颜色根据皮肤、动态主题和心情变化
  const theme = pet.visualTheme;
  const baseColor = theme.primary;
  const lighten = pet.happiness > 0 ? pet.happiness * 15 : pet.happiness * 10;

  // 身体光晕
  const glowAlpha = (pet.isAnimatedSkin ? 0.16 : 0.09) + Math.sin(frameCount * 0.03) * 0.04;
  ctx.fillStyle = hexToRgba(theme.accent, glowAlpha);
  ctx.beginPath();
  ctx.ellipse(x, drawY, bw / 2 + 13, bh / 2 + 12, 0, 0, Math.PI * 2);
  ctx.fill();

  drawEars(ctx, x, drawY, bw, bh, baseColor, theme, lighten);

  // 身体
  const bodyGradient = ctx.createLinearGradient(x - bw / 2, drawY - bh / 2, x + bw / 2, drawY + bh / 2);
  bodyGradient.addColorStop(0, adjustBrightness(theme.accent, lighten + 12));
  bodyGradient.addColorStop(0.5, adjustBrightness(baseColor, lighten));
  bodyGradient.addColorStop(1, adjustBrightness(theme.primaryDark, lighten - 3));
  ctx.fillStyle = bodyGradient;
  ctx.beginPath();
  roundRect(ctx, x - bw / 2, drawY - bh / 2, bw, bh, 22);
  ctx.fill();

  ctx.strokeStyle = "rgba(255, 255, 255, 0.45)";
  ctx.lineWidth = 1.25;
  ctx.stroke();

  // Hover glow ring
  if (isHovering) {
    ctx.strokeStyle = hexToRgba(theme.accent, 0.35 + Math.sin(frameCount * 0.1) * 0.1);
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.ellipse(x, drawY, bw / 2 + 6, bh / 2 + 5, 0, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Body highlights
  ctx.fillStyle = "rgba(255, 255, 255, 0.28)";
  ctx.beginPath();
  ctx.ellipse(x - 12, drawY - 16, 16, 8, -0.35, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "rgba(255, 255, 255, 0.18)";
  ctx.beginPath();
  ctx.ellipse(x + 12, drawY + 18, 18, 7, -0.15, 0, Math.PI * 2);
  ctx.fill();

  // Blush
  if (pet.happiness > 0) {
    const blushAlpha = Math.min(0.38, pet.happiness * 0.4);
    ctx.fillStyle = `rgba(255, 142, 162, ${blushAlpha})`;
    ctx.beginPath();
    ctx.ellipse(x - 25, drawY + 2, 9, 5, -0.15, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.ellipse(x + 25, drawY + 2, 9, 5, 0.15, 0, Math.PI * 2);
    ctx.fill();
  }

  // Eyes - with mouse tracking
  const eyeY = drawY - 10;
  const blinkFrame = frameCount % 150 < 5;
  ctx.fillStyle = "rgba(255, 255, 255, 0.96)";
  ctx.beginPath();

  if (pet.state === "stuffed") {
    // 满足的半闭眼
    ctx.arc(x - 14, eyeY + 2, 7, 0, Math.PI);
    ctx.arc(x + 14, eyeY + 2, 7, 0, Math.PI);
    ctx.fill();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.96)";
    ctx.lineWidth = 3;
    ctx.stroke();
  } else if (pet.state === "hungry") {
    // 期待的大圆眼 - 闪闪发光可爱风格
    // 大白眼框
    ctx.fillStyle = "rgba(255, 255, 255, 0.96)";
    ctx.beginPath();
    ctx.ellipse(x - 14, eyeY, 9, 10, 0, 0, Math.PI * 2);
    ctx.ellipse(x + 14, eyeY, 9, 10, 0, 0, Math.PI * 2);
    ctx.fill();
    // 大瞳孔（放大表示兴奋）
    ctx.fillStyle = "#263043";
    ctx.beginPath();
    ctx.arc(x - 13, eyeY + 1, 5.5, 0, Math.PI * 2);
    ctx.arc(x + 13, eyeY + 1, 5.5, 0, Math.PI * 2);
    ctx.fill();
    // 主高光（大亮点）
    const sparkleShift = Math.sin(frameCount * 0.12) * 0.8;
    ctx.fillStyle = "rgba(255, 255, 255, 0.92)";
    ctx.beginPath();
    ctx.arc(x - 15 + sparkleShift, eyeY - 1.5, 2.2, 0, Math.PI * 2);
    ctx.arc(x + 11 + sparkleShift, eyeY - 1.5, 2.2, 0, Math.PI * 2);
    ctx.fill();
    // 副高光（小亮点，微微跳动）
    const sparkle2 = Math.sin(frameCount * 0.18 + 1) * 0.6;
    ctx.fillStyle = "rgba(255, 255, 255, 0.7)";
    ctx.beginPath();
    ctx.arc(x - 11 + sparkle2, eyeY + 2, 1.2, 0, Math.PI * 2);
    ctx.arc(x + 15 + sparkle2, eyeY + 2, 1.2, 0, Math.PI * 2);
    ctx.fill();
  } else {
    ctx.ellipse(x - 14, eyeY, blinkFrame ? 7 : 7, blinkFrame ? 1 : 8, 0, 0, Math.PI * 2);
    ctx.ellipse(x + 14, eyeY, blinkFrame ? 7 : 7, blinkFrame ? 1 : 8, 0, 0, Math.PI * 2);
    ctx.fill();
  }

  if (!blinkFrame && pet.state !== "stuffed" && pet.state !== "hungry") {
    // Pupil tracks mouse position
    let pupilDx = 0, pupilDy = 0;
    if (petMx > -900) {
      const dx = petMx - x, dy = petMy - drawY;
      const dist = Math.sqrt(dx*dx + dy*dy);
      if (dist > 0) { pupilDx = (dx/dist) * Math.min(3, dist * 0.06); pupilDy = (dy/dist) * Math.min(2, dist * 0.04); }
    } else {
      pupilDx = Math.sin(frameCount * 0.02) * 2;
    }
    const pupilSize = pet.state === "happy" ? 4 : pet.state === "sleeping" ? 2 : 3;
    ctx.fillStyle = "#263043";
    ctx.beginPath();
    ctx.arc(x - 12 + pupilDx, eyeY + pupilDy, pupilSize, 0, Math.PI * 2);
    ctx.arc(x + 12 + pupilDx, eyeY + pupilDy, pupilSize, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "rgba(255,255,255,0.8)";
    ctx.beginPath();
    ctx.arc(x - 13 + pupilDx, eyeY - 1.5 + pupilDy, 1.5, 0, Math.PI * 2);
    ctx.arc(x + 11 + pupilDx, eyeY - 1.5 + pupilDy, 1.5, 0, Math.PI * 2);
    ctx.fill();
  }

  // 嘴巴
  ctx.strokeStyle = "#fff";
  ctx.lineWidth = 2;
  ctx.beginPath();
  if (pet.state === "happy") {
    // 大笑嘴
    ctx.arc(x, y + 8, 13, 0.1, Math.PI - 0.1);
    ctx.stroke();
    // 嘴巴里面
    ctx.fillStyle = "rgba(200, 80, 80, 0.3)";
    ctx.beginPath();
    ctx.arc(x, y + 8, 10, 0, Math.PI);
    ctx.fill();
  } else if (pet.state === "thinking") {
    // 思考嘴
    ctx.arc(x, drawY + 10, 5, 0, Math.PI * 2);
    ctx.stroke();
    // 旋转星星思考泡泡
    const bobbleY = drawY - 45 + Math.sin(frameCount * 0.08) * 4;
    drawThinkingBubbles(ctx, x, bobbleY, frameCount, theme);
    drawLegs(ctx, x, drawY, size, baseColor, lighten, frameCount);
    finishFrame(ctx);
    return;
  } else if (pet.state === "speaking") {
    const mouthH = 5 + Math.sin(frameCount * 0.3) * 4;
    ctx.ellipse(x, y + 10, 10, Math.max(2, mouthH), 0, 0, Math.PI * 2);
    ctx.stroke();
    ctx.fillStyle = "rgba(200, 80, 80, 0.2)";
    ctx.beginPath();
    ctx.ellipse(x, y + 10, 8, Math.max(1, mouthH - 1), 0, 0, Math.PI * 2);
    ctx.fill();
  } else if (pet.state === "sleeping") {
    drawSleeping(ctx, x, drawY, frameCount, theme);
    drawLegs(ctx, x, drawY, size, baseColor, lighten, frameCount);
    finishFrame(ctx);
    return;
  } else if (pet.state === "waving") {
    drawWaving(ctx, x, drawY, frameCount, size, baseColor, theme, lighten);
    drawLegs(ctx, x, drawY, size, baseColor, lighten, frameCount);
    finishFrame(ctx);
    return;
  } else if (pet.state === "confused") {
    // 波浪嘴
    ctx.beginPath();
    for (let i = 0; i <= 16; i++) {
      const px = x - 8 + i;
      const py = y + 12 + Math.sin(i * 0.8 + frameCount * 0.1) * 2;
      i === 0 ? ctx.moveTo(px, py) : ctx.lineTo(px, py);
    }
    ctx.stroke();
  } else if (pet.state === "hungry") {
    // 馋嘴张开
    ctx.arc(x, drawY + 8, 14, 0, Math.PI);
    ctx.stroke();
    ctx.fillStyle = "rgba(200, 80, 80, 0.4)";
    ctx.beginPath();
    ctx.arc(x, drawY + 8, 12, 0, Math.PI);
    ctx.fill();
    // 吞咽小舌头
    ctx.fillStyle = "rgba(255, 120, 120, 0.8)";
    ctx.beginPath();
    ctx.ellipse(x, drawY + 16 + Math.sin(frameCount * 0.4) * 2, 6, 4, 0, 0, Math.PI * 2);
    ctx.fill();
  } else if (pet.state === "stuffed") {
    // 满足的U型嘴
    ctx.arc(x, drawY + 8, 6, 0.2, Math.PI - 0.2);
    ctx.stroke();
  } else if (pet.state === "refusing") {
    // 拒绝的倒U嘴
    ctx.arc(x, drawY + 14, 6, Math.PI + 0.2, Math.PI * 2 - 0.2);
    ctx.stroke();
  } else {
    // idle 微笑
    ctx.arc(x, y + 10, 8, 0.15, Math.PI - 0.15);
  }
  ctx.stroke();

  // Legs
  drawLegs(ctx, x, drawY, size, baseColor, lighten, frameCount);
  finishFrame(ctx);
}

function finishFrame(ctx: CanvasRenderingContext2D) {
  ctx.restore(); // always restore the tilt transform save
  checkStateParticles();
  updateParticles();
  for (const p of particles) {
    drawParticle(ctx, p);
  }
}

function drawLegs(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, size: number,
  baseColor: string, lighten: number, frame: number,
) {
  const walkOffset = pet.state === "idle" ? Math.sin(frame * 0.1) * 2 : 0;
  const bounceOff = pet.state === "happy" ? Math.abs(Math.sin(frame * 0.15)) * 3 : 0;
  ctx.fillStyle = adjustBrightness(baseColor, lighten - 8);
  ctx.beginPath();
  ctx.ellipse(x - 15, y + size / 2 + 4 + walkOffset - bounceOff, 10, 6, 0, 0, Math.PI * 2);
  ctx.ellipse(x + 15, y + size / 2 + 4 - walkOffset - bounceOff, 10, 6, 0, 0, Math.PI * 2);
  ctx.fill();

  ctx.fillStyle = "rgba(255, 255, 255, 0.24)";
  ctx.beginPath();
  ctx.ellipse(x - 17, y + size / 2 + 2 + walkOffset - bounceOff, 3, 1.6, 0, 0, Math.PI * 2);
  ctx.ellipse(x + 13, y + size / 2 + 2 - walkOffset - bounceOff, 3, 1.6, 0, 0, Math.PI * 2);
  ctx.fill();
}

function drawEars(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, bw: number, bh: number,
  baseColor: string, theme: Theme, lighten: number,
) {
  const earLift = Math.sin(frameCount * (isHovering ? 0.2 : 0.04)) * (isHovering ? 3.5 : 1.3);
  const earY = y - bh / 2 + 8 + earLift;
  const earRadius = 13;
  const leftX = x - bw / 2 + 14;
  const rightX = x + bw / 2 - 14;

  for (const [earX, flip] of [[leftX, -1], [rightX, 1]] as const) {
    ctx.save();
    ctx.translate(earX, earY);
    ctx.rotate(flip * 0.34);
    ctx.fillStyle = adjustBrightness(baseColor, lighten - 3);
    ctx.beginPath();
    ctx.ellipse(0, 0, earRadius * 0.9, earRadius * 1.15, 0, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = hexToRgba(theme.accent, 0.34);
    ctx.beginPath();
    ctx.ellipse(0, 2, earRadius * 0.45, earRadius * 0.62, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
}

function drawThinkingBubbles(
  ctx: CanvasRenderingContext2D,
  x: number, bobbleY: number, frame: number, theme: Theme,
) {
  // 小圆→大圆 思考链
  const bubbles = [
    { ox: 22, oy: 10, r: 4 },
    { ox: 30, oy: 0, r: 6 },
    { ox: 38, oy: -10, r: 9 },
  ];
  for (const b of bubbles) {
    ctx.fillStyle = hexToRgba(theme.accent, 0.35);
    ctx.beginPath();
    ctx.arc(x + b.ox, bobbleY + b.oy, b.r, 0, Math.PI * 2);
    ctx.fill();
  }
  // 最大泡泡里旋转星星
  ctx.save();
  ctx.translate(x + 38, bobbleY - 10);
  ctx.rotate(frame * 0.05);
  ctx.fillStyle = hexToRgba(theme.primary, 0.7);
  ctx.beginPath();
  for (let i = 0; i < 5; i++) {
    const angle = (i * 4 * Math.PI) / 5 - Math.PI / 2;
    const method = i === 0 ? "moveTo" : "lineTo";
    ctx[method](Math.cos(angle) * 4, Math.sin(angle) * 4);
  }
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function drawSleeping(ctx: CanvasRenderingContext2D, x: number, y: number, frame: number, theme: any) {
  // 闭眼（弧线）
  ctx.strokeStyle = "#fff";
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.arc(x - 14, y - 10, 5, 0.2, Math.PI - 0.2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(x + 14, y - 10, 5, 0.2, Math.PI - 0.2);
  ctx.stroke();

  // 渐变 Zzz
  const zOff = Math.sin(frame * 0.04) * 6;
  const zAlpha = 0.4 + Math.sin(frame * 0.06) * 0.3;
  ctx.fillStyle = hexToRgba(theme.accent, zAlpha);
  ctx.font = "bold 16px sans-serif";
  ctx.fillText("Z", x + 25, y - 20 + zOff);
  ctx.font = "bold 12px sans-serif";
  ctx.fillText("z", x + 36, y - 32 + zOff * 0.8);
  ctx.font = "bold 9px sans-serif";
  ctx.fillText("z", x + 43, y - 42 + zOff * 0.5);

  // 睡觉嘴：小弧线
  ctx.strokeStyle = "#fff";
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.arc(x, y + 10, 6, 0.3, Math.PI - 0.3);
  ctx.stroke();
}

function drawWaving(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, frame: number, size: number,
  baseColor: string, theme: Theme, lighten: number,
) {
  // 微笑嘴
  ctx.strokeStyle = "#fff";
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.arc(x, y + 10, 10, 0.1, Math.PI - 0.1);
  ctx.stroke();

  // 挥手手臂
  const waveAngle = Math.sin(frame * 0.2) * 0.5;
  ctx.save();
  ctx.translate(x + size / 2, y - 10);
  ctx.rotate(-0.8 + waveAngle);
  const handGradient = ctx.createLinearGradient(-4, -12, 4, 18);
  handGradient.addColorStop(0, adjustBrightness(theme.accent, lighten + 8));
  handGradient.addColorStop(1, adjustBrightness(baseColor, lighten));
  ctx.fillStyle = handGradient;
  ctx.beginPath();
  ctx.roundRect(-4, -4, 8, 28, 4);
  ctx.fill();
  // 手掌
  ctx.beginPath();
  ctx.arc(0, -8, 7, 0, Math.PI * 2);
  ctx.fill();
  // 手掌高光
  ctx.fillStyle = "rgba(255,255,255,0.2)";
  ctx.beginPath();
  ctx.arc(-1, -9, 3, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

// ===== 工具函数 =====
function adjustBrightness(hex: string, percent: number): string {
  const num = parseInt(hex.replace("#", ""), 16);
  const r = Math.min(255, Math.max(0, ((num >> 16) & 0xff) + Math.round(percent * 2.55)));
  const g = Math.min(255, Math.max(0, ((num >> 8) & 0xff) + Math.round(percent * 2.55)));
  const b = Math.min(255, Math.max(0, (num & 0xff) + Math.round(percent * 2.55)));
  return `rgb(${r}, ${g}, ${b})`;
}

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

function isPetHit(e: MouseEvent) {
  const canvas = canvasRef.value;
  if (!canvas) return false;

  const rect = canvas.getBoundingClientRect();
  const px = e.clientX - rect.left;
  const py = e.clientY - rect.top;
  return isPetCanvasPoint(px, py, activeCharacter(), pet.state, pet.position);
}

async function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  if (!isPetHit(e)) return;
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
  const canvas = canvasRef.value;
  if (canvas) {
    const rect = canvas.getBoundingClientRect();
    petMx = e.clientX - rect.left;
    petMy = e.clientY - rect.top;
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
  if (!dragFromCanvas || e.button !== 0) return;
  dragFromCanvas = false;
  emit("dragging", false);
  if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
  squashAmt = 1;
  if (!dragStarted) {
    // Snappy Bounce!
    bounceVy = -8.0;
    bounceOffset = 0;
    spawnParticles('heart', 4, pet.position.x, pet.position.y - 30);
    emit("click");
  }
}

function onContextMenu(e: MouseEvent) {
  e.preventDefault();
  if (!isPetHit(e)) return;
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

onMounted(async () => {
  const canvas = canvasRef.value;
  if (canvas) {
    canvas.width = 120;
    canvas.height = 140;
    animate();
    if (!props.preview) {
      window.addEventListener("mousemove", onMouseMove);
      window.addEventListener("mouseup", onMouseUp);
      canvas.addEventListener("dblclick", resetPosition);
      canvas.addEventListener("mouseleave", () => { petMx = -999; petMy = -999; isHovering = false; });
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
  <canvas
    ref="canvasRef"
    class="pet-canvas"
    @mousedown="onMouseDown"
    @contextmenu="onContextMenu"
  />
</template>

<style scoped>
.pet-canvas {
  position: absolute;
  top: 0;
  left: 0;
  width: 120px;
  height: 140px;
  cursor: grab;
  pointer-events: auto;
}
.pet-canvas:active {
  cursor: grabbing;
}
</style>
