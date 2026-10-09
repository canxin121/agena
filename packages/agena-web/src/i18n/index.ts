import { createI18n } from 'vue-i18n'

import arSAMessages from './messages/ar-SA'
import enUSMessages from './messages/en-US'
import esESMessages from './messages/es-ES'
import frFRMessages from './messages/fr-FR'
import hiINMessages from './messages/hi-IN'
import ptBRMessages from './messages/pt-BR'
import zhCNMessages from './messages/zh-CN'
import {
  readStoredLocale,
  storeLocale,
  DEFAULT_LOCALE,
  SUPPORTED_LOCALES,
  normalizeAppLocale,
  type AppLocale,
} from './locale'
import arSAOverlay from './settings-overlays/ar-SA.json'
import esESOverlay from './settings-overlays/es-ES.json'
import frFROverlay from './settings-overlays/fr-FR.json'
import hiINOverlay from './settings-overlays/hi-IN.json'
import ptBROverlay from './settings-overlays/pt-BR.json'

type MessageSchema = typeof enUSMessages

function mergeMessageTree(base: unknown, overlay: unknown): unknown {
  if (!base || typeof base !== 'object' || Array.isArray(base)) return overlay
  if (!overlay || typeof overlay !== 'object' || Array.isArray(overlay)) return overlay
  const next: Record<string, unknown> = { ...(base as Record<string, unknown>) }
  for (const [key, value] of Object.entries(overlay as Record<string, unknown>)) {
    next[key] = key in next ? mergeMessageTree(next[key], value) : value
  }
  return next
}

// Catalogs are imported statically rather than through `import.meta.glob` so
// non-Vite consumers such as `bun test` can load this module directly.
const loadedMessages: Record<string, MessageSchema> = {
  'ar-SA': arSAMessages as unknown as MessageSchema,
  'en-US': enUSMessages,
  'es-ES': esESMessages as unknown as MessageSchema,
  'fr-FR': frFRMessages as unknown as MessageSchema,
  'hi-IN': hiINMessages as unknown as MessageSchema,
  'pt-BR': ptBRMessages as unknown as MessageSchema,
  'zh-CN': zhCNMessages as unknown as MessageSchema,
}

const loadedSettingsOverlays: Record<string, unknown> = {
  'ar-SA': arSAOverlay,
  'es-ES': esESOverlay,
  'fr-FR': frFROverlay,
  'hi-IN': hiINOverlay,
  'pt-BR': ptBROverlay,
}

const fallbackMessages = loadedMessages['en-US']
const messages = Object.fromEntries(
  SUPPORTED_LOCALES.map((locale) => {
    const base = loadedMessages[locale] || fallbackMessages
    const overlay = loadedSettingsOverlays[locale]
    return [locale, overlay ? mergeMessageTree(base, { settings: overlay }) : base]
  }),
) as Record<AppLocale, MessageSchema>

export const i18n = createI18n({
  legacy: false as const,
  globalInjection: true,
  locale: readStoredLocale(),
  fallbackLocale: 'en-US',
  messages,
})

export function setAppLocale(locale: AppLocale) {
  i18n.global.locale.value = locale
  storeLocale(locale)
  if (typeof document !== 'undefined') {
    try {
      document.documentElement.lang = locale
      document.documentElement.dir = locale === 'ar-SA' ? 'rtl' : 'ltr'
    } catch {
      // ignore
    }
  }
}

export function ensureDefaultLocale() {
  const current = normalizeAppLocale(i18n.global.locale.value)
  if (!current) {
    setAppLocale(DEFAULT_LOCALE)
    return
  }
  if (String(i18n.global.locale.value || '') !== current) {
    setAppLocale(current)
  }
}
