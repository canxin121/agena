import { nextTick, watch, type Ref } from 'vue'
import { hasOpenKeyboardOverlay, isVisibleKeyboardTarget } from '@/lib/keyboardNavigation'

/** Suggestions share focus with their own composer and dismiss when Tab leaves. */
export function useComposerPaletteFocus(
  root: Ref<HTMLElement | null>,
  search: Ref<HTMLInputElement | null>,
  isOpen: () => boolean,
  autoFocus: () => boolean,
  dismiss: () => void,
) {
  let source: HTMLElement | null = null
  function focusSearch() {
    void nextTick(() => {
      if (isOpen()) search.value?.focus()
    })
  }
  function onFocusIn(event: FocusEvent) {
    const next = event.target
    if (!(next instanceof HTMLElement) || root.value?.contains(next)) return
    const editor = root.value?.closest('.composer-shell')?.querySelector('[data-chat-input]')
    if (next === editor || editor?.contains(next)) return
    dismiss()
  }
  watch(
    isOpen,
    (open, _previous, onCleanup) => {
      if (open) {
        source = document.activeElement instanceof HTMLElement ? document.activeElement : null
        document.addEventListener('focusin', onFocusIn)
        onCleanup(() => document.removeEventListener('focusin', onFocusIn))
      } else if (root.value?.contains(document.activeElement)) {
        void nextTick(() => {
          const active = document.activeElement
          if (hasOpenKeyboardOverlay() || !source || !isVisibleKeyboardTarget(source)) return
          if (active instanceof HTMLElement && active !== document.body && isVisibleKeyboardTarget(active)) return
          source.focus({ preventScroll: true })
        })
      }
    },
    { immediate: true },
  )
  watch(
    () => [isOpen(), autoFocus()] as const,
    ([open, focus]) => {
      if (open && focus) focusSearch()
    },
    { immediate: true },
  )
  return { focusSearch }
}
