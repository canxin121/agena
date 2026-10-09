import { i18n } from '@/i18n'
import {
  ContentBuffer,
  contentChunkBytes,
  streamContent,
  type ContentChunk,
  type ContentCursor,
  type ContentRef,
} from './content'

export type ContentFrame = {
  buffer: ContentBuffer
  appended: ContentChunk[]
  reset: boolean
  text: string
  error: string
}

type Observer = (frame: ContentFrame) => void
type Subscription = { observer: Observer; cursor: ContentCursor | null }
type Interest = {
  sessionId: string
  resource: ContentRef
  buffer: ContentBuffer
  observers: Set<Subscription>
  controller?: AbortController
  retry?: ReturnType<typeof setTimeout>
  frame: number
  pending: ContentChunk[]
  pendingBytes: number
  reset: boolean
  error: string
  failures: number
}

export function createContentSubscriptions(
  transport: typeof streamContent = streamContent,
  frames = {
    request: (callback: FrameRequestCallback) => requestAnimationFrame(callback),
    cancel: (id: number) => cancelAnimationFrame(id),
  },
): (sessionId: string, resource: ContentRef, observer: Observer) => () => void {
  const interests = new Map<string, Interest>()
  const MAX_IDLE_RESOURCES = 16

  function isComplete(interest: Interest): boolean {
    return Boolean(interest.buffer.resource && interest.buffer.resource.state !== 'active')
  }

  function deliver(subscription: Subscription, frame: ContentFrame) {
    // A new widget may mount after the reducer accepted a page but before its
    // scheduled paint. Its bootstrap already covers those queued records.
    const appended = frame.reset
      ? frame.buffer.replayChunks()
      : frame.appended.filter(
          (chunk) =>
            !subscription.cursor ||
            chunk.cursor.epoch !== subscription.cursor.epoch ||
            chunk.cursor.sequence > subscription.cursor.sequence,
        )
    subscription.cursor = frame.buffer.cursor ? { ...frame.buffer.cursor } : null
    subscription.observer({ ...frame, appended })
  }

  function schedule(interest: Interest) {
    if (interest.frame || !interest.observers.size) return
    interest.frame = frames.request(() => {
      interest.frame = 0
      const frame: ContentFrame = {
        buffer: interest.buffer,
        appended: interest.pending,
        reset: interest.reset,
        text: interest.resource.kind === 'text' ? interest.buffer.text() : '',
        error: interest.error,
      }
      interest.pending = []
      interest.pendingBytes = 0
      interest.reset = false
      for (const subscription of interest.observers) deliver(subscription, frame)
    })
  }

  async function connect(interest: Interest) {
    if (!interest.observers.size || interest.controller || isComplete(interest)) return
    const controller = new AbortController()
    interest.controller = controller
    try {
      await transport(
        interest.sessionId,
        interest.resource.resource_id,
        interest.buffer.cursor,
        controller.signal,
        (page) => {
          const update = interest.buffer.apply(page)
          if (update.missing) throw new Error(i18n.global.t('errors.content.cursorRequiresRecovery'))
          interest.failures = 0
          interest.error = page.resource.capture_error || ''
          interest.reset ||= update.reset
          interest.pending.push(...update.appended)
          interest.pendingBytes += update.appended.reduce((bytes, chunk) => bytes + contentChunkBytes(chunk), 0)
          // A background browser tab must not collect unbounded frame deltas.
          if (interest.pending.length > 4096 || interest.pendingBytes > 1024 * 1024) {
            interest.pending = interest.buffer.replayChunks()
            interest.pendingBytes = interest.pending.reduce((bytes, chunk) => bytes + contentChunkBytes(chunk), 0)
            interest.reset = true
          }
          schedule(interest)
        },
      )
      if (!controller.signal.aborted && interest.buffer.resource?.state === 'active')
        throw new Error(i18n.global.t('errors.content.connectionClosedBeforeCursor'))
    } catch (error) {
      if (!controller.signal.aborted) {
        interest.error = error instanceof Error ? error.message : String(error)
        interest.failures = Math.min(interest.failures + 1, 5)
        schedule(interest)
      }
    } finally {
      if (interest.controller === controller) interest.controller = undefined
      if (interest.observers.size && controller.signal.aborted) {
        void connect(interest)
        return
      }
      if (
        interest.observers.size &&
        !controller.signal.aborted &&
        !isComplete(interest) &&
        interest.buffer.resource?.state !== 'interrupted'
      ) {
        interest.retry = setTimeout(() => void connect(interest), Math.min(250 * 2 ** interest.failures, 8000))
      }
    }
  }

  /** Widgets share one connection and one bounded reducer per authorized resource. */
  function observeContent(sessionId: string, resource: ContentRef, observer: Observer): () => void {
    const key = `${sessionId}/${resource.resource_id}`
    let interest = interests.get(key)
    if (!interest) {
      interest = {
        sessionId,
        resource,
        buffer: new ContentBuffer(512 * 1024),
        observers: new Set(),
        frame: 0,
        pending: [],
        pendingBytes: 0,
        reset: false,
        error: '',
        failures: 0,
      }
      interests.set(key, interest)
    }
    const current = interest
    const subscription: Subscription = { observer, cursor: null }
    current.observers.add(subscription)
    clearTimeout(current.retry)
    current.retry = undefined
    deliver(subscription, {
      buffer: current.buffer,
      appended: current.buffer.replayChunks(),
      reset: true,
      text: resource.kind === 'text' ? current.buffer.text() : '',
      error: current.error,
    })
    void connect(current)
    return () => {
      current.observers.delete(subscription)
      if (current.observers.size) return
      current.controller?.abort()
      clearTimeout(current.retry)
      frames.cancel(current.frame)
      current.frame = 0
      current.pending = []
      current.pendingBytes = 0
      const idle = [...interests].filter(([, value]) => !value.observers.size)
      for (const [victim] of idle.slice(0, Math.max(0, idle.length - MAX_IDLE_RESOURCES))) interests.delete(victim)
    }
  }

  return observeContent
}

export const observeContent = createContentSubscriptions()
