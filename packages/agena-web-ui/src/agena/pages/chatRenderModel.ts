import type { MessagePart, MessageResource } from '@/agena/lib/agenaApi'

import { formatUsageCount, formatUsageUsd } from './chatUsageModel'

export type RenderBlock = {
  body: string
  kind: 'text' | 'diff' | 'input_activity'
  activityLabel?: string
  summary?: string
  title?: string
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
      blocks.push({ body: output, kind: 'text' })
    }
    if (diff) {
      blocks.push({
        body: diff,
        kind: 'diff',
        summary: applyPatchDiffSummary(applyPatch),
      })
    }
    if (blocks.length) return blocks
  }

  const operationBlocks = operationRenderBlocks(content)
  if (operationBlocks.length) return operationBlocks

  const body = partBody(part)
  return body.trim().length > 0 ? [{ body, kind: 'text' }] : []
}

function operationRenderBlocks(content: Record<string, unknown> | null): RenderBlock[] {
  if (!content || content.type !== 'operation') return []
  const blocks = Array.isArray(content.blocks) ? content.blocks : []
  const rendered: RenderBlock[] = []

  for (const item of blocks) {
    const block = asRecord(item)
    if (!block) continue
    const blockType = readString(block.type)
    if (blockType === 'text' || blockType === 'markdown' || blockType === 'log') {
      const body = readString(block.text)
      if (body) rendered.push({ body, kind: 'text' })
      continue
    }
    if (blockType === 'diff') {
      const body = readString(block.diff)
      if (body) rendered.push({ body, kind: 'diff', summary: 'Diff' })
      continue
    }
    if (blockType === 'command') {
      const command = readString(block.command)
      const stdout = readString(block.stdout)
      const stderr = readString(block.stderr)
      const body = [command ? `$ ${command}` : '', stdout, stderr]
        .filter((value): value is string => Boolean(value))
        .join('\n\n')
      if (body) rendered.push({ body, kind: 'text' })
      continue
    }
    if (blockType === 'file_changes') {
      const changes = Array.isArray(block.changes) ? block.changes : []
      if (changes.length)
        rendered.push({ body: `${changes.length} file change${changes.length === 1 ? '' : 's'}`, kind: 'text' })
      continue
    }
    if (blockType === 'checklist') {
      const items = Array.isArray(block.items) ? block.items : []
      const lines = items
        .map((value) => asRecord(value))
        .filter((value): value is Record<string, unknown> => Boolean(value))
        .map((value) => readString(value.content))
        .filter((value): value is string => Boolean(value))
      if (lines.length) rendered.push({ body: lines.map((line) => `- ${line}`).join('\n'), kind: 'text' })
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
