import type { PetState } from "../stores/pet";
import type { PetCharacterId } from "./petCharacters";

type Point = { x: number; y: number };

function isInEllipse(px: number, py: number, cx: number, cy: number, rx: number, ry: number) {
  const dx = (px - cx) / rx;
  const dy = (py - cy) / ry;
  return dx * dx + dy * dy <= 1;
}

function isInRoundedRect(
  px: number,
  py: number,
  left: number,
  top: number,
  width: number,
  height: number,
  radius: number,
) {
  const right = left + width;
  const bottom = top + height;
  if (px < left || px > right || py < top || py > bottom) return false;
  const cx = px < left + radius ? left + radius : px > right - radius ? right - radius : px;
  const cy = py < top + radius ? top + radius : py > bottom - radius ? bottom - radius : py;
  const dx = px - cx;
  const dy = py - cy;
  return dx * dx + dy * dy <= radius * radius;
}

function isClassicHit(px: number, py: number, state: PetState, position: Point) {
  const { x, y } = position;
  const padding = 5;
  const size = 80;
  const body = isInRoundedRect(
    px,
    py,
    x - size / 2 - padding,
    y - size / 2 - padding,
    size + padding * 2,
    size + padding * 2,
    24 + padding,
  );
  const leftEar = isInEllipse(px, py, x - size / 2 + 14, y - size / 2 + 8, 16, 19);
  const rightEar = isInEllipse(px, py, x + size / 2 - 14, y - size / 2 + 8, 16, 19);
  const leftFoot = isInEllipse(px, py, x - 15, y + size / 2 + 4, 13, 9);
  const rightFoot = isInEllipse(px, py, x + 15, y + size / 2 + 4, 13, 9);
  const wavingHand =
    state === "waving" &&
    isInRoundedRect(px, py, x + size / 2 - 8, y - 36, 28, 54, 13);

  return body || leftEar || rightEar || leftFoot || rightFoot || wavingHand;
}

export function isPetCanvasPoint(
  px: number,
  py: number,
  character: PetCharacterId,
  state: PetState,
  position: Point = { x: 60, y: 60 },
) {
  if (character === "daimao-batiao") {
    return isInRoundedRect(px, py, 8, 12, 104, 122, 20);
  }

  if (character === "custom-pixel") {
    return isInRoundedRect(px, py, 12, 16, 96, 112, 12);
  }

  return isClassicHit(px, py, state, position);
}
