import type {
  MessageLike,
  MessagePartLike,
  MessageRenderBlock,
  RenderBlock,
  TranscriptDisplayPart,
  TranscriptPartKind,
} from '@/components/chat/messageList.types'
import {
  isKnownRunState,
  isRunCancelled,
  isRunFailureState,
  isRunInFlight,
  normalizeRunState,
} from '../../lib/chatRunState'
import { attachmentLabel, attachmentLabelFromRecord, attachmentLabelFromUrl } from '../../lib/attachmentLabels'
import type { JsonValue } from '@/types/json'

type JsonRecord = Record<string, JsonValue>

export type TranscriptProjectionLabels = {
  /**
   * Title for an attachment row whose part carries no runtime presentation
   * title. The TUI localizes the same row from its own catalogue
   * (`message-input-activity-attachment`), so the transcript must never keep
   * a hardcoded English label for something the user just attached.
   */
  attachment?: string
}

export type TranscriptProjectionOptions = {
  showReasoning: boolean
  labels?: TranscriptProjectionLabels
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function record(value: unknown): JsonRecord {
  return isRecord(value) ? value : {}
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function rawText(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function firstText(source: JsonRecord, keys: readonly string[]): string {
  for (const key of keys) {
    const candidate = text(source[key])
    if (candidate) return candidate
  }
  return ''
}

function fragmentText(source: JsonRecord, key: string): string {
  const value = source[key]
  if (!Array.isArray(value)) return ''
  return value.filter((item): item is string => typeof item === 'string').join('')
}

function compactJson(value: unknown): string {
  try {
    return JSON.stringify(value)
  } catch {
    return String(value ?? '')
  }
}

function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value ?? '')
  }
}

export function compareTranscriptIds(left: string, right: string): number {
  if (/^\d+$/.test(left) && /^\d+$/.test(right)) {
    const a = BigInt(left)
    const b = BigInt(right)
    return a < b ? -1 : a > b ? 1 : 0
  }
  return left.localeCompare(right)
}

export function durablePartKind(part: MessagePartLike): string {
  return text(part.agenaKind).toLowerCase() || 'unknown'
}

export function durablePartContent(part: MessagePartLike): JsonRecord {
  return record(part.agenaContent)
}

export function transcriptPartText(part: MessagePartLike): string {
  const kind = durablePartKind(part)
  const content = durablePartContent(part)
  if (kind === 'text' || kind === 'paste_ref') {
    return rawText(part.text) || rawText(content.text)
  }
  if (kind === 'think') {
    return rawText(part.text) || fragmentText(content, 'summary') || fragmentText(content, 'raw')
  }
  if (kind === 'compaction') {
    return firstText(content, ['summary', 'detail']) || text(part.agenaSummary) || rawText(part.text)
  }
  if (kind === 'error') {
    const problem = record(content.problem)
    const user = record(problem.user)
    return firstText(user, ['fallback']) || firstText(problem, ['message']) || firstText(content, ['message'])
  }
  return rawText(part.text)
}

function toolCallView(part: MessagePartLike): JsonRecord {
  const content = durablePartContent(part)
  const presentation = record(part.agenaPresentation)
  const title = firstText(presentation, ['title'])
  const summary = firstText(presentation, ['summary'])
  const invocation = {
    name: firstText(content, ['name']) || 'unknown',
    plugin_name: content.plugin,
    input: record(content.input),
    tool_api_call: record(content.tool_api_call),
  }
  const blocks = Array.isArray(presentation.blocks) ? presentation.blocks : []
  return {
    call_id: content.call_id ?? 0,
    invocation,
    title,
    summary,
    blocks,
    user_input: record(content.user_input),
    authorization: record(content.authorization),
    metadata: record(content.metadata),
    error: content.error ?? null,
    lifecycle: record(content.lifecycle),
    output: content.output ?? null,
  }
}

