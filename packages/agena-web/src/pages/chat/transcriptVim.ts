export type TranscriptVimMode = 'NAVIGATE' | 'INSERT' | 'VISUAL' | 'VISUAL LINE' | 'VISUAL BLOCK' | 'SEARCH'

export type TranscriptVimKey = {
  key: string
  ctrlKey?: boolean
  metaKey?: boolean
  altKey?: boolean
  shiftKey?: boolean
}

export type TranscriptVimAction =
  | { type: 'count'; digit: number }
  | { type: 'insert' }
  | { type: 'visual'; mode: 'character' | 'line' | 'block' }
  | { type: 'cancel' }
  | { type: 'copy' }
  | { type: 'yank-line' }
  | { type: 'toggle' }
  | { type: 'move'; direction: 'left' | 'right' | 'up' | 'down' }
  | { type: 'message'; direction: 'previous' | 'next' }
  | { type: 'line'; edge: 'start' | 'first-non-blank' | 'end' }
  | { type: 'word'; direction: 'forward' | 'backward'; edge: 'start' | 'end'; big: boolean }
  | { type: 'find'; direction: 'forward' | 'backward'; till: boolean }
  | { type: 'repeat-find'; reverse: boolean }
  | { type: 'goto-prefix' }
  | { type: 'end' }
  | { type: 'viewport-prefix' }
  | { type: 'view'; row: 'top' | 'middle' | 'bottom' }
  | { type: 'swap-endpoint'; blockCorner: boolean }
  | { type: 'text-object'; scope: 'around' | 'inner' | 'markdown' | 'message' | 'paragraph' }
  | { type: 'page'; direction: 'up' | 'down'; half: boolean }
  | { type: 'scroll-line'; direction: 'up' | 'down' }
  | { type: 'jump'; direction: 'backward' | 'forward' }
  | { type: 'search'; direction: 'forward' | 'backward' }
  | { type: 'search-repeat'; reverse: boolean }
  | { type: 'help' }
  | { type: 'interrupt' }
  | { type: 'new-session' }
  | { type: 'open-plan' }

function plain(key: TranscriptVimKey): boolean {
  return !key.ctrlKey && !key.metaKey && !key.altKey
}

function ctrlOnly(key: TranscriptVimKey): boolean {
  return key.ctrlKey === true && !key.metaKey && !key.altKey
}

