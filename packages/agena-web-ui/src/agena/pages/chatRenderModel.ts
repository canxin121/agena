import { readSessionPartAwaitingUserInput } from '@/agena/lib/agenaApi'
import type { MessagePart, MessageResource, SessionPart } from '@/agena/lib/agenaApi'

import { formatUsageCount, formatUsageUsd } from './chatUsageModel'

export type RenderBlock = {
  body: string
  kind: 'markdown' | 'terminal' | 'diff' | 'input_activity' | 'operation_outcome'
  activityLabel?: string
  summary?: string
  title?: string
  outcome?: 'policy_denied' | 'user_declined' | 'capability_unavailable' | 'tool_unavailable'
  language?: string
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function readFiniteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function textArtifactPreview(text: string, maxLength = 240): string {
  if (text.length <= maxLength) return text
  return `${text.slice(0, Math.max(1, maxLength - 1))}…`
}

/** Canonical part ordering basis (database-design-v2.md 4.2). */
function compareParts(left: SessionPart, right: SessionPart): number {
  const byTime = left.created_at_ms - right.created_at_ms
  if (byTime !== 0) return byTime
  return left.part_id - right.part_id
}

function mapPartStatus(state: SessionPart['state']): MessagePart['status'] {
  return state
}

function partName(part: SessionPart): string | null {
  const content = part.content || null
  switch (part.kind) {
    case 'tool_call':
      return readString(content?.name) || readString(content?.plugin) || 'tool'
    case 'tool_result':
      return readString(content?.name) || 'result'
    case 'file_ref':
      return readString(content?.name) || readString(content?.path) || 'attachment'
    case 'paste_ref':
      return 'Pasted text'
    case 'skill_ref':
      return readString(content?.skill) || 'skill'
    case 'notice':
      return readString(content?.kind) || readString(content?.summary) || 'notice'
    case 'hook':
      return readString(content?.hook) || 'hook'
    case 'compaction':
      return 'Compaction'
    case 'error':
      return readString(content?.category) || 'error'
    case 'interaction':
      return readString(content?.type) || 'interaction'
    case 'run': {
      const runKind = readString(content?.run_kind)
      if (runKind) return runKind === 'user_send' ? 'Run' : runKind
      return 'Run'
    }
    case 'think':
      return 'Reasoning'
    default:
      return readString(content?.name) || readString(content?.title) || part.kind
  }
}

/**
 * Adapter from a v2 `SessionPart` (4.1.1 canonical shape) onto the legacy
 * `MessagePart` render shape so the existing block renderers keep working.
 * `content` is carried through unchanged — the canonical raw payload the AI
 * sees — and the renderers dispatch on `part.kind`.
 */
function sessionPartToMessagePart(part: SessionPart, messageId: string, partIndex: number): MessagePart {
  return {
    id: part.part_id,
    message_id: messageId,
    part_index: partIndex,
    status: mapPartStatus(part.state),
    kind: part.kind,
    name: partName(part),
    summary: part.summary ?? null,
    has_detail: true,
    operation_id: part.kind === 'tool_call' ? String(part.part_id) : null,
    created_at: new Date(part.created_at_ms).toISOString(),
    content: part.content || null,
    // The single-activity interaction surface: a tool_call awaiting a host
    // ask carries its request here so the render pipeline can treat it as a
    // pending interaction part without re-parsing `content.operation`.
    userInput: readSessionPartAwaitingUserInput(part),
  }
}

function canonicalRunMessage(
  run: SessionPart | null,
  content: SessionPart[],
  sessionId: number,
  key: number,
): MessageResource {
  const messageId = run ? `run:${run.part_id}` : `orphan:${key}`
  const sourceRole = run?.role ?? content.find((part) => part.role)?.role
  const role: MessageResource['role'] =
    sourceRole === 'user' ? 'user' : sourceRole === 'assistant' ? 'assistant' : 'system'
  const state: MessageResource['state'] = run ? run.state : 'completed'
  const createdAtMs = run ? run.created_at_ms : (content[0]?.created_at_ms ?? 0)
  const runUsage = run ? asRecord(run.content?.usage) : null

  const sortedContent = [...content].sort(compareParts)
  const parts: MessagePart[] = []
  if (run) {
    parts.push(sessionPartToMessagePart(run, messageId, parts.length))
  }
  for (const part of sortedContent) {
    parts.push(sessionPartToMessagePart(part, messageId, parts.length))
  }

  const metadata: Record<string, unknown> = {}
  if (run) {
    metadata.run_part_id = run.part_id
    metadata.run_kind = readString(run.content?.run_kind) || run.kind
    metadata.run_revision = run.revision ?? 1
  }

  return {
    id: messageId,
    session_id: sessionId,
    role,
    state,
    created_at: new Date(createdAtMs).toISOString(),
    updated_at: new Date(createdAtMs).toISOString(),
    metadata,
    usage: runUsage as Record<string, unknown> | null | undefined,
    part_count: parts.length,
    parts,
  }
}

/**
 * The v2 Web conversation view: a flat, run-marker-grouped projection of the
 * session's parts. Each run marker becomes one display message (the run is the
 * group header — its own first part), with its content parts beneath it ordered
 * by `(created_at_ms, part_id)`.
 */
export function partsToMessages(parts: SessionPart[], sessionId: number): MessageResource[] {
  const sorted = [...parts].sort(compareParts)

  const contentByRun = new Map<number, SessionPart[]>()
  const runMarkers: SessionPart[] = []
  const orphanParts: SessionPart[] = []

  for (const part of sorted) {
    if (part.kind === 'run') {
      runMarkers.push(part)
      continue
    }
    if (part.run_id != null) {
      let bucket = contentByRun.get(part.run_id)
      if (!bucket) {
        bucket = []
        contentByRun.set(part.run_id, bucket)
      }
      bucket.push(part)
    } else {
      orphanParts.push(part)
    }
  }

  const messages: MessageResource[] = []
  for (const run of runMarkers) {
    const content = contentByRun.get(run.part_id) ?? []
    contentByRun.delete(run.part_id)
    messages.push(canonicalRunMessage(run, content, sessionId, run.part_id))
  }
  // Content parts whose run marker is missing locally (streamed in first, or
  // the marker was removed): still render them under their run id.
  const unclaimedRunIds = [...contentByRun.keys()].sort((left, right) => left - right)
  for (const runId of unclaimedRunIds) {
    messages.push(canonicalRunMessage(null, contentByRun.get(runId) ?? [], sessionId, runId))
  }
  if (orphanParts.length) {
    messages.push(canonicalRunMessage(null, orphanParts, sessionId, 0))
  }
  return messages
}

function readReasoningText(content: Record<string, unknown> | null): string {
  if (!content) return ''
  if (Array.isArray(content.summary)) {
    return content.summary.filter((item): item is string => typeof item === 'string').join('\n')
  }
  if (Array.isArray(content.raw)) {
    return content.raw.filter((item): item is string => typeof item === 'string').join('\n')
  }
  return readString(content.summary) || ''
}

function toolCallInputText(content: Record<string, unknown> | null): string {
  if (!content) return ''
  const input = content.input
  if (input === undefined || input === null) return ''
  if (typeof input === 'string') return input
  try {
    return JSON.stringify(input, null, 2)
  } catch {
    return String(input)
  }
}

/** JSON value rendered as nested Markdown bullets — the human-facing
 * replacement for a raw tool-argument dump. Objects render one bullet per
 * field (`- **path**: `README.md``), multi-line strings as a fenced block,
 * scalar arrays as an inline code list, and nested objects/arrays as indented
 * sub-bullets. Mirrors the TUI's `json_value_to_markdown`. */
function jsonToMarkdown(value: unknown): string {
  if (isJsonScalar(value)) return jsonScalarText(value)
  if (Array.isArray(value)) {
    if (value.every(isJsonScalar)) {
      return value.map((item) => `\`${String(item).replaceAll('`', '\\`')}\``).join(', ')
    }
    return value
      .map((item) => jsonItemMarkdown(item, 0))
      .filter(Boolean)
      .join('\n')
  }
  if (value && typeof value === 'object') {
    return Object.entries(value as Record<string, unknown>)
      .map(([name, field]) => jsonFieldMarkdown(name, field, 0))
      .join('\n')
  }
  return String(value)
}

function isJsonScalar(value: unknown): boolean {
  return value === null || ['string', 'boolean', 'number'].includes(typeof value)
}

function jsonScalarText(value: unknown): string {
  if (value === null) return 'null'
  if (typeof value === 'string') return `\`${value.replaceAll('`', '\\`')}\``
  return String(value)
}

/** A named object field: `- **name**: …` with nested content indented one
 * level under the bullet (fenced block for multi-line strings, sub-bullets
 * for nested objects/arrays). */
function jsonFieldMarkdown(name: string, value: unknown, indent: number): string {
  const prefix = '  '.repeat(indent)
  if (isJsonScalar(value)) {
    return `${prefix}- **${name}**: ${jsonScalarText(value)}`
  }
  if (Array.isArray(value)) {
    if (value.every(isJsonScalar)) {
      return `${prefix}- **${name}**: ${value.map((item) => `\`${String(item).replaceAll('`', '\\`')}\``).join(', ')}`
    }
    const sub = value
      .map((item) => jsonItemMarkdown(item, indent + 1))
      .filter(Boolean)
      .join('\n')
    return `${prefix}- **${name}**:\n${sub}`
  }
  if (value && typeof value === 'object') {
    const sub = Object.entries(value as Record<string, unknown>)
      .map(([fieldName, field]) => jsonFieldMarkdown(fieldName, field, indent + 1))
      .join('\n')
    return `${prefix}- **${name}**:\n${sub}`
  }
  return `${prefix}- **${name}**: ${String(value)}`
}

/** A non-scalar array element: objects render their own bullets, other
 * containers recurse, multi-line strings become fenced blocks. */
function jsonItemMarkdown(value: unknown, indent: number): string {
  const prefix = '  '.repeat(indent)
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    return Object.entries(value as Record<string, unknown>)
      .map(([name, field]) => jsonFieldMarkdown(name, field, indent))
      .join('\n')
  }
  if (typeof value === 'string' && value.includes('\n')) {
    return `${prefix}\`\`\`text\n${value}\n${prefix}\`\`\``
  }
  if (Array.isArray(value)) {
    return jsonFieldMarkdown('', value, indent)
  }
  return `${prefix}${jsonScalarText(value)}`
}

