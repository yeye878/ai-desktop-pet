import { AppContext } from '../commons/AppContext';

/**
 * 桌宠主题色板（对应桌面版 stores/pet.ts THEMES 7 套主题）
 */
export class PetTheme {
  name: string = '';
  primary: string = '#FF8FA3';
  deep: string = '#F76C8A';
  light: string = '#FFE3EA';
  accent: string = '#FFD166';

  constructor(name: string, primary: string, deep: string, light: string, accent: string) {
    this.name = name;
    this.primary = primary;
    this.deep = deep;
    this.light = light;
    this.accent = accent;
  }
}

export class ThemeEngine {
  static readonly THEMES: PetTheme[] = [
    new PetTheme('default', '#FF8FA3', '#F76C8A', '#FFE3EA', '#FFD166'),
    new PetTheme('ocean', '#5EB7F7', '#3D8BD4', '#DDF0FF', '#7BE0E0'),
    new PetTheme('galaxy', '#9D7BFF', '#7C5CE0', '#EAE2FF', '#FFCF5C'),
    new PetTheme('forest', '#6FCF97', '#4CA973', '#E0F5E9', '#FFD166'),
    new PetTheme('sunset', '#FF9A62', '#F07B3F', '#FFE8D9', '#FFD166'),
    new PetTheme('sakura', '#F7A8C4', '#E87BA4', '#FDEAF2', '#B58BFF'),
    new PetTheme('aurora', '#7CF5D5', '#3EDBB4', '#DDFBF2', '#9D7BFF')
  ];

  static currentIndex(): number {
    return ThemeEngine.clampIndex(AppContext.getNumber('pet_theme', 0));
  }

  static async setTheme(i: number): Promise<void> {
    await AppContext.setSetting('pet_theme', `${ThemeEngine.clampIndex(i)}`);
  }

  static clampIndex(i: number): number {
    if (i < 0 || i >= ThemeEngine.THEMES.length) {
      return 0;
    }
    return i;
  }

  static get(): PetTheme {
    return ThemeEngine.THEMES[ThemeEngine.currentIndex()];
  }
}
