import type { IBuffer } from '@xterm/xterm'
import { type ContentCursor, type ContentPage } from './content'

const MAX_COPY_BYTES = 16 * 1024 * 1024

/** Read a fixed source position, including explicit markers for missing tails
 * and empty loss pages. Copy never silently stitches disjoint retained ranges. */
export async function readContentForCopy(
  target: ContentCursor,
  read: (after: ContentCursor) => Promise<ContentPage>,
  gapMarker: string,
  limitMessage: string,
  consume: (text: string) => Promise<void>,
): Promise<void> {
  let cursor = { ...target, sequence: 0 }
  let bytes = 0
  let marked = false
  const encoder = new TextEncoder()
  const mark = async () => { await consume(`\n[${gapMarker}]\n`); marked = true }
  for (;;) {
    const page = await read(cursor)
    if (page.next_cursor.epoch !== target.epoch) throw new Error('Content generation changed during copy')
    const before = cursor.sequence
    for (const chunk of page.chunks) {
      if (chunk.cursor.epoch !== target.epoch) throw new Error('Content generation changed during copy')
      if (chunk.cursor.sequence > target.sequence) break
      if (chunk.cursor.sequence <= cursor.sequence) continue
      if (chunk.cursor.sequence !== cursor.sequence + 1) await mark()
      if ('text' in chunk.payload) {
        bytes += encoder.encode(chunk.payload.text).byteLength
        if (bytes > MAX_COPY_BYTES) throw new Error(limitMessage)
        await consume(chunk.payload.text)
      }
      cursor = chunk.cursor
    }
    if (!page.has_more || cursor.sequence >= target.sequence || page.next_cursor.sequence >= target.sequence) {
      if (cursor.sequence < target.sequence || (page.resource.dropped_bytes > 0 && !marked)) await mark()
      return
    }
    if (cursor.sequence === before) throw new Error(gapMarker)
  }
}

/** Standard emulator cells distinguish written spaces from empty padding.
 * Wrapped rows join without introducing line breaks; the final cursor row
 * preserves real blank and trailing lines. */
export function terminalBufferPlainText(buffer: IBuffer): string {
  let end = buffer.baseY + buffer.cursorY
  for (let row = buffer.length - 1; row > end; row--) {
    const line = buffer.getLine(row)
    if (line && Array.from({ length: line.length }, (_, col) => line.getCell(col)?.getCode() || 0).some(Boolean)) { end = row; break }
  }
  const output: string[] = []
  for (let row = 0; row <= end; row++) {
    const line = buffer.getLine(row)
    if (!line) continue
    let col = line.length
    while (col > 0 && !line.getCell(col - 1)?.getCode()) col--
    output.push(line.translateToString(false, 0, col))
    if (row < end && !buffer.getLine(row + 1)?.isWrapped) output.push('\n')
  }
  return output.join('')
}
