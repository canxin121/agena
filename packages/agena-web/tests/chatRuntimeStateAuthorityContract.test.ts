import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('chat runtime phase and assistant placeholder use the server session state, not local awaiting flags', () => {
  const source = readFileSync(resolve(import.meta.dir, '../src/pages/chat/useChatRunUi.ts'), 'utf8')

  assert.ok(source.includes("if (canonicalRunActive.value) return 'busy'"))
  assert.ok(source.includes('return canonicalRunActive.value'))
  assert.ok(!source.includes("if (canonicalRunActive.value || awaitingAssistant.value) return 'busy'"))
  assert.ok(!source.includes('return Boolean(awaitingAssistant.value || busyLike)'))
})
