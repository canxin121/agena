import { createI18n } from 'vue-i18n'

import enUS from './messages/en-US'
import {
  readStoredLocale,
  storeLocale,
  DEFAULT_LOCALE,
  SUPPORTED_LOCALES,
  normalizeAppLocale,
  type AppLocale,
} from './locale'

type MessageSchema = typeof enUS

const messageModules = import.meta.glob('./messages/*.ts', { eager: true }) as Record<string, { default?: unknown }>
const settingsOverlayModules = import.meta.glob('./settings-overlays/*.json', { eager: true }) as Record<
  string,
  { default?: unknown }
>

function mergeMessageTree(base: unknown, overlay: unknown): unknown {
  if (!base || typeof base !== 'object' || Array.isArray(base)) return overlay
  if (!overlay || typeof overlay !== 'object' || Array.isArray(overlay)) return overlay
  const next: Record<string, unknown> = { ...(base as Record<string, unknown>) }
  for (const [key, value] of Object.entries(overlay as Record<string, unknown>)) {
    next[key] = key in next ? mergeMessageTree(next[key], value) : value
  }
  return next
}

const loadedMessages: Record<string, MessageSchema> = {}
for (const [path, mod] of Object.entries(messageModules)) {
  const match = path.match(/\/([^/]+)\.ts$/)
  if (!match) continue
  const locale = match[1]
  if (locale && mod?.default && typeof mod.default === 'object') {
    loadedMessages[locale] = mod.default as MessageSchema
  }
}

const loadedSettingsOverlays: Record<string, unknown> = {}
for (const [path, mod] of Object.entries(settingsOverlayModules)) {
  const match = path.match(/\/([^/]+)\.json$/)
  if (!match) continue
  const locale = match[1]
  if (locale && mod?.default && typeof mod.default === 'object') loadedSettingsOverlays[locale] = mod.default
}

const enUSMessages = loadedMessages['en-US'] || enUS
const messages = Object.fromEntries(
  SUPPORTED_LOCALES.map((locale) => {
    const base = loadedMessages[locale] || enUSMessages
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
