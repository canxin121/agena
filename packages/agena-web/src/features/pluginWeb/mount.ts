import type { AgenaPluginUI, PluginPageHandle, PluginPageModule } from './sdk'

type MountLease = {
  controller: AbortController
  root: HTMLElement
  releaseStyles?: () => void
  handle?: PluginPageHandle
  retired: boolean
}

export type PluginMountOptions = {
  container: HTMLElement
  load: () => Promise<unknown>
  styles: (signal: AbortSignal) => Promise<() => void>
  sdk: (signal: AbortSignal) => AgenaPluginUI
}

/** Each load owns a detached-on-retirement root. An import or mount that settles
 * after navigation cannot attach itself to the next page's DOM or callbacks.
 */
export class PluginPageMount {
  private current: MountLease | undefined

  private readonly cleanupError: (error: unknown) => void

  constructor(cleanupError: (error: unknown) => void) {
    this.cleanupError = cleanupError
  }

  dispose(): void {
    const lease = this.current
    this.current = undefined
    if (lease) this.retire(lease)
  }

  private disposeHandle(handle: PluginPageHandle): void {
    try {
      Promise.resolve(handle.dispose()).catch(this.cleanupError)
    } catch (error) {
      this.cleanupError(error)
    }
  }

  private retire(lease: MountLease): void {
    if (lease.retired) return
    lease.retired = true
    lease.controller.abort()
    lease.root.remove()
    lease.releaseStyles?.()
    if (lease.handle) this.disposeHandle(lease.handle)
  }

  async mount(options: PluginMountOptions): Promise<boolean> {
    this.dispose()
    const root = options.container.ownerDocument.createElement('div')
    root.className = 'agena-plugin-page-root'
    root.style.minHeight = '100%'
    const lease: MountLease = { controller: new AbortController(), root, retired: false }
    this.current = lease
    options.container.append(root)
    const current = () => this.current === lease && !lease.retired
    try {
      // Register both promises immediately so an early style failure is handled
      // while the browser is still fetching the ESM graph.
      const [module, release] = await Promise.all([
        options.load(),
        options.styles(lease.controller.signal).then((release) => {
          lease.releaseStyles = release
          if (!current()) release()
          return release
        }),
      ])
      lease.releaseStyles = release
      if (!current()) return false
      if (!module || typeof module !== 'object' || !('mount' in module) || typeof module.mount !== 'function')
        throw new Error('Plugin entry must export mount(container, sdk)')
      const handle = await (module as PluginPageModule).mount(root, options.sdk(lease.controller.signal))
      if (!handle || typeof handle.dispose !== 'function') throw new Error('Plugin mount must return a dispose handle')
      if (!current()) {
        this.disposeHandle(handle)
        return false
      }
      lease.handle = handle
      return true
    } catch (error) {
      const wasCurrent = current()
      this.retire(lease)
      if (this.current === lease) this.current = undefined
      if (wasCurrent) throw error
      return false
    }
  }
}
