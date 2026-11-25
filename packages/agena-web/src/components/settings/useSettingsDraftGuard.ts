import { onBeforeUnmount, onMounted } from 'vue'
import { onBeforeRouteLeave, onBeforeRouteUpdate, type RouteLocationNormalized } from 'vue-router'
import { readWindowIdFromQuery } from '@/app/windowScope'
import { useWorkspacePaneContext } from '@/app/workspace/workspacePaneContext'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction, type AppConfirmOptions } from '@/lib/appConfirm'

type Options = {
  dirty: () => boolean
  busy: () => boolean
  enabled?: () => boolean
  dialog?: () => Partial<AppConfirmOptions>
  sameTarget?: (to: RouteLocationNormalized, from: RouteLocationNormalized) => boolean
  afterRouteUpdate?: () => void
  unload?: false | (() => boolean)
}

export function useSettingsDraftGuard(options: Options) {
  const pane = useWorkspacePaneContext()

  function ownsNavigation(to: RouteLocationNormalized): boolean {
    if (options.enabled && !options.enabled()) return false
    if (!pane) return true
    const windowId = readWindowIdFromQuery(to.query)
    return windowId ? windowId === pane.windowId.value : pane.isFocused.value
  }

  async function canDiscard(): Promise<boolean> {
    if (options.busy()) return false
    if (!options.dirty()) return true
    return confirmAction({
      title: st('Discard unsaved changes?'),
      confirmText: st('Discard changes'),
      cancelText: st('Keep editing'),
      ...options.dialog?.(),
    })
  }

  onBeforeRouteLeave((to) => !ownsNavigation(to) || canDiscard())
  onBeforeRouteUpdate(async (to, from) => {
    if (!ownsNavigation(to) || options.sameTarget?.(to, from)) return true
    if (!(await canDiscard())) return false
    options.afterRouteUpdate?.()
    return true
  })

  const unload = options.unload
  if (unload !== false) {
    function beforeUnload(event: BeforeUnloadEvent) {
      if (options.enabled && !options.enabled()) return
      const dirty = unload ? unload() : options.dirty() || options.busy()
      if (!dirty) return
      event.preventDefault()
      event.returnValue = ''
    }
    onMounted(() => window.addEventListener('beforeunload', beforeUnload))
    onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))
  }

  return { canDiscard }
}
