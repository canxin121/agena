import { nextTick, onBeforeUnmount, onMounted, shallowRef, watch, type Ref } from 'vue'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { copyTextToClipboard, readTextFromClipboard } from '@/lib/clipboard'
import { isImeComposing } from '@/lib/imeKeyboard'
import { resizeTerminal } from '../api/terminalApi'
import { handleTerminalKeyboardShortcut } from './terminalKeyboardShortcuts'

export const terminalBackground = '#101415'

type Options = {
  container: Readonly<Ref<HTMLElement | null>>
  visible: () => boolean
  sessionId: () => string | null
  fontSize: number
  autoFocus?: () => boolean
  onData: (data: string) => void
  onInterrupt?: () => void
  onReady?: (terminal: Terminal) => void
  onResizeError?: (error: unknown, sessionId: string) => void
}

export function useTerminalViewport(options: Options) {
  const term = shallowRef<Terminal | null>(null)
  let host: HTMLElement | null = null
  let fit: FitAddon | null = null
  let observer: ResizeObserver | undefined
  let timer: number | undefined
  let mounted = false
  let lastSize = ''
  let resizeVersion = 0
  let resizeInFlight = false
  let resizePending = false

  function resetResize() {
    resizeVersion++
    lastSize = ''
  }

  function releaseTerminal() {
    term.value?.dispose()
    term.value = null
    fit = null
    host = null
    resetResize()
  }

  function repaint(terminal: Terminal) {
    // The first paint in a resized pane can lag until the next character.
    terminal.refresh(0, Math.max(0, terminal.rows - 1))
  }

  function ensureTerminalMounted() {
    const element = options.container.value
    if (!mounted || !options.visible() || !element?.clientWidth || !element.clientHeight) return null
    if (term.value && host === element) return term.value
    if (term.value) releaseTerminal()

    const terminal = new Terminal({
      cursorBlink: true,
      scrollback: 5000,
      fontFamily: '"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
      fontSize: options.fontSize,
      theme: {
        background: terminalBackground,
        foreground: '#e9efe7',
        cursor: '#e9efe7',
        selectionBackground: 'rgba(111, 176, 255, 0.25)',
      },
    })
    const addon = new FitAddon()
    terminal.loadAddon(addon)
    terminal.open(element)
    terminal.onData(options.onData)
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown' || isImeComposing(event)) return true
      return !handleTerminalKeyboardShortcut(event, {
        terminal,
        copyTextToClipboard,
        readTextFromClipboard,
        sendInterrupt: options.onInterrupt ?? (() => options.onData('\x03')),
      })
    })

    fit = addon
    host = element
    term.value = terminal
    addon.fit()
    repaint(terminal)
    options.onReady?.(terminal)
    if (options.autoFocus?.()) terminal.focus()
    return terminal
  }

  async function sendCurrentSize() {
    resizePending = true
    if (resizeInFlight) return
    resizeInFlight = true
    try {
      while (resizePending && mounted && options.visible()) {
        resizePending = false
        const sessionId = options.sessionId()
        const terminal = term.value
        if (!sessionId || !terminal) continue
        const { cols, rows } = terminal
        const size = `${sessionId}:${cols}x${rows}`
        if (size === lastSize) continue

        const version = resizeVersion
        try {
          await resizeTerminal(sessionId, cols, rows)
          if (version === resizeVersion) lastSize = size
        } catch (error) {
          if (mounted && version === resizeVersion && options.sessionId() === sessionId) {
            options.onResizeError?.(error, sessionId)
          }
        }
      }
    } finally {
      resizeInFlight = false
    }
  }

  function scheduleResize() {
    window.clearTimeout(timer)
    if (!mounted || !options.visible()) return
    timer = window.setTimeout(() => {
      timer = undefined
      const terminal = ensureTerminalMounted()
      if (!terminal || !fit) return
      fit.fit()
      repaint(terminal)
      void sendCurrentSize()
    }, 80)
  }

  function syncHost() {
    if (!mounted) return
    const container = options.container.value
    if (host && host !== container) releaseTerminal()
    observer?.disconnect()
    if (container && options.visible()) {
      observer?.observe(container)
      ensureTerminalMounted()
    }
    scheduleResize()
  }

  watch([options.container, options.visible], syncHost, { flush: 'post' })
  onMounted(() => {
    mounted = true
    if (typeof ResizeObserver !== 'undefined') observer = new ResizeObserver(scheduleResize)
    window.addEventListener('resize', scheduleResize)
    void nextTick(syncHost)
  })
  onBeforeUnmount(() => {
    mounted = false
    window.clearTimeout(timer)
    observer?.disconnect()
    window.removeEventListener('resize', scheduleResize)
    releaseTerminal()
  })

  return { term, ensureTerminalMounted, scheduleResize, resetResize }
}
