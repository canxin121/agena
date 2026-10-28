import { i18n } from '@/i18n'
import { renderMarkdown, renderMarkdownPlainText, type MarkdownUiLabels, type MarkdownReferences } from './markdown'
import type { MarkdownWork } from './markdown.worker'

type Work = {
  request: MarkdownWork
  resolve: (parts: string[]) => void
  reject: (error: Error) => void
  finish: () => void
}
let worker: Worker | undefined
let workerReady = false
let sequence = 0
let streamSequence = 0
let running: Work | undefined
const pending = new Map<number, Work>()
// One expensive Markdown body must not hold every Part's renderer forever.
const WORK_BUDGET_MS = 250
const STARTUP_BUDGET_MS = 5000
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

function armDeadline(currentWorker: Worker, work: Work, budget: number) {
  clearDeadline()
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
  }, budget)
}

function next() {
  if (running) return
  const entry = pending.entries().next().value
  if (!entry) return
  pending.delete(entry[0])
  running = entry[1]
  try {
    if (!worker) createWorker()
    const currentWorker = worker!
    const work = running!
    // Module download/initialization is not parsing time. In particular, a cold
    // dev worker must not be killed every 250 ms before it finishes loading.
    armDeadline(currentWorker, work, workerReady ? WORK_BUDGET_MS : STARTUP_BUDGET_MS)
    currentWorker.postMessage(work.request)
  } catch (error) {
    // Worker construction/postMessage can throw before an error event exists.
    // Release the running slot so subsequent bodies can retry normally.
    failWorker(error instanceof Error ? error : new Error(String(error)))
  }
}

function failWorker(error: Error) {
  worker?.terminate()
  worker = undefined
  clearDeadline()
  const work = [running, ...pending.values()]
  running = undefined
  pending.clear()
  for (const item of work) {
    if (!item) continue
    item.finish()
    item.reject(error)
  }
}

function createWorker() {
  const created = new Worker(new URL('./markdown.worker.ts', import.meta.url), { type: 'module' })
  worker = created
  workerReady = false
  created.onmessage = ({ data }: MessageEvent<{ ready?: boolean; id?: number; parts?: string[]; error?: string }>) => {
    if (worker !== created) return
    if (data.ready) {
      workerReady = true
      if (running) armDeadline(created, running, WORK_BUDGET_MS)
      return
    }
    if (!running || data.id !== running.request.id) return
    workerReady = true
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
    failWorker(new Error(i18n.global.t('errors.markdown.workerFailed')))
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
  references?: MarkdownReferences,
): Promise<string[]> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'))
  if (typeof Worker === 'undefined') return Promise.resolve([renderMarkdown(content, labels, references)])
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
        references,
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