/** The tool arguments as nested Markdown bullets, preferring the flattened
 * `content.input` (plain JSON) and falling back to the stored operation's
 * invocation input when it is already a plain object. */
function toolCallInputMarkdown(content: Record<string, unknown> | null): string {
  if (!content) return ''
  const input = content.input
  if (input !== undefined && input !== null) {
    const markdown = jsonToMarkdown(input)
    if (markdown.trim().length > 0) return markdown
  }
  const operation = asRecord(content.operation)
  const invocation = asRecord(operation?.invocation)
  const invocationInput = invocation?.input
  if (invocationInput !== undefined && invocationInput !== null && typeof invocationInput === 'object') {
    const markdown = jsonToMarkdown(invocationInput)
    if (markdown.trim().length > 0) return markdown
  }
  return ''
}

function interactionStatusLabel(part: MessagePart): string {
  if (part.status === 'pending' || part.status === 'in_progress') return 'Waiting for input'
  if (part.status === 'completed') return 'Answered'
  return 'Interaction'
}

export function partBody(part: MessagePart): string {
  const content = part.content || null
  if (!content) return part.summary || ''

  const type = typeof content.type === 'string' ? content.type : ''

  if (part.kind === 'think' || type === 'think') {
    return readReasoningText(content)
  }

  if (part.kind === 'tool_call' || type === 'tool_call') {
    return toolCallInputText(content)
  }

  if (part.kind === 'tool_result' || type === 'tool_result') {
    const output = readString(content.output)
    if (output) return output
    return part.summary || 'Tool result'
  }

  if (part.kind === 'file_ref' || type === 'file_ref') {
    return readString(content.path) || readString(content.name) || 'File reference'
  }

  if (part.kind === 'paste_ref' || type === 'paste_ref') {
    return readString(content.text) || 'Pasted text'
  }

  if (part.kind === 'skill_ref' || type === 'skill_ref') {
    const args = content.args
    if (args !== undefined && args !== null) {
      try {
        return JSON.stringify(args)
      } catch {
        return String(args)
      }
    }
    return readString(content.skill) || 'Skill reference'
  }

  if (part.kind === 'notice' || type === 'notice') {
    return readString(content.detail) || readString(content.summary) || part.summary || 'Notice'
  }

  if (part.kind === 'hook' || type === 'hook') {
    return (
      readString(content.message) ||
      readString(content.detail) ||
      readString(content.summary) ||
      part.summary ||
      'Hook activity'
    )
  }

  if (part.kind === 'system_notification' || type === 'system_notification') {
    return (
      readString(content.body) ||
      readString(content.detail) ||
      readString(content.summary) ||
      part.summary ||
      'Background notification'
    )
  }

  if (part.kind === 'compaction' || type === 'compaction') {
    return readString(content.summary) || part.summary || 'Session compacted'
  }

  if (part.kind === 'error' || type === 'error') {
    const category = readString(content.category) || 'error'
    const message = readString(content.message) || ''
    return message ? `${category}: ${message}` : category
  }

  if (part.kind === 'interaction' || type === 'interaction') {
    return readString(content.prompt) || part.summary || interactionStatusLabel(part)
  }

  if (type === 'text' && typeof content.text === 'string') {
    return content.text
  }

  if (type === 'reasoning' && Array.isArray(content.summary)) {
    const summary = content.summary.filter((item): item is string => typeof item === 'string').join('\n')
    if (summary) return summary
  }

  if (type === 'operation') {
    const modelOutput = asRecord(content.model_output)
    const output = readString(modelOutput?.text)
    if (output) return output

    const error = asRecord(content.error)
    const errorMessage = readString(error?.message)
    if (errorMessage) return errorMessage

    const title = readString(content.title)
    if (title) return title
  }

  if (type === 'request') {
    const requestType = readString(content.request_type) || 'request'
    return part.summary || requestType
  }

  if (type === 'error') {
    const code = typeof content.code === 'string' ? content.code : 'error'
    const message = typeof content.message === 'string' ? content.message : ''
    return `${code}: ${message}`.trim()
  }

  return part.summary || JSON.stringify(content, null, 2)
}

