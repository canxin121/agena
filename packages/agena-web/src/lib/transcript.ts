import type { JsonObject as PartRecord, JsonValue } from '@/types/json'
import { formatDateTimeYMDHM } from '@/i18n/intl'

export type TranscriptOptions = {
  thinking: boolean
  toolDetails: boolean
  assistantMetadata: boolean
}

export type TranscriptSession = {
  id: string
  title?: string
  time?: { created?: number; updated?: number }
}

export type TranscriptMessage = {
  info: {
    role?: string
    modelID?: string
    time?: { created?: number; completed?: number }
  }
  parts: JsonValue[]
}

function asPartRecord(part: JsonValue): PartRecord | null {
  return part && typeof part === 'object' ? (part as PartRecord) : null
}

function normalizeText(value: JsonValue): string {
  return typeof value === 'string' ? value : ''
}

function titleCase(raw: string): string {
  const v = raw.trim()
  if (!v) return ''
  return v
    .split(/[_-]/)
    .map((chunk) => (chunk ? chunk[0]!.toUpperCase() + chunk.slice(1) : ''))
    .join(' ')
}

function formatDurationMs(start?: number, end?: number): string {
  if (!start || !end || end <= start) return ''
  const sec = (end - start) / 1000
  return `${sec.toFixed(1)}s`
}

function isReasoningPart(part: JsonValue): boolean {
  const rec = asPartRecord(part)
  const t = normalizeText(rec?.type).toLowerCase()
  return t === 'reasoning'
}

function isToolPart(part: JsonValue): boolean {
  const rec = asPartRecord(part)
  return normalizeText(rec?.type).toLowerCase() === 'tool'
}

function partText(part: JsonValue): string {
  const rec = asPartRecord(part)
  return typeof rec?.text === 'string' ? rec.text : ''
}

export function formatTranscript(
  session: TranscriptSession,
  messages: TranscriptMessage[],
  options: TranscriptOptions,
): string {
  const title = normalizeText(session.title) || session.id
  let transcript = `# ${title}\n\n`
  transcript += `**Session ID:** ${session.id}\n`
  if (session.time?.created) {
    transcript += `**Created:** ${formatDateTimeYMDHM(session.time.created)}\n`
  }
  if (session.time?.updated) {
    transcript += `**Updated:** ${formatDateTimeYMDHM(session.time.updated)}\n`
  }
  transcript += `\n---\n\n`

  for (const msg of messages) {
    transcript += formatMessage(msg, options)
    transcript += `---\n\n`
  }

  return transcript
}

function formatMessage(message: TranscriptMessage, options: TranscriptOptions): string {
  const role = normalizeText(message.info?.role).toLowerCase()
  let result = ''

  if (role === 'assistant') {
    result += formatAssistantHeader(message, options.assistantMetadata)
  } else if (role === 'user') {
    result += `## User\n\n`
  } else {
    result += `## ${role ? titleCase(role) : 'Message'}\n\n`
  }

  for (const part of message.parts || []) {
    result += formatPart(part, options)
  }

  return result
}

function formatAssistantHeader(message: TranscriptMessage, includeMetadata: boolean): string {
  if (!includeMetadata) return `## Assistant\n\n`

  const model = normalizeText(message.info?.modelID)
  const duration = formatDurationMs(message.info?.time?.created, message.info?.time?.completed)
  const meta = [model, duration].filter(Boolean).join(' · ')
  return meta ? `## Assistant (${meta})\n\n` : `## Assistant\n\n`
}

function formatPart(part: JsonValue, options: TranscriptOptions): string {
  const rec = asPartRecord(part)

  if (rec?.type === 'text' && !rec?.synthetic && !rec?.ignored) {
    const text = partText(part)
    return text ? `${text}\n\n` : ''
  }

  if (isReasoningPart(part)) {
    if (!options.thinking) return ''
    const text = partText(part)
    return text ? `_Thinking:_\n\n${text}\n\n` : ''
  }

  if (rec?.type === 'file') {
    const label = normalizeText(rec?.filename) || normalizeText(rec?.url) || 'attachment'
    return `_Attachment:_ ${label}\n\n`
  }

  if (isToolPart(part)) {
    if (!options.toolDetails) return ''
    const tool = normalizeText(rec?.tool) || 'tool'
    let result = `\`\`\`\nTool: ${tool}\n`
    const state = asPartRecord(rec?.state)
    const input = state?.input
    const output = state?.output
    const error = state?.error
    if (input !== undefined) {
      result += `\n**Input:**\n\`\`\`json\n${safeJson(input)}\n\`\`\``
    }
    if (output !== undefined) {
      result += `\n**Output:**\n\`\`\`\n${safeText(output)}\n\`\`\``
    }
    if (error !== undefined) {
      result += `\n**Error:**\n\`\`\`\n${safeText(error)}\n\`\`\``
    }
    result += `\n\`\`\`\n\n`
    return result
  }

  return ''
}

function safeJson(value: JsonValue): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function safeText(value: JsonValue): string {
  if (typeof value === 'string') return value
  if (value === null || value === undefined) return ''
  return safeJson(value)
}
