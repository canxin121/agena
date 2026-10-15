<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { isImeComposing } from '@/lib/imeKeyboard'
import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

const props = defineProps<{
  root: HTMLElement | null
  hasOlder: boolean
  loadOlder: () => Promise<boolean>
  planAvailable: boolean
}>()
const emit = defineEmits<{
  (event: 'copy', id: string): void
  (event: 'search'): void
  (event: 'input'): void
  (event: 'plan'): void
}>()
const { t } = useI18n()
const activeId = ref('')
const count = ref(0)
const status = ref('')
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
  status.value = `${next + 1} / ${items.length}`
}

function copy() {
  const id = active()?.dataset.messageId
  if (id) emit('copy', id)
}
function selectText() {
  const row = active()
  if (!row) return
  row.focus({ preventScroll: true })
  const range = document.createRange()
  range.selectNodeContents(row.querySelector('[data-transcript-copy-root]') || row)
  const selection = window.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)
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
    emit('search')
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
    copy()
  } else if (message && plain && event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('input')
  }
}

let dispose: (() => void) | undefined
watch(
  () => props.root,
  (root) => {
    dispose?.()
    if (!root) return
    const sync = () => {
      count.value = rows().length
    }
    const observer = new MutationObserver(sync)
    observer.observe(root, { childList: true, subtree: true })
    root.addEventListener('keydown', onKey)
    root.addEventListener('focusin', onFocus)
    sync()
    dispose = () => {
      observer.disconnect()
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
  <div
    role="group"
    :aria-label="String(t('keyboard.transcriptNavigation'))"
    class="chat-message-column flex flex-wrap items-center gap-1 border-t border-border/40 py-1 text-xs"
  >
    <button
      type="button"
      :disabled="loading || !count"
      class="rounded px-2 py-1 hover:bg-secondary disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring"
      @click="move(-1)"
    >
      {{ t('keyboard.previousMessage') }}
    </button>
    <button
      type="button"
      :disabled="loading || !count"
      class="rounded px-2 py-1 hover:bg-secondary disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring"
      @click="move(1)"
    >
      {{ t('keyboard.nextMessage') }}
    </button>
    <button
      type="button"
      :disabled="!count"
      class="rounded px-2 py-1 hover:bg-secondary disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring"
      @click="copy"
    >
      {{ t('keyboard.copyMessage') }}
    </button>
    <button
      type="button"
      :disabled="!count"
      class="rounded px-2 py-1 hover:bg-secondary disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring"
      @click="selectText"
    >
      {{ t('keyboard.selectMessageText') }}
    </button>
    <button
      type="button"
      class="rounded px-2 py-1 hover:bg-secondary focus-visible:ring-2 focus-visible:ring-ring"
      @click="emit('search')"
    >
      {{ t('common.search') }}
    </button>
    <button
      type="button"
      :disabled="!planAvailable"
      class="rounded px-2 py-1 hover:bg-secondary disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-ring"
      @click="emit('plan')"
    >
      {{ t('keyboard.openPlan') }}
    </button>
    <button
      type="button"
      class="rounded px-2 py-1 hover:bg-secondary focus-visible:ring-2 focus-visible:ring-ring"
      @click="emit('input')"
    >
      {{ t('keyboard.focusInput') }}
    </button>
    <span role="status" aria-live="polite" class="font-mono text-muted-foreground">{{ status }}</span>
    <span class="w-full text-[10px] text-muted-foreground">{{ t('keyboard.transcriptHint') }}</span>
  </div>
</template>
