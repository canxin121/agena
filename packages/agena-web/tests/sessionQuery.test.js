import test from 'node:test'
import assert from 'node:assert/strict'

import {
  patchSessionIdInQuery,
  readSessionIdFromFullPath,
  readSessionIdFromQuery,
} from '../src/app/navigation/sessionQuery.ts'

test('readSessionIdFromQuery: reads canonical sessionId key only', () => {
  assert.equal(readSessionIdFromQuery({ sessionid: 'legacy-1' }), '')
  assert.equal(readSessionIdFromQuery({ sessionId: 'camel-1' }), 'camel-1')
  assert.equal(readSessionIdFromQuery({ session: 'current-1' }), '')
})

test('patchSessionIdInQuery: writes canonical key and preserves unrelated parameters', () => {
  const query = { foo: 'bar', sessionId: 'old' }
  const next = patchSessionIdInQuery(query, ' new ')
  assert.equal(next.foo, 'bar')
  assert.equal(next.sessionId, 'new')
  assert.equal(query.sessionId, 'old')
})

test('patchSessionIdInQuery: defaults to canonical key for new query', () => {
  const next = patchSessionIdInQuery({ foo: 'bar' }, 'new')
  assert.equal(next.foo, 'bar')
  assert.equal(next.sessionId, 'new')
})

test('patchSessionIdInQuery: clears the canonical session key for empty values', () => {
  const next = patchSessionIdInQuery({ sessionId: 'b', foo: 'bar' }, '   ')
  assert.equal(next.foo, 'bar')
  assert.equal('sessionId' in next, false)
})

test('readSessionIdFromFullPath: parses canonical key from URL path', () => {
  assert.equal(readSessionIdFromFullPath('/chat?sessionid=legacy-2'), '')
  assert.equal(readSessionIdFromFullPath('/chat?sessionId=camel-2'), 'camel-2')
  assert.equal(readSessionIdFromFullPath('/chat?session=current-2'), '')
  assert.equal(readSessionIdFromFullPath('/chat?foo=bar'), '')
})

test('readSessionIdFromFullPath: excludes fragments without losing encoded hash characters', () => {
  assert.equal(readSessionIdFromFullPath('/chat?sessionId=session-1#message-2'), 'session-1')
  assert.equal(readSessionIdFromFullPath('/chat#message?sessionId=session-1'), '')
  assert.equal(readSessionIdFromFullPath('/chat?sessionId=session%23one#message-2'), 'session#one')
})

test('readSessionIdFromFullPath: matches query parsing for repeated sessionId values', () => {
  assert.equal(readSessionIdFromFullPath('/chat?sessionId=&sessionId=%20session-2%20'), 'session-2')
  assert.equal(readSessionIdFromQuery({ sessionId: ['', ' session-2 '] }), 'session-2')
})
