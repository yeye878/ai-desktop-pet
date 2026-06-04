from __future__ import annotations

import shutil
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from scipy import ndimage as ndi


SOURCE_FILES = [
    "2768a33a938b81d910160d32876cf9c5.jpg",
    "3806a11618b1ccc2650a52826561c29d.jpg",
    "a5aed623995060364f097afee5d42203.jpg",
    "bdf22ad31976884e8a33926a8b91a3a6.jpg",
    "c58f011aa9d45cf4d9dc6e593c812a89.jpg",
]
VIDEO_FILE = "4e513af41b45d29692ff6abeb4c1e0ae.mp4"
TARGET_SIZE = (160, 180)


def find_source(name: str) -> Path:
    desktop = Path.home() / "Desktop"
    matches = list(desktop.rglob(name))
    if not matches:
        raise FileNotFoundError(f"Could not find {name!r} under {desktop}")
    return matches[0]


def ellipse_mask(shape: tuple[int, int], cx: float, cy: float, rx: float, ry: float) -> np.ndarray:
    h, w = shape
    yy, xx = np.ogrid[:h, :w]
    return ((xx - cx * w) / (rx * w)) ** 2 + ((yy - cy * h) / (ry * h)) ** 2 <= 1


def subject_hint(index: int, shape: tuple[int, int]) -> np.ndarray:
    h, w = shape
    hint = np.zeros((h, w), dtype=bool)

    def add_ellipse(cx: float, cy: float, rx: float, ry: float) -> None:
        nonlocal hint
        hint |= ellipse_mask(shape, cx, cy, rx, ry)

    if index == 1:
        add_ellipse(0.50, 0.60, 0.48, 0.30)
        add_ellipse(0.11, 0.78, 0.12, 0.12)
        add_ellipse(0.91, 0.80, 0.13, 0.14)
    elif index == 2:
        add_ellipse(0.56, 0.61, 0.43, 0.34)
        add_ellipse(0.18, 0.64, 0.15, 0.28)
        add_ellipse(0.09, 0.74, 0.07, 0.22)
    elif index == 3:
        add_ellipse(0.58, 0.57, 0.39, 0.43)
        add_ellipse(0.24, 0.60, 0.18, 0.34)
        add_ellipse(0.51, 0.16, 0.10, 0.14)
        add_ellipse(0.46, 0.94, 0.08, 0.05)
        add_ellipse(0.69, 0.91, 0.07, 0.05)
    elif index == 4:
        add_ellipse(0.53, 0.63, 0.35, 0.34)
        add_ellipse(0.24, 0.68, 0.13, 0.27)
        add_ellipse(0.61, 0.31, 0.28, 0.09)
    elif index == 5:
        add_ellipse(0.38, 0.44, 0.42, 0.27)
        add_ellipse(0.84, 0.42, 0.17, 0.36)
        add_ellipse(0.58, 0.69, 0.11, 0.20)
        add_ellipse(0.23, 0.73, 0.12, 0.18)

    return hint


def manual_mask_for_four(img: Image.Image) -> np.ndarray:
    w, h = img.size
    mask = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(mask)

    def xy(points: list[tuple[float, float]]) -> list[tuple[int, int]]:
        return [(int(x * w), int(y * h)) for x, y in points]

    draw.ellipse((int(0.17 * w), int(0.39 * h), int(0.87 * w), int(0.90 * h)), fill=255)
    draw.ellipse((int(0.12 * w), int(0.51 * h), int(0.39 * w), int(0.89 * h)), fill=255)
    draw.polygon(
        xy([
            (0.31, 0.36),
            (0.39, 0.28),
            (0.54, 0.27),
            (0.70, 0.31),
            (0.84, 0.40),
            (0.79, 0.48),
            (0.62, 0.42),
            (0.40, 0.41),
        ]),
        fill=255,
    )
    draw.ellipse((int(0.39 * w), int(0.78 * h), int(0.77 * w), int(0.91 * h)), fill=255)
    return np.asarray(mask.filter(ImageFilter.GaussianBlur(4))) > 32


def build_alpha_mask(img: Image.Image, index: int) -> np.ndarray:
    if index == 4:
        return manual_mask_for_four(img)

    rgb = np.asarray(img.convert("RGB"))
    h, w = rgb.shape[:2]
    gray = (0.299 * rgb[:, :, 0] + 0.587 * rgb[:, :, 1] + 0.114 * rgb[:, :, 2]).astype(np.uint8)
    yy = np.indices((h, w))[0]

    dark = gray < 155
    nonwhite = np.any(rgb < 242, axis=2)
    if index == 1:
        orange_floor = (
            (rgb[:, :, 0] > 210)
            & (rgb[:, :, 1] > 150)
            & (rgb[:, :, 2] < 145)
            & (yy > h * 0.65)
        )
        nonwhite &= ~orange_floor

    low_text_zone = (yy > h * 0.83) & (gray > 165) & (gray < 245)
    nonwhite &= ~low_text_zone

    seed = ndi.binary_opening(dark | nonwhite, structure=np.ones((2, 2)))
    hint = subject_hint(index, (h, w))

    labels, _ = ndi.label(seed)
    keep_seed = np.zeros_like(seed)
    for component_index, region in enumerate(ndi.find_objects(labels), start=1):
        if region is None:
            continue
        component = labels[region] == component_index
        area = int(component.sum())
        if area < h * w * 0.0003:
            continue
        overlap = int((component & hint[region]).sum())
        if overlap > max(20, area * 0.04):
            keep_seed[region] |= component

    mask = keep_seed | (
        hint & ndi.binary_dilation(keep_seed, iterations=max(20, int(min(h, w) * 0.04)))
    )
    mask = ndi.binary_dilation(mask, iterations=4)
    mask = ndi.binary_closing(mask, structure=np.ones((35, 35)))
    mask = ndi.binary_fill_holes(mask)

    labels, _ = ndi.label(mask)
    candidates: list[tuple[int, int, int, object]] = []
    for component_index, region in enumerate(ndi.find_objects(labels), start=1):
        if region is None:
            continue
        component = labels[region] == component_index
        area = int(component.sum())
        if area < h * w * 0.001:
            continue
        overlap = int((component & hint[region]).sum())
        candidates.append((overlap, area, component_index, region))

    if not candidates:
        return mask

    candidates.sort(reverse=True)
    final = labels == candidates[0][2]
    core = ndi.binary_dilation(keep_seed, iterations=max(16, int(min(h, w) * 0.035))) | hint
    final &= ndi.binary_dilation(core, iterations=max(12, int(min(h, w) * 0.025)))
    final = ndi.binary_closing(final, structure=np.ones((15, 15)))
    final = ndi.binary_fill_holes(final)
    final = ndi.binary_opening(final, structure=np.ones((3, 3)))
    return final


