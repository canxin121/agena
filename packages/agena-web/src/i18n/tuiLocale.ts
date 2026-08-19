export const TUI_SUPPORTED_LOCALES = ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'fr-FR', 'de-DE', 'es-ES'] as const
export type TuiLocale = (typeof TUI_SUPPORTED_LOCALES)[number]

export type TuiLocaleOption = {
  value: TuiLocale
  label: string
}

/**
 * TUI language names are intentionally autonyms so the picker remains usable
 * even when the current Web locale differs from the target terminal locale.
 */
export const TUI_LOCALE_OPTIONS: readonly TuiLocaleOption[] = [
  { value: 'en-US', label: 'English (United States)' },
  { value: 'zh-CN', label: '简体中文' },
  { value: 'zh-TW', label: '繁體中文' },
  { value: 'ja-JP', label: '日本語' },
  { value: 'ko-KR', label: '한국어' },
  { value: 'fr-FR', label: 'Français' },
  { value: 'de-DE', label: 'Deutsch' },
  { value: 'es-ES', label: 'Español' },
]
