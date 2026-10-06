import { renderMarkdown, type MarkdownUiLabels } from './markdown'

export type WorkerMarkdownLabels = Partial<Omit<MarkdownUiLabels, 'expandLinesTitle'>> & { expandLinesTemplate: string }
export type MarkdownWork = { id: number; content: string; labels: WorkerMarkdownLabels }
const workerGlobal = globalThis as unknown as {
  onmessage: (event: MessageEvent<MarkdownWork>) => void
  postMessage: (value: { id: number; html?: string; error?: string }) => void
}
workerGlobal.onmessage = ({ data }) => {
  try {
    workerGlobal.postMessage({
      id: data.id,
      html: renderMarkdown(data.content, {
        ...data.labels,
        expandLinesTitle: (lines) => data.labels.expandLinesTemplate.replace('__AGENA_LINES__', String(lines)),
      }),
    })
  } catch (error) {
    workerGlobal.postMessage({ id: data.id, error: String(error) })
  }
}
