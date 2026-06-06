export const CUSTOM_PIXEL_PET_SETTING_KEY = "custom_pixel_pet_asset_id";

const FRAME_SIZE = 64;
const SHEET_COLUMNS = 10;
const MAX_SOURCE_SIZE = 6 * 1024 * 1024;

type Rgb = { r: number; g: number; b: number };

export type PixelPetAnimation = {
  frames: number[];
  fps: number;
  loop: boolean;
};

export type PixelPetManifest = {
  version: 1;
  renderer: "pixel-sprite";
  name: string;
  frameSize: { width: number; height: number };
  displayScale: number;
  sheet: { columns: number; rows: number };
  animations: Record<string, PixelPetAnimation>;
};

export type CustomPixelPetAsset = {
  id: string;
  name: string;
  kind: string;
  manifest: string;
  sprite_path: string;
  preview_path: string;
  created_at: number;
  updated_at: number;
};

export type GeneratedPixelPet = {
  name: string;
  manifest: PixelPetManifest;
  spriteDataUrl: string;
  previewDataUrl: string;
};

type FrameTransform = {
  offsetX?: number;
  offsetY?: number;
  scaleX?: number;
  scaleY?: number;
  alpha?: number;
  overlay?: (ctx: CanvasRenderingContext2D, cellX: number, cellY: number) => void;
};

function createCanvas(width: number, height: number) {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  return canvas;
}

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ""));
    reader.onerror = () => reject(reader.error || new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("Failed to load image"));
    image.src = src;
  });
}

function colorDistance(a: Rgb, b: Rgb) {
  const dr = a.r - b.r;
  const dg = a.g - b.g;
  const db = a.b - b.b;
  return Math.sqrt(dr * dr + dg * dg + db * db);
}

function estimateEdgeColor(data: Uint8ClampedArray, width: number, height: number): Rgb | null {
  let r = 0;
  let g = 0;
  let b = 0;
  let count = 0;

  const sample = (x: number, y: number) => {
    const index = (y * width + x) * 4;
    if (data[index + 3] < 128) return;
    r += data[index];
    g += data[index + 1];
    b += data[index + 2];
    count++;
  };

  for (let x = 0; x < width; x++) {
    sample(x, 0);
    sample(x, height - 1);
  }
  for (let y = 1; y < height - 1; y++) {
    sample(0, y);
    sample(width - 1, y);
  }

  if (count === 0) return null;
  return { r: r / count, g: g / count, b: b / count };
}

function removeEdgeBackground(imageData: ImageData) {
  const { data, width, height } = imageData;
  const bg = estimateEdgeColor(data, width, height);
  if (!bg) return;

  const visited = new Uint8Array(width * height);
  const queue: number[] = [];
  const enqueue = (x: number, y: number) => {
    if (x < 0 || x >= width || y < 0 || y >= height) return;
    const key = y * width + x;
    if (visited[key]) return;
    visited[key] = 1;
    queue.push(key);
  };

  for (let x = 0; x < width; x++) {
    enqueue(x, 0);
    enqueue(x, height - 1);
  }
  for (let y = 1; y < height - 1; y++) {
    enqueue(0, y);
    enqueue(width - 1, y);
  }

  while (queue.length) {
    const key = queue.shift() ?? 0;
    const index = key * 4;
    if (data[index + 3] < 20) continue;

    const current = { r: data[index], g: data[index + 1], b: data[index + 2] };
    if (colorDistance(current, bg) > 48) continue;

    data[index + 3] = 0;
    const x = key % width;
    const y = Math.floor(key / width);
    enqueue(x + 1, y);
    enqueue(x - 1, y);
    enqueue(x, y + 1);
    enqueue(x, y - 1);
  }
}

function quantizeChannel(value: number) {
  return Math.max(0, Math.min(255, Math.round(value / 32) * 32));
}

function applyPixelPalette(imageData: ImageData) {
  const { data } = imageData;
  for (let i = 0; i < data.length; i += 4) {
    if (data[i + 3] < 72) {
      data[i + 3] = 0;
      continue;
    }

    const contrast = 1.08;
    data[i] = quantizeChannel((data[i] - 128) * contrast + 128);
    data[i + 1] = quantizeChannel((data[i + 1] - 128) * contrast + 128);
    data[i + 2] = quantizeChannel((data[i + 2] - 128) * contrast + 128);
    data[i + 3] = 255;
  }
}

