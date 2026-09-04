import test from 'node:test'
import assert from 'node:assert/strict'

import {
  binarySearchById,
  compareChatIds,
  foldedMessageCount,
  upsertMessageEntryIn,
  upsertPart,
} from '../src/stores/chat/messageIndex'
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

test('compareChatIds rejects removed non-numeric chat ids', () => {
  assert.throws(() => compareChatIds('msg_10', 'msg_2'), /must be decimal integers/)
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

test('foldedMessageCount ignores adjacent assistant rounds but keeps user boundaries', () => {
  const messages: MessageEntry[] = [
    { info: { ...info('10'), role: 'assistant' }, parts: [part('11', 'old')] },
    { info: { ...info('12'), role: 'assistant' }, parts: [part('13', 'new')] },
    { info: { ...info('14'), role: 'user' }, parts: [part('15', 'question')] },
    { info: { ...info('16'), role: 'assistant' }, parts: [part('17', 'answer')] },
  ]

  assert.equal(foldedMessageCount(messages), 3)
})

test('session 13 shaped history crosses a multi-thousand-part assistant span', () => {
  const messages: MessageEntry[] = [
    { info: { ...info('1110'), role: 'user' }, parts: [part('1111', 'previous question')] },
    ...Array.from({ length: 2_000 }, (_, index) => ({
      info: { ...info(String(1200 + index)), role: 'assistant' },
      parts: [part(String(2200 + index), `activity ${index}`)],
    })),
    { info: { ...info('3550'), role: 'user' }, parts: [part('3551', 'latest question')] },
  ]

  // The 2,000 assistant rounds are one folded block; the two user turns are
  // the boundaries an upward load must expose.
  assert.equal(foldedMessageCount(messages), 3)
})
