/**
 * 真实照片背景注册表。
 *
 * 素材来源分两类：
 *   1. wallhaven.cc 实拍壁纸（极光 / 水墨 / 夏荷）—— 都用 codex 验证过写真度
 *   2. 用户提供的参考图（春柳 / 秋菊 / 冬梅 / 细雨 / 樱花）—— 按用户指定直接采用
 *
 * ⚠️ 后一组是**手机截图**，上下带黑 UI 条。处理时必须先按行扫描
 *    "接近黑像素占比 > 85%" 的连续带把它裁掉，否则背景里会露出播放器/按钮。
 *
 * 处理流程（离线做，见 docs/verification/photo-bg/README.md）：
 *   截图 → 裁掉 UI 黑带 → 裁成 16:10（竖直方向偏上取，保主体）
 *   → 缩到 2560 宽 → 轻度调亮/降对比 → webp q84
 */
import auroraPhoto from "../assets/bg/aurora.webp";
import inkPhoto from "../assets/bg/ink.webp";
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
    tint: "rgba(18, 30, 52, 0.10)",
    vignette: 0.28,
    drift: 0.018,
  },
  ink: {
    id: "ink",
    src: inkPhoto,
    source: "4592x2584",
    credit: "wallhaven / 实拍云雾山脊",
    opacity: 1,
    tint: "rgba(40, 46, 52, 0.06)",
    vignette: 0.20,
    drift: 0.012,
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
    tint: "rgba(255, 252, 254, 0.05)",
    vignette: 0.06,
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
    source: "1200x661",
    credit: "用户提供 / 雨窗垂柳（暖阳改造）",
    opacity: 1,
    tint: "rgba(255, 246, 224, 0.06)",
    vignette: 0.05,
    drift: 0.012,
  },
  "summer-lotus": {
    id: "summer-lotus",
    src: summerLotusPhoto,
    source: "3840x2160",
    credit: "wallhaven / 荷塘睡莲",
    opacity: 1,
    tint: "rgba(250, 255, 250, 0.04)",
    vignette: 0.06,
    drift: 0.014,
  },
  "autumn-chrys": {
    id: "autumn-chrys",
    src: autumnChrysPhoto,
    source: "1200x2670",
    credit: "用户参考图 / 白菊侧光",
    opacity: 1,
    tint: "rgba(255, 253, 248, 0.04)",
    vignette: 0.06,
    drift: 0.012,
  },
  "winter-plum": {
    id: "winter-plum",
    src: winterPlumPhoto,
    source: "1193x672",
    credit: "用户裁剪 / 雪压梅枝",
    opacity: 1,
    tint: "rgba(250, 253, 255, 0.05)",
    vignette: 0.05,
    drift: 0.01,
  },
};

export const PHOTO_BACKGROUND_IDS = Object.keys(PHOTO_BACKGROUNDS);

export function photoBackground(id: string | null | undefined): PhotoBackground | null {
  if (!id) return null;
  return PHOTO_BACKGROUNDS[id] ?? null;
}