function operationTitle(part: MessagePartLike): string {
  const content = durablePartContent(part)
  const operation = toolCallView(part)
  return firstText(operation, ['title']) || firstText(content, ['name']) || text(part.tool) || 'Operation'
}

function operationSummary(part: MessagePartLike): string {
  const operation = toolCallView(part)
  return firstText(operation, ['summary']) || text(part.agenaSummary) || firstText(record(part.state), ['title'])
}

function operationCopyText(part: MessagePartLike): string {
  const content = durablePartContent(part)
  const operation = toolCallView(part)
  const invocation = record(operation.invocation)
  const presentation = record(part.agenaPresentation)
  const input = Object.keys(record(content.input)).length ? record(content.input) : record(invocation.input)
  const blocks = Array.isArray(presentation.blocks) ? presentation.blocks : []
  const output = firstText(presentation, ['summary']) || (blocks.length ? prettyJson(blocks) : '')
  const sections = [operationTitle(part)]
  if (Object.keys(input).length) sections.push(`Input\n${prettyJson(input)}`)
  if (output) sections.push(`Output\n${output}`)
  return sections.join('\n\n')
}

function attachmentLabels(part: MessagePartLike): string[] {
  const content = durablePartContent(part)
  const attachments = Array.isArray(content.attachments) ? content.attachments : []
  const labels = attachments.map(attachmentLabelFromRecord).filter(Boolean)
  const own = attachmentLabel({
    title: content.title,
    filename: part.filename,
    name: content.name,
    path: content.path,
  })
  if (own && !labels.includes(own)) labels.unshift(own)
  return labels
}

/**
 * Local parts for a pending user turn. They use the same projection as the
 * persisted transcript, so a row never changes shape when the server
 * acknowledges the user run it came from.
 */
export function optimisticUserParts(args: {
  key: string
  text: string
  files: Array<{
    id?: string
    filename: string
    size?: number
    mime: string
    url?: string
    serverPath?: string
    delivery?: 'reference' | 'model_input'
  }>
  status: 'sending' | 'sent'
  fileFallbackLabel: string
  attachmentTitle?: string
}): TranscriptDisplayPart[] {
  const status = args.status === 'sending' ? 'in_progress' : 'completed'
  const parts: TranscriptDisplayPart[] = []
  if (args.text.trim()) {
    parts.push({
      key: `${args.key}:text`,
      id: `${args.key}:text`,
      kind: 'text',
      status,
      role: 'user',
      source: {
        id: `${args.key}:text`,
        type: 'text',
        partState: status,
        agenaKind: 'text',
        agenaRole: 'user',
        text: args.text,
        agenaContent: { text: args.text },
      },
      title: '',
      summary: '',
      copyText: args.text,
      toggleable: false,
      defaultExpanded: true,
    })
  }
  for (const [index, file] of args.files.entries()) {
    const id = `${args.key}:file:${index}`
    const label =
      attachmentLabel({ filename: file.filename, path: file.serverPath }) ||
      attachmentLabelFromUrl(file.url) ||
      args.fileFallbackLabel
    parts.push({
      key: id,
      id,
      kind: 'resource',
      status,
      role: 'user',
      source: {
        id,
        type: 'file',
        partState: status,
        agenaKind: 'file_ref',
        agenaRole: 'user',
        ...(file.filename ? { filename: file.filename } : {}),
        ...(file.mime ? { mime: file.mime } : {}),
        ...(file.url ? { url: file.url } : {}),
        ...(file.serverPath ? { serverPath: file.serverPath } : {}),
        agenaContent: {
          ...(file.filename ? { name: file.filename } : {}),
          ...(file.mime ? { mime: file.mime } : {}),
          ...(file.url ? { url: file.url } : {}),
          ...(file.serverPath ? { path: file.serverPath } : {}),
        },
      },
      title: text(args.attachmentTitle) || 'Attachment',
      summary: label,
      copyText: label,
      toggleable: false,
      defaultExpanded: true,
    })
  }
  return parts
}

