/** Streaming SSE delimiter scanner. Retain incomplete segments without
 * joining/rescanning a potentially multi-megabyte JSON frame on each read.
 */
export class SseFrames {
  private segments: string[] = []
  private pendingNewline = false
  private skipLf = false

  push(decoded: string): string[] {
    if (!decoded) return []
    const text = (this.skipLf && decoded.startsWith('\n') ? decoded.slice(1) : decoded)
      .replace(/\r\n/g, '\n')
      .replace(/\r/g, '\n')
    this.skipLf = decoded.endsWith('\r')
    const frames: string[] = []
    let start = 0
    for (let i = 0; i < text.length; i++) {
      if (text[i] === '\n' && this.pendingNewline) {
        // The previous newline was deferred, so neither separator belongs
        // to the payload, including when the pair crosses network chunks.
        this.segments.push(text.slice(start, i))
        frames.push(this.segments.join(''))
        this.segments = []
        this.pendingNewline = false
        start = i + 1
      } else if (text[i] === '\n') {
        this.segments.push(text.slice(start, i))
        this.pendingNewline = true
        start = i + 1
      } else if (this.pendingNewline) {
        this.segments.push('\n')
        this.pendingNewline = false
      }
    }
    if (start < text.length) this.segments.push(text.slice(start))
    return frames
  }
}
