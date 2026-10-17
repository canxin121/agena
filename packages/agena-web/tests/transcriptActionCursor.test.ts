import { describe, expect, test } from 'bun:test'
import { actionCursorNeighbour, type ActionCursorRect } from '../src/pages/chat/transcriptActionCursor'

const rect = (left: number, top: number, right: number, bottom: number): ActionCursorRect => ({
  left,
  top,
  right,
  bottom,
})
const stop = (value: string, box: ActionCursorRect) => ({ value, rect: box })

describe('one transcript cursor for text and actions', () => {
  test('a multiline option is one destination and Down leaves its full rectangle', () => {
    const title = rect(0, 0, 240, 20)
    const long = rect(0, 30, 240, 150)
    const next = rect(0, 160, 240, 200)
    expect(actionCursorNeighbour(title, [stop('long choice', long), stop('next choice', next)], 'down')).toBe(
      'long choice',
    )
    expect(actionCursorNeighbour(long, [stop('title', title), stop('next choice', next)], 'down')).toBe('next choice')
    expect(actionCursorNeighbour(long, [stop('title', title), stop('next choice', next)], 'up')).toBe('title')
  })

  test('horizontal arrows move between whole buttons while vertical movement preserves the desired column', () => {
    const left = rect(0, 30, 80, 60),
      right = rect(100, 30, 240, 60)
    const above = rect(0, 0, 240, 20)
    expect(actionCursorNeighbour(left, [stop('right', right)], 'right')).toBe('right')
    expect(actionCursorNeighbour(right, [stop('left', left)], 'left')).toBe('left')
    const candidates = [stop('left', left), stop('right', right)]
    expect(actionCursorNeighbour(above, candidates, 'down', 170)).toBe('right')
    expect(actionCursorNeighbour(above, candidates, 'down', 20)).toBe('left')
  })

  test('responsive wrapping follows the next displayed row and can return to prose outside the operation', () => {
    const first = rect(0, 30, 110, 60),
      wrapped = rect(0, 70, 190, 100)
    const prose = rect(0, 115, 500, 135)
    expect(
      actionCursorNeighbour(first, [stop('wrapped action', wrapped), stop('following prose', prose)], 'down'),
    ).toBe('wrapped action')
    expect(actionCursorNeighbour(wrapped, [stop('first', first), stop('following prose', prose)], 'down')).toBe(
      'following prose',
    )
    expect(actionCursorNeighbour(prose, [stop('first', first), stop('wrapped action', wrapped)], 'up')).toBe(
      'wrapped action',
    )
  })

  test('document boundaries do not wrap or manufacture an activation', () => {
    const current = rect(0, 0, 100, 20)
    expect(actionCursorNeighbour(current, [], 'down')).toBeNull()
    expect(actionCursorNeighbour(current, [stop('below', rect(0, 30, 100, 50))], 'up')).toBeNull()
    expect(actionCursorNeighbour(current, [stop('below', rect(0, 30, 100, 50))], 'right')).toBeNull()
  })
})
