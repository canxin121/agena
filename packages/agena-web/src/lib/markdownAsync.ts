import { renderMarkdown, type MarkdownUiLabels } from './markdown'
import type { MarkdownWork } from './markdown.worker'

type Work = {
  request: MarkdownWork
  resolve: (html: string) => void
  reject: (error: Error) => void
  finish: () => void
}
let worker: Worker | undefined
let sequence = 0
let running: Work | undefined
const pending = new Map<number, Work>()

function next() {
  if (running || !worker) return
  const entry = pending.entries().next().value
  if (!entry) return
  pending.delete(entry[0])
  running = entry[1]
  worker.postMessage(running.request)
}

/** One shared worker and one in-flight parse. Cancellation removes queued
 * bodies so streaming and hidden tabs cannot create an ever-growing backlog.
 */
export function renderMarkdownAsync(
  content: string,
  labels: Partial<MarkdownUiLabels>,
  signal: AbortSignal,
): Promise<string> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
  if (typeof Worker === 'undefined') return Promise.resolve(renderMarkdown(content, labels))
  if (!worker) {
    worker = new Worker(new URL('./markdown.worker.ts', import.meta.url), { type: 'module' })
    worker.onmessage = ({ data }: MessageEvent<{ id: number; html?: string; error?: string }>) => {
      if (data.id !== running?.request.id) return
      const work = running
      running = undefined
      work.finish()
      if (data.error) work.reject(new Error(data.error))
      else work.resolve(data.html ?? '')
      next()
    }
    worker.onerror = () => {
      worker?.terminate()
      worker = undefined
      const work = [running, ...pending.values()]
      running = undefined
      pending.clear()
      for (const item of work) {
        if (!item) continue
        item.finish()
        item.reject(new Error('Markdown worker failed'))
      }
    }
  }
  return new Promise((resolve, reject) => {
    const id = ++sequence
    const abort = () => {
      pending.delete(id)
      signal.removeEventListener('abort', abort)
      reject(new DOMException('Aborted', 'AbortError'))
    }
    const { expandLinesTitle = (lines: number) => `Expand (${lines} lines)`, ...serialLabels } = labels
    pending.set(id, {
      request: {
        id,
        content,
        labels: { ...serialLabels, expandLinesTemplate: expandLinesTitle('__AGENA_LINES__' as unknown as number) },
      },
      resolve,
      reject,
      finish: () => signal.removeEventListener('abort', abort),
    })
    signal.addEventListener('abort', abort, { once: true })
    next()
  })
}
