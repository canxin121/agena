import { replyTextForCopy } from '@/features/inlineUi/copy'
import { i18n } from '@/i18n'
import { nextTick, onBeforeUnmount, ref, type Ref } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import { useI18n } from 'vue-i18n'

import { patchSessionIdInQuery } from '@/app/navigation/sessionQuery'
import { useWorkspaceNavigation } from '@/app/navigation/useWorkspaceNavigation'
import { isEmbeddedWorkspacePaneContext } from '@/app/windowScope'
import { isAcpSessionId } from '@/lib/acpSession'
import type { JsonValue } from '@/types/json'
import type { MessageEntry, Session } from '@/types/chat'
import { attachmentLabel, attachmentLabelFromUrl } from '../../lib/attachmentLabels'
import { getComposerInput, type ComposerExpose, type ComposerSegment } from './composerInput'
import type { AttachedFile } from './useChatAttachments'

type ToastKind = 'info' | 'success' | 'error'
type ToastsStore = { push: (kind: ToastKind, message: string) => void }

type MessagePartLike = {
  agenaKind?: string
  agenaContent?: JsonValue
  agenaInlineUi?: JsonValue
  agenaPresentation?: JsonValue
  type?: string
  text?: string
  filename?: string
  mime?: string
  url?: string
}

type MessageLike = {
  info?: {
    id?: string
    role?: string
  }
  parts?: MessagePartLike[]
}

type ChatLike = {
  selectedSessionId: string | null
  messages?: MessageEntry[]
  selectSession: (sessionId: string) => Promise<void>
  forkSession: (sessionId: string, opts?: { at_message_id?: number }) => Promise<JsonValue | null>
  revertToMessage: (
    sessionId: string,
    messageId: string,
    mode?: 'fork' | 'in_place',
  ) => Promise<{ session: Session; message: MessageEntry } | null>
}

function filePartLabel(part: MessagePartLike): string {
  const name = attachmentLabel({ filename: part?.filename })
  if (name) return name
  const url = typeof part?.url === 'string' ? part.url : ''
  if (!url) return 'file'
  if (url.startsWith('data:')) return 'attachment'
  return attachmentLabelFromUrl(url) || 'file'
}