function addOutline(source: HTMLCanvasElement) {
  const ctx = source.getContext("2d");
  if (!ctx) return source;

  const imageData = ctx.getImageData(0, 0, source.width, source.height);
  const { data, width, height } = imageData;
  const output = new Uint8ClampedArray(data);
  const outline = { r: 36, g: 31, b: 42 };

  const alphaAt = (x: number, y: number) => {
    if (x < 0 || x >= width || y < 0 || y >= height) return 0;
    return data[(y * width + x) * 4 + 3];
  };

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const index = (y * width + x) * 4;
      if (data[index + 3] > 0) continue;
      if (
        alphaAt(x + 1, y) > 0 ||
        alphaAt(x - 1, y) > 0 ||
        alphaAt(x, y + 1) > 0 ||
        alphaAt(x, y - 1) > 0
      ) {
        output[index] = outline.r;
        output[index + 1] = outline.g;
        output[index + 2] = outline.b;
        output[index + 3] = 255;
      }
    }
  }

  const outlined = createCanvas(width, height);
  const outlinedCtx = outlined.getContext("2d");
  outlinedCtx?.putImageData(new ImageData(output, width, height), 0, 0);
  return outlined;
}

function createBaseSprite(image: HTMLImageElement) {
  const canvas = createCanvas(FRAME_SIZE, FRAME_SIZE);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is unavailable");

  ctx.clearRect(0, 0, FRAME_SIZE, FRAME_SIZE);
  ctx.imageSmoothingEnabled = true;
  const maxSize = FRAME_SIZE - 8;
  const scale = Math.min(maxSize / image.naturalWidth, maxSize / image.naturalHeight);
  const drawW = Math.max(1, Math.round(image.naturalWidth * scale));
  const drawH = Math.max(1, Math.round(image.naturalHeight * scale));
  const drawX = Math.round((FRAME_SIZE - drawW) / 2);
  const drawY = Math.round((FRAME_SIZE - drawH) / 2);
  ctx.drawImage(image, drawX, drawY, drawW, drawH);

  const imageData = ctx.getImageData(0, 0, FRAME_SIZE, FRAME_SIZE);
  removeEdgeBackground(imageData);
  applyPixelPalette(imageData);
  ctx.clearRect(0, 0, FRAME_SIZE, FRAME_SIZE);
  ctx.putImageData(imageData, 0, 0);

  return addOutline(canvas);
}

function drawSpark(ctx: CanvasRenderingContext2D, x: number, y: number, color: string) {
  ctx.fillStyle = color;
  ctx.fillRect(x, y - 3, 2, 8);
  ctx.fillRect(x - 3, y, 8, 2);
}

function drawDots(ctx: CanvasRenderingContext2D, cellX: number, cellY: number, frame: number) {
  ctx.fillStyle = "#f8fafc";
  for (let i = 0; i <= frame % 3; i++) {
    ctx.fillRect(cellX + 42 + i * 5, cellY + 12, 3, 3);
  }
  ctx.fillStyle = "#334155";
  ctx.fillRect(cellX + 41, cellY + 11, 16, 5);
}

function drawMouth(ctx: CanvasRenderingContext2D, cellX: number, cellY: number, open: boolean) {
  ctx.fillStyle = "#30233a";
  ctx.fillRect(cellX + 29, cellY + 42, open ? 7 : 5, open ? 5 : 2);
  ctx.fillStyle = "#fb7185";
  if (open) ctx.fillRect(cellX + 31, cellY + 45, 3, 1);
}

function drawFrame(
  sheetCtx: CanvasRenderingContext2D,
  base: HTMLCanvasElement,
  index: number,
  transform: FrameTransform = {},
) {
  const cellX = (index % SHEET_COLUMNS) * FRAME_SIZE;
  const cellY = Math.floor(index / SHEET_COLUMNS) * FRAME_SIZE;
  sheetCtx.clearRect(cellX, cellY, FRAME_SIZE, FRAME_SIZE);
  sheetCtx.save();
  sheetCtx.globalAlpha = transform.alpha ?? 1;
  sheetCtx.imageSmoothingEnabled = false;
  sheetCtx.translate(cellX + FRAME_SIZE / 2 + (transform.offsetX ?? 0), cellY + FRAME_SIZE / 2 + (transform.offsetY ?? 0));
  sheetCtx.scale(transform.scaleX ?? 1, transform.scaleY ?? 1);
  sheetCtx.drawImage(base, -FRAME_SIZE / 2, -FRAME_SIZE / 2);
  sheetCtx.restore();
  transform.overlay?.(sheetCtx, cellX, cellY);
}

