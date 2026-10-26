;(() => {
  'use strict'

  const root = document.documentElement
  const catalogs = JSON.parse(document.getElementById('oauth-translations').textContent)
  let preferences = {}
  let storedLocale = ''
  try {
    const stored = JSON.parse(localStorage.getItem('agena.settings.ui-prefs') || '{}')
    if (stored && typeof stored === 'object' && !Array.isArray(stored)) preferences = stored
    storedLocale = localStorage.getItem('studio.ui.locale') || ''
  } catch {
    // Native form submission and system preferences also work without storage.
  }

  const systemTheme = window.matchMedia('(prefers-color-scheme: light)')
  function applyTheme() {
    const light = preferences.useSystemTheme !== false ? systemTheme.matches : preferences.themeVariant === 'light'
    root.classList.toggle('dark', !light)
    document.querySelector('meta[name="theme-color"]').content = light ? '#fbf8f3' : '#151313'
  }
  applyTheme()

  const fonts = {
    system: 'ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif',
    'ibm-plex-sans':
      '"IBM Plex Sans", ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif',
    atkinson: '"Atkinson Hyperlegible", ui-sans-serif, system-ui, sans-serif',
    serif: 'ui-serif, Georgia, Cambria, "Times New Roman", Times, serif',
  }
  root.style.setProperty('--font-sans', fonts[preferences.uiFont] || preferences.uiFont || fonts.system)
  if (Number.isFinite(preferences.cornerRadius)) {
    root.style.setProperty('--radius', `${Math.max(4, Math.min(24, preferences.cornerRadius))}px`)
  }
  if (Number.isFinite(preferences.fontSize)) {
    root.style.setProperty(
      '--base-font-size',
      `${Math.round((16 * Math.max(80, Math.min(125, preferences.fontSize))) / 100)}px`,
    )
  }

  document.addEventListener('DOMContentLoaded', () => {
    const picker = document.getElementById('ui-locale')
    const themeToggle = document.getElementById('theme-toggle')
    const passwordToggle = document.getElementById('password-toggle')
    const password = document.getElementById('password')
    const form = document.getElementById('authorize-form')
    let locale = Object.hasOwn(catalogs, storedLocale) ? storedLocale : root.lang

    function updateControlLabels() {
      const messages = catalogs[locale]
      const themeLabel = root.classList.contains('dark') ? messages.light_theme : messages.dark_theme
      themeToggle.setAttribute('aria-label', themeLabel)
      themeToggle.title = themeLabel
      if (passwordToggle) {
        const label = password.type === 'password' ? messages.show_password : messages.hide_password
        passwordToggle.setAttribute('aria-label', label)
        passwordToggle.title = label
      }
    }

    function translate() {
      const messages = catalogs[locale]
      root.lang = locale
      root.dir = locale === 'ar-SA' ? 'rtl' : 'ltr'
      picker.value = locale
      for (const node of document.querySelectorAll('[data-i18n]')) {
        if (Object.hasOwn(messages, node.dataset.i18n)) node.textContent = messages[node.dataset.i18n]
      }
      for (const node of document.querySelectorAll('[data-i18n-placeholder]')) {
        if (Object.hasOwn(messages, node.dataset.i18nPlaceholder))
          node.placeholder = messages[node.dataset.i18nPlaceholder]
      }
      document.title = document.querySelector('h1').textContent
      updateControlLabels()
    }

    translate()
    systemTheme.addEventListener('change', () => {
      applyTheme()
      updateControlLabels()
    })
    themeToggle.hidden = false
    picker.addEventListener('change', () => {
      if (!Object.hasOwn(catalogs, picker.value)) return
      locale = picker.value
      translate()
      try {
        localStorage.setItem('studio.ui.locale', locale)
      } catch {
        /* Storage is optional. */
      }
    })
    themeToggle.addEventListener('click', () => {
      preferences.useSystemTheme = false
      preferences.themeVariant = root.classList.contains('dark') ? 'light' : 'dark'
      applyTheme()
      updateControlLabels()
      try {
        localStorage.setItem('agena.settings.ui-prefs', JSON.stringify(preferences))
      } catch {
        /* Storage is optional. */
      }
    })
    if (passwordToggle) {
      passwordToggle.hidden = false
      passwordToggle.addEventListener('click', () => {
        const reveal = password.type === 'password'
        password.type = reveal ? 'text' : 'password'
        passwordToggle.setAttribute('aria-pressed', String(reveal))
        updateControlLabels()
        password.focus({ preventScroll: true })
      })
    }
    if (form) {
      form.addEventListener('submit', (event) => {
        if (form.getAttribute('aria-busy') === 'true') {
          event.preventDefault()
          return
        }
        form.setAttribute('aria-busy', 'true')
        // Keep successful submit controls enabled so the browser includes the
        // allow/deny decision in the native POST, including an empty-password denial.
        const label = event.submitter?.querySelector('[data-i18n]')
        if (label) {
          label.dataset.i18n = event.submitter.value === 'deny' ? 'cancelling' : 'authorizing'
          label.textContent = catalogs[locale][label.dataset.i18n]
        }
      })
      window.addEventListener('pageshow', (event) => {
        if (!event.persisted) return
        form.removeAttribute('aria-busy')
        password.value = ''
        form.querySelector('[value="allow"] span').dataset.i18n = 'authorize'
        form.querySelector('[value="deny"] span').dataset.i18n = 'cancel'
        translate()
      })
    }
  })
})()
