import type {
  MessageLike,
  MessagePartLike,
  MessageRenderBlock,
  RenderBlock,
  RevertLike,
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
import type { JsonValue } from '@/types/json'

type JsonRecord = Record<string, JsonValue>

export type TranscriptProjectionOptions = {
  showReasoning: boolean
  revert: RevertLike | null
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
  const labels = attachments
    .map((value) => {
      const item = record(value)
      return firstText(item, ['title', 'filename', 'mime'])
    })
    .filter(Boolean)
  const own = text(part.filename) || firstText(content, ['name', 'title', 'path'])
  if (own && !labels.includes(own)) labels.unshift(own)
  return labels
}

function skillLabels(part: MessagePartLike): string[] {
  const content = durablePartContent(part)
  const skills = Array.isArray(content.skills) ? content.skills : []
  const labels = skills.map((value) => firstText(record(value), ['name'])).filter(Boolean)
  const own = firstText(content, ['skill', 'name'])
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
  if (kind === 'skill_ref') return 'skill'
  if (kind === 'notice' || kind === 'hook' || kind === 'system_notification') return 'notice'
  if (kind === 'compaction') return 'compaction'
  if (kind === 'assistant_reply_lifecycle') return 'lifecycle'
  if (kind === 'error') return 'error'
  return 'unknown'
}

function displayFields(
  part: MessagePartLike,
  kind: TranscriptPartKind,
): Pick<TranscriptDisplayPart, 'title' | 'summary' | 'copyText'> {
  if (kind === 'text' || kind === 'answer' || kind === 'text_segment' || kind === 'reasoning') {
    const body = transcriptPartText(part)
    const firstLine = body
      .split('\n')
      .map((line) => line.trim())
      .find(Boolean)
    return {
      title: kind === 'answer' ? 'Answer' : kind === 'reasoning' ? 'thinking' : kind === 'text_segment' ? 'Text' : '',
      summary: firstLine || '',
      copyText: body,
    }
  }
  if (kind === 'operation') {
    return { title: operationTitle(part), summary: operationSummary(part), copyText: operationCopyText(part) }
  }
  if (kind === 'resource') {
    const labels = attachmentLabels(part)
    return { title: 'Attachment', summary: labels.join(', '), copyText: labels.join('\n') }
  }
  if (kind === 'skill') {
    const labels = skillLabels(part)
    return { title: 'Skill', summary: labels.join(', '), copyText: labels.join('\n') }
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
    return { title, summary: '', copyText: title }
  }
  if (kind === 'error') {
    const body = transcriptPartText(part) || 'The run failed.'
    return { title: 'Error', summary: body, copyText: `${body}\n${prettyJson(durablePartContent(part))}` }
  }
  if (kind === 'notice' || kind === 'compaction') {
    const title = noticeTitle(part)
    const summary = noticeSummary(part)
    return { title, summary, copyText: [title, summary].filter(Boolean).join('\n') }
  }
  const content = durablePartContent(part)
  const body = transcriptPartText(part) || compactJson(content)
  return { title: durablePartKind(part), summary: body, copyText: body }
}

function projectPart(part: MessagePartLike, role: string, answerPartId: string | null): TranscriptDisplayPart {
  const id = String(part.id || '')
  const kind = classifyPart(part, answerPartId, role === 'assistant')
  const fields = displayFields(part, kind)
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
    defaultExpanded: kind === 'answer' || kind === 'text' || pendingInteraction,
  }
}

function lifecyclePart(message: MessageLike, runIds: string[]): TranscriptDisplayPart | null {
  if (text(message.info.role) !== 'assistant') return null
  const state = text(message.info.runState) || text(message.info.finish)
  if (!state) return null

  const id = runIds.at(-1) || String(message.info.id || '')
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

export function projectTranscriptBlocks(messages: MessageLike[], options: TranscriptProjectionOptions): RenderBlock[] {
  const visibleMessages: MessageLike[] = []
  const revertId = options.revert?.messageID || ''
  for (const message of messages || []) {
    const id = String(message.info.id || '')
    if (revertId && id && compareTranscriptIds(id, revertId) >= 0) break
    visibleMessages.push(message)
  }

  const blocks: RenderBlock[] = foldAssistantMessages(visibleMessages).map(
    ({ message, runIds }, messageIndex): MessageRenderBlock => {
      const role = text(message.info.role) || 'assistant'
      const ordered = [...(message.parts || [])].sort((a, b) =>
        compareTranscriptIds(String(a.id || ''), String(b.id || '')),
      )
      const answerId = finalAnswerPartId(role, ordered)
      const displayParts = ordered
        .map((part) => projectPart(part, role, answerId))
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
    },
  )

  if (options.revert) {
    blocks.push({ kind: 'revert', key: `revert:${options.revert.messageID}`, revert: options.revert })
  }
  return blocks
}