function commandLabels(part: MessagePartLike): string[] {
  const content = durablePartContent(part)
  const raw = Array.isArray(content.commands) ? content.commands : content.skills
  const commands = Array.isArray(raw) ? raw : []
  const labels = commands.map((value) => firstText(record(value), ['name'])).filter(Boolean)
  const own = firstText(content, ['command', 'skill', 'name'])
  if (own && !labels.includes(own)) labels.unshift(own)
  return labels
}

function noticeTitle(part: MessagePartLike): string {
  const kind = durablePartKind(part)
  const content = durablePartContent(part)
  if (kind === 'system_notification') {
    const operationKind = firstText(content, ['operation_kind']) || 'background'
    const operationId = firstText(content, ['operation_id'])
    return operationId ? `${operationKind}:${operationId}` : operationKind
  }
  return firstText(content, ['title', 'hook', 'kind']) || (kind === 'compaction' ? 'Compaction' : 'Notice')
}

function noticeSummary(part: MessagePartLike): string {
  const content = durablePartContent(part)
  return firstText(content, ['summary', 'body', 'message', 'detail']) || text(part.agenaSummary)
}

export function partHasPendingInteraction(part: MessagePartLike): boolean {
  const operation = toolCallView(part)
  const userInput = record(operation.user_input)
  const userInputPending = (Array.isArray(userInput.requests) ? userInput.requests : []).some((value) => {
    const request = record(value)
    return request.reply === null || request.reply === undefined
  })
  if (userInputPending) return true

  const authorization = record(operation.authorization)
  return (Array.isArray(authorization.permissions) ? authorization.permissions : []).some((value) => {
    const permission = record(value)
    return permission.reply === null || permission.reply === undefined
  })
}

function classifyPart(part: MessagePartLike, answerPartId: string | null, assistant: boolean): TranscriptPartKind {
  const kind = durablePartKind(part)
  if (kind === 'text') {
    if (!assistant) return 'text'
    return String(part.id || '') === answerPartId ? 'answer' : 'text_segment'
  }
  if (kind === 'paste_ref') return 'text'
  if (kind === 'think') return 'reasoning'
  if (kind === 'tool_call') return 'operation'
  if (kind === 'file_ref') return 'resource'
  if (kind === 'skill_ref') return 'command'
  if (kind === 'notice' || kind === 'hook' || kind === 'system_notification') return 'notice'
  if (kind === 'compaction') return 'compaction'
  if (kind === 'assistant_reply_lifecycle') return 'lifecycle'
  if (kind === 'error') return 'error'
  return 'unknown'
}

/// The label a row shows is the one its part carries; the derived label is
/// only the fallback. Copy/jump text must use the same value or it would
/// disagree with the visible transcript.
function shownTitle(_part: MessagePartLike, presented: string, derived: string): string {
  return presented || derived
}

function shownSummary(_part: MessagePartLike, presented: string, derived: string): string {
  return presented || derived
}

