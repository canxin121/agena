import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'

const source = readFileSync(new URL('../src/components/ui/Button.vue', import.meta.url), 'utf8')

test('shared Button defaults native buttons to type=button while preserving explicit types', () => {
  assert.match(source, /const\s+nativeType\s*=\s*computed\(/)
  assert.match(source, /normalizeText\(attrs\.type\)/)
  assert.match(source, /props\.as === 'button' && !props\.asChild \? 'button' : undefined/)
  assert.match(source, /type:\s*_type/)
  assert.match(source, /:type="nativeType"/)
})
