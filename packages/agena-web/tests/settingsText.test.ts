import assert from 'node:assert/strict'
import test from 'node:test'

import { settingsTextCatalog, settingsTextForLocale } from '../src/i18n/settingsTextCatalog'
import { SUPPORTED_LOCALES } from '../src/i18n/locale'

test('Settings source-string translations interpolate named parameters in locale order', () => {
  assert.equal(
    settingsTextForLocale('zh-CN', 'Delete {layer} override {path}?', {
      layer: '工作区',
      path: 'permission',
    }),
    '删除工作区覆盖 permission？',
  )
  assert.equal(settingsTextForLocale('fr-FR', 'Page {page} of {pages}', { page: 2, pages: 8 }), 'Page 2 de 8')
})

test('Settings translation falls back to English source text for unknown locale or key', () => {
  assert.equal(settingsTextForLocale('xx-YY', 'Save'), 'Save')
  assert.equal(settingsTextForLocale('zh-CN', 'Unregistered Settings source'), 'Unregistered Settings source')
  assert.equal(settingsTextForLocale('zh-CN', 'Keep {missing} intact'), 'Keep {missing} intact')
})

test('every Web locale exposes the same complete Settings source catalog', () => {
  const english = settingsTextCatalog('en-US')
  const keys = Object.keys(english).sort()
  for (const locale of SUPPORTED_LOCALES) {
    const catalog = settingsTextCatalog(locale)
    assert.deepEqual(Object.keys(catalog).sort(), keys, `${locale} Settings catalog differs from en-US`)
    for (const key of keys)
      assert.ok(String(catalog[key] || '').trim(), `${locale} has an empty translation for ${key}`)
  }
})
