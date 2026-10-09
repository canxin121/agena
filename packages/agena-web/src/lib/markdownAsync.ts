import { i18n } from '@/i18n'
import { renderMarkdown, renderMarkdownPlainText, type MarkdownUiLabels } from './markdown'
import type { MarkdownWork } from './markdown.worker'

type Work = {
  request: MarkdownWork
  resolve: (parts: string[]) => void
  reject: (error: Error) => void
  finish: () => void
}
let worker: Worker | undefined
let sequence = 0
let streamSequence = 0
let running: Work | undefined
const pending = new Map<number, Work>()
// One expensive Markdown body must not hold every Part's renderer forever.
const WORK_BUDGET_MS = 250
let deadline: ReturnType<typeof setTimeout> | undefined

function clearDeadline() {
  clearTimeout(deadline)
  deadline = undefined
}

export function createMarkdownStreamKey(): number {
  return ++streamSequence
}
export function releaseMarkdownStream(key: number) {
  worker?.postMessage({ release: key })
}

function next() {
  if (running) return
  const entry = pending.entries().next().value
  if (!entry) return
  pending.delete(entry[0])
  running = entry[1]
  if (!worker) createWorker()
  const currentWorker = worker!
  const work = running!
  deadline = setTimeout(() => {
    if (worker !== currentWorker || running !== work) return
    currentWorker.terminate()
    worker = undefined
    running = undefined
    deadline = undefined
    work.finish()
    // Preserve all source text when rich interpretation exceeds its budget.
    work.resolve([renderMarkdownPlainText(work.request.content)])
    next()
  }, WORK_BUDGET_MS)
  currentWorker.postMessage(work.request)
}

function createWorker() {
  const created = new Worker(new URL('./markdown.worker.ts', import.meta.url), { type: 'module' })
  worker = created
  created.onmessage = ({ data }: MessageEvent<{ id: number; parts?: string[]; error?: string }>) => {
    if (worker !== created || data.id !== running?.request.id) return
    const work = running
    running = undefined
    clearDeadline()
    work.finish()
    if (data.error) work.reject(new Error(data.error))
    else work.resolve(data.parts ?? [])
    next()
  }
  created.onerror = () => {
    if (worker !== created) return
    created.terminate()
    worker = undefined
    clearDeadline()
    const work = [running, ...pending.values()]
    running = undefined
    pending.clear()
    for (const item of work) {
      if (!item) continue
      item.finish()
      item.reject(new Error(i18n.global.t('errors.markdown.workerFailed')))
    }
  }
}

/** One shared worker and one bounded parse. Cancellation removes queued
 * bodies so streaming and hidden tabs cannot create an ever-growing backlog.
 */
export function renderMarkdownAsync(
  content: string,
  labels: Partial<MarkdownUiLabels>,
  signal: AbortSignal,
  streamKey?: number,
): Promise<string[]> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
  if (typeof Worker === 'undefined') return Promise.resolve([renderMarkdown(content, labels)])
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
        streamKey,
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
