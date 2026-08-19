import { createApp } from 'vue'
import { createPinia } from 'pinia'

import '@fontsource/ibm-plex-sans/400.css'
import '@fontsource/ibm-plex-sans/500.css'
import '@fontsource/ibm-plex-sans/600.css'
import '@fontsource/ibm-plex-mono/400.css'
import '@fontsource/ibm-plex-mono/500.css'

import 'katex/dist/katex.min.css'
import 'vue-sonner/style.css'

import './style.css'
import App from './App.vue'
import { router } from './router'
import { i18n, ensureDefaultLocale, setAppLocale } from './i18n'
import { DEFAULT_LOCALE, normalizeAppLocale } from './i18n/locale'
import { settingsText } from './i18n/settingsText'
import { useToastsStore } from './stores/toasts'
import { useAuthStore } from './stores/auth'
import { sessionStorageKeys } from './lib/persistence/storageKeys'
import {
  OC_AUTH_REQUIRED_EVENT,
  readAuthRequiredFromStorage,
  clearAuthRequiredFromStorage,
  type AuthRequiredDetail,
} from './lib/authEvents.ts'
import { readUiAuthTokenVersion } from './lib/uiAuthToken'

// Capture initial page-load context so components that mount lazily (e.g. mobile sidebar)
// can still tell whether a session query came from a fresh load vs in-app navigation.
const PAGE_LOAD_TOKEN_KEY = sessionStorageKeys.app.pageLoadToken
const INITIAL_SESSION_QUERY_KEY = sessionStorageKeys.app.initialSessionQuery
try {
  const token = String(performance.timeOrigin || Date.now())
  sessionStorage.setItem(PAGE_LOAD_TOKEN_KEY, token)
  sessionStorage.setItem(INITIAL_SESSION_QUERY_KEY, '')
} catch {
  // ignore
}

const app = createApp(App)
const pinia = createPinia()
app.use(pinia)
app.use(i18n)
app.use(router)
app.config.globalProperties.$st = settingsText

// Keep <html lang> in sync with i18n locale.
ensureDefaultLocale()
setAppLocale(normalizeAppLocale(i18n.global.locale.value) || DEFAULT_LOCALE)
const toasts = useToastsStore(pinia)
const auth = useAuthStore(pinia)

// Global auth-required handler: toast then show login screen.
let lastAuthToastAt = 0
let lastAuthToastMsg = ''
let authRefreshInFlight: Promise<void> | null = null

function ensureAuthRefreshSoon() {
  if (authRefreshInFlight) return
  authRefreshInFlight = auth
    .refresh()
    .catch(() => {})
    .finally(() => {
      authRefreshInFlight = null
    })
}

function handleAuthRequired(detail?: AuthRequiredDetail) {
  // Requests started before login can finish with 401 after the new token has
  // been written. Their event is stale and must not bounce the app back to the
  // login page. The version is intentionally metadata only; the bearer token
  // itself is never placed in the event or sessionStorage.
  if (typeof detail?.authTokenVersion === 'number' && detail.authTokenVersion !== readUiAuthTokenVersion()) {
    return
  }

  const msg =
    String(detail?.message || i18n.global.t('auth.uiAuthRequired')).trim() ||
    String(i18n.global.t('auth.uiAuthRequired'))

  const now = Date.now()
  if (msg !== lastAuthToastMsg || now - lastAuthToastAt > 4000) {
    lastAuthToastMsg = msg
    lastAuthToastAt = now
    toasts.push('error', msg, 4500)
  }

  // Switch to the login screen immediately, then reconcile state from /auth/session.
  try {
    auth.requireLogin()
  } catch {
    // ignore
  }
  ensureAuthRefreshSoon()
}

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.addEventListener(OC_AUTH_REQUIRED_EVENT, (evt) => {
    const detail = (evt as unknown as { detail?: AuthRequiredDetail }).detail
    handleAuthRequired(detail)
  })
}

if (typeof document !== 'undefined' && typeof document.addEventListener === 'function') {
  document.addEventListener(OC_AUTH_REQUIRED_EVENT, (evt) => {
    const detail = (evt as unknown as { detail?: AuthRequiredDetail }).detail
    handleAuthRequired(detail)
  })
}

// Best-effort replay for early or non-CustomEvent environments.
const stored = readAuthRequiredFromStorage()
clearAuthRequiredFromStorage()
if (stored && stored.at > 0 && Date.now() - stored.at < 30_000) {
  handleAuthRequired(stored.detail)
}

app.mount('#app')