function displayFields(
  part: MessagePartLike,
  kind: TranscriptPartKind,
  presentationLabels?: TranscriptProjectionLabels,
): Pick<TranscriptDisplayPart, 'title' | 'summary' | 'copyText'> {
  // Presentation is part metadata: when the runtime already projects a
  // human title/summary for this part, that wins over the client default.
  const presented = record(part.agenaPresentation)
  const presentedTitle = firstText(presented, ['title'])
  const presentedSummary = firstText(presented, ['summary'])
  if (kind === 'text' || kind === 'answer' || kind === 'text_segment' || kind === 'reasoning') {
    const body = transcriptPartText(part)
    const firstLine = body
      .split('\n')
      .map((line) => line.trim())
      .find(Boolean)
    return {
      title:
        presentedTitle ||
        (kind === 'answer' ? 'Answer' : kind === 'reasoning' ? 'thinking' : kind === 'text_segment' ? 'Text' : ''),
      summary: presentedSummary || firstLine || '',
      copyText: body,
    }
  }
  if (kind === 'operation') {
    return { title: operationTitle(part), summary: operationSummary(part), copyText: operationCopyText(part) }
  }
  if (kind === 'resource') {
    const labels = attachmentLabels(part)
    return {
      title: presentedTitle || text(presentationLabels?.attachment) || 'Attachment',
      summary: presentedSummary || labels.join(', '),
      copyText: labels.join('\n'),
    }
  }
  if (kind === 'command') {
    const labels = commandLabels(part)
    return {
      title: presentedTitle || 'Command',
      summary: presentedSummary || labels.join(', '),
      copyText: labels.join('\n'),
    }
  }
  if (kind === 'lifecycle') {
    const state = normalizeRunState(text(part.partState) || firstText(durablePartContent(part), ['state']))
    const title = isRunInFlight(state)
      ? 'Response running'
      : state === 'completed'
        ? 'Response completed'
        : isRunCancelled(state)
          ? 'Response cancelled'
          : isRunFailureState(state)
            ? 'Response failed'
            : state
    return { title: presentedTitle || title, summary: presentedSummary || '', copyText: title }
  }
  if (kind === 'error') {
    const body = transcriptPartText(part) || 'The run failed.'
    return {
      title: presentedTitle || 'Error',
      summary: presentedSummary || body,
      copyText: `${body}\n${prettyJson(durablePartContent(part))}`,
    }
  }
  if (kind === 'notice' || kind === 'compaction') {
    const title = shownTitle(part, presentedTitle, noticeTitle(part))
    const summary = shownSummary(part, presentedSummary, noticeSummary(part))
    return { title, summary, copyText: [title, summary].filter(Boolean).join('\n') }
  }
  const content = durablePartContent(part)
  const body = transcriptPartText(part) || compactJson(content)
  return { title: durablePartKind(part), summary: body, copyText: body }
}

/// Project a client-local element (optimistic row, in-flight placeholder)
/// through the same part projection the durable transcript uses, so a local
/// element can never drift from the server-rendered one.
export function projectLocalPart(
  source: MessagePartLike,
  role: string,
  labels?: TranscriptProjectionLabels,
): TranscriptDisplayPart {
  return projectPart(source, role, null, labels)
}

function projectPart(
  part: MessagePartLike,
  role: string,
  answerPartId: string | null,
  labels?: TranscriptProjectionLabels,
): TranscriptDisplayPart {
  const id = String(part.id || '')
  const kind = classifyPart(part, answerPartId, role === 'assistant')
  const fields = displayFields(part, kind, labels)
  const toggleable = !['text', 'lifecycle'].includes(kind)
  const pendingInteraction = kind === 'operation' && partHasPendingInteraction(part)
  return {
    key: `part:${id || compactJson(part).slice(0, 48)}`,
    id,
    kind,
    status: text(part.partState),
    role: text(part.agenaRole) || role,
    source: part,
    ...fields,
    toggleable,
    // A user attachment is part of the message itself. Keep its preview open
    // like the assistant's attachment rows instead of collapsing it into a
    // label the moment the server acknowledges the send.
    defaultExpanded:
      kind === 'answer' || kind === 'text' || pendingInteraction || (role === 'user' && kind === 'resource'),
  }
}

function lifecyclePart(message: MessageLike, runIds: string[]): TranscriptDisplayPart | null {
  if (text(message.info.role) !== 'assistant') return null
  const state = text(message.info.runState) || text(message.info.finish)
  if (!state) return null

  // The reply id is the durable identity of this lifecycle row; run ids only
  // remain as a fallback for markers written before it was published.
  const id = text(message.info.replyId) || runIds.at(-1) || String(message.info.id || '')
  const source: MessagePartLike = {
    id: `lifecycle:${id}`,
    type: 'tool',
    partState: state,
    agenaKind: 'assistant_reply_lifecycle',
    agenaRole: 'assistant',
    agenaContent: {
      state,
      run_ids: runIds,
      run_content: message.info.runContent ?? null,
    },
  }
  return projectPart(source, 'assistant', null)
}

