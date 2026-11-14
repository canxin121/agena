type StyleEntry = {
  link: HTMLLinkElement
  ready: Promise<void>
  reject: (error: Error) => void
  users: number
  cleanup: () => void
}

/** One stylesheet can be used by multiple mounted pages and workspace panes. */
export class PluginStyleRegistry {
  private readonly entries = new Map<string, StyleEntry>()

  private readonly document: Document

  constructor(document: Document) {
    this.document = document
  }

  async acquire(urls: string[], signal: AbortSignal): Promise<() => void> {
    signal.throwIfAborted()
    const owned: Array<[string, StyleEntry]> = []
    let released = false
    const release = () => {
      if (released) return
      released = true
      signal.removeEventListener('abort', release)
      for (const [url, entry] of owned) {
        if (--entry.users !== 0) continue
        if (this.entries.get(url) === entry) this.entries.delete(url)
        entry.cleanup()
        entry.link.remove()
        entry.reject(new DOMException('Stylesheet released', 'AbortError'))
      }
    }
    try {
      for (const url of new Set(urls)) {
        let entry = this.entries.get(url)
        if (!entry) {
          const link = this.document.createElement('link')
          link.rel = 'stylesheet'
          link.href = url
          link.dataset.agenaPluginStyle = url
          let resolve!: () => void
          let reject!: (error: Error) => void
          const ready = new Promise<void>((ok, fail) => {
            resolve = ok
            reject = fail
          })
          const onLoad = () => {
            cleanup()
            resolve()
          }
          const onError = () => {
            cleanup()
            reject(new Error('Plugin stylesheet failed to load'))
          }
          const timer = setTimeout(onError, 20000)
          const cleanup = () => {
            clearTimeout(timer)
            link.removeEventListener('load', onLoad)
            link.removeEventListener('error', onError)
          }
          entry = { link, ready, reject, cleanup, users: 0 }
          this.entries.set(url, entry)
          link.addEventListener('load', onLoad, { once: true })
          link.addEventListener('error', onError, { once: true })
          this.document.head.append(link)
        }
        entry.users++
        owned.push([url, entry])
      }
      signal.addEventListener('abort', release, { once: true })
      if (signal.aborted) release()
      await Promise.all(owned.map(([, entry]) => entry.ready))
      signal.throwIfAborted()
      return release
    } catch (error) {
      release()
      throw error
    }
  }
}