/** Browser-key equivalent of crates/agena-tui/src/keymap/core.rs. */
export function resolveTranscriptVimAction(input: TranscriptVimKey): TranscriptVimAction | null {
  const key = input.key

  if (ctrlOnly(input) && key.toLowerCase() === 'c') return { type: 'interrupt' }
  if (ctrlOnly(input) && key.toLowerCase() === 'h') return { type: 'help' }
  if (ctrlOnly(input) && key.toLowerCase() === 'n') return { type: 'new-session' }
  if (ctrlOnly(input) && key.toLowerCase() === 'p') return { type: 'open-plan' }
  if (ctrlOnly(input) && key.toLowerCase() === 'k') return { type: 'message', direction: 'previous' }
  if (ctrlOnly(input) && key.toLowerCase() === 'j') return { type: 'message', direction: 'next' }
  if (ctrlOnly(input) && key.toLowerCase() === 'v') return { type: 'visual', mode: 'block' }
  if (ctrlOnly(input) && key.toLowerCase() === 'b') return { type: 'page', direction: 'up', half: false }
  if (ctrlOnly(input) && key.toLowerCase() === 'f') return { type: 'page', direction: 'down', half: false }
  if (ctrlOnly(input) && key.toLowerCase() === 'u') return { type: 'page', direction: 'up', half: true }
  if (ctrlOnly(input) && key.toLowerCase() === 'd') return { type: 'page', direction: 'down', half: true }
  if (ctrlOnly(input) && key.toLowerCase() === 'e') return { type: 'scroll-line', direction: 'down' }
  if (ctrlOnly(input) && key.toLowerCase() === 'y') return { type: 'scroll-line', direction: 'up' }
  if (ctrlOnly(input) && key.toLowerCase() === 'o') return { type: 'jump', direction: 'backward' }
  if (ctrlOnly(input) && key.toLowerCase() === 'i') return { type: 'jump', direction: 'forward' }

  if (!plain(input)) return null
  if (/^[1-9]$/.test(key)) return { type: 'count', digit: Number(key) }
  if (key === 'i') return { type: 'insert' }
  if (key.toLowerCase() === 'v' && input.shiftKey) return { type: 'visual', mode: 'line' }
  if (key === 'v') return { type: 'visual', mode: 'character' }
  if (key === 'V') return { type: 'visual', mode: 'line' }
  if (key === 'Escape') return { type: 'cancel' }
  if (key === 'y') return { type: 'copy' }
  if (key === 'Y') return { type: 'yank-line' }
  if (key === 'Enter') return { type: 'toggle' }
  if (key === 'ArrowLeft' || key === 'h') return { type: 'move', direction: 'left' }
  if (key === 'ArrowRight' || key === 'l') return { type: 'move', direction: 'right' }
  if (key === 'ArrowUp' || key === 'k') return { type: 'move', direction: 'up' }
  if (key === 'ArrowDown' || key === 'j') return { type: 'move', direction: 'down' }
  if (key === 'Home' || key === '0') return { type: 'line', edge: 'start' }
  if (key === '^') return { type: 'line', edge: 'first-non-blank' }
  if (key === 'End' || key === '$') return { type: 'line', edge: 'end' }
  if (key === 'w') return { type: 'word', direction: 'forward', edge: 'start', big: false }
  if (key === 'b') return { type: 'word', direction: 'backward', edge: 'start', big: false }
  if (key === 'e') return { type: 'word', direction: 'forward', edge: 'end', big: false }
  if (key === 'W') return { type: 'word', direction: 'forward', edge: 'start', big: true }
  if (key === 'B') return { type: 'word', direction: 'backward', edge: 'start', big: true }
  if (key === 'E') return { type: 'word', direction: 'forward', edge: 'end', big: true }
  if (key === 'f') return { type: 'find', direction: 'forward', till: false }
  if (key === 'F') return { type: 'find', direction: 'backward', till: false }
  if (key === 't') return { type: 'find', direction: 'forward', till: true }
  if (key === 'T') return { type: 'find', direction: 'backward', till: true }
  if (key === ';') return { type: 'repeat-find', reverse: false }
  if (key === ',') return { type: 'repeat-find', reverse: true }
  if (key === 'g') return { type: 'goto-prefix' }
  if (key === 'G') return { type: 'end' }
  if (key === 'z') return { type: 'viewport-prefix' }
  if (key === 'H') return { type: 'view', row: 'top' }
  if (key === 'M') return { type: 'view', row: 'middle' }
  if (key === 'L') return { type: 'view', row: 'bottom' }
  if (key === 'o') return { type: 'swap-endpoint', blockCorner: false }
  if (key === 'O') return { type: 'swap-endpoint', blockCorner: true }
  if (key === 'a') return { type: 'text-object', scope: 'around' }
  if (key === 'm') return { type: 'text-object', scope: 'markdown' }
  if (key === 'p') return { type: 'text-object', scope: 'paragraph' }
  if (key === 'PageUp' || (key === ' ' && input.shiftKey)) return { type: 'page', direction: 'up', half: false }
  if (key === 'PageDown' || key === ' ') return { type: 'page', direction: 'down', half: false }
  if (key === '/') return { type: 'search', direction: 'forward' }
  if (key === '?') return { type: 'search', direction: 'backward' }
  if (key === 'n') return { type: 'search-repeat', reverse: false }
  if (key === 'N') return { type: 'search-repeat', reverse: true }
  return null
}
