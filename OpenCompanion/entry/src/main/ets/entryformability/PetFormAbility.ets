import { FormExtensionAbility, formBindingData, formProvider } from '@kit.FormKit';
import { Want } from '@kit.AbilityKit';
import { preferences } from '@kit.ArkData';
import { hilog } from '@kit.PerformanceAnalysisKit';

/**
 * 桌宠服务卡片（替代桌面版悬浮窗的鸿蒙原生形态）：
 * 展示伙伴状态 + 快捷入口，状态由应用侧刷新推送。
 */
export default class PetFormAbility extends FormExtensionAbility {
  onAddForm(want: Want): formBindingData.FormBindingData {
    const formId = want.parameters !== undefined ? `${want.parameters['ohos.extra.param.key.form_identity']}` : '';
    hilog.info(0x0C0C, 'OpenCompanion', `form added: ${formId}`);
    const data = formBindingData.createFormBindingData({
      'status': '我在等你哦～',
      'emoji': '🐹'
    });
    return data;
  }

  onUpdateForm(formId: string): void {
    this.readStatus().then((status: string) => {
      const data = formBindingData.createFormBindingData({
        'status': status,
        'emoji': '🐹'
      });
      formProvider.updateForm(formId, data);
    });
  }

  private async readStatus(): Promise<string> {
    try {
      const prefs = await preferences.getPreferences(this.context, { name: 'companion_settings' });
      const v = await prefs.get('pet_mood_text', '我在等你哦～');
      return typeof v === 'string' ? v : '我在等你哦～';
    } catch (e) {
      return '我在等你哦～';
    }
  }

  onRemoveForm(_formId: string): void {
  }

  onFormEvent(_formId: string): void {
  }
}
