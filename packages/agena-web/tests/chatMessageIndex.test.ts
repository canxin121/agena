import test from 'node:test'
import assert from 'node:assert/strict'

import { binarySearchById, compareChatIds, upsertMessageEntryIn, upsertPart } from '../src/stores/chat/messageIndex'
import type { MessageEntry, MessageInfo, MessagePart } from '../src/types/chat'

function info(id: string): MessageInfo {
  return { id, sessionID: '1', role: 'assistant' }
}

function part(id: string, text: string): MessagePart {
  return { id, sessionID: '1', messageID: '2', type: 'text', text }
}

test('binarySearchById finds Agena numeric ids in numeric order', () => {
  const values = [{ id: '2' }, { id: '10' }, { id: '100' }]

  assert.deepEqual(
    binarySearchById(values, '10', (value) => value.id),
    { found: true, index: 1 },
  )
  assert.deepEqual(
    binarySearchById(values, '3', (value) => value.id),
    { found: false, index: 1 },
  )
})

test('compareChatIds retains lexical ordering for non-numeric legacy ids', () => {
  assert.equal(compareChatIds('msg_10', 'msg_2') < 0, true)
})

test('message and part upserts preserve numeric Agena order without duplicates', () => {
  const messages: MessageEntry[] = [
    { info: info('2'), parts: [part('2', 'two'), part('10', 'ten')] },
    { info: info('10'), parts: [] },
  ]

  upsertMessageEntryIn(messages, info('3'))
  upsertMessageEntryIn(messages, { ...info('10'), finish: 'stop' })
  upsertPart(messages[0]!, part('3', 'three'), '')
  upsertPart(messages[0]!, part('10', 'updated'), '')

  assert.deepEqual(
    messages.map((entry) => entry.info.id),
    ['2', '3', '10'],
  )
  assert.equal(messages.filter((entry) => entry.info.id === '10').length, 1)
  assert.deepEqual(
    messages[0]!.parts.map((entry) => entry.id),
    ['2', '3', '10'],
  )
  assert.equal(messages[0]!.parts.find((entry) => entry.id === '10')?.text, 'updated')
})