function runNeedsLifecycle(stateInput: string, displayParts: TranscriptDisplayPart[]): boolean {
  const state = normalizeRunState(stateInput)
  if (!isKnownRunState(state)) return false
  if (!displayParts.length) return true
  if (!isRunFailureState(state) && !isRunCancelled(state)) return false
  return !displayParts.some((part) => part.kind === 'error')
}

function cloneMessage(message: MessageLike): MessageLike {
  return {
    info: { ...message.info },
    parts: [...(message.parts || [])],
    ...(message.folds ? { folds: [...message.folds] } : {}),
  }
}

export function foldAssistantMessages(messages: MessageLike[]): Array<{ message: MessageLike; runIds: string[] }> {
  const folded: Array<{ message: MessageLike; runIds: string[] }> = []
  for (const source of messages) {
    const message = cloneMessage(source)
    const id = String(message.info.id || '')
    const role = text(message.info.role)
    const previous = folded.at(-1)
    if (role === 'assistant' && previous && text(previous.message.info.role) === 'assistant') {
      previous.message.parts.push(...message.parts)
      if (message.folds?.length) {
        previous.message.folds = [...(previous.message.folds || []), ...message.folds]
      }
      previous.message.parts.sort((a, b) => compareTranscriptIds(String(a.id || ''), String(b.id || '')))
      previous.runIds.push(id)

      // Adjacent assistant runs form one visual reply, but their lifecycle is
      // not cumulative. Keep the latest run marker's server-owned metadata
      // instead of promoting an older failure over a newer running/completed
      // run. The first id/created time remain the stable visual block anchor.
      const firstId = previous.message.info.id
      const firstCreated = previous.message.info.time?.created
      previous.message.info = {
        ...message.info,
        id: firstId || message.info.id,
        ...(message.info.time || firstCreated !== undefined
          ? {
              time: {
                ...message.info.time,
                ...(firstCreated !== undefined ? { created: firstCreated } : {}),
              },
            }
          : {}),
      }
      continue
    }
    folded.push({ message, runIds: id ? [id] : [] })
  }
  return folded
}

function finalAnswerPartId(role: string, parts: MessagePartLike[]): string | null {
  if (role !== 'assistant') return null
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    const candidate = parts[index]
    if (!candidate || durablePartKind(candidate) !== 'text' || !transcriptPartText(candidate).trim()) continue
    const operationFollows = parts.slice(index + 1).some((later) => durablePartKind(later) === 'tool_call')
    if (!operationFollows) return String(candidate.id || '') || null
  }
  return null
}

export function projectTranscriptBlocks(
  messages: MessageLike[],
  options: TranscriptProjectionOptions = { showReasoning: true },
): RenderBlock[] {
  return foldAssistantMessages(messages || []).map(({ message, runIds }, messageIndex): MessageRenderBlock => {
    const role = text(message.info.role) || 'assistant'
    const ordered = [...(message.parts || [])].sort((a, b) =>
      compareTranscriptIds(String(a.id || ''), String(b.id || '')),
    )
    const answerId = finalAnswerPartId(role, ordered)
    const displayParts = ordered
      .map((part) => projectPart(part, role, answerId, options.labels))
      .filter((part) => {
        if (part.kind === 'reasoning') return options.showReasoning
        return true
      })
    const runState = text(message.info.runState) || text(message.info.finish)
    if (runNeedsLifecycle(runState, displayParts)) {
      const lifecycle = lifecyclePart(message, runIds)
      if (lifecycle) displayParts.push(lifecycle)
    }
    return {
      kind: 'message',
      key: `msg:${String(message.info.id || messageIndex)}`,
      message,
      displayParts,
      runIds,
      hasActivity: displayParts.some((part) => part.kind !== 'text'),
    }
  })
}
