import { describe, expect, test } from 'bun:test'

import { readChatRouteSessionId, readChatRouteSlash, readChatRouteWorkspaceId } from './chatRouteState'

describe('chatRouteState', () => {
  test('reads numeric session and workspace ids from route query values', () => {
    expect(readChatRouteSessionId('8')).toBe(8)
    expect(readChatRouteWorkspaceId('2')).toBe(2)
  })

  test('returns null for non-string or invalid numeric route ids', () => {
    expect(readChatRouteSessionId(undefined)).toBe(null)
    expect(readChatRouteSessionId('abc')).toBe(null)
    expect(readChatRouteWorkspaceId(['2'])).toBe(null)
  })

  test('trims slash command text and normalizes unsupported values to empty string', () => {
    expect(readChatRouteSlash('  /continue  ')).toBe('/continue')
    expect(readChatRouteSlash(null)).toBe('')
  })
})
