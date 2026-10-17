import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

test('message item renders durable Agena parts as linear transcript nodes', () => {
  const file = resolve(import.meta.dir, '../src/components/chat/MessageItem.vue')
  const source = readFileSync(file, 'utf8')

  assert.ok(source.includes("import AgenaTranscriptPart from '@/components/chat/AgenaTranscriptPart.vue'"))
  assert.ok(source.includes('data-transcript-node="message"'))
  assert.ok(source.includes('data-transcript-node="part"'))
  assert.ok(source.includes('<AgenaTranscriptPart'))
  assert.ok(!source.includes('<details'))
  assert.ok(!source.includes('<summary'))
})

test('transcript has no independent session or message error blocks', () => {
  const list = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageList.vue'), 'utf8')
  const item = readFileSync(resolve(import.meta.dir, '../src/components/chat/MessageItem.vue'), 'utf8')
  assert.ok(!list.includes('showSessionError'))
  assert.ok(!list.includes('sessionError'))
  assert.ok(!item.includes('fallbackError'))
  assert.ok(!item.includes('info.error'))
  assert.ok(item.includes('<AgenaTranscriptPart'))
})
