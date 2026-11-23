import { describe, expect, test } from 'bun:test'
import { composerProcesses, type SessionProcess } from '../src/pages/chat/sessionProcesses'
import type { MessageEntry, MessagePart } from '../src/types/chat'

const process: SessionProcess = {
  process_id: 'proc-42',
  command: 'bun run build',
  description: 'Compile',
  status: 'running',
  ready: false,
  started_at_ms: 10,
  output_resource: { resource_id: 'build', kind: 'log' },
}
function part(fields: Partial<MessagePart> = {}): MessagePart {
  return {
    id: '10',
    sessionID: '7',
    messageID: '5',
    agenaKind: 'tool_call',
    type: 'tool',
    tool: 'agena.shell.spawn',
    partState: 'completed',
    agenaContent: { resources: [process.output_resource!] },
    agenaPresentation: { blocks: [{ type: 'shell_process', process_id: process.process_id }] },
    ...fields,
  }
}
const message = (...parts: MessagePart[]): MessageEntry => ({
  info: { id: '5', sessionID: '7', role: 'assistant' },
  parts,
})

describe('composer process discovery', () => {
  test('keeps an open foreground command anchored from invocation through streaming and exit', () => {
    const initial = part({
      tool: 'shell.exec',
      partState: 'in_progress',
      agenaContent: { input: { command: 'bun test' } },
      agenaPresentation: {},
    })
    const row = (p: MessagePart) => composerProcesses('7', [message(p)], [])[0]!
    expect(row(initial).label).toBe('bun test')
    const streaming = { ...initial, agenaContent: { resources: [process.output_resource!] } }
    const finished = {
      ...streaming,
      partState: 'completed',
      agenaPresentation: { blocks: [{ type: 'command', command: 'bun test', exit_code: 0 }] },
    }
    expect(row(streaming).key).toBe(row(initial).key)
    expect(row(finished).key).toBe(row(initial).key)
    expect(row(finished).exitCode).toBe(0)
  })
  test('deduplicates a managed command against its persisted stream before and after the launch returns', () => {
    const pending = part({ partState: 'running', agenaPresentation: { blocks: [] } })
    const rows = composerProcesses('7', [message(pending, part())], [process])
    expect(rows).toHaveLength(1)
    expect(rows[0]?.process).toBe(process)
    expect(composerProcesses('7', [message(pending)], [])[0]?.key).toBe(rows[0]?.key)
    const retained = composerProcesses('7', [message(part())], [])[0]
    expect(retained?.key).toBe(rows[0]?.key)
    expect(retained?.part?.id).toBe('10')
    expect(retained?.retained).toBe(true)
  })
  test('shows a session-owned process even outside the loaded transcript and puts live work first', () => {
    const service = {
      ...process,
      process_id: 'service',
      command: 'bun run dev',
      output_resource: null,
    }
    const foreground = part({
      id: '20',
      tool: 'agena_shell_exec',
      createdAt: 100,
      agenaContent: {},
      agenaPresentation: { blocks: [{ type: 'command', command: 'bun test', exit_code: 17 }] },
    })
    const rows = composerProcesses('7', [message(foreground)], [service])
    expect(rows.map((row) => row.label)).toEqual(['bun run dev', 'bun test'])
    expect(rows[1]?.exitCode).toBe(17)
  })
  test('ignores inherited or foreign parts, unrelated tools and watch updates with no stream', () => {
    const entries = message(
      part({ originSessionId: 9 }),
      part({ sessionID: '9' }),
      part({ tool: 'other.exec' }),
      part({ tool: 'agena.monitor.start' }),
      part({ tool: 'shell.watch', agenaContent: {}, agenaPresentation: {} }),
    )
    expect(composerProcesses('7', [entries], [])).toEqual([])
  })
})
