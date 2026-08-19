import assert from 'node:assert/strict'
import test from 'node:test'

import { SUPPORTED_LOCALES } from '../src/i18n/locale'
import { TUI_LOCALE_OPTIONS, TUI_SUPPORTED_LOCALES } from '../src/i18n/tuiLocale'

test('Web and TUI locale catalogs remain explicit and independent', () => {
  assert.deepEqual(TUI_SUPPORTED_LOCALES, ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'fr-FR', 'de-DE', 'es-ES'])
  assert.ok(!SUPPORTED_LOCALES.includes('ja-JP' as never))
  assert.ok(!TUI_SUPPORTED_LOCALES.includes('ar-SA' as never))
  assert.deepEqual(
    TUI_LOCALE_OPTIONS.map((option) => option.value),
    [...TUI_SUPPORTED_LOCALES],
  )
})
