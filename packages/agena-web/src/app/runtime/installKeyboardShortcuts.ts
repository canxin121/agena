import { router } from '@/router'
import { useChatStore } from '@/stores/chat'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { patchSessionIdInQuery } from '@/app/navigation/sessionQuery'
import { i18n } from '@/i18n'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { pickComposerFocusTarget } from '@/app/runtime/composerFocus'
import { resolveAppShortcut } from '@/app/runtime/keyboardShortcutActions'
import { keyboardOverrides } from '@/app/runtime/keyboardPreferences'
import { mainTabPath } from '@/app/navigation/mainTabs'
import { deferKeyboardFocus } from '@/lib/deferredKeyboardFocus'
import {
  cycleKeyboardRegion,
  focusKeyboardRegion,
  hasOpenKeyboardOverlay,
  isTextEditingTarget,
  isVisibleKeyboardTarget,
} from '@/lib/keyboardNavigation'
import { isImeComposing } from '@/lib/imeKeyboard'
import { nextTick } from 'vue'

export function installKeyboardShortcuts(): () => void {
  const ui = useUiStore()
  const chat = useChatStore()
  const settings = useSettingsStore()
  const workspaceNavigation = useWorkspaceNavigation()

  const isOnSettingsPage = () => router.currentRoute.value.path.startsWith('/settings')

  /**
   * Cmd+I belongs to the pane the user is working in.  Every workspace pane keeps
   * its own composer in the same document, and background tabs keep theirs mounted
   * but hidden, so a plain document query focuses the first composer - or nothing
   * at all when that one cannot take focus.
   */
  let cancelFocus: (() => void) | undefined
  let composerNavigations = 0
  let composerFocusRequest = 0
  const composerTarget = () =>
    pickComposerFocusTarget({
      windowId: ui.focusedWorkspaceWindowId,
      queryAll: (selector) => document.querySelectorAll<HTMLElement>(selector),
      // A background tab keeps its editor mounted; focusing a hidden element would
      // silently do nothing, so keep looking for a visible one.
      isVisible: isVisibleKeyboardTarget,
    })
  async function focusFocusedPaneComposer() {
    const request = ++composerFocusRequest
    cancelFocus?.()
    const target = composerTarget()
    if (target) {
      target.focus()
      return
    }
    const origin = document.activeElement
    composerNavigations++
    try {
      const existing = ui.workspaceWindows.find((window) => window.mainTab === 'chat')
      if (existing) await workspaceNavigation.navigateToWorkspaceWindow(existing.id)
      else {
        const sessionId = String(chat.selectedSessionId || '').trim()
        if (sessionId) {
          if (ui.isCompactLayout) await router.push({ path: '/chat', query: { sessionId } })
          else await workspaceNavigation.openWorkspaceLocation('chat', { query: { sessionId }, reuseExisting: true })
        } else {
          // A composer needs a real session. Preserve the workspace and enter the
          // session chooser, where both selecting and creating have native controls.
          if (ui.isCompactLayout) ui.setSessionSwitcherOpen(true)
          else ui.setSidebarOpen(true, { preserveWidth: true })
          await nextTick()
          const sidebar = document.querySelector<HTMLElement>('[data-keyboard-region][data-keyboard-sidebar]')
          if (sidebar) focusKeyboardRegion(sidebar)
          return
        }
      }
    } catch {
      // Navigation can be cancelled by another destination; keep its focus.
      return
    } finally {
      composerNavigations--
    }
    if (
      request !== composerFocusRequest ||
      (document.activeElement !== origin && document.activeElement !== document.body)
    )
      return
    cancelFocus = deferKeyboardFocus(composerTarget, origin)
  }

  let routeOrigin: HTMLElement | null = null
  let routeShouldRestore = false
  const removeBefore = router.beforeEach((to, from) => {
    cancelFocus?.()
    routeOrigin = to.path !== from.path && document.activeElement instanceof HTMLElement ? document.activeElement : null
    routeShouldRestore =
      composerNavigations === 0 &&
      Boolean(
        routeOrigin &&
        (routeOrigin === document.body || routeOrigin.closest('main, [data-workspace-pane-window]')) &&
        !routeOrigin.closest('nav, aside, [role="tablist"], [data-keyboard-sidebar]'),
      )
  })
  const removeAfter = router.afterEach((to, _from, failure) => {
    const origin = routeOrigin
    if (failure || !origin || !routeShouldRestore) return
    cancelFocus = deferKeyboardFocus(() => {
      if (origin !== document.body && isVisibleKeyboardTarget(origin)) return null
      const pane = Array.from(document.querySelectorAll<HTMLElement>('[data-workspace-pane-window]')).find(
        (element) =>
          isVisibleKeyboardTarget(element) &&
          element.dataset.keyboardRoute === to.path &&
          element.getAttribute('aria-busy') !== 'true',
      )
      return pane || document.querySelector<HTMLElement>('main[data-keyboard-region]')
    }, origin)
  })

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.defaultPrevented || isImeComposing(e)) return
    const terminalOwnsInput = e.target instanceof Element && Boolean(e.target.closest('.xterm'))
    const action = resolveAppShortcut(e, terminalOwnsInput, keyboardOverrides.value)
    if (hasOpenKeyboardOverlay()) {
      if (action) e.preventDefault()
      // Help may toggle itself, but a nested picker owns its own dismissal.
      if (action?.type === 'help' && ui.isHelpDialogOpen && !document.querySelector('[data-keyboard-overlay="true"]')) {
        e.preventDefault()
        ui.toggleHelpDialog()
      }
      return
    }

    if (action?.type === 'help') {
      e.preventDefault()
      ui.toggleHelpDialog()
      return
    }

    // Cmd/Ctrl+,: settings
    if (action?.type === 'settings') {
      e.preventDefault()
      ui.setSessionSwitcherOpen(false)
      if (isOnSettingsPage()) {
        if (ui.isCompactLayout) void router.push('/chat')
        else void workspaceNavigation.openMainTab('chat')
      } else {
        if (ui.isCompactLayout) void router.push('/settings/interface')
        else void workspaceNavigation.openMainTab('settings', { path: '/settings/interface' })
      }
      return
    }

    // Cmd/Ctrl+L: sessions list (desktop sidebar or mobile drilldown)
    if (action?.type === 'sidebar') {
      e.preventDefault()
      ui.toggleSidebar()
      void nextTick(() => {
        const sidebar = document.querySelector<HTMLElement>('[data-keyboard-region][data-keyboard-sidebar]')
        const open = ui.isCompactLayout ? ui.isSessionSwitcherOpen : ui.isSidebarOpen
        if (open && sidebar) focusKeyboardRegion(sidebar)
        else {
          const main = document.querySelector<HTMLElement>('main')
          if (main) focusKeyboardRegion(main)
        }
      })
      return
    }

    // Cmd+I remains an application shortcut on macOS. Plain `i` enters
    // INSERT mode and Ctrl+I is Vim jump-forward while the transcript owns
    // focus, matching the TUI keymap.
    if (action?.type === 'composer') {
      e.preventDefault()
      focusFocusedPaneComposer()
      return
    }

    // Cmd/Ctrl+N: create session
    if (action?.type === 'new-session') {
      e.preventDefault()
      ui.setSessionSwitcherOpen(false)
      void (async () => {
        const created = await chat.createSession().catch(() => null)
        const sid = (created?.id || chat.selectedSessionId || '').trim()

        if (sid) {
          const nextQuery = patchSessionIdInQuery(router.currentRoute.value.query || {}, sid)
          await workspaceNavigation.openWorkspaceLocation('chat', {
            query: nextQuery,
            title: String(i18n.global.t('nav.chat')),
            matchKeys: ['sessionId'],
          })
        } else {
          await router.push('/chat')
        }
      })()
      return
    }

    // Cmd/Ctrl+/: cycle theme (Light -> Dark -> System)
    if (action?.type === 'theme') {
      e.preventDefault()
      const useSystem = Boolean(settings.data?.useSystemTheme)
      const variant = settings.data?.themeVariant === 'dark' ? 'dark' : 'light'
      if (useSystem) {
        void settings.save({ useSystemTheme: false, themeVariant: 'light' }).catch(() => {})
      } else if (variant === 'light') {
        void settings.save({ useSystemTheme: false, themeVariant: 'dark' }).catch(() => {})
      } else {
        void settings.save({ useSystemTheme: true }).catch(() => {})
      }
      return
    }

    if (action?.type === 'main-tab') {
      e.preventDefault()
      if (ui.isCompactLayout) void router.push(mainTabPath(action.tab))
      else void workspaceNavigation.openMainTab(action.tab)
      return
    }

    // Esc closes Settings. On chat it belongs to the TUI-parity Vim surface:
    // INSERT -> NAVIGATE, cancel Visual selection, or collapse an interaction.
    // Ctrl+C owns run interruption.
    if (e.key === 'Escape' && ui.isCompactLayout && ui.isSessionSwitcherOpen) {
      e.preventDefault()
      ui.setSessionSwitcherOpen(false)
      void nextTick(() => {
        const region = document.querySelector<HTMLElement>('main[data-keyboard-region], main [data-keyboard-region]')
        if (region) focusKeyboardRegion(region)
        else focusFocusedPaneComposer()
      })
      return
    }
    if (e.key === 'Escape' && !isTextEditingTarget(e.target)) {
      if (isOnSettingsPage()) {
        e.preventDefault()
        if (ui.isCompactLayout) void router.push('/chat')
        else void workspaceNavigation.openMainTab('chat')
        return
      }
    }
  }

  // Capture this one shell-navigation key before editors/terminals consume it.
  const onRegionKeyDown = (event: KeyboardEvent) => {
    if (
      event.key !== 'F6' ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.defaultPrevented ||
      isImeComposing(event) ||
      hasOpenKeyboardOverlay()
    )
      return
    event.preventDefault()
    event.stopPropagation()
    cycleKeyboardRegion(event.shiftKey)
  }

  window.addEventListener('keydown', onRegionKeyDown, true)
  window.addEventListener('keydown', onKeyDown)

  return () => {
    composerFocusRequest++
    cancelFocus?.()
    removeBefore()
    removeAfter()
    window.removeEventListener('keydown', onKeyDown)
    window.removeEventListener('keydown', onRegionKeyDown, true)
  }
}
