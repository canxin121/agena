// Conversation-level actions a Bot sidebar row can perform. This reuses the
// project-session helpers instead of keeping a second implementation.
import { useI18n } from 'vue-i18n'

import { downloadMarkdown, sessionTranscriptText } from '@/lib/sessionTranscript'

type ToastKind = 'info' | 'success' | 'error'

type ChatLike = {
  getSessionById: (sessionId: string) => { id: string; title?: string; time?: { created?: number; updated?: number } } | null
  renameSession: (sessionId: string, title: string) => Promise<unknown>
}

export function useBotConversationActions(opts: {
  chat: ChatLike
  toasts: { push: (kind: ToastKind, message: string) => void }
  showThinking: () => boolean
  copyToClipboard: (text: string | Promise<string>) => Promise<void>
}) {
  const { t } = useI18n()
  const { chat, toasts, showThinking, copyToClipboard } = opts

  async function transcript(sessionId: string): Promise<string> {
    return sessionTranscriptText(chat.getSessionById(sessionId), '', { thinking: showThinking() })
  }

  async function rename(sessionId: string, title: string) {
    const next = String(title || '').trim()
    if (!sessionId || !next) {
      toasts.push('error', t('chat.toasts.titleCannotBeEmpty'))
      return
    }
    try {
      await chat.renameSession(sessionId, next)
      toasts.push('success', t('chat.toasts.sessionRenamed'))
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : String(error))
    }
  }

  async function copyTranscript(sessionId: string) {
    try {
      const text = await transcript(sessionId)
      if (!text) {
        toasts.push('error', t('chat.toasts.noTranscriptAvailable'))
        return
      }
      await copyToClipboard(text)
      toasts.push('success', t('chat.toasts.transcriptCopied'))
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : t('common.copyFailed'))
    }
  }

  async function exportTranscript(sessionId: string) {
    try {
      const text = await transcript(sessionId)
      if (!text) {
        toasts.push('error', t('chat.toasts.noTranscriptAvailable'))
        return
      }
      const filename = `session-${sessionId.slice(0, 8)}.md`
      downloadMarkdown(filename, text)
      toasts.push('success', t('chat.toasts.transcriptExportedAs', { filename }))
    } catch (error) {
      toasts.push('error', error instanceof Error ? error.message : String(error))
    }
  }

  return { rename, copyTranscript, exportTranscript }
}
