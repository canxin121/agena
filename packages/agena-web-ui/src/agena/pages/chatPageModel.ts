import {
  type DomainEventRecord,
  type MessagePart,
  type MessageResource,
  type SessionExecutionResource,
} from '../lib/agenaApi'

export type ChatEventState = {
  messages: MessageResource[]
  timelineEvents: DomainEventRecord[]
  sessionState: SessionExecutionResource | null
  selectedSessionId: number | null
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function readNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function messageRole(value: unknown): MessageResource['role'] {
  return value === 'user' || value === 'system' ? value : 'assistant'
}

function messageState(value: unknown): MessageResource['state'] {
  const allowed: MessageResource['state'][] = [
    'pending',
    'in_progress',
    'completed',
    'policy_denied',
    'user_declined',
    'capability_unavailable',
    'tool_unavailable',
    'failed',
    'cancelled',
  ]
  return allowed.includes(value as MessageResource['state']) ? (value as MessageResource['state']) : 'in_progress'
}

function normalizeLiveContent(value: unknown): Record<string, unknown> | null {
  const content = asRecord(value)
  if (!content) return null
  if (content.type !== 'activity') return content
  const activityType = typeof content.activity_type === 'string' ? content.activity_type : 'activity'
  const payload = asRecord(content.payload) || {}
  if (activityType === 'resource') return { ...payload, type: 'attachment' }
  if (activityType === 'interaction') return { ...payload, type: 'request' }
  if (activityType === 'skill_reference') return { ...payload, type: 'skill_reference' }
  return { ...payload, type: activityType }
}

function livePartFromPayload(value: unknown): MessagePart | null {
  const part = asRecord(value)
  if (!part || (typeof part.id !== 'number' && typeof part.id !== 'string')) return null
  if (typeof part.message_id !== 'number' && typeof part.message_id !== 'string') return null
  const content = part.content
  return {
    id: part.id as number | string,
    message_id: part.message_id as number | string,
    part_index: readNumber(part.part_index) ?? 0,
    status: messageState(part.status),
    kind: typeof part.kind === 'string' ? part.kind : 'activity',
    name: typeof part.name === 'string' ? part.name : null,
    summary: typeof part.summary === 'string' ? part.summary : null,
    has_detail: part.has_detail !== false,
    operation_id: typeof part.operation_id === 'string' ? part.operation_id : null,
    created_at: readString(part.created_at) || new Date().toISOString(),
    content: normalizeLiveContent(content),
  }
}

function isTerminalPartStatus(status: MessagePart['status']): boolean {
  return !['pending', 'in_progress'].includes(status)
}

function mergeOperationContent(
  currentContent: Record<string, unknown> | null | undefined,
  incomingContent: Record<string, unknown> | null | undefined,
): Record<string, unknown> | null {
  if (!currentContent || currentContent.type !== 'operation') return incomingContent || currentContent || null
  if (!incomingContent || incomingContent.type !== 'operation') return incomingContent || currentContent

  const currentBlocks = Array.isArray(currentContent.blocks) ? currentContent.blocks : []
  const incomingBlocks = Array.isArray(incomingContent.blocks)
    ? incomingContent.blocks.map((block) => (asRecord(block) ? { ...block } : block))
    : []
  const currentCommandBlocks = currentBlocks
    .map((block) => asRecord(block))
    .filter((block): block is Record<string, unknown> => block?.type === 'command')

  for (const currentCommand of currentCommandBlocks) {
    const incomingIndex = incomingBlocks.findIndex((block) => asRecord(block)?.type === 'command')
    if (incomingIndex < 0) {
      incomingBlocks.unshift({ ...currentCommand })
      continue
    }
    const incomingCommand = asRecord(incomingBlocks[incomingIndex]) || {}
    const mergedCommand: Record<string, unknown> = { ...currentCommand, ...incomingCommand }
    for (const stream of ['stdout', 'stderr']) {
      const currentOutput = typeof currentCommand[stream] === 'string' ? currentCommand[stream] : ''
      const incomingOutput = typeof incomingCommand[stream] === 'string' ? incomingCommand[stream] : ''
      if (currentOutput && !incomingOutput) mergedCommand[stream] = currentOutput
    }
    incomingBlocks[incomingIndex] = mergedCommand
  }

  const merged: Record<string, unknown> = { ...currentContent, ...incomingContent, blocks: incomingBlocks }
  const currentSequences = asRecord(currentContent.__live_command_seq)
  const incomingSequences = asRecord(incomingContent.__live_command_seq)
  if (currentSequences || incomingSequences) {
    merged.__live_command_seq = { ...(currentSequences || {}), ...(incomingSequences || {}) }
  }
  const currentBytes = asRecord(currentContent.__live_command_bytes)
  const incomingBytes = asRecord(incomingContent.__live_command_bytes)
  if (currentBytes || incomingBytes) {
    merged.__live_command_bytes = { ...(currentBytes || {}), ...(incomingBytes || {}) }
  }
  return merged
}

function mergeLiveMessagePart(current: MessagePart, incoming: MessagePart): MessagePart {
  const status =
    isTerminalPartStatus(current.status) && !isTerminalPartStatus(incoming.status) ? current.status : incoming.status
  return {
    ...current,
    ...incoming,
    status,
    name: incoming.name || current.name,
    summary: incoming.summary || current.summary,
    operation_id: incoming.operation_id || current.operation_id,
    content: mergeOperationContent(current.content, incoming.content),
  }
}

/** Reduce a full typed runtime part directly into the conversation. This is
 * intentionally independent from the canonical transcript GET: ephemeral
 * stream updates may exist before the projection is durable. */
export function upsertLiveMessagePart(
  messages: MessageResource[],
  event: DomainEventRecord,
  sessionId: number,
): MessageResource[] {
  const payload = asRecord(event.payload)
  const part = livePartFromPayload(payload?.part)
  if (!payload || !part) return messages

  const role = messageRole(payload.message_role)
  const replyId = readString(payload.reply_id)
  const turnId = readString(payload.turn_id)
  const messageId =
    role === 'assistant' && replyId
      ? `reply:${replyId}`
      : role === 'user' && turnId
        ? `turn:${turnId}:input`
        : `live:${part.message_id}`
  const metadata = asRecord(payload.message_metadata) || {}
  const eventTimestamp = readString(payload.message_created_at) || part.created_at
  const existingIndex = messages.findIndex((message) => {
    if (String(message.id) === messageId) return true
    if (
      (replyId && message.metadata.canonical_reply_id === replyId) ||
      (turnId && message.metadata.canonical_turn_id === turnId && message.role === role)
    )
      return true
    return message.parts?.some((candidate) => {
      if (String(candidate.id) === String(part.id)) return true
      const candidateContent = asRecord(candidate.content)
      const incomingContent = asRecord(part.content)
      return (
        candidateContent?.type === 'operation' &&
        incomingContent?.type === 'operation' &&
        candidateContent.call_id != null &&
        String(candidateContent.call_id) === String(incomingContent.call_id)
      )
    })
  })

  const current = existingIndex >= 0 ? messages[existingIndex] : null
  const previousRevision = asRecord(current?.metadata.__live_part_revisions)?.[String(part.id)]
  if (typeof previousRevision === 'number' && previousRevision >= event.seq_global) return messages

  const nextParts = [...(current?.parts || [])]
  const partIndex = nextParts.findIndex((candidate) => String(candidate.id) === String(part.id))
  if (partIndex >= 0) nextParts[partIndex] = mergeLiveMessagePart(nextParts[partIndex]!, part)
  else nextParts.push(part)
  nextParts.sort((left, right) => left.part_index - right.part_index)

  const revisions = {
    ...(asRecord(current?.metadata.__live_part_revisions) || {}),
    [String(part.id)]: event.seq_global,
  }
  const nextMetadata: Record<string, unknown> = {
    ...(current?.metadata || {}),
    ...metadata,
    ...(replyId ? { canonical_reply_id: replyId } : {}),
    ...(turnId ? { canonical_turn_id: turnId } : {}),
    __live_part_revisions: revisions,
  }
  if (nextMetadata.__live_command_temp) delete nextMetadata.__live_command_temp
  if (nextMetadata.__live_command_key) delete nextMetadata.__live_command_key

  const nextMessage: MessageResource = {
    id: current?.metadata.__live_command_temp ? messageId : (current?.id ?? messageId),
    session_id: current?.session_id ?? sessionId,
    role: current?.role ?? role,
    state: messageState(payload.message_state ?? current?.state),
    created_at: current?.created_at ?? eventTimestamp,
    updated_at: new Date().toISOString(),
    metadata: nextMetadata,
    usage: current?.usage ?? null,
    part_count: nextParts.length,
    parts: nextParts,
  }

  if (existingIndex >= 0) {
    const next = [...messages]
    next[existingIndex] = nextMessage
    return next
  }
  return [...messages, nextMessage]
}

function commandDeltaBytes(payload: Record<string, unknown>): number[] {
  if (!Array.isArray(payload.chunk)) return []
  return payload.chunk
    .filter((value): value is number => typeof value === 'number' && Number.isFinite(value))
    .map((value) => Math.max(0, Math.min(255, value)))
}

function decodeCommandBytes(bytes: number[]): string {
  if (!bytes.length) return ''
  try {
    return new TextDecoder('utf-8', { fatal: false }).decode(Uint8Array.from(bytes))
  } catch {
    return String.fromCharCode(...bytes)
  }
}

function commandDeltaText(payload: Record<string, unknown>): string {
  const bytes = commandDeltaBytes(payload)
  return bytes.length ? decodeCommandBytes(bytes) : typeof payload.preview_text === 'string' ? payload.preview_text : ''
}

function commandPartMatches(part: MessagePart, partId: number | null, callId: number | null): boolean {
  if (partId != null && String(part.id) === String(partId)) return true
  if (callId == null) return false
  if (part.operation_id != null && String(part.operation_id) === String(callId)) return true
  const operation = asRecord(part.content)
  return operation?.type === 'operation' && String(operation.call_id) === String(callId)
}

function commandCorrelationKey(
  physicalMessageId: number | null,
  partId: number | null,
  callId: number | null,
  event: DomainEventRecord,
): string {
  if (physicalMessageId == null && partId == null && callId == null) return `event:${event.seq_global}`
  return [physicalMessageId ?? '', partId ?? '', callId ?? ''].join(':')
}

function liveCommandPart(
  payload: Record<string, unknown>,
  event: DomainEventRecord,
  sessionId: number,
  key: string,
): MessagePart {
  const partId = readNumber(payload.part_id)
  const callId = readNumber(payload.call_id)
  const messageId = readNumber(payload.message_id) ?? `live-command:${sessionId}:${key}`
  const command = readString(payload.command) || 'shell'
  const operation: Record<string, unknown> = {
    type: 'operation',
    call_id: callId ?? undefined,
    title: 'Running shell command',
    invocation: { name: 'shell', input: {} },
    blocks: [{ type: 'command', command }],
    __live_command_seq: {},
  }
  return {
    id: partId ?? `command:${key}`,
    message_id: messageId,
    part_index: 0,
    status: 'in_progress',
    kind: 'operation',
    name: 'shell',
    summary: 'Command running…',
    has_detail: true,
    operation_id: callId == null ? null : String(callId),
    created_at: event.created_at,
    content: operation,
  }
}

/** Reduce process lifecycle/output events into the already-visible operation.
 * These events are intentionally ephemeral: the terminal Operation snapshot
 * remains the source of truth after execution_finished, while this reducer
 * keeps shell output visible during a long-running command. */
export function applyLiveCommandEvent(
  messages: MessageResource[],
  event: DomainEventRecord,
  sessionId: number,
): MessageResource[] {
  const payload = asRecord(event.payload)
  if (!payload) return messages
  const partId = readNumber(payload.part_id)
  const callId = readNumber(payload.call_id)
  const physicalMessageId = readNumber(payload.message_id)
  const correlationKey = commandCorrelationKey(physicalMessageId, partId, callId, event)
  const messageIndex = messages.findIndex(
    (message) =>
      message.metadata.__live_command_key === correlationKey ||
      (physicalMessageId != null &&
        message.parts?.some((part) => String(part.message_id) === String(physicalMessageId))) ||
      message.parts?.some((part) => commandPartMatches(part, partId, callId)),
  )
  const next = [...messages]
  let resolvedMessageIndex = messageIndex
  if (resolvedMessageIndex < 0) {
    const temporaryPart = liveCommandPart(payload, event, sessionId, correlationKey)
    next.push({
      id: `live-command:${sessionId}:${correlationKey}`,
      session_id: sessionId,
      role: 'assistant',
      state: 'in_progress',
      created_at: event.created_at,
      updated_at: event.created_at,
      metadata: {
        __live_command_temp: true,
        __live_command_key: correlationKey,
        __live_part_revisions: {},
      },
      usage: null,
      part_count: 1,
      parts: [temporaryPart],
    })
    resolvedMessageIndex = next.length - 1
  }

  let current = next[resolvedMessageIndex]
  if (!current) return messages
  let partIndex = (current.parts || []).findIndex((part) => commandPartMatches(part, partId, callId))
  if (partIndex < 0) {
    const parts = [...(current.parts || []), liveCommandPart(payload, event, sessionId, correlationKey)]
    current = { ...current, parts, part_count: parts.length }
    next[resolvedMessageIndex] = current
    partIndex = parts.length - 1
  }
  const currentPart = current.parts?.[partIndex]
  if (!currentPart) return messages
  const operation = asRecord(currentPart.content)
  if (!operation || operation.type !== 'operation') return next

  const nextOperation: Record<string, unknown> = { ...operation }
  const nextBlocks = Array.isArray(operation.blocks)
    ? operation.blocks.map((block) => (asRecord(block) ? { ...block } : block))
    : []
  let commandBlock = nextBlocks.find((block) => asRecord(block)?.type === 'command') as
    Record<string, unknown> | undefined
  if (!commandBlock) {
    commandBlock = {
      type: 'command',
      command: readString(payload.command) || 'shell',
    }
    nextBlocks.unshift(commandBlock)
  }

  const stream = payload.stream === 'stderr' ? 'stderr' : 'stdout'
  const liveSequences = asRecord(operation.__live_command_seq) || {}
  const nextSequences = { ...liveSequences }
  const incomingSequence = readNumber(payload.seq)
  const liveBytes = asRecord(operation.__live_command_bytes) || {}
  const nextBytes = { ...liveBytes }
  const livePrefixes = asRecord(operation.__live_command_prefixes) || {}
  const nextPrefixes = { ...livePrefixes }
  if (event.kind === 'command_output_delta') {
    const previousSequence = readNumber(liveSequences[stream])
    if (incomingSequence != null && previousSequence != null && previousSequence >= incomingSequence) return next
    const bytes = commandDeltaBytes(payload)
    if (bytes.length) {
      const previousBytes = Array.isArray(liveBytes[stream])
        ? (liveBytes[stream] as unknown[]).filter((value): value is number => typeof value === 'number')
        : []
      if (!(stream in nextPrefixes)) {
        nextPrefixes[stream] = typeof commandBlock[stream] === 'string' ? commandBlock[stream] : ''
      }
      const combinedBytes = [...previousBytes, ...bytes]
      nextBytes[stream] = combinedBytes
      commandBlock[stream] =
        `${typeof nextPrefixes[stream] === 'string' ? nextPrefixes[stream] : ''}${decodeCommandBytes(combinedBytes)}`
    } else {
      const delta = commandDeltaText(payload)
      if (delta) {
        commandBlock[stream] = `${typeof commandBlock[stream] === 'string' ? commandBlock[stream] : ''}${delta}`
      }
    }
    if (incomingSequence != null) nextSequences[stream] = incomingSequence
  }

  if (event.kind === 'command_begin') {
    if (readString(payload.command)) commandBlock.command = payload.command
    if (readString(payload.cwd)) commandBlock.cwd = payload.cwd
  } else if (event.kind === 'command_end') {
    if (typeof payload.stdout === 'string') commandBlock.stdout = payload.stdout
    if (typeof payload.stderr === 'string') commandBlock.stderr = payload.stderr
    if (typeof payload.exit_code === 'number') commandBlock.exit_code = payload.exit_code
    if (typeof payload.cwd === 'string') commandBlock.cwd = payload.cwd
    nextOperation.__live_command_seq = nextSequences
    delete nextOperation.__live_command_bytes
    delete nextOperation.__live_command_prefixes
  } else if (event.kind === 'command_output_delta') {
    nextOperation.__live_command_seq = nextSequences
    nextOperation.__live_command_bytes = nextBytes
    nextOperation.__live_command_prefixes = nextPrefixes
  }
  nextOperation.blocks = nextBlocks

  const status = event.kind === 'command_end' ? messageState(payload.status) : 'in_progress'
  const nextParts = [...(current.parts || [])]
  nextParts[partIndex] = {
    ...currentPart,
    status,
    summary:
      event.kind === 'command_end'
        ? `Command exited with code ${readNumber(payload.exit_code) ?? -1}.`
        : 'Command running…',
    content: nextOperation,
  }
  const nextMessage: MessageResource = {
    ...current,
    session_id: current.session_id || sessionId,
    updated_at: new Date().toISOString(),
    part_count: nextParts.length,
    parts: nextParts,
  }
  next[resolvedMessageIndex] = nextMessage
  return next
}

export function appendTimelineEvent(
  timelineEvents: DomainEventRecord[],
  event: DomainEventRecord,
): DomainEventRecord[] {
  if (timelineEvents.some((item) => item.seq_global === event.seq_global)) {
    return timelineEvents
  }
  return [...timelineEvents, event].sort((left, right) => left.seq_global - right.seq_global)
}

function requestConversationRefresh(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  return {
    state: {
      ...state,
      timelineEvents: appendTimelineEvent(state.timelineEvents, event),
    },
    shouldRefresh: true,
  }
}

function patchSessionStateFromEvent(
  state: ChatEventState,
  event: DomainEventRecord,
  payload: Record<string, unknown>,
): { state: ChatEventState; shouldRefresh: boolean } {
  const nextTimelineEvents = appendTimelineEvent(state.timelineEvents, event)
  if (!state.sessionState) {
    return {
      state: {
        ...state,
        timelineEvents: nextTimelineEvents,
      },
      shouldRefresh: true,
    }
  }

  switch (event.kind) {
    case 'execution_started':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            active_execution: {
              execution_id: readString(payload.execution_id) || 'unknown',
              phase: 'starting',
            },
          },
        },
        shouldRefresh: false,
      }
    case 'execution_finished':
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
          sessionState: {
            ...state.sessionState,
            active_execution: null,
          },
        },
        shouldRefresh: true,
      }
    default:
      return {
        state: {
          ...state,
          timelineEvents: nextTimelineEvents,
        },
        shouldRefresh: false,
      }
  }
}