export function applyPatchPayload(content: Record<string, unknown> | null): Record<string, unknown> | null {
  if (!content || content.type !== 'operation') return null

  const structured = asRecord(content.structured)
  if (structured && (Array.isArray(structured.changes) || typeof structured.diff === 'string')) return structured

  const details = asRecord(content.details)
  const payload = asRecord(details?.payload)
  if (payload && (Array.isArray(payload.changes) || typeof payload.diff === 'string')) return payload

  return null
}

export function applyPatchDiffSummary(payload: Record<string, unknown>): string {
  const changes = Array.isArray(payload.changes) ? payload.changes : []
  if (!changes.length) return 'Patch diff'
  return `Patch diff (${changes.length} file${changes.length === 1 ? '' : 's'})`
}

export function partBlocks(part: MessagePart): RenderBlock[] {
  const content = part.content || null

  if (part.kind === 'run') {
    const runKind = readString(content?.run_kind) || 'run'
    const abortReason = readString(content?.abort_reason)
    const title = runKind === 'user_send' ? 'User run' : runKind === 'continue' ? 'Continued run' : runKind
    return [
      {
        title,
        body: abortReason ? `abort_reason: ${abortReason}` : `state: ${part.status}`,
        kind: 'input_activity',
        activityLabel: 'Run',
        summary: part.summary || undefined,
      },
    ]
  }

  if (part.kind === 'think' || content?.type === 'think') {
    const body = partBody(part)
    return body.trim().length > 0 ? [{ body, kind: 'markdown', title: 'Reasoning' }] : []
  }

  if (part.kind === 'tool_call' || content?.type === 'tool_call') {
    // A tool_call awaiting a host ask is the pending-interaction surface, not
    // a tool block: it renders as the foldable inline form instead.
    if (part.userInput) return []
    const toolName = readString(content?.name) || readString(content?.plugin) || 'tool'
    // Denied or unavailable calls show the readable outcome (who blocked it /
    // why) instead of a raw input dump.
    if (
      part.status === 'policy_denied' ||
      part.status === 'user_declined' ||
      part.status === 'capability_unavailable' ||
      part.status === 'tool_unavailable'
    ) {
      const outcome = toolCallOutcomeBlock(part, content)
      if (outcome) return [outcome]
    }
    const operation = asRecord(content?.operation)
    const input = toolCallInputMarkdown(content)
    const blocks: RenderBlock[] = [
      {
        title: toolName,
        body: input || 'Tool call',
        kind: 'markdown',
        summary: part.status,
      },
    ]
    // The stored operation's result (v1 `OperationPart` JSON) carries the
    // output the tool produced: `model_preview.text` first, then the rich
    // blocks — deduplicated so the same content is never shown twice.
    const result = asRecord(operation?.result)
    if (result) {
      const output = readString((asRecord(result.model_preview)?.text as string) ?? '')
      if (output) blocks.push({ body: output, kind: 'markdown' })
      blocks.push(...storedOperationBlocks(result, output))
    }
    return blocks
  }

  if (part.kind === 'tool_result' || content?.type === 'tool_result') {
    const output = readString(content?.output) || partBody(part)
    const ok = content?.ok !== false
    return output.trim().length > 0
      ? [{ body: output, kind: 'terminal', language: 'text', title: ok ? 'Result' : 'Tool failed' }]
      : []
  }

  if (part.kind === 'file_ref' || content?.type === 'file_ref') {
    const name = readString(content?.name) || readString(content?.path) || 'file'
    const mime = readString(content?.mime)
    const sha = readString(content?.sha)
    const summary = [mime, sha ? sha.slice(0, 12) : null].filter((value): value is string => Boolean(value)).join(' · ')
    return [
      {
        title: name,
        body: readString(content?.path) || 'File reference',
        kind: 'input_activity' as const,
        activityLabel: 'Attachment',
        summary: summary || undefined,
      },
    ]
  }

  if (part.kind === 'paste_ref' || content?.type === 'paste_ref') {
    const text = readString(content?.text) || ''
    return [
      {
        title: 'Pasted text',
        body: text ? textArtifactPreview(text) : 'Pasted text was attached to this message.',
        kind: 'input_activity' as const,
        activityLabel: 'Pasted text',
      },
    ]
  }

  if (part.kind === 'skill_ref' || content?.type === 'skill_ref') {
    const skill = readString(content?.skill) || part.summary?.replace(/^Skill:\s*/i, '') || 'Skill'
    const args = content?.args
    const argsText = args === undefined || args === null ? '' : safeStringify(args)
    return [
      {
        title: skill,
        body: argsText ? argsText : 'User-selected Skill instructions were attached to this message.',
        kind: 'input_activity' as const,
        activityLabel: 'Skill',
      },
    ]
  }

  if (part.kind === 'notice' || content?.type === 'notice') {
    return [
      {
        title: readString(content?.kind) || 'Notice',
        body: readString(content?.detail) || readString(content?.summary) || part.summary || 'Notice',
        kind: 'input_activity' as const,
        activityLabel: 'Notice',
      },
    ]
  }

  if (part.kind === 'hook' || content?.type === 'hook') {
    return [
      {
        title: readString(content?.hook) || 'Hook',
        body:
          readString(content?.message) ||
          readString(content?.detail) ||
          readString(content?.summary) ||
          part.summary ||
          'Hook activity',
        kind: 'input_activity' as const,
        activityLabel: 'Hook',
      },
    ]
  }

  if (part.kind === 'system_notification' || content?.type === 'system_notification') {
    const operationKind = readString(content?.operation_kind) || 'background'
    const status = readString(content?.status)
    const summary = readString(content?.summary) || part.summary || 'Background notification'
    return [
      {
        title: status ? `${operationKind} · ${status}` : operationKind,
        body: readString(content?.body) || readString(content?.detail) || summary,
        summary,
        kind: 'input_activity' as const,
        activityLabel: 'Hook',
      },
    ]
  }

  if (part.kind === 'compaction' || content?.type === 'compaction') {
    return [
      {
        title: 'Compaction',
        body: readString(content?.summary) || part.summary || 'Session was compacted.',
        kind: 'input_activity' as const,
        activityLabel: 'Compaction',
      },
    ]
  }

  if (part.kind === 'error' || content?.type === 'error') {
    const category = readString(content?.category) || 'error'
    const message = readString(content?.message) || part.summary || 'The session reported an error.'
    return [
      {
        title: category,
        body: message,
        kind: 'input_activity' as const,
        activityLabel: 'Error',
      },
    ]
  }

  if (part.kind === 'interaction' || content?.type === 'interaction') {
    const type = readString(content?.type) || 'ask_user'
    const prompt = readString(content?.prompt) || part.summary || ''
    const waiting = part.status === 'pending' || part.status === 'in_progress'
    const blocks: RenderBlock[] = [
      {
        title: prompt || type,
        body: waiting ? 'Waiting for a user response.' : partBody(part) || 'Interaction resolved.',
        kind: 'input_activity' as const,
        activityLabel: waiting ? 'Waiting' : interactionStatusLabel(part),
      },
    ]
    return blocks
  }

  if (
    content?.type === 'operation' &&
    (part.status === 'policy_denied' ||
      part.status === 'user_declined' ||
      part.status === 'capability_unavailable' ||
      part.status === 'tool_unavailable')
  ) {
    const details = asRecord(content.details)
    const payload = asRecord(details?.payload)
    const denial = asRecord(payload?.denial)
    const unavailable = asRecord(payload?.unavailable)
    const source = readString(denial?.source) || readString(unavailable?.source)
    const scope = readString(denial?.scope)
    const ruleId = readFiniteNumber(denial?.rule_id)
    const provenance = [source, scope, ruleId == null ? null : `rule #${ruleId}`]
      .filter((value): value is string => Boolean(value))
      .join(' · ')
    const title = {
      policy_denied: 'Blocked by permission policy',
      user_declined: 'Declined by user',
      capability_unavailable: 'Capability unavailable',
      tool_unavailable: 'Tool unavailable',
    }[part.status]
    return [
      {
        body: partBody(part),
        kind: 'operation_outcome',
        outcome: part.status,
        title,
        summary: provenance || undefined,
      },
    ]
  }
  if (part.kind === 'skill_reference' || content?.type === 'skill_reference') {
    const skills = Array.isArray(content?.skills) ? content.skills : []
    if (skills.length) {
      return skills.flatMap((value) => {
        const skill = asRecord(value)
        const name = readString(skill?.name)
        if (!name) return []
        return [
          {
            title: name,
            body:
              readString(skill?.instructions) || readString(skill?.description) || 'User-selected Skill instructions',
            kind: 'input_activity' as const,
            activityLabel: 'Skill',
            summary: readString(skill?.source) || undefined,
          },
        ]
      })
    }
    const summary = part.summary || 'Skill reference'
    return [
      {
        title: summary.replace(/^Skill:\s*/i, '') || 'Skill',
        body: 'User-selected Skill instructions were attached to this message.',
        kind: 'input_activity',
        activityLabel: 'Skill',
      },
    ]
  }
  if (part.kind === 'attachment' || content?.type === 'attachment') {
    const attachments = Array.isArray(content?.attachments) ? content.attachments : []
    return attachments.flatMap((value) => {
      const attachment = asRecord(value)
      const name = readString(attachment?.title) || readString(attachment?.filename) || readString(attachment?.mime)
      if (!name) return []
      const kind = readString(attachment?.kind) || 'file'
      const size = readFiniteNumber(attachment?.size_bytes)
      return [
        {
          title: name,
          body: `${kind}${size == null ? '' : ` · ${size} bytes`}`,
          kind: 'input_activity' as const,
          activityLabel: 'Attachment',
          summary: readString(attachment?.mime) || undefined,
        },
      ]
    })
  }
  if (part.kind === 'text_artifact' || content?.type === 'text_artifact') {
    const label = readString(content?.label) || 'Pasted text'
    const text = readString(content?.text)
    return [
      {
        title: label,
        body: text ? textArtifactPreview(text) : 'Pasted text was attached to this message.',
        kind: 'input_activity' as const,
        activityLabel: 'Pasted text',
      },
    ]
  }
  const applyPatch = applyPatchPayload(content)
  if (applyPatch) {
    const modelOutput = content ? asRecord(content.model_output) : null
    const output = readString(modelOutput?.text)
    const diff = readString(applyPatch.diff)
    const blocks: RenderBlock[] = []
    if (output) {
      blocks.push({ body: output, kind: 'markdown' })
    }
    if (diff) {
      blocks.push({
        body: diff,
        kind: 'diff',
        summary: applyPatchDiffSummary(applyPatch),
      })
    }
    const changes = Array.isArray(applyPatch.changes) ? applyPatch.changes : []
    if (!diff && changes.length) {
      blocks.push({
        body: changes
          .map((value) => {
            const change = asRecord(value)
            const path = readString(change?.path) || 'unknown path'
            const kind = readString(change?.kind) || 'updated'
            return `- **${kind}** \`${path}\``
          })
          .join('\n'),
        kind: 'markdown',
        summary: applyPatchDiffSummary(applyPatch),
      })
    }
    if (blocks.length) return blocks
  }

  const operationBlocks = operationRenderBlocks(content)
  if (operationBlocks.length) return operationBlocks

  const body = partBody(part)
  return body.trim().length > 0 ? [{ body, kind: 'markdown' }] : []
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function operationRenderBlocks(content: Record<string, unknown> | null): RenderBlock[] {
  if (!content || content.type !== 'operation') return []
  const blocks = Array.isArray(content.blocks) ? content.blocks : []
  const operationTitle = readString(content.title)
  const rendered: RenderBlock[] = []

  for (const item of blocks) {
    const block = asRecord(item)
    if (!block) continue
    const blockType = readString(block.type)
    if (blockType === 'text' || blockType === 'markdown') {
      const body = readString(block.text)
      if (body) rendered.push({ body, kind: 'markdown', title: operationTitle || undefined })
      continue
    }
    if (blockType === 'log') {
      const body = readString(block.text)
      if (body) rendered.push({ body, kind: 'terminal', language: 'text', title: operationTitle || undefined })
      continue
    }
    if (blockType === 'diff') {
      const body = readString(block.diff)
      if (body) rendered.push({ body, kind: 'diff', summary: 'Diff', language: readString(block.language) || 'diff' })
      continue
    }
    if (blockType === 'command') {
      const command = readString(block.command)
      const stdout = readString(block.stdout)
      const stderr = readString(block.stderr)
      const cwd = readString(block.cwd)
      const exitCode = readFiniteNumber(block.exit_code)
      const commandSummary = [cwd ? `cwd ${cwd}` : '', exitCode == null ? '' : `exit ${exitCode}`]
        .filter(Boolean)
        .join(' · ')
      if (command) {
        rendered.push({
          body: `$ ${command}`,
          kind: 'terminal',
          language: 'shell',
          title: operationTitle || 'Command',
          summary: commandSummary || undefined,
        })
      }
      if (stdout) rendered.push({ body: stdout, kind: 'terminal', language: 'text', title: 'stdout' })
      if (stderr) rendered.push({ body: stderr, kind: 'terminal', language: 'text', title: 'stderr' })
      continue
    }
    if (blockType === 'file_changes') {
      const changes = Array.isArray(block.changes) ? block.changes : []
      if (changes.length) {
        const lines = changes.map((value) => {
          const change = asRecord(value)
          const path = readString(change?.path) || 'unknown path'
          const kind = readString(change?.kind) || 'updated'
          const from = readString(change?.from_path)
          return `- **${kind}** \`${from ? `${from} → ` : ''}${path}\``
        })
        rendered.push({ body: lines.join('\n'), kind: 'markdown', title: 'File changes' })
      }
      continue
    }
    if (blockType === 'checklist') {
      const items = Array.isArray(block.items) ? block.items : []
      const lines = items
        .map((value) => asRecord(value))
        .filter((value): value is Record<string, unknown> => Boolean(value))
        .map((value) => readString(value.content))
        .filter((value): value is string => Boolean(value))
      if (lines.length) rendered.push({ body: lines.map((line) => `- ${line}`).join('\n'), kind: 'markdown' })
      continue
    }
    if (blockType === 'json') {
      if (block.value !== undefined) {
        rendered.push({ body: JSON.stringify(block.value, null, 2), kind: 'terminal', language: 'json' })
      }
      continue
    }
    if (blockType === 'table') {
      const columns = Array.isArray(block.columns) ? block.columns : []
      const rows = Array.isArray(block.rows) ? block.rows : []
      const labels = columns.map((value) => {
        const column = asRecord(value)
        return readString(column?.label) || readString(column?.key) || ''
      })
      if (labels.length) {
        const separator = labels.map(() => '---')
        const tableRows = rows.map((row) =>
          Array.isArray(row) ? `| ${row.map((value) => String(value ?? '')).join(' | ')} |` : '| |',
        )
        rendered.push({
          body: [`| ${labels.join(' | ')} |`, `| ${separator.join(' | ')} |`, ...tableRows].join('\n'),
          kind: 'markdown',
        })
      }
    }
  }

  return rendered
}

/**
 * The rich output blocks of a stored operation (`content.operation.result`),
 * rendered from the runtime `ViewBlock` JSON the tool produced. Text/Markdown
 * blocks whose body duplicates the model preview already shown are skipped so
 * the same content never renders twice. Mirrors the TUI's `render_operation_blocks`.
 */
function storedOperationBlocks(result: Record<string, unknown>, outputText: string | null): RenderBlock[] {
  const blocks = Array.isArray(result.content) ? result.content : []
  const rendered: RenderBlock[] = []
  for (const item of blocks) {
    const block = asRecord(item)
    if (!block) continue
    const blockType = readString(block.type)
    if (blockType === 'text' || blockType === 'markdown') {
      const body = readString(block.text)
      if (body && outputText !== body) rendered.push({ body, kind: 'markdown' })
      continue
    }
    if (blockType === 'json') {
      if (block.value !== undefined) {
        rendered.push({ body: JSON.stringify(block.value, null, 2), kind: 'terminal', language: 'json' })
      }
      continue
    }
    if (blockType === 'table') {
      // Runtime tables carry string columns; API tables carry {key,label}.
      const columns = Array.isArray(block.columns) ? block.columns : []
      const labels = columns.map((value) =>
        typeof value === 'string'
          ? value
          : readString(asRecord(value)?.label) || readString(asRecord(value)?.key) || '',
      )
      const rows = Array.isArray(block.rows) ? block.rows : []
      if (labels.length) {
        const separator = labels.map(() => '---')
        const tableRows = rows.map((row) =>
          Array.isArray(row) ? `| ${row.map((value) => String(value ?? '')).join(' | ')} |` : '| |',
        )
        rendered.push({
          body: [`| ${labels.join(' | ')} |`, `| ${separator.join(' | ')} |`, ...tableRows].join('\n'),
          kind: 'markdown',
        })
      }
      continue
    }
    if (blockType === 'log') {
      const body = readString(block.text)
      if (body)
        rendered.push({ body, kind: 'terminal', language: 'text', title: readString(block.stream) || undefined })
      continue
    }
    if (blockType === 'command') {
      const command = readString(block.command)
      const stdout = readString(block.stdout)
      const stderr = readString(block.stderr)
      if (command) rendered.push({ body: `$ ${command}`, kind: 'terminal', language: 'shell', title: 'Command' })
      if (stdout) rendered.push({ body: stdout, kind: 'terminal', language: 'text', title: 'stdout' })
      if (stderr) rendered.push({ body: stderr, kind: 'terminal', language: 'text', title: 'stderr' })
      continue
    }
    if (blockType === 'diff') {
      const body = readString(block.diff)
      if (body) rendered.push({ body, kind: 'diff', summary: 'Diff', language: readString(block.language) || 'diff' })
      continue
    }
    if (blockType === 'file_changes') {
      const changes = Array.isArray(block.changes) ? block.changes : []
      if (changes.length) {
        const lines = changes.map((value) => {
          const change = asRecord(value)
          const path = readString(change?.path) || 'unknown path'
          const kind = readString(change?.kind) || 'updated'
          const from = readString(change?.from_path)
          return `- **${kind}** \`${from ? `${from} → ` : ''}${path}\``
        })
        rendered.push({ body: lines.join('\n'), kind: 'markdown', title: 'File changes' })
      }
      continue
    }
    if (blockType === 'search_results') {
      const items = Array.isArray(block.items) ? block.items : []
      if (items.length) {
        const lines = items.map((value) => {
          const item = asRecord(value)
          const title = readString(item?.title)
          const url = readString(item?.url)
          const snippet = readString(item?.snippet)
          const heading = title ? (url ? `- [${title}](${url})` : `- ${title}`) : url ? `- ${url}` : ''
          return [heading, snippet ? `  ${snippet}` : ''].filter(Boolean).join('\n')
        })
        rendered.push({ body: lines.filter(Boolean).join('\n'), kind: 'markdown', title: 'Search results' })
      }
      continue
    }
    if (blockType === 'custom') {
      // Plugin payloads: the `presentation` map reads best as bullets.
      const presentation = asRecord(block.presentation)
      if (presentation && Object.keys(presentation).length) {
        const body = Object.entries(presentation)
          .map(([name, field]) => `- **${name}**: \`${String(field)}\``)
          .join('\n')
        rendered.push({ body, kind: 'markdown' })
        continue
      }
      if (block.value !== undefined) {
        rendered.push({ body: JSON.stringify(block.value, null, 2), kind: 'terminal', language: 'json' })
      }
      continue
    }
  }
  return rendered
}

/** A denied or unavailable tool call renders as the readable outcome: the
 * authorization reply reason or the failure message, with a status-specific
 * title — instead of a raw input dump. */
function toolCallOutcomeBlock(part: MessagePart, content: Record<string, unknown> | null): RenderBlock | null {
  const operation = asRecord(content?.operation)
  const outcomeTitle: { status: NonNullable<RenderBlock['outcome']>; title: string } | null =
    part.status === 'policy_denied'
      ? { status: 'policy_denied', title: 'Blocked by permission policy' }
      : part.status === 'user_declined'
        ? { status: 'user_declined', title: 'Declined by user' }
        : part.status === 'capability_unavailable'
          ? { status: 'capability_unavailable', title: 'Capability unavailable' }
          : part.status === 'tool_unavailable'
            ? { status: 'tool_unavailable', title: 'Tool unavailable' }
            : null
  if (!outcomeTitle) return null
  const { status, title } = outcomeTitle

  // Denial provenance: the permission reply that refused the call.
  const authorization = asRecord(operation?.authorization)
  const permissions = Array.isArray(authorization?.permissions) ? authorization.permissions : []
  const denial = permissions
    .map((value) => asRecord(value))
    .filter((value): value is Record<string, unknown> => Boolean(value))
    .flatMap((permission) => {
      const reply = asRecord(permission.reply)
      if (!reply) return []
      const kind = readString(reply.kind)
      if (kind !== 'deny_once' && kind !== 'deny_always') return []
      const reason = readString(reply.reason)
      const request = asRecord(permission.request)
      const source = readString(request?.source)
      const provenance = [reason, source ? `by ${source}` : null].filter(Boolean).join(' · ')
      return [`${provenance || 'Denied'}${kind === 'deny_always' ? ' (always)' : ''}`]
    })[0]
  if (denial) {
    return { body: denial, kind: 'operation_outcome', outcome: status, title }
  }

  // Fall back to the failure's user-facing message (e.g. tool_unavailable).
  const error = asRecord(operation?.result) ? asRecord(asRecord(operation?.result)?.error) : null
  const failure = asRecord(error?.failure)
  const fallback = readString((failure?.user as Record<string, unknown> | null)?.fallback as string | undefined)
  if (fallback) {
    return { body: fallback, kind: 'operation_outcome', outcome: status, title }
  }

  return null
}

export function messageBlocks(message: MessageResource): RenderBlock[] {
  const parts = Array.isArray(message.parts)
    ? [...message.parts].sort((left, right) => left.part_index - right.part_index)
    : []
  if (!parts.length) return []
  // Pending interaction parts are the interactive surface ("everything is a
  // part"): they render as foldable inline forms via the messages panel, so
  // their flat "Waiting for input" block is skipped here to avoid showing
  // both a summary and the live form for the same part.
  return parts.flatMap((part) => (isPendingInteractionPart(part) ? [] : partBlocks(part)))
}

/**
 * The interaction parts of a message that are still awaiting a reply. These
 * are the "everything is a part" surface: each pending interaction part
 * renders as a foldable inline form (plan body + questions) inside the
 * message, instead of a separate card. Part identity is preserved (unlike
 * `messageBlocks`, which flattens), so the form can drive its own submit /
 * cancel lifecycle per `request_id`.
 */
export function pendingInteractionParts(message: MessageResource): MessagePart[] {
  const parts = Array.isArray(message.parts) ? message.parts : []
  return parts.filter(isPendingInteractionPart)
}

function isPendingInteractionPart(part: MessagePart): boolean {
  if (part.status !== 'pending' && part.status !== 'in_progress') return false
  // The single-activity shape: a `tool_call` operation with an unanswered
  // `user_input` record IS the interaction surface (one ask = one activity).
  if (part.userInput) return true
  const content = part.content || null
  return part.kind === 'interaction' || content?.type === 'interaction'
}

export function rewindMessageComposerText(message: MessageResource): string {
  const parts = Array.isArray(message.parts) ? message.parts : []
  return [...parts]
    .sort((left, right) => left.part_index - right.part_index)
    .flatMap((part) => {
      const content = part.content || null
      if (!content || content.type !== 'text' || typeof content.text !== 'string') return []
      if (content.synthetic === true || content.ignored === true || !content.text.trim()) return []
      return [content.text]
    })
    .join('\n\n')
}

export function messageUsageFacts(message: MessageResource): string[] {
  const usage = message.usage as Record<string, unknown> | null | undefined
  if (!usage) return []

  const facts: string[] = []
  const pushFact = (label: string, key: string) => {
    const value = usage[key]
    if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
      facts.push(`${label} ${formatUsageCount(value)}`)
    }
  }

  pushFact('in', 'input_tokens')
  pushFact('out', 'output_tokens')
  pushFact('reasoning', 'reasoning_tokens')

  const totalCost = usage.total_cost
  if (typeof totalCost === 'number' && Number.isFinite(totalCost) && totalCost > 0) {
    facts.push(`cost ${formatUsageUsd(totalCost)}`)
  }

  return facts
}

export function readPayloadMessageId(payload: Record<string, unknown>): number | null {
  return readFiniteNumber(payload.message_id)
}

export function readPayloadPartId(payload: Record<string, unknown>): number | null {
  return readFiniteNumber(payload.part_id)
}
