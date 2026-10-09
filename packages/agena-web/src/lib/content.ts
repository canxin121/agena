import { i18n } from '@/i18n'
import { apiJson, apiResponse, apiUrl } from './api'
import { SseFrames } from './sseFrames'
import { updateDocument, validateDocument, type ContentDocument, type DocumentMutation } from './contentDocument'

export type ContentCursor = { epoch: string; sequence: number }
export type ContentRef = { resource_id: string; kind: 'text' | 'log' | 'structured' | 'document' | 'terminal' }
export type TerminalColor =
  | { type: 'indexed'; index: number }
  | { type: 'rgb'; red: number; green: number; blue: number }
export type TerminalSnapshot = {
  rows: number
  cols: number
  cursor_row: number
  cursor_col: number
  cursor_visible: boolean
  alternate_screen: boolean
  bracketed_paste: boolean
  application_cursor: boolean
  cells: {
    row: number
    col: number
    text: string
    attributes: {
      foreground: TerminalColor | null
      background: TerminalColor | null
      bold: boolean
      dim: boolean
      italic: boolean
      underline: boolean
      inverse: boolean
    }
  }[]
}
export type ContentPayload =
  | { type: 'text'; text: string }
  | { type: 'log'; stream: 'stdout' | 'stderr'; text: string }
  | { type: 'structured'; base_cursor: ContentCursor; event: DocumentMutation }
  | { type: 'structured_snapshot'; document: ContentDocument }
  | { type: 'terminal'; screen: TerminalSnapshot }
  | { type: 'terminal_patch'; base_cursor: ContentCursor; screen: TerminalSnapshot; rows_changed: number[] }
export type ContentChunk = { cursor: ContentCursor; captured_at_ms: number; payload: ContentPayload }
const contentEncoder = new TextEncoder()
const chunkCosts = new WeakMap<ContentChunk, number>()
export function contentChunkBytes(chunk: ContentChunk): number {
  let bytes = chunkCosts.get(chunk)
  if (bytes === undefined) {
    bytes = contentEncoder.encode(JSON.stringify(chunk)).byteLength
    chunkCosts.set(chunk, bytes)
  }
  return bytes
}
export type ContentResource = ContentRef & {
  owner_session_id: number
  part_id: number
  state: 'active' | 'complete' | 'interrupted'
  cursor: ContentCursor
  committed_cursor: ContentCursor
  retained_ranges: { first: number; last: number }[]
  total_bytes: number
  dropped_bytes: number
  capture_error?: string
}
export type ContentPage = {
  resource: ContentResource
  chunks: ContentChunk[]
  next_cursor: ContentCursor
  has_more: boolean
  gap: boolean
}

/** Standard VT re-encoding of server-owned cells; observer layout never resizes the process. */
export function terminalSnapshotText(screen: TerminalSnapshot, rows?: number[]): string {
  const output = ['\x1b[0m']
  if (rows) output.push(...rows.map((row) => `\x1b[${row + 1};1H\x1b[2K`))
  else output.push(`\x1b[?1049${screen.alternate_screen ? 'h' : 'l'}\x1b[2J\x1b[H`)
  for (const run of screen.cells) {
    output.push(`\x1b[${run.row + 1};${run.col + 1}H\x1b[0m`)
    const attrs = run.attributes
    for (const [enabled, code] of [
      [attrs.bold, 1],
      [attrs.dim, 2],
      [attrs.italic, 3],
      [attrs.underline, 4],
      [attrs.inverse, 7],
    ]) {
      if (enabled) output.push(`\x1b[${code}m`)
    }
    for (const [color, channel] of [
      [attrs.foreground, 38],
      [attrs.background, 48],
    ] as const) {
      if (color?.type === 'indexed') output.push(`\x1b[${channel};5;${color.index}m`)
      else if (color?.type === 'rgb') output.push(`\x1b[${channel};2;${color.red};${color.green};${color.blue}m`)
    }
    output.push(run.text)
  }
  output.push(
    `\x1b[0m\x1b[${screen.cursor_row + 1};${screen.cursor_col + 1}H\x1b[?25${screen.cursor_visible ? 'h' : 'l'}`,
  )
  output.push(`\x1b[?2004${screen.bracketed_paste ? 'h' : 'l'}\x1b[?1${screen.application_cursor ? 'h' : 'l'}`)
  return output.join('')
}