function buildSpriteSheet(base: HTMLCanvasElement) {
  const frameCount = 40;
  const rows = Math.ceil(frameCount / SHEET_COLUMNS);
  const sheet = createCanvas(SHEET_COLUMNS * FRAME_SIZE, rows * FRAME_SIZE);
  const ctx = sheet.getContext("2d");
  if (!ctx) throw new Error("Canvas is unavailable");
  ctx.imageSmoothingEnabled = false;

  const frames: FrameTransform[] = [
    { offsetY: 0 },
    { offsetY: -1 },
    { offsetY: 0 },
    { offsetY: 1 },
    { offsetY: -3, scaleX: 1.04, scaleY: 0.96, overlay: (c, x, y) => drawSpark(c, x + 48, y + 16, "#facc15") },
    { offsetY: -7, scaleX: 0.96, scaleY: 1.06, overlay: (c, x, y) => drawSpark(c, x + 16, y + 20, "#fb7185") },
    { offsetY: -3, scaleX: 1.03, scaleY: 0.97, overlay: (c, x, y) => drawSpark(c, x + 50, y + 24, "#67e8f9") },
    { offsetY: 1 },
    { overlay: (c, x, y) => drawDots(c, x, y, 0) },
    { offsetY: -1, overlay: (c, x, y) => drawDots(c, x, y, 1) },
    { overlay: (c, x, y) => drawDots(c, x, y, 2) },
    { offsetY: 1, overlay: (c, x, y) => drawDots(c, x, y, 3) },
    { overlay: (c, x, y) => drawMouth(c, x, y, false) },
    { offsetY: -1, overlay: (c, x, y) => drawMouth(c, x, y, true) },
    { overlay: (c, x, y) => drawMouth(c, x, y, false) },
    { offsetY: 1, overlay: (c, x, y) => drawMouth(c, x, y, true) },
    { offsetY: 4, alpha: 0.86, scaleX: 0.98, scaleY: 0.94, overlay: (c, x, y) => { c.fillStyle = "#e2e8f0"; c.fillRect(x + 45, y + 11, 9, 3); c.fillRect(x + 50, y + 6, 7, 3); } },
    { offsetY: 5, alpha: 0.8, scaleX: 0.98, scaleY: 0.92, overlay: (c, x, y) => { c.fillStyle = "#e2e8f0"; c.fillRect(x + 43, y + 14, 9, 3); c.fillRect(x + 49, y + 9, 7, 3); } },
    { offsetY: 4, alpha: 0.84, scaleX: 0.98, scaleY: 0.94, overlay: (c, x, y) => { c.fillStyle = "#e2e8f0"; c.fillRect(x + 44, y + 12, 9, 3); } },
    { offsetY: 5, alpha: 0.78, scaleX: 0.98, scaleY: 0.92 },
    { offsetX: -3, scaleX: 1.08, scaleY: 0.96 },
    { offsetX: 3, scaleX: 1.08, scaleY: 0.96 },
    { offsetX: -2, scaleX: 1.04, scaleY: 0.98 },
    { offsetX: 2, scaleX: 1.04, scaleY: 0.98 },
    { overlay: (c, x, y) => { c.fillStyle = "#334155"; c.fillRect(x + 18, y + 49, 28, 7); c.fillStyle = "#94a3b8"; c.fillRect(x + 22, y + 51, 4, 2); c.fillRect(x + 30, y + 51, 4, 2); } },
    { offsetY: -1, overlay: (c, x, y) => { c.fillStyle = "#334155"; c.fillRect(x + 18, y + 49, 28, 7); c.fillStyle = "#38bdf8"; c.fillRect(x + 39, y + 50, 3, 3); } },
    { overlay: (c, x, y) => { c.fillStyle = "#334155"; c.fillRect(x + 18, y + 49, 28, 7); c.fillStyle = "#fbbf24"; c.fillRect(x + 28, y + 51, 4, 2); } },
    { offsetY: 1, overlay: (c, x, y) => { c.fillStyle = "#334155"; c.fillRect(x + 18, y + 49, 28, 7); } },
    { scaleX: 1.03, scaleY: 1.02, overlay: (c, x, y) => drawSpark(c, x + 46, y + 18, "#fbbf24") },
    { scaleX: 1.06, scaleY: 1.04, overlay: (c, x, y) => drawSpark(c, x + 16, y + 24, "#f97316") },
    { scaleX: 1.03, scaleY: 1.02, overlay: (c, x, y) => drawSpark(c, x + 48, y + 28, "#fde68a") },
    { scaleX: 1.01, scaleY: 1.01 },
    { offsetY: -1, overlay: (c, x, y) => { c.fillStyle = "#fb7185"; c.fillRect(x + 16, y + 14, 5, 4); c.fillRect(x + 45, y + 14, 5, 4); } },
    { offsetY: -2, overlay: (c, x, y) => { c.fillStyle = "#fda4af"; c.fillRect(x + 17, y + 12, 5, 4); c.fillRect(x + 44, y + 12, 5, 4); } },
    { offsetY: -1, overlay: (c, x, y) => { c.fillStyle = "#fb7185"; c.fillRect(x + 18, y + 14, 5, 4); c.fillRect(x + 43, y + 14, 5, 4); } },
    { offsetY: 1 },
    { offsetX: -3, overlay: (c, x, y) => { c.fillStyle = "#ef4444"; c.fillRect(x + 45, y + 12, 10, 3); } },
    { offsetX: 3, overlay: (c, x, y) => { c.fillStyle = "#ef4444"; c.fillRect(x + 13, y + 12, 10, 3); } },
    { offsetX: -2, overlay: (c, x, y) => { c.fillStyle = "#ef4444"; c.fillRect(x + 45, y + 14, 10, 3); } },
    { offsetX: 2 },
  ];

  frames.forEach((frame, index) => drawFrame(ctx, base, index, frame));
  return { sheet, rows };
}

