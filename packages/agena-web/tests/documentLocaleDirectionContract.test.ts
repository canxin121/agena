import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/i18n/index.ts', import.meta.url), 'utf8')
const main = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8')

test('document language and direction follow the selected locale', () => {
  assert.match(source, /document\.documentElement\.lang = locale/)
  assert.match(source, /document\.documentElement\.dir = locale === 'ar-SA' \? 'rtl' : 'ltr'/)
})

test('stored locale is applied before the app mounts', () => {
  const localeCall = main.indexOf('setAppLocale(normalizeAppLocale')
  const mountCall = main.indexOf("app.mount('#app')")
  assert.ok(localeCall >= 0)
  assert.ok(mountCall > localeCall)
})
