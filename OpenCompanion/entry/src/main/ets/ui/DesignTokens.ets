import { ThemeEngine } from '../pet/ThemeEngine';

/**
 * OpenCompanion 设计系统 —— 「墨随宠动」
 *
 * 一套高级感的暖调极简语言：
 *  - 表面：暖纸白（faint warm paper），无渐变噪点，层级靠明度差与发丝线
 *  - 文字：墨色单色阶（ink 阶梯），层级靠字重与明度，不靠颜色
 *  - 强调：唯一强调色 = 桌宠主题 deep 色（UI 随宠变色，设置页可切换）
 *  - 形状：卡片 18 / 内层 12 / 控件全胶囊，一套圆角体系
 *  - 图标：SymbolGlyph 系统线性图标（禁 emoji 做 UI 装饰；emoji 仅保留为用户数据如头像）
 *
 * 强调色通过 AppStorage 广播（uiAccentDeep / uiAccentLight / uiAccentTint / uiAccentTintStrong），
 * 页面用 @StorageProp 订阅，切换主题时全 UI 即时变色。
 */
export class Ink {
  /** 页面底 · 暖纸 */
  static readonly bg: string = '#F4F1EC';
  /** 卡片 · 提亮纸面 */
  static readonly card: string = '#FCFBF9';
  /** 下沉区 · 输入/代码底 */
  static readonly sunken: string = '#EDE9E2';
  /** 主文字 · 暖近黑（禁纯黑） */
  static readonly text: string = '#211C18';
  /** 次级文字 */
  static readonly text2: string = '#6C655D';
  /** 弱化文字 */
  static readonly text3: string = '#A39B91';
  /** 反白文字（墨底上用） */
  static readonly onInk: string = '#FAF8F4';
  /** 发丝分隔线 · 8% 墨 */
  static readonly hairline: string = '#14211C18';
  /** 发丝边框 · 12% 墨 */
  static readonly border: string = '#1F211C18';
  /** 纯功能语义色（仅状态，不参与装饰） */
  static readonly ok: string = '#4C7A5A';
  static readonly okSoft: string = '#1A4C7A5A';
  static readonly warn: string = '#96660F';
  static readonly warnSoft: string = '#1A96660F';
  static readonly danger: string = '#A6493C';
  static readonly dangerSoft: string = '#1AA6493C';
}

/** 圆角体系 */
export class Radius {
  static readonly sheet: number = 24;
  static readonly card: number = 18;
  static readonly inner: number = 12;
  static readonly pill: number = 999;
}

/** 间距体系（4 的倍数） */
export class Gap {
  static readonly xs: number = 4;
  static readonly sm: number = 8;
  static readonly md: number = 12;
  static readonly lg: number = 16;
  static readonly xl: number = 24;
}

/** 常用弥散阴影（色相随墨，禁止纯黑投影） */
export class Shadow {
  static readonly soft: ShadowOptions = {
    radius: 20,
    color: '#26201914',
    offsetY: 6
  };
  static readonly lift: ShadowOptions = {
    radius: 34,
    color: '#33201A14',
    offsetY: 10
  };
  static readonly hair: ShadowOptions = {
    radius: 8,
    color: '#1A201914',
    offsetY: 2
  };
}

/**
 * 强调色桥：把桌宠主题色同步进 AppStorage，驱动全 UI 变色。
 * 深色 deep 做操作强调，浅色 light 做氛围，tint 系列做底色。
 */
export class AccentBridge {
  static readonly KEY_DEEP: string = 'uiAccentDeep';
  static readonly KEY_LIGHT: string = 'uiAccentLight';
  static readonly KEY_TINT: string = 'uiAccentTint';
  static readonly KEY_TINT_STRONG: string = 'uiAccentTintStrong';

  static sync(): void {
    const theme = ThemeEngine.get();
    AppStorage.setOrCreate(AccentBridge.KEY_DEEP, theme.deep);
    AppStorage.setOrCreate(AccentBridge.KEY_LIGHT, theme.light);
    AppStorage.setOrCreate(AccentBridge.KEY_TINT, AccentBridge.rgba(theme.deep, 0.12));
    AppStorage.setOrCreate(AccentBridge.KEY_TINT_STRONG, AccentBridge.rgba(theme.deep, 0.22));
  }

  static defaults(): void {
    AppStorage.setOrCreate(AccentBridge.KEY_DEEP, '#F76C8A');
    AppStorage.setOrCreate(AccentBridge.KEY_LIGHT, '#FFE3EA');
    AppStorage.setOrCreate(AccentBridge.KEY_TINT, AccentBridge.rgba('#F76C8A', 0.12));
    AppStorage.setOrCreate(AccentBridge.KEY_TINT_STRONG, AccentBridge.rgba('#F76C8A', 0.22));
  }

  /** '#RRGGBB' → rgba(r,g,b,a) */
  static rgba(hex: string, alpha: number): string {
    const h = hex.replace('#', '');
    if (h.length !== 6) {
      return hex;
    }
    const r = parseInt(h.substring(0, 2), 16);
    const g = parseInt(h.substring(2, 4), 16);
    const b = parseInt(h.substring(4, 6), 16);
    return `rgba(${r},${g},${b},${alpha})`;
  }
}
