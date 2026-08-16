import { router } from '@/router'
import { useChatStore } from '@/stores/chat'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { patchSessionIdInQuery } from '@/app/navigation/sessionQuery'
import { isEmbeddedWorkspacePaneContext, withEmbeddedWorkspaceScopeQuery } from '@/app/windowScope'
import { i18n } from '@/i18n'

function hasModifier(e: KeyboardEvent): boolean {
  // Treat Meta on macOS, Ctrl elsewhere.
  return e.metaKey || e.ctrlKey
}

function keyLower(e: KeyboardEvent): string {
  return (e.key || '').toLowerCase()
}

export function installKeyboardShortcuts(): () => void {
  const ui = useUiStore()
  const chat = useChatStore()
  const settings = useSettingsStore()

  const isOnSettingsPage = () => router.currentRoute.value.path.startsWith('/settings')

  const onKeyDown = (e: KeyboardEvent) => {
    // TUI parity: Ctrl+H is contextual help from every main-surface mode.
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && keyLower(e) === 'h') {
      e.preventDefault()
      ui.toggleHelpDialog()
      return
    }

    // Cmd/Ctrl+.: help dialog
    if (hasModifier(e) && !e.shiftKey && keyLower(e) === '.') {
      e.preventDefault()
      ui.toggleHelpDialog()
      return
    }

    // Cmd/Ctrl+,: settings
    if (hasModifier(e) && !e.shiftKey && e.key === ',') {
      e.preventDefault()
      ui.setSessionSwitcherOpen(false)
      if (isOnSettingsPage()) {
        void router.push('/chat')
      } else {
        void router.push('/settings/general')
      }
      return
    }

    // Cmd/Ctrl+L: sessions list (desktop sidebar or mobile drilldown)
    if (hasModifier(e) && !e.shiftKey && keyLower(e) === 'l') {
      e.preventDefault()
      ui.toggleSidebar()
      return
    }

    // Cmd+I remains an application shortcut on macOS. Plain `i` enters
    // INSERT mode and Ctrl+I is Vim jump-forward while the transcript owns
    // focus, matching the TUI keymap.
    if (e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && keyLower(e) === 'i') {
      e.preventDefault()
      const textarea = document.querySelector<HTMLTextAreaElement>('textarea[data-chat-input="true"]')
      textarea?.focus()
      return
    }

    // Cmd/Ctrl+N: create session
    if (hasModifier(e) && keyLower(e) === 'n') {
      e.preventDefault()
      ui.setSessionSwitcherOpen(false)
      void (async () => {
        const created = await chat.createSession().catch(() => null)
        const sid = (created?.id || chat.selectedSessionId || '').trim()

        if (sid) {
          const currentQuery = router.currentRoute.value.query || {}
          const isEmbeddedWorkspacePane = isEmbeddedWorkspacePaneContext(currentQuery)
          const nextQuery = patchSessionIdInQuery(currentQuery, sid)
          ui.createWorkspaceWindow('chat', {
            activate: true,
            query: nextQuery,
            title: String(i18n.global.t('nav.chat')),
          })
          if (isEmbeddedWorkspacePane) {
            await router.push({ path: '/chat', query: withEmbeddedWorkspaceScopeQuery(nextQuery, currentQuery) })
          } else {
            await router.push('/chat')
          }
        } else {
          await router.push('/chat')
        }
      })()
      return
    }

    // Cmd/Ctrl+/: cycle theme (Light -> Dark -> System)
    if (hasModifier(e) && !e.shiftKey && keyLower(e) === '/') {
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

    // Cmd/Ctrl + number: switch tabs by index.
    if (hasModifier(e) && !e.shiftKey && !e.altKey) {
      const n = Number.parseInt(e.key, 10)
      if (Number.isFinite(n) && n >= 1 && n <= 9) {
        // NOTE: The exact mapping depends on whether Plan is available.
        // The Header will enforce parity mapping; here we only prevent default when
        // a mapping exists.
        // Keep the browser from focusing the tab bar / address bar.
        e.preventDefault()
      }
    }

    // Esc closes Settings. On chat it belongs to the TUI-parity Vim surface:
    // INSERT -> NAVIGATE, cancel Visual selection, or collapse an interaction.
    // Ctrl+C owns run interruption.
    if (e.key === 'Escape') {
      if (isOnSettingsPage()) {
        e.preventDefault()
        void router.push('/chat')
        return
      }
    }
  }

  window.addEventListener('keydown', onKeyDown)

  return () => {
    window.removeEventListener('keydown', onKeyDown)
  }
}
