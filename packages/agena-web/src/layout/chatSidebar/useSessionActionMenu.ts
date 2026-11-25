import { computed, ref, type Component } from 'vue'
import {
  RiArchiveLine,
  RiClipboardLine,
  RiDeleteBack2Line,
  RiEditLine,
  RiFileUploadLine,
  RiGitBranchLine,
  RiHardDrive2Line,
  RiStarFill,
  RiStarLine,
} from '@remixicon/vue'
import { useI18n } from 'vue-i18n'
import { settingsText as st } from '@/i18n/settingsText'

import type { DirectoryEntry } from '@/features/sessions/model/types'
import type { SessionMenuTarget } from './sessionMenu.types'

type SessionLike = SessionMenuTarget['session']

export type SessionActionItem = {
  id: string
  label: string
  description?: string
  icon?: Component
  disabled?: boolean
  /** A destructive action asks for confirmation inside the menu. */
  variant?: 'default' | 'destructive'
  confirmTitle?: string
  confirmDescription?: string
  confirmText?: string
  cancelText?: string
}

type TFunction = (key: string, params?: Record<string, unknown>) => string

export function buildSessionActionItemsForSession(session: SessionLike | null | undefined): SessionActionItem[] {
  const { t } = useI18n()
  return buildSessionActionItemsForSessionI18n(t, session)
}

export function buildSessionActionItemsForSessionI18n(
  t: TFunction,
  session: SessionLike | null | undefined,
  // A Bot row shows the same conversation actions as a project session row,
  // with the Bot-specific profile and archive entries in place of "Fork" (a
  // fork has no project association to inherit).
  options?: { fork?: boolean; bot?: boolean },
): SessionActionItem[] {
  const items: SessionActionItem[] = [
    {
      id: 'toggle-favorite',
      label: String(
        t(
          session?.favorite
            ? 'chat.sidebar.sessionActions.unfavorite.label'
            : 'chat.sidebar.sessionActions.favorite.label',
        ),
      ),
      description: String(
        t(
          session?.favorite
            ? 'chat.sidebar.sessionActions.unfavorite.description'
            : 'chat.sidebar.sessionActions.favorite.description',
        ),
      ),
      icon: session?.favorite ? RiStarFill : RiStarLine,
    },
    {
      id: 'rename',
      label: String(t('chat.sidebar.sessionActions.rename.label')),
      description: String(t('chat.sidebar.sessionActions.rename.description')),
      icon: RiEditLine,
    },
    {
      id: 'copy-transcript',
      label: String(t('chat.sidebar.sessionActions.copyTranscript.label')),
      description: String(t('chat.sidebar.sessionActions.copyTranscript.description')),
      icon: RiClipboardLine,
    },
    {
      id: 'export-transcript',
      label: String(t('chat.sidebar.sessionActions.exportTranscript.label')),
      description: String(t('chat.sidebar.sessionActions.exportTranscript.description')),
      icon: RiFileUploadLine,
    },
    {
      id: 'storage',
      label: st('Storage management'),
      description: st('Review stored data and recover space by conversation.'),
      icon: RiHardDrive2Line,
    },
  ]
  if (options?.bot) {
    items.push({
      id: 'edit-bot',
      label: String(t('bot.profiles.edit')),
      description: String(t('bot.profiles.scope')),
      icon: RiEditLine,
    })
    items.push({
      id: 'archive-bot',
      label: String(t('bot.profiles.archive')),
      description: String(t('bot.profiles.archiveHint')),
      icon: RiArchiveLine,
    })
  } else if (options?.fork !== false) {
    items.push({
      id: 'fork',
      label: String(t('chat.sidebar.sessionActions.fork.label')),
      description: String(t('chat.sidebar.sessionActions.fork.description')),
      icon: RiGitBranchLine,
    })
    items.push({
      id: 'clear',
      label: String(t('chat.sidebar.sessionActions.clear.label')),
      description: String(t('chat.sidebar.sessionActions.clear.description')),
      icon: RiDeleteBack2Line,
    })
  }
  return items
}

export function useSessionActionMenu(opts: {
  chat: { selectedSessionId: string | null }
  ui: { requestSessionAction: (id: string, sessionId?: string) => void }
  selectSession: (sessionId: string) => Promise<void>
  // Project rows select the conversation before acting; a Bot row already owns
  // its own pane, so it rides the same menu without changing selection.
  runAction?: (item: SessionActionItem, target: SessionMenuTarget) => void | Promise<boolean>
}) {
  const { t } = useI18n()

  const sessionActionMenuTarget = ref<SessionMenuTarget | null>(null)
  const sessionActionMenuQuery = ref('')
  const sessionActionMenuAnchorRef = ref<HTMLElement | null>(null)

  function closeSessionActionMenu() {
    sessionActionMenuTarget.value = null
    sessionActionMenuAnchorRef.value = null
    sessionActionMenuQuery.value = ''
  }

  const sessionActionItems = computed<SessionActionItem[]>(() =>
    buildSessionActionItemsForSessionI18n(t, sessionActionMenuTarget.value?.session),
  )

  const filteredSessionActionItems = computed<SessionActionItem[]>(() => {
    const q = sessionActionMenuQuery.value.trim().toLowerCase()
    const list = sessionActionItems.value
    if (!q) return list
    return list.filter((item) => {
      const label = item.label.toLowerCase()
      const desc = String(item.description || '').toLowerCase()
      return label.includes(q) || desc.includes(q) || item.id.includes(q)
    })
  })

  function openSessionActionMenu(
    directory: DirectoryEntry | null,
    session: SessionLike,
    event?: MouseEvent | PointerEvent,
  ) {
    const sid = String(session?.id || '').trim()
    if (!sid) return
    if (sessionActionMenuTarget.value?.session?.id === sid) {
      closeSessionActionMenu()
      return
    }
    sessionActionMenuTarget.value = { directory, session }
    sessionActionMenuAnchorRef.value = event?.currentTarget instanceof HTMLElement ? event.currentTarget : null
    sessionActionMenuQuery.value = ''
  }

  async function runSessionActionMenu(item: SessionActionItem) {
    if (item.disabled) return
    const target = sessionActionMenuTarget.value
    closeSessionActionMenu()
    // A caller that owns the action (a Bot row acting on its own conversation)
    // handles it and reports whether the shared dispatch should stop here.
    if (target && opts.runAction && (await opts.runAction(item, target))) return
    const targetSessionId = typeof target?.session?.id === 'string' ? target.session.id.trim() : ''
    if (targetSessionId && targetSessionId !== opts.chat.selectedSessionId) {
      await opts.selectSession(targetSessionId)
    }
    opts.ui.requestSessionAction(item.id, targetSessionId)
  }

  return {
    sessionActionMenuAnchorRef,
    sessionActionMenuQuery,
    sessionActionMenuTarget,
    filteredSessionActionItems,
    openSessionActionMenu,
    runSessionActionMenu,
    closeSessionActionMenu,
  }
}
