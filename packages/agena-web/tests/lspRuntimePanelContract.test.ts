import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('LSP runtime panel reads the canonical runtime status and remains bounded', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/components/chat/ChatRuntimeStatusOverlay.vue'), 'utf8')

  assert.ok(source.includes("apiJson<RuntimeStatus>('/api/v1/runtime')"))
  assert.ok(source.includes('normalizeLspRuntimeList(runtime.value || {})'))
  assert.ok(source.includes('max-h-[min(56dvh,32rem)]'))
  assert.ok(source.includes('tooltip="Refresh runtime status"'))
  assert.ok(source.includes('<RiRefreshLine v-else class="h-4 w-4" />'))
})
