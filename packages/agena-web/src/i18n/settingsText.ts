import { i18n } from './index'
import { settingsTextForLocale, type SettingsTextParams } from './settingsTextCatalog'

export type { SettingsTextParams } from './settingsTextCatalog'
export { settingsTextCatalog, settingsTextForLocale } from './settingsTextCatalog'

export function settingsText(source: string, params?: SettingsTextParams): string {
  return settingsTextForLocale(i18n.global.locale.value, source, params)
}