/** A cursor reducer shared by content widgets. A stale snapshot never
 * replaces newer displayed bytes; explicit retention gaps remain visible. */
export class ContentBuffer {
  cursor: ContentCursor | null = null
  resource: ContentResource | null = null
  chunks: ContentChunk[] = []
  gap = false
  windowed = false
  private recovering = false
  private documentCursor: ContentCursor | null = null
  document: ContentDocument | null = null
  private bytes = 0
  private terminalCursor: ContentCursor | null = null
  private terminalScreen: TerminalSnapshot | null = null
  private terminalCapturedAt = 0
  private readonly maxBytes: number

  constructor(maxBytes = 2 * 1024 * 1024) {
    this.maxBytes = maxBytes
  }

  apply(page: ContentPage): { appended: ContentChunk[]; reset: boolean; missing: boolean } {
    if (this.resource && this.resource.resource_id !== page.resource.resource_id)
      throw new Error(i18n.global.t('errors.content.bufferIdentityChanged'))
    const reset = this.recovering || (this.cursor !== null && this.cursor.epoch !== page.next_cursor.epoch)
    if (page.resource.cursor.epoch !== page.next_cursor.epoch)
      throw new Error(i18n.global.t('errors.content.mixedResourceEpochs'))
    const appended: ContentChunk[] = []
    let expected = reset ? undefined : this.cursor?.sequence
    let terminalCursor = reset ? null : this.terminalCursor
    let documentCursor = reset ? null : this.documentCursor
    let document = reset ? null : this.document
    const recover = () => {
      // Keep the last usable presentation while reconnecting from a checkpoint.
      // Only a validated replacement page clears the old display window.
      this.cursor = null
      this.recovering = true
      return { appended: [], reset: false, missing: true }
    }
    for (const chunk of page.chunks) {
      if (chunk.cursor.epoch !== page.next_cursor.epoch)
        throw new Error(i18n.global.t('errors.content.mixedContentEpochs'))
      if (expected !== undefined && chunk.cursor.sequence <= expected) continue
      if (expected !== undefined && chunk.cursor.sequence !== expected + 1 && !page.gap)
        return { appended: [], reset: false, missing: true }
      appended.push(chunk)
      expected = chunk.cursor.sequence
      if (chunk.payload.type === 'structured_snapshot') {
        try {
          document = validateDocument(chunk.payload.document)
        } catch {
          return recover()
        }
        documentCursor = chunk.cursor
      } else if (chunk.payload.type === 'structured') {
        const base = chunk.payload.base_cursor
        if (
          !document ||
          !documentCursor ||
          documentCursor.epoch !== base.epoch ||
          documentCursor.sequence !== base.sequence
        )
          return recover()
        try {
          document = updateDocument(document, chunk.payload.event)
        } catch {
          return recover()
        }
        documentCursor = chunk.cursor
      }
      if (chunk.payload.type === 'terminal') terminalCursor = chunk.cursor
      else if (chunk.payload.type === 'terminal_patch') {
        const base = chunk.payload.base_cursor
        if (!terminalCursor || terminalCursor.epoch !== base.epoch || terminalCursor.sequence !== base.sequence)
          return recover()
        terminalCursor = chunk.cursor
      }
    }
    if (reset) {
      this.chunks = []
      this.bytes = 0
      this.cursor = null
      this.resource = null
      this.gap = false
      this.windowed = false
      this.terminalScreen = null
    }
    if (!this.cursor && appended[0] && appended[0].cursor.sequence > 1 && page.resource.kind !== 'structured')
      this.windowed = true
    for (const chunk of appended) {
      if (chunk.payload.type === 'terminal') {
        this.terminalScreen = chunk.payload.screen
        this.terminalCapturedAt = chunk.captured_at_ms
      } else if (chunk.payload.type === 'terminal_patch' && this.terminalScreen) {
        this.terminalCapturedAt = chunk.captured_at_ms
        const changed = new Set(chunk.payload.rows_changed)
        this.terminalScreen = {
          ...chunk.payload.screen,
          cells: [
            ...this.terminalScreen.cells.filter((run) => !changed.has(run.row)),
            ...chunk.payload.screen.cells,
          ].sort((left, right) => left.row - right.row || left.col - right.col),
        }
      }
      this.chunks.push(chunk)
      this.bytes += contentChunkBytes(chunk) + 64
    }
    let trim = 0
    while ((this.bytes > this.maxBytes || this.chunks.length - trim > 4096) && this.chunks.length - trim > 1) {
      this.windowed = true
      this.bytes -= contentChunkBytes(this.chunks[trim++]!) + 64
    }
    if (trim) this.chunks = this.chunks.slice(trim)
    if (!this.cursor || page.next_cursor.sequence >= this.cursor.sequence) this.cursor = page.next_cursor
    if (!this.resource || page.resource.cursor.sequence >= this.resource.cursor.sequence) {
      // Terminal state wins an equal-position race with a delayed live page.
      if (
        this.resource?.state !== 'active' &&
        page.resource.state === 'active' &&
        this.resource?.cursor.sequence === page.resource.cursor.sequence
      ) {
        this.resource = { ...page.resource, state: this.resource.state }
      } else this.resource = page.resource
    }
    this.gap ||= page.gap || page.resource.dropped_bytes > 0
    this.recovering = false
    this.documentCursor = documentCursor
    this.terminalCursor = terminalCursor
    this.document = document
    return { appended, reset, missing: false }
  }

