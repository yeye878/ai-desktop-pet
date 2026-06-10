export type Rgb = { r: number; g: number; b: number };
export type PixelBounds = { left: number; top: number; right: number; bottom: number };

type CornerName = "top-left" | "top-right" | "bottom-left" | "bottom-right";

type ColorBucket = {
  r: number;
  g: number;
  b: number;
  count: number;
  corners: Set<CornerName>;
};

function colorDistance(a: Rgb, b: Rgb) {
  const dr = a.r - b.r;
  const dg = a.g - b.g;
  const db = a.b - b.b;
  return Math.sqrt(dr * dr + dg * dg + db * db);
}

function normalizeBounds(bounds: PixelBounds | undefined, width: number, height: number): PixelBounds | null {
  const normalized = {
    left: Math.max(0, Math.min(width, Math.floor(bounds?.left ?? 0))),
    top: Math.max(0, Math.min(height, Math.floor(bounds?.top ?? 0))),
    right: Math.max(0, Math.min(width, Math.ceil(bounds?.right ?? width))),
    bottom: Math.max(0, Math.min(height, Math.ceil(bounds?.bottom ?? height))),
  };

  if (normalized.right <= normalized.left || normalized.bottom <= normalized.top) return null;
  return normalized;
}

function addBucketSample(
  buckets: Map<string, ColorBucket>,
  sample: Rgb,
  corner: CornerName,
  bucketSize: number,
) {
  const key = [
    Math.floor(sample.r / bucketSize),
    Math.floor(sample.g / bucketSize),
    Math.floor(sample.b / bucketSize),
  ].join(":");
  const bucket = buckets.get(key) || { r: 0, g: 0, b: 0, count: 0, corners: new Set<CornerName>() };
  bucket.r += sample.r;
  bucket.g += sample.g;
  bucket.b += sample.b;
  bucket.count++;
  bucket.corners.add(corner);
  buckets.set(key, bucket);
}

function collectCornerBuckets(data: Uint8ClampedArray, width: number, bounds: PixelBounds) {
  const areaWidth = bounds.right - bounds.left;
  const areaHeight = bounds.bottom - bounds.top;
  const cornerSize = Math.max(2, Math.min(12, Math.floor(Math.min(areaWidth, areaHeight) * 0.12)));
  const corners: Array<{ name: CornerName; left: number; top: number; right: number; bottom: number }> = [
    {
      name: "top-left",
      left: bounds.left,
      top: bounds.top,
      right: Math.min(bounds.right, bounds.left + cornerSize),
      bottom: Math.min(bounds.bottom, bounds.top + cornerSize),
    },
    {
      name: "top-right",
      left: Math.max(bounds.left, bounds.right - cornerSize),
      top: bounds.top,
      right: bounds.right,
      bottom: Math.min(bounds.bottom, bounds.top + cornerSize),
    },
    {
      name: "bottom-left",
      left: bounds.left,
      top: Math.max(bounds.top, bounds.bottom - cornerSize),
      right: Math.min(bounds.right, bounds.left + cornerSize),
      bottom: bounds.bottom,
    },
    {
      name: "bottom-right",
      left: Math.max(bounds.left, bounds.right - cornerSize),
      top: Math.max(bounds.top, bounds.bottom - cornerSize),
      right: bounds.right,
      bottom: bounds.bottom,
    },
  ];
  const buckets = new Map<string, ColorBucket>();
  let opaqueSamples = 0;

  for (const corner of corners) {
    for (let y = corner.top; y < corner.bottom; y++) {
      for (let x = corner.left; x < corner.right; x++) {
        const index = (y * width + x) * 4;
        if (data[index + 3] < 128) continue;
        opaqueSamples++;
        addBucketSample(
          buckets,
          { r: data[index], g: data[index + 1], b: data[index + 2] },
          corner.name,
          24,
        );
      }
    }
  }

  return { buckets, corners, opaqueSamples };
}

function estimateBackgroundColors(data: Uint8ClampedArray, width: number, bounds: PixelBounds) {
  const { buckets, opaqueSamples } = collectCornerBuckets(data, width, bounds);
  if (opaqueSamples === 0) return [];

  const ranked = Array.from(buckets.values()).sort((a, b) => {
    const supportDelta = b.corners.size - a.corners.size;
    return supportDelta || b.count - a.count;
  });
  const minCount = Math.max(2, opaqueSamples * 0.05);
  const sharedCornerColors = ranked.filter((bucket) => bucket.count >= minCount && bucket.corners.size >= 3);
  const dominantColors = ranked.filter((bucket) => bucket.count >= opaqueSamples * 0.65);
  const candidates = sharedCornerColors.length > 0 ? sharedCornerColors : dominantColors;

  return candidates
    .slice(0, 3)
    .map((bucket) => ({
      r: bucket.r / bucket.count,
      g: bucket.g / bucket.count,
      b: bucket.b / bucket.count,
    }));
}

export function removeEdgeBackground(imageData: ImageData, bounds?: PixelBounds) {
  const { data, width, height } = imageData;
  const area = normalizeBounds(bounds, width, height);
  if (!area) return;
  const bgColors = estimateBackgroundColors(data, width, area);
  const { corners } = collectCornerBuckets(data, width, area);
  const visited = new Uint8Array(width * height);
  const queue: number[] = [];
  const enqueue = (x: number, y: number) => {
    if (x < area.left || x >= area.right || y < area.top || y >= area.bottom) return;
    const key = y * width + x;
    if (visited[key]) return;
    visited[key] = 1;
    queue.push(key);
  };

  for (const corner of corners) {
    for (let y = corner.top; y < corner.bottom; y++) {
      for (let x = corner.left; x < corner.right; x++) {
        enqueue(x, y);
      }
    }
  }

  let cursor = 0;
  while (cursor < queue.length) {
    const key = queue[cursor++] ?? 0;
    const index = key * 4;
    const x = key % width;
    const y = Math.floor(key / width);
    if (data[index + 3] < 20) {
      data[index] = 0;
      data[index + 1] = 0;
      data[index + 2] = 0;
      data[index + 3] = 0;
      enqueue(x + 1, y);
      enqueue(x - 1, y);
      enqueue(x, y + 1);
      enqueue(x, y - 1);
      continue;
    }

    if (bgColors.length === 0) continue;

    const current = { r: data[index], g: data[index + 1], b: data[index + 2] };
    const distance = Math.min(...bgColors.map((bg) => colorDistance(current, bg)));
    const tolerance = 42 + (data[index + 3] < 220 ? 10 : 0);
    if (distance > tolerance) continue;

    data[index] = 0;
    data[index + 1] = 0;
    data[index + 2] = 0;
    data[index + 3] = 0;
    enqueue(x + 1, y);
    enqueue(x - 1, y);
    enqueue(x, y + 1);
    enqueue(x, y - 1);
  }
}
