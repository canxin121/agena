import { computed, getCurrentScope, onScopeDispose, ref, type ComputedRef, type Ref } from 'vue'
import { useWorkspacePaneContext } from '../app/workspace/workspacePaneContext'
import { isDocumentVisible } from '../lib/backgroundReads'

const documents = new WeakMap<Document, { visible: Ref<boolean>; users: number; update: () => void }>()

/** Share the document listener while keeping each split pane's visibility independent. */
export function usePaneVisibility(): ComputedRef<boolean> {
  const pane = useWorkspacePaneContext()
  if (typeof document === 'undefined' || !getCurrentScope())
    return computed(() => isDocumentVisible() && (!pane || pane.isVisible.value))

  const target = document
  let entry = documents.get(target)
  if (!entry) {
    const visible = ref(isDocumentVisible())
    entry = {
      visible,
      users: 0,
      update: () => {
        visible.value = !target.hidden && target.visibilityState !== 'hidden'
      },
    }
    documents.set(target, entry)
    target.addEventListener('visibilitychange', entry.update)
  }
  const observed = entry
  observed.users++
  onScopeDispose(() => {
    if (--observed.users) return
    target.removeEventListener('visibilitychange', observed.update)
    documents.delete(target)
  })
  return computed(() => observed.visible.value && (!pane || pane.isVisible.value))
}
