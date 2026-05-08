import { describe, expect, test } from 'bun:test'

import { normalizeSseBuffer, parseSseEventBlock } from './sse'

describe('agenaApi SSE helpers', () => {
  test('normalizeSseBuffer rewrites CRLF and CR to LF', () => {
    expect(normalizeSseBuffer('a\r\nb\rc')).toBe('a\nb\nc')
  })

  test('parseSseEventBlock parses event id and multiline data', () => {
    expect(
      parseSseEventBlock('event: session_event\nid: 12\ndata: {"a":1}\ndata: {"b":2}'),
    ).toEqual({
      event: 'session_event',
      id: '12',
      data: '{"a":1}\n{"b":2}',
    })
  })

  test('parseSseEventBlock ignores comments and defaults event to message', () => {
    expect(parseSseEventBlock(':keepalive\ndata: hello')).toEqual({
      event: 'message',
      id: '',
      data: 'hello',
    })
  })
})
