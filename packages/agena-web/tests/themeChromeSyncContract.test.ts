import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const appearance = readFileSync(new URL('../src/lib/appearance.ts', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8')

test('browser theme color follows the resolved application appearance', () => {
  assert.match(appearance, /variant === 'dark' \? '#151313' : '#fbf8f3'/)
  assert.match(appearance, /querySelectorAll<HTMLMetaElement>\('meta\[name="theme-color"\]'\)/)
  assert.match(html, /prefers-color-scheme: light[^>]+#fbf8f3/)
  assert.match(html, /prefers-color-scheme: dark[^>]+#151313/)
})

test('system-theme changes are observed at the App root', () => {
  assert.match(app, /matchMedia\('\(prefers-color-scheme: light\)'\)/)
  assert.match(app, /addEventListener\?\.\('change', handleSystemThemeChange\)/)
  assert.match(app, /removeEventListener\?\.\('change', handleSystemThemeChange\)/)
})
