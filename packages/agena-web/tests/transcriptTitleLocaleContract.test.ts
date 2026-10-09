import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

test('transcript reads ask the server for the interface language', () => {
  const api = read('../src/stores/chat/api.ts')
  assert.ok(api.includes("import { normalizeAppLocale } from '@/i18n/locale'"))
  assert.ok(api.includes('function withInterfaceLocale(url: string): string {'))
  // A part headline is stored English data; the server maps its vocabulary for
  // the requested language, and the parameter must merge into existing query
  // strings instead of replacing them.
  assert.ok(api.includes("url.includes('?') ? '&' : '?'"))

  for (const call of [
    'withInterfaceLocale(`/api/v1/sessions/${encodeURIComponent(sessionId)}/parts${suffix}`)',
    'withInterfaceLocale(`/api/v1/sessions/${encodeURIComponent(sid)}/runs?${params.toString()}`)',
  ]) {
    assert.ok(api.includes(call), call)
  }
  // One definition plus one call for each transcript part read.
  assert.equal(api.match(/withInterfaceLocale\(/g)?.length, 8)
})
