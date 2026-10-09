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
  const padding = 6;
  const size = 80;
  const body = isInRoundedRect(
    px,
    py,
    x - size / 2 - padding,
    y - size / 2 - padding,
    size + padding * 2,
    size + padding * 2,
    26 + padding,
  );
  const leftEar = isInEllipse(px, py, x - 20, y - 34, 18, 24);
  const rightEar = isInEllipse(px, py, x + 20, y - 34, 18, 24);
  const leftFoot = isInEllipse(px, py, x - 16, y + size / 2 + 4, 14, 10);
  const rightFoot = isInEllipse(px, py, x + 16, y + size / 2 + 4, 14, 10);
  const tail = isInEllipse(px, py, x - 30, y + 18, 14, 14);
  const desk = isInRoundedRect(px, py, x - 52, y + 18, 104, 34, 8);
  const wavingHand =
    state === "waving" &&
    isInRoundedRect(px, py, x + size / 2 - 10, y - 36, 32, 54, 14);

  return body || leftEar || rightEar || leftFoot || rightFoot || tail || desk || wavingHand;
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
