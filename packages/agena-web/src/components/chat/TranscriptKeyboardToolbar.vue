<script setup lang="ts">
// Keyboard-only transcript navigation.
//
// This component exists to install the shortcut listeners the desktop Vim model
// cannot provide in its non-Vim mode. It renders nothing on purpose: message
// movement belongs to the floating navigation buttons already overlaid on the
// transcript, and copy/search/plan are reachable through those controls and the
// command palette. A visible button row here only duplicated them and, on a
// phone, pushed a strip of unusable keys into the transcript.
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { isImeComposing } from '@/lib/imeKeyboard'
import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

const props = defineProps<{
  root: HTMLElement | null
  hasOlder: boolean
  loadOlder: () => Promise<boolean>
  /** Copy one message by id for the focused message shortcut. */
  copyMessage: (id: string) => void
  search: () => void
  input: () => void
}>()

const activeId = ref('')
const loading = ref(false)
let disposed = false

const rows = () =>
  Array.from(props.root?.querySelectorAll<HTMLElement>('[data-transcript-node="message"]') || []).filter(
    isVisibleKeyboardTarget,
  )
const active = () => rows().find((row) => row.dataset.messageId === activeId.value) || rows().at(-1)

async function move(direction: number) {
  if (loading.value) return
  const focusOrigin = document.activeElement
  let items = rows()
  const origin = active()
  let index = origin ? items.indexOf(origin) : items.length
  if (direction < 0 && index <= 0 && props.hasOlder) {
    loading.value = true
    try {
      await props.loadOlder()
      await nextTick()
    } catch {
      return
    } finally {
      loading.value = false
    }
    if (
      disposed ||
      hasOpenKeyboardOverlay() ||
      (document.activeElement !== focusOrigin && document.activeElement !== document.body)
    )
      return
    items = rows()
    index = origin ? items.findIndex((row) => row.dataset.messageId === origin.dataset.messageId) : 0
  }
  const next =
    direction === -Infinity
      ? 0
      : direction === Infinity
        ? items.length - 1
        : Math.max(0, Math.min(items.length - 1, index + direction))
  const target = items[next]
  if (!target) return
  activeId.value = target.dataset.messageId || ''
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'center' })
}

function onFocus(event: FocusEvent) {
  const row =
    event.target instanceof Element ? event.target.closest<HTMLElement>('[data-transcript-node="message"]') : null
  if (row?.dataset.messageId) activeId.value = row.dataset.messageId
}

function onKey(event: KeyboardEvent) {
  if (event.defaultPrevented || isImeComposing(event) || hasOpenKeyboardOverlay()) return
  const altShift = event.altKey && event.shiftKey && !event.ctrlKey && !event.metaKey
  const message = event.target instanceof Element && event.target.matches('[data-transcript-node="message"]')
  const plain = !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey
  if ((altShift || (message && plain)) && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
    event.preventDefault()
    event.stopPropagation()
    void move(
      event.key === 'ArrowUp' ? -1 : event.key === 'ArrowDown' ? 1 : event.key === 'Home' ? -Infinity : Infinity,
    )
  } else if (altShift && (event.code === 'KeyF' || event.key.toLowerCase() === 'f')) {
    event.preventDefault()
    event.stopPropagation()
    props.search()
  } else if (
    message &&
    event.key.toLowerCase() === 'c' &&
    (event.ctrlKey || event.metaKey) &&
    !event.altKey &&
    !event.shiftKey &&
    window.getSelection()?.isCollapsed !== false
  ) {
    event.preventDefault()
    event.stopPropagation()
    const id = active()?.dataset.messageId
    if (id) props.copyMessage(id)
  } else if (message && plain && event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    props.input()
  }
}

let dispose: (() => void) | undefined
watch(
  () => props.root,
  (root) => {
    dispose?.()
    if (!root) return
    root.addEventListener('keydown', onKey)
    root.addEventListener('focusin', onFocus)
    dispose = () => {
      root.removeEventListener('keydown', onKey)
      root.removeEventListener('focusin', onFocus)
    }
  },
  { immediate: true, flush: 'post' },
)
onBeforeUnmount(() => {
  disposed = true
  dispose?.()
})
</script>

<template>
  <!-- Deliberately renders nothing: see the module comment above. -->
  <span hidden aria-hidden="true" />
</template>