export function useChatMessageActions(opts: {
  chat: ChatLike
  toasts: ToastsStore
  route: RouteLocationNormalizedLoaded
  router: Router
  sessionDirectory: Ref<string>

  draft: Ref<string>
  attachedFiles: Ref<AttachedFile[]>
  clearAttachments: () => void
  composerRef: Ref<ComposerExpose | null>

  getTextParts: (parts: MessagePartLike[]) => Array<{ text?: string }>
  copyToClipboard: (text: string) => Promise<void>
  scrollToBottom: (behavior: 'auto' | 'smooth') => void
  onAcpForkFromMessage?: (sessionId: string, beforeMessageId: number) => Promise<string | null>
  onAcpRewindFromMessage?: (sessionId: string, beforeMessageId: number) => Promise<void>
}) {
  const { t } = useI18n()

  const {
    chat,
    toasts,
    route,
    router,
    draft,
    attachedFiles,
    clearAttachments,
    composerRef,
    getTextParts,
    copyToClipboard,
    scrollToBottom,
    onAcpForkFromMessage,
    onAcpRewindFromMessage,
  } = opts

  const copiedMessageId = ref('')
  const historyActionBusy = ref(false)
  const revertBusyMessageId = ref('')
  let copiedTimer: number | null = null

  onBeforeUnmount(() => {
    if (copiedTimer) {
      window.clearTimeout(copiedTimer)
      copiedTimer = null
    }
  })

  async function handleCopyMessage(message: MessageLike) {
    const id = typeof message?.info?.id === 'string' ? message.info.id : ''
    let text: string
    try {
      text = await replyTextForCopy(
        Array.isArray(message?.parts) ? message.parts : [],
        (part) =>
          getTextParts([part])
            .map((p) => (typeof p?.text === 'string' ? p.text : ''))
            .join('\n\n'),
        chat.selectedSessionId,
        message?.info?.role === 'assistant',
      )
    } catch {
      toasts.push('error', t('common.copyFailed'))
      return
    }
    if (!text) return
    try {
      await copyToClipboard(text)
    } catch {
      toasts.push('error', t('common.copyFailed'))
      return
    }
    copiedMessageId.value = id
    if (copiedTimer) window.clearTimeout(copiedTimer)
    copiedTimer = window.setTimeout(() => {
      copiedMessageId.value = ''
      copiedTimer = null
    }, 1200)
  }

  async function openBranch(newId: string) {
    const isEmbeddedWorkspacePane = isEmbeddedWorkspacePaneContext(route.query)
    const isAcpBranch = isAcpSessionId(newId) || route.path === '/acp'
    if (isAcpBranch) {
      if (isEmbeddedWorkspacePane) {
        await router.replace({
          path: '/acp',
          query: patchSessionIdInQuery(route.query, newId),
        })
        await chat.selectSession(newId)
      } else {
        await useWorkspaceNavigation().openWorkspaceLocation('acp', {
          path: '/acp',
          query: { sessionId: newId },
          matchKeys: ['sessionId'],
          reuseExisting: true,
        })
      }
      return
    }
    if (isEmbeddedWorkspacePane) {
      await router.replace({ path: '/chat', query: patchSessionIdInQuery(route.query, newId) })
    } else {
      await router.replace({ path: '/chat' })
    }
    await chat.selectSession(newId)
  }

  async function restoreMessageIntoComposer(sessionId: string, message: MessageEntry | undefined) {
    if (!message) return
    const segments: ComposerSegment[] = []
    const files: AttachedFile[] = []
    for (const part of message.parts) {
      if (part.type === 'text' && (!part.synthetic || part.agenaKind === 'paste_ref')) {
        segments.push({ type: 'text', text: part.text || '' })
        continue
      }
      if (part.type !== 'file') continue
      const content = part.agenaContent && typeof part.agenaContent === 'object' ? part.agenaContent : {}
      const dataUrl = typeof part.url === 'string' && part.url.startsWith('data:') ? part.url : ''
      const path = typeof part.serverPath === 'string' ? part.serverPath : ''
      if (!dataUrl && !path) continue
      const id = `rewind-${sessionId}-${part.id}`
      files.push({
        id,
        filename: filePartLabel(part),
        size: 0,
        mime: part.mime || 'application/octet-stream',
        ...(dataUrl ? { url: dataUrl } : {}),
        ...(path ? { serverPath: path } : {}),
        delivery: content.delivery === 'model_input' ? 'model_input' : 'reference',
        state: 'ready',
      })
      segments.push({ type: 'attachment', id })
    }
    draft.value = segments
      .filter((segment): segment is Extract<ComposerSegment, { type: 'text' }> => segment.type === 'text')
      .map((segment) => segment.text)
      .join('')
    clearAttachments()
    attachedFiles.value = files
    await nextTick()
    composerRef.value?.restoreSegments?.(segments)
    getComposerInput(composerRef.value)?.focus()
  }

  async function handleForkFromMessage(messageId: string) {
    const sid = chat.selectedSessionId
    if (!sid || historyActionBusy.value) return
    historyActionBusy.value = true
    try {
      const atMessageId = Number(messageId)
      if (!Number.isSafeInteger(atMessageId) || atMessageId <= 0)
        throw new Error(i18n.global.t('chat.errors.messageIdRequired'))
      if (isAcpSessionId(sid) && onAcpForkFromMessage) {
        const newId = await onAcpForkFromMessage(sid, atMessageId)
        if (!newId) throw new Error(i18n.global.t('chat.errors.forkedSessionMissing'))
        await openBranch(newId)
        await nextTick()
        scrollToBottom('auto')
        toasts.push('success', t('chat.nativeAgent.forkedToast'))
        return
      }
      const created = await chat.forkSession(sid, { at_message_id: atMessageId })
      const newId = typeof created?.id === 'string' ? created.id.trim() : ''
      if (!newId) throw new Error(i18n.global.t('chat.errors.forkedSessionMissing'))
      await openBranch(newId)
      await nextTick()
      scrollToBottom('auto')
      toasts.push('success', t('chat.toasts.sessionForked'))
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      historyActionBusy.value = false
    }
  }

  async function handleRevertFromMessage(messageId: string, mode: 'fork' | 'in_place' = 'fork') {
    const sid = chat.selectedSessionId
    if (!sid || historyActionBusy.value) return

    const sourceRoute = route.fullPath
    historyActionBusy.value = true
    revertBusyMessageId.value = messageId
    try {
      if (isAcpSessionId(sid) && onAcpRewindFromMessage) {
        const atMessageId = Number(messageId)
        if (!Number.isSafeInteger(atMessageId) || atMessageId <= 0)
          throw new Error(i18n.global.t('chat.errors.messageIdRequired'))
        const retracted = chat.messages?.find((entry) => String(entry.info.id) === String(atMessageId))
        await onAcpRewindFromMessage(sid, atMessageId)
        if (chat.selectedSessionId !== sid || route.fullPath !== sourceRoute) return
        await restoreMessageIntoComposer(sid, retracted)
        scrollToBottom('auto')
        toasts.push('success', t('chat.nativeAgent.rewoundToast'))
        return
      }
      const rewound = await chat.revertToMessage(sid, messageId, mode)
      if (!rewound || chat.selectedSessionId !== sid || route.fullPath !== sourceRoute) return

      const { session, message } = rewound
      if (mode === 'fork') await openBranch(session.id)
      await restoreMessageIntoComposer(session.id, message)
      scrollToBottom('auto')
    } catch (err) {
      toasts.push('error', err instanceof Error ? err.message : String(err))
    } finally {
      revertBusyMessageId.value = ''
      historyActionBusy.value = false
    }
  }

  return {
    copiedMessageId,
    revertBusyMessageId,
    handleCopyMessage,
    handleForkFromMessage,
    handleRevertFromMessage,
  }
}
