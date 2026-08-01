import type {
  MessagePart,
  MessageResource,
  TranscriptActivity,
  TranscriptContentNode,
  TranscriptSnapshot,
} from '@/agena/lib/agenaApi'

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

function activityTitle(activity: TranscriptActivity): string {
  const payload = activity.payload
  return (
    readString(payload.title) ||
    readString(payload.name) ||
    readString(payload.skill_name) ||
    readString(payload.query) ||
    payload.activity_type
  )
}

function activitySummary(activity: TranscriptActivity): string {
  const payload = activity.payload
  const problemUser = asRecord(asRecord(payload.problem)?.user)
  return readString(payload.summary) || readString(problemUser?.fallback) || ''
}

function canonicalPart(
  node: TranscriptContentNode,
  messageId: string,
  partIndex: number,
  messageStatus: MessageResource['state'],
  createdAtMs: number,
): MessagePart {
  if (node.type === 'text') {
    return {
      id: node.segment.id,
      message_id: messageId,
      part_index: partIndex,
      status: messageStatus === 'pending' ? 'in_progress' : messageStatus,
      kind: 'text',
      has_detail: true,
      created_at: new Date(createdAtMs).toISOString(),
      content: { type: 'text', text: node.segment.text, synthetic: false },
    }
  }

  const activity = node.activity
  const activityType = activity.payload.activity_type
  const content: Record<string, unknown> = {
    ...activity.payload,
    type: activityType === 'interaction' ? 'request' : activityType,
    actor: activity.actor,
    state: activity.state,
    lifecycle: activity.lifecycle,
    provenance: activity.provenance || {},
  }
  if (activityType === 'resource') {
    content.type = 'attachment'
    content.attachments = [
      {
        kind: activity.payload.kind,
        title: activity.payload.name,
        mime: activity.payload.media_type,
        size_bytes: activity.payload.size_bytes,
        reference: activity.payload.reference,
      },
    ]
  } else if (activityType === 'operation') {
    content.model_output = { text: activity.payload.model_output_text || '' }
    const problem = asRecord(asRecord(activity.payload.error)?.problem)
    const user = asRecord(problem?.user)
    if (problem) content.error = { message: readString(user?.fallback) || readString(user?.detail) || 'Tool failed' }
  } else if (activityType === 'error') {
    const problem = asRecord(activity.payload.problem)
    const user = asRecord(problem?.user)
    content.code = readString(problem?.code) || 'error'
    content.message = readString(user?.fallback) || readString(user?.detail) || 'The reply failed.'
  }
  return {
    id: activity.id,
    message_id: messageId,
    part_index: partIndex,
    status: activity.state,
    kind: activityType,
    name: activityTitle(activity),
    summary: activitySummary(activity),
    has_detail: true,
    operation_id: activityType === 'operation' ? String(activity.payload.call_id || activity.id) : null,
    created_at: new Date(activity.lifecycle.started_at_ms).toISOString(),
    content,
  }
}

function canonicalMessage(input: {
  id: string
  sessionId: number
  role: 'user' | 'assistant'
  state: MessageResource['state']
  createdAtMs: number
  updatedAtMs: number
  nodes: TranscriptContentNode[]
  metadata: Record<string, unknown>
}): MessageResource {
  const nodes = [...input.nodes].sort((left, right) => {
    const leftPosition = left.type === 'text' ? left.segment.position.index : left.activity.position.index
    const rightPosition = right.type === 'text' ? right.segment.position.index : right.activity.position.index
    return leftPosition - rightPosition
  })
  return {
    id: input.id,
    session_id: input.sessionId,
    role: input.role,
    state: input.state,
    created_at: new Date(input.createdAtMs).toISOString(),
    updated_at: new Date(input.updatedAtMs).toISOString(),
    metadata: input.metadata,
    usage: null,
    part_count: nodes.length,
    parts: nodes.map((node, index) => canonicalPart(node, input.id, index, input.state, input.createdAtMs)),
  }
}

/**
 * The Web conversation view is a one-to-one adapter over the canonical
 * Turn/AssistantReply aggregate. It never scans or merges provider model
 * messages.
 */
export function transcriptMessages(transcript: TranscriptSnapshot): MessageResource[] {
  return [...transcript.turns]
    .sort((left, right) => left.sequence - right.sequence)
    .flatMap((turn) => {
      const userId = `turn:${turn.id}:input`
      const replyId = `reply:${turn.reply.id}`
      return [
        canonicalMessage({
          id: userId,
          sessionId: turn.session_id,
          role: 'user',
          state: 'completed',
          createdAtMs: turn.created_at_ms,
          updatedAtMs: turn.created_at_ms,
          nodes: turn.input,
          metadata: { canonical_turn_id: turn.id, turn_sequence: turn.sequence },
        }),
        canonicalMessage({
          id: replyId,
          sessionId: turn.session_id,
          role: 'assistant',
          state: turn.reply.status,
          createdAtMs: turn.reply.created_at_ms,
          updatedAtMs: turn.reply.finished_at_ms ?? turn.reply.created_at_ms,
          nodes: turn.reply.content,
          metadata: {
            canonical_turn_id: turn.id,
            canonical_reply_id: turn.reply.id,
            turn_sequence: turn.sequence,
          },
        }),
      ]
    })
}

export function partBody(part: MessagePart): string {
  const content = part.content || null
  if (!content) return part.summary || ''

  const type = typeof content.type === 'string' ? content.type : ''
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

export function messageBlocks(message: MessageResource): RenderBlock[] {
  const parts = Array.isArray(message.parts)
    ? [...message.parts].sort((left, right) => left.part_index - right.part_index)
    : []
  if (!parts.length) return []
  const blocks = parts.flatMap((part) => partBlocks(part))
  return blocks
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
