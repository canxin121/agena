import type { settingsText } from '../i18n/settingsText'

export {}

declare module '@vue/runtime-core' {
  interface ComponentCustomProperties {
    $st: typeof settingsText
  }
}
