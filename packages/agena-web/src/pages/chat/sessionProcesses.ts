import type { ContentRef } from '@/lib/content'
import { toolInputRecord } from '@/lib/toolInput'
import type { MessageEntry, MessagePart } from '@/types/chat'
import { jsonArray, jsonRecord, stringValue } from './transcriptPartPresentation'

export type SessionProcess = {
  process_id: string
  command: string
  description: string
  status: 'running' | 'exited' | 'timed_out' | 'stopped' | 'failed'
  ready: boolean
  exit_code?: number | null
  output_resource?: ContentRef | null
  started_at_ms: number
}

export type ComposerProcess = {
  key: string
  label: string
  started: number
  process?: SessionProcess
  part?: MessagePart
  exitCode?: number
  retained?: boolean
}

export function processCollection(sessionId: string) {
  return `/api/v1/sessions/${encodeURIComponent(sessionId)}/shell/processes`
}

/** Foreground commands only have a Part. Managed output can also outlive the live registry. */
export function composerProcesses(
  sessionId: string,
  messages: MessageEntry[],
  shells: SessionProcess[],
): ComposerProcess[] {
  const entries = new Map<string, ComposerProcess>()
  const processKeys = new Map<string, string>()
  for (const process of shells) {
    const key = process.output_resource
      ? `output:${process.output_resource.resource_id}`
      : `shell:${process.process_id}`
    processKeys.set(process.process_id, key)
    entries.set(key, {
      key,
      process,
      started: process.started_at_ms,
      label: process.command || process.description || process.process_id,
    })
  }

  for (const message of messages) {
    for (const part of message.parts) {
      if (part.sessionID !== sessionId || (part.originSessionId != null && String(part.originSessionId) !== sessionId))
        continue
      if (part.agenaKind !== 'tool_call' && part.type !== 'tool') continue
      const source = jsonRecord(part.agenaContent)
      const name = (part.tool || stringValue(source.name))
        .toLowerCase()
        .replace(/^agena[._]/, '')
        .replaceAll('__', '.')
        .replace(/[\/_]/g, '.')
      if (!['shell.exec', 'shell.spawn', 'shell.open', 'shell.watch'].includes(name)) continue
      const document = jsonRecord(part.agenaPresentation)
      const blocks = jsonArray(document.blocks).map(jsonRecord)
      const control = blocks.find((block) => block.type === 'shell_process')
      const id = stringValue(control?.process_id)
      const resource = jsonArray(source.resources)
        .map(jsonRecord)
        .find((item) => item.kind === 'log' || item.kind === 'terminal')
      // Managed launches become a resource when their process ID or output stream arrives.
      if (name !== 'shell.exec' && !resource && !id) continue
      const resourceId = stringValue(resource?.resource_id)
      // A foreground call keeps its popover while the first output reference arrives.
      const key =
        name === 'shell.exec'
          ? `part:${part.id}`
          : processKeys.get(id) || (resourceId ? `output:${resourceId}` : `shell:${id}`)
      const existing = entries.get(key)
      if (existing) {
        if (existing.process && !existing.process.output_resource && !existing.part) existing.part = part
        continue
      }
      const command = blocks.find((block) => block.type === 'command')
      entries.set(key, {
        key,
        part,
        started: part.createdAt ?? message.info.time?.created ?? 0,
        label:
          stringValue(command?.command) ||
          stringValue(toolInputRecord(source.input).command) ||
          stringValue(document.title) ||
          name,
        retained: Boolean(id),
        exitCode: typeof command?.exit_code === 'number' ? command.exit_code : undefined,
      })
    }
  }
  const running = (row: ComposerProcess) =>
    row.process
      ? row.process.status === 'running'
      : ['running', 'in_progress', 'pending'].includes(row.part?.partState || '')
  return [...entries.values()].sort(
    (a, b) => Number(running(b)) - Number(running(a)) || b.started - a.started || a.key.localeCompare(b.key),
  )
}
