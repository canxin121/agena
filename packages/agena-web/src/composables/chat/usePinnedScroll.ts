import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { captureTranscriptScrollAnchor, restoreTranscriptScrollAnchor } from './transcriptScrollAnchor'

// ChatPage scroll management:
// - "Pinned" bottom-following behavior while user is at bottom
// - Programmatic initial bottom landing on session switch
// - Optional progressive "load older" when user scrolls to top

export function shouldAutoLoadOlder(opts: {
  canLoadOlder: boolean
  suppressed: boolean
  scrollTop: number
  autoLoadUnlocked: boolean
  atBottom: boolean
}) {
  if (!opts.canLoadOlder) return false
  if (opts.suppressed) return false
  if (!opts.autoLoadUnlocked) return false
  if (opts.atBottom) return false
  return opts.scrollTop <= 240
}

export function isScrollableY(el: Pick<HTMLElement, 'scrollHeight' | 'clientHeight'> | null | undefined): boolean {
  if (!el) return false
  return el.scrollHeight - el.clientHeight > 1
}

export function usePinnedScroll(opts: {
  bottomThresholdPx?: number
  // Called on every scroll update (used for nav index updates).
  onScroll?: () => void
  canLoadOlder?: () => boolean
  // Should prepend older messages; returns true if any were loaded.
  loadOlder?: () => Promise<boolean>
  isVisible?: () => boolean
  sessionId?: () => string | null
}) {
  const scrollEl = ref<HTMLDivElement | null>(null)
  const contentEl = ref<HTMLDivElement | null>(null)
  const bottomEl = ref<HTMLDivElement | null>(null)

  const bottomThreshold = typeof opts.bottomThresholdPx === 'number' ? opts.bottomThresholdPx : 140

  const isAtBottom = ref(true)

  function remainingToBottomPx(): number {
    const el = scrollEl.value
    if (!el) return Number.POSITIVE_INFINITY
    return el.scrollHeight - el.scrollTop - el.clientHeight
  }

  function isNearBottomNow(extraThresholdPx = 0): boolean {
    const threshold = Math.max(0, bottomThreshold + Math.max(0, Math.floor(extraThresholdPx)))
    return remainingToBottomPx() < threshold
  }

  // Used by the page to temporarily hide the list while performing a stable bottom landing.
  const pendingInitialScrollSessionId = ref<string | null>(null)
  let initialScrollNonce = 0

  // Throttle load-older and suppress auto-load during programmatic navigation.
  let lastLoadOlderAt = 0
  let historyLoadInFlight = false
  let historyLoadGeneration = 0
  const suppressAutoLoadOlderUntil = ref(0)
  const autoLoadOlderUnlocked = ref(false)

  // Batch auto-scroll work to at most once per frame while streaming.
  let scrollRaf: number | null = null

  // Keep the scroll pinned to bottom while the user is "following".
  // This prevents entry jitter from immediate reflows (markdown, fonts, keyboard insets).
  let followResizeObserver: ResizeObserver | null = null
  let followResizeUnlockRaf: number | null = null
  let followResizeLocked = false
  const isVisible = () => opts.isVisible?.() ?? true
  let visibilityGeneration = 0
  let pausedPosition: { sessionId: string | null; top: number; key: string; offset: number; pinned: boolean } | null =
    null

  function requestInitialScroll(sessionId: string | null | undefined) {
    historyLoadGeneration += 1
    historyLoadInFlight = false
    lastLoadOlderAt = 0
    const sid = typeof sessionId === 'string' ? sessionId.trim() : ''
    pendingInitialScrollSessionId.value = sid || null
    if (!sid) return
    // Session switches / initial entry should feel chat-like: land at bottom.
    // Also avoid triggering auto-load-older from a programmatic scroll.
    isAtBottom.value = true
    suppressAutoLoadOlderUntil.value = Date.now() + 1400
    autoLoadOlderUnlocked.value = false
  }

  function scrollToBottom(behavior: ScrollBehavior = 'auto') {
    if (!isVisible()) return
    const scroller = scrollEl.value
    if (!scroller) return
    // Prefer an explicit anchor to avoid scrollHeight races.
    const anchor = bottomEl.value
    if (anchor) {
      anchor.scrollIntoView({ behavior, block: 'end' })
      return
    }
    scroller.scrollTo({ top: scroller.scrollHeight, behavior })
  }

  function scheduleScrollToBottom() {
    if (!isVisible() || scrollRaf) return
    scrollRaf = window.requestAnimationFrame(async () => {
      scrollRaf = null
      if (!isVisible() || !isAtBottom.value) return
      await nextTick()
      scrollToBottom('auto')
    })
  }

  function pinToBottomNow() {
    if (!isVisible()) return
    if (!isAtBottom.value) return
    if (followResizeLocked) return
    followResizeLocked = true
    scrollToBottom('auto')
    followResizeUnlockRaf = window.requestAnimationFrame(() => {
      followResizeLocked = false
      followResizeUnlockRaf = null
    })
  }

  function syncFollowResizeObserver() {
    followResizeObserver?.disconnect()
    followResizeObserver = null

    const scroller = scrollEl.value
    const content = contentEl.value
    if (!isVisible() || !scroller || !content) return

    followResizeObserver = new ResizeObserver(() => {
      pinToBottomNow()
    })
    followResizeObserver.observe(scroller)
    followResizeObserver.observe(content)
  }

  watch(() => [scrollEl.value, contentEl.value] as const, syncFollowResizeObserver, { flush: 'post', immediate: true })

  watch(
    isVisible,
    async (visible, previous) => {
      const generation = ++visibilityGeneration
      const scroller = scrollEl.value
      if (!visible) {
        // Capture before Vue removes the hidden pane's message subtree. Keep
        // only its key/coordinates so a paused tab retains no detached DOM.
        if (previous && scroller) {
          const anchor = captureTranscriptScrollAnchor(scroller)
          pausedPosition = {
            sessionId: opts.sessionId?.() ?? null,
            top: scroller.scrollTop,
            key: anchor.element?.dataset.transcriptKey ?? '',
            offset: anchor.offset,
            pinned: isAtBottom.value,
          }
        }
        initialScrollNonce++
        historyLoadGeneration++
        historyLoadInFlight = false
        if (scrollRaf) window.cancelAnimationFrame(scrollRaf)
        scrollRaf = null
        if (followResizeUnlockRaf) window.cancelAnimationFrame(followResizeUnlockRaf)
        followResizeUnlockRaf = null
        followResizeLocked = false
        syncFollowResizeObserver()
        return
      }
      await nextTick()
      if (generation !== visibilityGeneration || !isVisible() || scrollEl.value !== scroller) return
      const saved = pausedPosition
      pausedPosition = null
      if (
        scroller &&
        saved &&
        saved.sessionId === (opts.sessionId?.() ?? null) &&
        !pendingInitialScrollSessionId.value
      ) {
        isAtBottom.value = saved.pinned
        suppressAutoLoadOlderUntil.value = Date.now() + 1400
        if (saved.pinned) scrollToBottom('auto')
        else {
          scroller.scrollTop = saved.top
          const anchor = Array.from(scroller.querySelectorAll<HTMLElement>('[data-transcript-key]')).find(
            (element) => element.dataset.transcriptKey === saved.key,
          )
          if (anchor)
            scroller.scrollTop +=
              anchor.getBoundingClientRect().top - scroller.getBoundingClientRect().top - saved.offset
        }
      }
      syncFollowResizeObserver()
      if (pendingInitialScrollSessionId.value) void scrollToBottomOnceAfterLoad(pendingInitialScrollSessionId.value)
    },
    { flush: 'sync' },
  )

  async function loadOlderAndPreserveViewport(): Promise<boolean> {
    if (!isVisible()) return false
    const el = scrollEl.value
    if (!el) return false
    if (!opts.loadOlder) return false
    if (!opts.canLoadOlder?.()) return false

    const now = Date.now()
    if (historyLoadInFlight || now - lastLoadOlderAt < 250) return false
    lastLoadOlderAt = now
    suppressAutoLoadOlderUntil.value = now + 300
    const generation = historyLoadGeneration
    const anchor = captureTranscriptScrollAnchor(el)
    historyLoadInFlight = true
    isAtBottom.value = false
    try {
      const ok = await opts.loadOlder()
      if (!ok || !isVisible() || generation !== historyLoadGeneration || scrollEl.value !== el) return false
      await nextTick()
      if (!isVisible() || generation !== historyLoadGeneration || scrollEl.value !== el) return false
      restoreTranscriptScrollAnchor(el, anchor)
      return true
    } finally {
      if (generation === historyLoadGeneration) historyLoadInFlight = false
    }
  }

  async function maybeLoadOlder() {
    const el = scrollEl.value
    if (!el) return
    const canLoadOlder = !!opts.canLoadOlder?.()
    const shouldAutoLoad = shouldAutoLoadOlder({
      canLoadOlder,
      suppressed: Date.now() < suppressAutoLoadOlderUntil.value,
      scrollTop: el.scrollTop,
      autoLoadUnlocked: autoLoadOlderUnlocked.value,
      atBottom: isAtBottom.value,
    })
    if (!shouldAutoLoad) return

    void loadOlderAndPreserveViewport()
  }

  // A wheel-up gesture is explicit user intent even when the recent page is
  // shorter than the viewport and the browser cannot emit a meaningful
  // scrollTop change at the boundary.
  function handleWheel(event: WheelEvent) {
    if (!isVisible()) return
    if (event.deltaY >= 0) return
    autoLoadOlderUnlocked.value = true
    const el = scrollEl.value
    if (!el || !opts.canLoadOlder?.()) return
    if (el.scrollTop > 240 && isScrollableY(el)) return
    void loadOlderAndPreserveViewport()
  }

  function handleScroll() {
    if (!isVisible() || !scrollEl.value) return
    isAtBottom.value = isNearBottomNow()
    if (!autoLoadOlderUnlocked.value && !isAtBottom.value) {
      autoLoadOlderUnlocked.value = true
    }
    opts.onScroll?.()
    void maybeLoadOlder()
  }

  async function scrollToBottomOnceAfterLoad(sessionId: string) {
    if (!isVisible()) return
    const sid = (sessionId || '').trim()
    if (!sid) return
    if (pendingInitialScrollSessionId.value !== sid) return
    if (!scrollEl.value) return

    const nonce = ++initialScrollNonce
    // We hide the list while `pendingInitialScrollSessionId` is set; do a stable
    // bottom landing before revealing to avoid entry jitter.
    await nextTick()
    if (!isVisible() || nonce !== initialScrollNonce) return
    scrollToBottom('auto')

    // One extra frame absorbs immediate reflows (font swap, markdown highlight).
    await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()))
    if (!isVisible() || nonce !== initialScrollNonce) return
    scrollToBottom('auto')

    if (pendingInitialScrollSessionId.value !== sid) return

    isAtBottom.value = true
    pendingInitialScrollSessionId.value = null
  }

  onBeforeUnmount(() => {
    visibilityGeneration++
    initialScrollNonce++
    pausedPosition = null
    if (scrollRaf) {
      window.cancelAnimationFrame(scrollRaf)
      scrollRaf = null
    }
    if (followResizeObserver) {
      followResizeObserver.disconnect()
      followResizeObserver = null
    }
    if (followResizeUnlockRaf) {
      window.cancelAnimationFrame(followResizeUnlockRaf)
      followResizeUnlockRaf = null
    }
  })

  return {
    scrollEl,
    contentEl,
    bottomEl,
    isAtBottom,
    pendingInitialScrollSessionId,
    suppressAutoLoadOlderUntil,
    requestInitialScroll,
    scrollToBottom,
    scheduleScrollToBottom,
    scrollToBottomOnceAfterLoad,
    loadOlderAndPreserveViewport,
    handleScroll,
    handleWheel,
  }
}