export function applySessionEvent(
  state: ChatEventState,
  event: DomainEventRecord,
): { state: ChatEventState; shouldRefresh: boolean } {
  const payload = asRecord(event.payload)
  if (!payload) {
    return { state, shouldRefresh: true }
  }
  const nextTimelineEvents = appendTimelineEvent(state.timelineEvents, event)

  switch (event.kind) {
    case 'message_part_checkpointed':
    case 'transcript_part_upserted':
      return {
        state: {
          ...state,
          messages: state.selectedSessionId
            ? upsertLiveMessagePart(state.messages, event, state.selectedSessionId)
            : state.messages,
          timelineEvents: nextTimelineEvents,
        },
        shouldRefresh: false,
      }
    case 'command_begin':
    case 'command_output_delta':
    case 'command_end':
      return {
        state: {
          ...state,
          messages: state.selectedSessionId
            ? applyLiveCommandEvent(state.messages, event, state.selectedSessionId)
            : state.messages,
          timelineEvents: nextTimelineEvents,
        },
        shouldRefresh: false,
      }
    case 'user_message_appended':
      return requestConversationRefresh(state, event)
    case 'assistant_message_finished': {
      const withTimeline = {
        ...state,
        timelineEvents: appendTimelineEvent(state.timelineEvents, event),
      }
      if (!withTimeline.sessionState) {
        return {
          state: withTimeline,
          shouldRefresh: true,
        }
      }
      return {
        state: {
          ...withTimeline,
        },
        shouldRefresh: true,
      }
    }
    case 'execution_started':
    case 'execution_finished':
    case 'run_started':
    case 'run_completed':
    case 'run_aborted':
      return patchSessionStateFromEvent(state, event, payload)
    default:
      return {
        state: {
          ...state,
          timelineEvents: appendTimelineEvent(state.timelineEvents, event),
        },
        shouldRefresh: true,
      }
  }
}