function buildManifest(name: string, rows: number): PixelPetManifest {
  return {
    version: 1,
    renderer: "pixel-sprite",
    name,
    frameSize: { width: FRAME_SIZE, height: FRAME_SIZE },
    displayScale: 1.65,
    sheet: { columns: SHEET_COLUMNS, rows },
    animations: {
      idle: { frames: [0, 1, 2, 3], fps: 7, loop: true },
      happy: { frames: [4, 5, 6, 7], fps: 12, loop: false },
      thinking: { frames: [8, 9, 10, 11], fps: 6, loop: true },
      speaking: { frames: [12, 13, 14, 15], fps: 10, loop: true },
      sleeping: { frames: [16, 17, 18, 19], fps: 5, loop: true },
      dragging: { frames: [20, 21, 22, 23], fps: 9, loop: true },
      working: { frames: [24, 25, 26, 27], fps: 8, loop: true },
      hungry: { frames: [28, 29, 30, 31], fps: 8, loop: true },
      stuffed: { frames: [32, 33, 34, 35], fps: 9, loop: false },
      refusing: { frames: [36, 37, 38, 39], fps: 12, loop: false },
    },
  };
}

function buildPreview(base: HTMLCanvasElement) {
  const preview = createCanvas(96, 96);
  const ctx = preview.getContext("2d");
  if (!ctx) throw new Error("Canvas is unavailable");
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(base, 0, 0, FRAME_SIZE, FRAME_SIZE, 0, 0, 96, 96);
  return preview.toDataURL("image/png");
}

export async function generateCustomPixelPetFromFile(file: File): Promise<GeneratedPixelPet> {
  if (!file.type.startsWith("image/")) {
    throw new Error("Please choose an image file.");
  }
  if (file.size > MAX_SOURCE_SIZE) {
    throw new Error("Image is too large.");
  }

  const sourceDataUrl = await readFileAsDataUrl(file);
  const image = await loadImage(sourceDataUrl);
  const base = createBaseSprite(image);
  const { sheet, rows } = buildSpriteSheet(base);
  const name = file.name.replace(/\.[^.]+$/, "").slice(0, 24) || "Custom Pixel Pet";
  const manifest = buildManifest(name, rows);

  return {
    name,
    manifest,
    spriteDataUrl: sheet.toDataURL("image/png"),
    previewDataUrl: buildPreview(base),
  };
}

export function parsePixelPetManifest(raw: string | undefined): PixelPetManifest | null {
  if (!raw) return null;
  try {
    const manifest = JSON.parse(raw) as PixelPetManifest;
    if (
      manifest?.renderer !== "pixel-sprite" ||
      manifest.frameSize?.width <= 0 ||
      manifest.frameSize?.height <= 0 ||
      !manifest.animations?.idle
    ) {
      return null;
    }
    return manifest;
  } catch {
    return null;
  }
}
