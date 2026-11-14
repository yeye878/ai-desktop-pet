/**
 * 真实照片背景注册表。
 *
 * 这些是从 wallhaven.cc 挑的真实摄影壁纸（都用 codex 视觉模型验证过写真度）：
 *   极光  aurora  3840x2160  photo 10/10  湖面上的极光帘幕与雪山
 *   水墨  ink     4592x2584  photo  9/10  云雾中的层叠山脊
 *   细雨  rain    2560x1600  photo  9/10  玻璃上的雨滴与窗外散景
 *   科幻  scifi   5288x2975  photo  9/10  长曝光车流光轨
 *   樱花  sakura  7360x4912  photo 10/10  压暗的樱花枝
 *
 * 处理流程（离线做，见 docs/verification/photo-bg/README.md）：
 *   原图 → 缩到 2560 宽 → 高斯模糊（部分）→ 压暗到适合做背景 → webp q80
 * 目标是把照片压到「能看出是什么，但不抢文字」的程度。
 */
import auroraPhoto from "../assets/bg/aurora.webp";
import inkPhoto from "../assets/bg/ink.webp";
import rainPhoto from "../assets/bg/rain.webp";
import scifiPhoto from "../assets/bg/scifi.webp";
import sakuraPhoto from "../assets/bg/sakura.webp";
import springWillowPhoto from "../assets/bg/spring-willow.webp";
import summerLotusPhoto from "../assets/bg/summer-lotus.webp";
import autumnChrysPhoto from "../assets/bg/autumn-chrys.webp";
import winterPlumPhoto from "../assets/bg/winter-plum.webp";

export interface PhotoBackground {
  /** 对应 ChatBackgroundId */
  id: string;
  /** 图片地址 */
  src: string;
  /** 原始分辨率，用于说明（不参与渲染） */
  source: string;
  /** 出处 */
  credit: string;
  /** 图片整体不透明度（照片本身已经压暗过，通常满值） */
  opacity: number;
  /** 叠加的色调，用来统一到应用配色 */
  tint: string;
  /** 暗角强度 */
  vignette: number;
  /**
   * 缓慢漂移幅度（相对画布尺寸的比例）。
   * 照片是静态的，加一点点呼吸感才不像贴图。
   */
  drift: number;
}

export const PHOTO_BACKGROUNDS: Record<string, PhotoBackground> = {
  aurora: {
    id: "aurora",
    src: auroraPhoto,
    source: "3840x2160",
    credit: "wallhaven / 实拍极光",
    opacity: 1,
    tint: "rgba(18, 30, 52, 0.18)",
    vignette: 0.42,
    drift: 0.018,
  },
  ink: {
    id: "ink",
    src: inkPhoto,
    source: "4592x2584",
    credit: "wallhaven / 实拍云雾山脊",
    opacity: 1,
    tint: "rgba(40, 46, 52, 0.12)",
    vignette: 0.3,
    drift: 0.012,
  },
  rain: {
    id: "rain",
    src: rainPhoto,
    source: "2560x1600",
    credit: "wallhaven / 实拍雨窗散景",
    opacity: 1,
    tint: "rgba(16, 24, 32, 0.2)",
    vignette: 0.4,
    drift: 0.008,
  },
  scifi: {
    id: "scifi",
    src: scifiPhoto,
    source: "5288x2975",
    credit: "wallhaven / 实拍长曝光车流",
    opacity: 1,
    tint: "rgba(12, 18, 28, 0.22)",
    vignette: 0.44,
    drift: 0.014,
  },
  sakura: {
    id: "sakura",
    src: sakuraPhoto,
    source: "5712x3214",
    credit: "wallhaven / 实拍樱花单枝（背景虚化）",
    opacity: 1,
    tint: "rgba(30, 28, 30, 0.16)",
    vignette: 0.44,
    drift: 0.01,
  },
  /*
   * 四季意象。
   * 每个季节取一种传统意象（柳/荷/菊/梅），全部是 wallhaven 上的真实摄影，
   * 用 codex 逐张确认过 photorealism 10/10。
   */
  "spring-willow": {
    id: "spring-willow",
    src: springWillowPhoto,
    source: "6000x4000",
    credit: "wallhaven / 雾中垂柳",
    opacity: 1,
    tint: "rgba(24, 32, 26, 0.16)",
    vignette: 0.4,
    drift: 0.012,
  },
  "summer-lotus": {
    id: "summer-lotus",
    src: summerLotusPhoto,
    source: "3840x2160",
    credit: "wallhaven / 荷塘睡莲",
    opacity: 1,
    tint: "rgba(20, 34, 28, 0.16)",
    vignette: 0.4,
    drift: 0.014,
  },
  "autumn-chrys": {
    id: "autumn-chrys",
    src: autumnChrysPhoto,
    source: "4608x3072",
    credit: "wallhaven / 秋菊特写",
    opacity: 1,
    tint: "rgba(34, 26, 22, 0.16)",
    vignette: 0.42,
    drift: 0.012,
  },
  "winter-plum": {
    id: "winter-plum",
    src: winterPlumPhoto,
    source: "4928x3264",
    credit: "wallhaven / 腊月梅枝",
    opacity: 1,
    tint: "rgba(26, 26, 32, 0.16)",
    vignette: 0.44,
    drift: 0.01,
  },
};

export const PHOTO_BACKGROUND_IDS = Object.keys(PHOTO_BACKGROUNDS);

export function photoBackground(id: string | null | undefined): PhotoBackground | null {
  if (!id) return null;
  return PHOTO_BACKGROUNDS[id] ?? null;
}
