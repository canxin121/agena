export type MermaidResult = string | { svg?: string; bindFunctions?: (element: Element) => void }
type MermaidApi = {
  initialize: (config: { startOnLoad: boolean; theme: 'dark' | 'neutral'; securityLevel: 'strict' }) => void
  render: (id: string, source: string) => Promise<MermaidResult>
}

// Mermaid has global configuration and temporary DOM nodes. Serialize renders
// across Markdown instances, and skip jobs whose source has been replaced.
export function createMermaidRenderer(load: () => Promise<MermaidApi>) {
  let api: Promise<MermaidApi> | undefined
  let theme: string | undefined
  let sequence = 0
  let tail: Promise<unknown> = Promise.resolve()
  return (
    source: string,
    requestedTheme: 'dark' | 'neutral',
    signal: AbortSignal,
  ): Promise<MermaidResult | undefined> => {
    const result = tail.then(async () => {
      if (signal.aborted) return
      api ??= load().catch((error) => {
        api = undefined
        throw error
      })
      const mermaid = await api
      if (signal.aborted) return
      if (theme !== requestedTheme) {
        mermaid.initialize({ startOnLoad: false, theme: requestedTheme, securityLevel: 'strict' })
        theme = requestedTheme
      }
      const output = await mermaid.render(`oc-mermaid-${++sequence}`, source)
      return signal.aborted ? undefined : output
    })
    // A malformed diagram must not block the next one or cause an unhandled rejection.
    tail = result.catch(() => {})
    return result
  }
}

export const renderMermaid = createMermaidRenderer(async () => (await import('mermaid')).default)