  text(): string {
    return this.chunks.map(({ payload }) => ('text' in payload ? payload.text : '')).join('')
  }

  replayChunks(): ContentChunk[] {
    if (this.resource?.kind !== 'terminal') return this.chunks.slice()
    if (!this.terminalCursor || !this.terminalScreen) return []
    return [
      {
        cursor: this.terminalCursor,
        captured_at_ms: this.terminalCapturedAt,
        payload: { type: 'terminal', screen: this.terminalScreen },
      },
      ...this.chunks.filter((chunk) => chunk.cursor.sequence > this.terminalCursor!.sequence),
    ]
  }
}

export function readContent(sessionId: string, resourceId: string, after: ContentCursor): Promise<ContentPage> {
  const query = new URLSearchParams({
    epoch: after.epoch,
    after: String(after.sequence),
    max_bytes: String(1024 * 1024),
  })
  return apiJson(`/api/v1/sessions/${encodeURIComponent(sessionId)}/content/${encodeURIComponent(resourceId)}?${query}`)
}

/** The endpoint carries the same pages used by REST and the Rust SDK.
 * Reconnection is owned by the resource consumer and resumes its cursor. */
export async function streamContent(
  sessionId: string,
  resourceId: string,
  after: ContentCursor | null,
  signal: AbortSignal,
  onPage: (page: ContentPage) => void,
): Promise<void> {
  const query = new URLSearchParams({ max_bytes: String(64 * 1024) })
  if (after) {
    query.set('epoch', after.epoch)
    query.set('after', String(after.sequence))
  }
  const endpoint = `/api/v1/sessions/${encodeURIComponent(sessionId)}/content/${encodeURIComponent(resourceId)}/stream?${query}`
  const response = await apiResponse(apiUrl(endpoint), { signal, headers: { accept: 'text/event-stream' } })
  if (!response.body) throw new Error(i18n.global.t('errors.content.streamingUnavailable'))
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  const frames = new SseFrames()
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read()
      if (done) break
      for (const frame of frames.push(decoder.decode(value, { stream: true }))) {
        const lines = frame.split('\n')
        const event = lines
          .find((line) => line.startsWith('event:'))
          ?.slice(6)
          .trim()
        const body = lines
          .filter((line) => line.startsWith('data:'))
          .map((line) => line.slice(5).trimStart())
          .join('\n')
        if (!body) continue
        if (event === 'content_error') {
          const failure = JSON.parse(body)
          throw new Error(failure.problem?.user?.fallback || 'Content is unavailable')
        }
        if (event === 'content') onPage(JSON.parse(body) as ContentPage)
      }
    }
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
export type ContentFormat = 'plain' | 'markdown' | 'code' | 'diff' | 'json'
