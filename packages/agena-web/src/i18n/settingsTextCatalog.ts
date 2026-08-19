import enUS from './settings-text/en-US.json'
import zhCN from './settings-text/zh-CN.json'
import esES from './settings-text/es-ES.json'
import frFR from './settings-text/fr-FR.json'
import hiIN from './settings-text/hi-IN.json'
import arSA from './settings-text/ar-SA.json'
import ptBR from './settings-text/pt-BR.json'
import { normalizeAppLocale, type AppLocale } from './locale'

export type SettingsTextParams = Record<string, string | number | boolean | null | undefined>
type SettingsTextCatalog = Record<string, string>

const catalogs: Record<AppLocale, SettingsTextCatalog> = {
  'en-US': enUS,
  'zh-CN': zhCN,
  'es-ES': esES,
  'fr-FR': frFR,
  'hi-IN': hiIN,
  'ar-SA': arSA,
  'pt-BR': ptBR,
}

function interpolate(template: string, params?: SettingsTextParams): string {
  if (!params) return template
  return template.replace(/\{([A-Za-z0-9_]+)\}/g, (match, key: string) => {
    const value = params[key]
    return value === undefined || value === null ? match : String(value)
  })
}

export function settingsTextForLocale(locale: unknown, source: string, params?: SettingsTextParams): string {
  const normalized = normalizeAppLocale(locale) || 'en-US'
  const translated = catalogs[normalized]?.[source] || catalogs['en-US'][source] || source
  return interpolate(translated, params)
}

export function settingsTextCatalog(locale: AppLocale): Readonly<SettingsTextCatalog> {
  return catalogs[locale]
}
