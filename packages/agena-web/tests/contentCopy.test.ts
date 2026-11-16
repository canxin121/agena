import { describe, expect, test } from 'bun:test'
import type { IBuffer, IBufferLine } from '@xterm/xterm'
import { terminalBufferPlainText } from '../src/lib/contentCopy'

type Row = string | { text: string; wrapped: boolean }

// A cell distinguishes a written space (code 32) from unused padding (code 0).
function buffer(rows: Row[], baseY = 0, cursorY = 0, cols = 24): IBuffer {
  const lines = rows.map((row): IBufferLine => {
    const text = typeof row === 'string' ? row : row.text
    const cells = Array.from({ length: cols }, (_, col) => text[col] ?? null)
    return {
      length: cols,
      isWrapped: typeof row !== 'string' && row.wrapped,
      getCell: (col: number) =>
        col >= 0 && col < cols ? { getCode: () => cells[col]?.codePointAt(0) ?? 0 } : undefined,
      translateToString: (trimRight = false, start = 0, end = cols) => {
        const value = cells
          .slice(start, end)
          .map((cell) => cell ?? ' ')
          .join('')
        return trimRight ? value.trimEnd() : value
      },
    } as IBufferLine
  })
  return { baseY, cursorY, length: lines.length, getLine: (row: number) => lines[row] } as IBuffer
}

describe('terminal output text', () => {
  test('the default includes log scrollback and the current screen', () => {
    const source = buffer(['history one', 'history two', 'screen first', 'screen last'], 2, 1)
    expect(terminalBufferPlainText(source)).toBe('history one\nhistory two\nscreen first\nscreen last')
  })

  test('a final screen excludes scrollback retained by a terminal resize', () => {
    const history = Array.from({ length: 56 }, (_, index) => `old row ${index}`)
    const screen = ['final first', ...Array.from({ length: 22 }, () => ''), 'final last']
    const source = buffer([...history, ...screen], 56, 23)
    expect(terminalBufferPlainText(source, source.baseY)).toBe(screen.join('\n'))
    expect(terminalBufferPlainText(source)).toStartWith('old row 0\n')
  })

  test('soft-wrapped rows join without inserting a newline', () => {
    const source = buffer(['prior', 'abcde', { text: 'fgh', wrapped: true }, ''], 1, 2, 5)
    expect(terminalBufferPlainText(source, source.baseY)).toBe('abcdefgh\n')
  })

  test('written spaces and cursor trailing blank lines are preserved', () => {
    const source = buffer(['old', 'done  ', '', '', ''], 1, 2)
    expect(terminalBufferPlainText(source, source.baseY)).toBe('done  \n\n')
  })

  test('an empty screen omits unused rows', () => {
    expect(terminalBufferPlainText(buffer(['', '', '']))).toBe('')
    const source = buffer(['old', '', '', ''], 1, 0)
    expect(terminalBufferPlainText(source, source.baseY)).toBe('')
  })

  test('text below the cursor remains part of the final screen', () => {
    const source = buffer(['old', 'first', '', 'last', ''], 1, 0)
    expect(terminalBufferPlainText(source, source.baseY)).toBe('first\n\nlast')
  })

  test('a negative start includes row zero and a start past the buffer is empty', () => {
    const source = buffer(['first', 'last'], 1, 0)
    expect(terminalBufferPlainText(source, -5)).toBe('first\nlast')
    expect(terminalBufferPlainText(source, source.length)).toBe('')
  })
})
