import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const read = (path: string) => readFileSync(resolve(import.meta.dir, path), 'utf8')

test('transcript reads ask the server for the interface language', () => {
  const api = read('../src/stores/chat/api.ts')
  assert.ok(api.includes("import { withInterfaceLocale } from '@/i18n/interfaceLocale'"))

  for (const call of [
    'withInterfaceLocale(`/api/v1/sessions/${encodeURIComponent(sessionId)}/parts${suffix}`)',
    'withInterfaceLocale(`/api/v1/sessions/${encodeURIComponent(sid)}/runs?${params.toString()}`)',
  ]) {
    assert.ok(api.includes(call), call)
  }
  // One call for each transcript part read.
  assert.equal(api.match(/withInterfaceLocale\(/g)?.length, 7)
})

test('the language parameter merges into an existing query string', () => {
  const helper = read('../src/i18n/interfaceLocale.ts')
  assert.ok(helper.includes('export function withInterfaceLocale(url: string): string {'))
  // A part headline is stored English data; the server maps its vocabulary for
  // the requested language, and the parameter must merge into existing query
  // strings instead of replacing them.
  assert.ok(helper.includes("url.includes('?') ? '&' : '?'"))
})

test('the live stream carries the interface language too', () => {
  const runtime = read('../src/app/runtime/useAppRuntime.ts')
  assert.ok(
    runtime.includes("endpoint: withInterfaceLocale('/api/v1/changes/stream?scope_kind=global'),"),
  )
})