def remove_tiny_top_artifacts(canvas: Image.Image) -> Image.Image:
    arr = np.asarray(canvas).copy()
    alpha_mask = arr[:, :, 3] > 10
    labels, _ = ndi.label(alpha_mask)
    components: list[tuple[int, int, tuple[int, int, int, int]]] = []

    for component_index, region in enumerate(ndi.find_objects(labels), start=1):
        if region is None:
            continue
        ys, xs = region
        area = int((labels[region] == component_index).sum())
        components.append((area, component_index, (xs.start, ys.start, xs.stop, ys.stop)))

    if not components:
        return canvas

    components.sort(reverse=True)
    main_area = components[0][0]
    keep = np.zeros_like(alpha_mask)
    h, _ = alpha_mask.shape
    for area, component_index, bbox in components:
        _, y1, _, y2 = bbox
        top_small = y2 < h * 0.43 and area < main_area * 0.18
        if component_index == components[0][1] or not top_small:
            keep |= labels == component_index

    clean_alpha = np.asarray(
        Image.fromarray((keep.astype(np.uint8) * 255), "L").filter(ImageFilter.GaussianBlur(0.35))
    )
    arr[:, :, 3] = np.minimum(arr[:, :, 3], clean_alpha)
    return Image.fromarray(arr, "RGBA")


def normalize_cutout(img: Image.Image, mask: np.ndarray, target: tuple[int, int] = TARGET_SIZE) -> Image.Image:
    rgba = img.convert("RGBA")
    alpha = Image.fromarray((mask.astype(np.uint8) * 255), "L").filter(ImageFilter.GaussianBlur(0.75))
    rgba.putalpha(alpha)

    alpha_array = np.asarray(alpha)
    ys, xs = np.where(alpha_array > 8)
    x1, x2 = xs.min(), xs.max() + 1
    y1, y2 = ys.min(), ys.max() + 1
    source_pad = max(5, int(min(img.size) * 0.018))
    x1 = max(0, x1 - source_pad)
    y1 = max(0, y1 - source_pad)
    x2 = min(img.width, x2 + source_pad)
    y2 = min(img.height, y2 + source_pad)

    crop = rgba.crop((x1, y1, x2, y2))
    target_w, target_h = target
    pad = 8
    scale = min((target_w - pad * 2) / crop.width, (target_h - pad * 2) / crop.height)
    new_size = (max(1, int(crop.width * scale)), max(1, int(crop.height * scale)))
    crop = crop.resize(new_size, Image.Resampling.LANCZOS)

    canvas = Image.new("RGBA", target, (0, 0, 0, 0))
    canvas.alpha_composite(crop, ((target_w - new_size[0]) // 2, target_h - new_size[1] - pad))
    return remove_tiny_top_artifacts(canvas)


def write_contact_sheet(still_dir: Path, output: Path) -> None:
    images = [Image.open(still_dir / f"still-{index:02}.png").convert("RGBA") for index in range(1, 6)]
    cell_w, cell_h = 200, 220
    sheet = Image.new("RGBA", (cell_w * len(images), cell_h), (255, 255, 255, 255))
    draw = ImageDraw.Draw(sheet)
    for index, image in enumerate(images):
        x0 = index * cell_w
        for y in range(0, cell_h, 10):
            for x in range(x0, x0 + cell_w, 10):
                color = (232, 236, 242, 255) if ((x // 10 + y // 10) % 2 == 0) else (255, 255, 255, 255)
                draw.rectangle([x, y, x + 9, y + 9], fill=color)
        sheet.alpha_composite(image, (x0 + (cell_w - image.width) // 2, (cell_h - image.height) // 2))
        draw.text((x0 + 8, 8), f"still-{index:02}", fill=(20, 24, 36, 255))
    sheet.convert("RGB").save(output, quality=95)


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    asset_root = root / "src" / "assets" / "pets" / "daimao-batiao"
    still_dir = asset_root / "stills"
    still_dir.mkdir(parents=True, exist_ok=True)

    for index, source_name in enumerate(SOURCE_FILES, start=1):
        source = find_source(source_name)
        image = Image.open(source).convert("RGB")
        mask = build_alpha_mask(image, index)
        cutout = normalize_cutout(image, mask)
        cutout.save(still_dir / f"still-{index:02}.png")

    video_source = find_source(VIDEO_FILE)
    shutil.copy2(video_source, asset_root / "daimao-batiao.mp4")
    write_contact_sheet(still_dir, asset_root / "preview-contact-sheet.jpg")
    print(f"Wrote assets to {asset_root}")


if __name__ == "__main__":
    main()
