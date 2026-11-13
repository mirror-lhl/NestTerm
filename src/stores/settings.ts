import { ref } from 'vue'
import { settingsGet, settingsUpdate, type Settings, type SettingsPatch } from '../lib/settings'

/** 全局设置单例：App 启动加载，设置对话框保存后即时生效。 */
export const appSettings = ref<Settings | null>(null)

export async function loadSettings(): Promise<void> {
  try {
    appSettings.value = await settingsGet()
  } catch (error) {
    console.warn('[nestterm] 加载设置失败，使用默认值：', error)
  }
}

export async function saveSettings(patch: SettingsPatch): Promise<void> {
  appSettings.value = await settingsUpdate(patch)
}
