import { MarkdownStreamRenderer, renderMarkdown, type MarkdownUiLabels } from './markdown'

export type WorkerMarkdownLabels = Partial<Omit<MarkdownUiLabels, 'expandLinesTitle'>> & { expandLinesTemplate: string }
export type MarkdownWork = { id: number; content: string; labels: WorkerMarkdownLabels; streamKey?: number }
const streams = new Map<number, MarkdownStreamRenderer>()
const workerGlobal = globalThis as unknown as {
  onmessage: (event: MessageEvent<MarkdownWork | { release: number }>) => void
  postMessage: (value: { id: number; parts?: string[]; error?: string }) => void
}
workerGlobal.onmessage = ({ data }) => {
  if ('release' in data) { streams.delete(data.release); return }
  try {
    const labels = { ...data.labels, expandLinesTitle: (lines: number) => data.labels.expandLinesTemplate.replace('__AGENA_LINES__', String(lines)) }
    let parts: string[]
    if (data.streamKey !== undefined) {
      const stream = streams.get(data.streamKey) || new MarkdownStreamRenderer()
      streams.delete(data.streamKey)
      streams.set(data.streamKey, stream)
      parts = stream.render(data.content, labels)
      let retained = [...streams.values()].reduce((bytes, item) => bytes + item.retainedCharacters * 2, 0)
      while (streams.size > 128 || retained > 16 * 1024 * 1024) {
        const oldest = streams.entries().next().value!
        retained -= oldest[1].retainedCharacters * 2
        streams.delete(oldest[0])
      }
    } else { parts = [renderMarkdown(data.content, labels)] }
    workerGlobal.postMessage({
      id: data.id,
      parts,
    })
  } catch (error) {
    workerGlobal.postMessage({ id: data.id, error: String(error) })
  }
}
