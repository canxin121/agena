import { computed, type ComputedRef, ref, type Ref } from 'vue'
import { useI18n } from 'vue-i18n'

import { formatTranscript } from '@/lib/transcript'
import { listSessionMessages } from '@/stores/chat/api'
import type { JsonValue } from '@/types/json'

type ToastKind = 'info' | 'success' | 'error'

type ToastsLike = {
  push: (kind: ToastKind, message: string, timeoutMs?: number) => void
}

type SessionLike = {
  id: string
  time?: { created?: number; updated?: number }
}

type ChatLike = {
  selectedSessionId: string | null
  selectedSession: SessionLike | null
  renameSession: (sessionId: string, title: string) => Promise<JsonValue>
  forkSession: (sessionId: string) => Promise<JsonValue>
  compactSession: (sessionId: string) => Promise<JsonValue>
}

export function useChatSessionActions(opts: {
  chat: ChatLike
  toasts: ToastsLike

  sessionTitle: ComputedRef<string>
  showThinking: Ref<boolean>

  copyToClipboard: (text: string | Promise<string>) => Promise<void>

  // Navigate to a freshly created fork.
  onSessionForked?: (sessionId: string) => void | Promise<void>
}) {
  const { t } = useI18n()

  const { chat, toasts, sessionTitle, showThinking, copyToClipboard, onSessionForked } = opts

  const renameDialogOpen = ref(false)
  const renameDraft = ref('')
  const renameBusy = ref(false)

  const forkBusy = ref(false)
  const compactBusy = ref(false)
  const transcriptBusy = ref(false)

  function openRenameDialog() {
    renameDraft.value = sessionTitle.value || ''
    renameDialogOpen.value = true
  }

  async function saveRename() {
    const sid = chat.selectedSessionId
    const next = renameDraft.value.trim()
    if (!sid) return
    if (!next) {
      toasts.push('error', t('chat.toasts.titleCannotBeEmpty'))
      return
    }
    renameBusy.value = true
    try {
      await chat.renameSession(sid, next)
      renameDialogOpen.value = false
      toasts.push('success', t('chat.toasts.sessionRenamed'))
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      renameBusy.value = false
    }
  }

  const includeThinking = computed(() => Boolean(showThinking.value))

  async function buildTranscriptText(session: SessionLike, title: string): Promise<string> {
    const options = {
      thinking: includeThinking.value,
      toolDetails: false,
      assistantMetadata: true,
    }
    const messages = await listSessionMessages(session.id)
    if (!messages.length) return ''
    return formatTranscript(
      {
        id: session.id,
        title: title || session.id,
        time: session.time,
      },
      messages,
      options,
    )
  }

  async function copyTranscript() {
    if (transcriptBusy.value) return
    const session = chat.selectedSession
    if (!session) {
      toasts.push('error', t('chat.toasts.noTranscriptAvailable'))
      return
    }
    transcriptBusy.value = true
    try {
      const text = buildTranscriptText(session, sessionTitle.value).then((value) => {
        if (!value) throw new Error(t('chat.toasts.noTranscriptAvailable'))
        return value
      })
      await copyToClipboard(text)
      toasts.push('success', t('chat.toasts.transcriptCopied'))
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : t('common.copyFailed'))
    } finally {
      transcriptBusy.value = false
    }
  }

  function downloadTranscript(filename: string, content: string) {
    const blob = new Blob([content], { type: 'text/markdown' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    document.body.appendChild(link)
    link.click()
    link.remove()
    URL.revokeObjectURL(url)
  }

  async function exportTranscript() {
    if (transcriptBusy.value) return
    const session = chat.selectedSession
    if (!session) {
      toasts.push('error', t('chat.toasts.noTranscriptAvailable'))
      return
    }
    transcriptBusy.value = true
    try {
      const text = await buildTranscriptText(session, sessionTitle.value)
      if (!text) {
        toasts.push('error', t('chat.toasts.noTranscriptAvailable'))
        return
      }
      const filename = `session-${session.id.slice(0, 8)}.md`
      downloadTranscript(filename, text)
      toasts.push('success', t('chat.toasts.transcriptExportedAs', { filename }))
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      transcriptBusy.value = false
    }
  }

  async function handleForkSession() {
    const sid = chat.selectedSessionId
    if (!sid || forkBusy.value) return
    forkBusy.value = true
    try {
      const created = await chat.forkSession(sid)
      const createdId = typeof created?.id === 'string' ? created.id.trim() : ''
      if (createdId) {
        if (typeof onSessionForked === 'function') await onSessionForked(createdId)
        toasts.push('success', t('chat.toasts.sessionForked'))
      } else {
        throw new Error('The server did not return a forked session.')
      }
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      forkBusy.value = false
    }
  }

  async function handleCompactSession() {
    const sid = chat.selectedSessionId
    if (!sid) return
    compactBusy.value = true
    try {
      await chat.compactSession(sid)
      toasts.push('success', t('chat.toasts.compactionStarted'))
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      compactBusy.value = false
    }
  }

  return {
    renameDialogOpen,
    renameDraft,
    renameBusy,
    forkBusy,
    compactBusy,
    openRenameDialog,
    saveRename,
    copyTranscript,
    exportTranscript,
    handleForkSession,
    handleCompactSession,
  }
}
