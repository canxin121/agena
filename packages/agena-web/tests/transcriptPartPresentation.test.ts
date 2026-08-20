import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

import type { TranscriptDisplayPart } from '../src/components/chat/messageList.types'
import {
  decodeStructuredValue,
  operationPresentation,
  partInteractionRequestIds,
  partStatusPresentation,
  structuredValueMarkdown,
} from '../src/pages/chat/transcriptPartPresentation'

function operationPart(
  content: Record<string, unknown>,
  presentation: Record<string, unknown> = { title: 'Tool operation', summary: '', blocks: [] },
): TranscriptDisplayPart {
  return {
    key: 'part:4',
    id: '4',
    kind: 'operation',
    status: 'completed',
    role: 'assistant',
    title: 'fallback',
    summary: '',
    copyText: '',
    toggleable: true,
    defaultExpanded: false,
    source: {
      id: '4',
      agenaKind: 'tool_call',
      agenaRole: 'assistant',
      partState: 'completed',
      agenaContent: content,
      agenaPresentation: presentation,
    },
  }
}

describe('TUI-parity part presentation', () => {
  test('decodes structured operation input and renders nested Markdown bullets', () => {
    const structured = {
      fields: [
        { name: 'path', value: { kind: 'text', value: 'src/main.rs' } },
        {
          name: 'flags',
          value: {
            kind: 'array',
            items: [
              { kind: 'boolean', value: true },
              { kind: 'integer', value: 2 },
            ],
          },
        },
      ],
    }
    expect(decodeStructuredValue(structured)).toEqual({ path: 'src/main.rs', flags: [true, 2] })
    expect(structuredValueMarkdown(structured)).toContain('- **path**: `src/main.rs`')
  })

  test('preserves operation title, human output, rich blocks, metadata, and duration', () => {
    const projected = operationPresentation(
      operationPart(
        {
          name: 'fs.read',
          input: { path: 'README.md' },
          lifecycle: { start_ms: 10, end_ms: 35 },
          state: 'completed',
          output: { payload: { preview: 'raw value' } },
          metadata: { cache: true },
        },
        {
          title: 'fs.read · README.md',
          summary: 'Read 42 lines',
          blocks: [{ type: 'json', value: { preview: '**README**' } }],
        },
      ),
    )
    expect(projected.title).toBe('fs.read · README.md')
    expect(projected.humanMarkdown).toBe('')
    expect(projected.blocks).toEqual([{ type: 'json', value: { preview: '**README**' } }])
    expect(projected.metadata).toEqual({ cache: true })
    expect(projected.durationMs).toBe(25)
  })

  test('projects operation-owned user input as the interaction part', () => {
    const projected = operationPresentation(
      operationPart({
        name: 'interaction.ask',
        user_input: {
          requests: [
            {
              request: {
                request_id: 'request-1',
                kind: 'ask_user',
                questions: [
                  {
                    header: 'Target',
                    question: 'Choose one',
                    options: [{ label: 'Workspace', description: 'Search files' }],
                  },
                ],
              },
              reply: { kind: 'submit', answers: { '0': ['Workspace'] } },
            },
          ],
        },
        state: 'completed',
      }),
    )
    expect(projected.userInputs).toHaveLength(1)
    expect(projected.userInputs[0]?.requestId).toBe('request-1')
    expect(projected.userInputs[0]?.questions[0]?.options[0]?.label).toBe('Workspace')
    expect(projected.userInputs[0]?.pending).toBe(false)
  })

  test('matches TUI lifecycle glyphs for denied and unavailable parts', () => {
    expect(partStatusPresentation('policy_denied')).toMatchObject({ icon: '⊘', tone: 'warning', terminal: true })
    expect(partStatusPresentation('tool_unavailable')).toMatchObject({ icon: '◇', tone: 'warning', terminal: true })
    expect(partStatusPresentation('failed')).toMatchObject({ icon: '×', tone: 'danger', terminal: true })
  })

  test('does not assign a failure or pending glyph to unknown status values', () => {
    expect(partStatusPresentation('')).toMatchObject({ icon: '', label: '', tone: 'muted', terminal: false })
    expect(partStatusPresentation('future_state')).toMatchObject({
      icon: '',
      label: '',
      tone: 'muted',
      terminal: false,
    })
  })

  test('projects web search structured results once and suppresses duplicate model/log output', () => {
    const structured = {
      query: 'agena',
      results: [{ title: 'Agena', url: 'https://example.test/agena', description: 'Result' }],
    }
    const part = operationPart(
      { name: 'web.search', state: 'completed', output: { payload: structured } },
      {
        title: 'Web search',
        summary: '1 result',
        blocks: [{ type: 'search_results', query: 'agena', results: structured.results }],
      },
    )
    const projected = operationPresentation(part)
    expect(projected.modelOutput).toBe('')
    expect(projected.blocks).toEqual([
      {
        type: 'search_results',
        query: 'agena',
        results: structured.results,
      },
    ])
  })

  test('retains request identities for inline interaction and permission ownership', () => {
    const part = operationPart({
      user_input: {
        requests: [{ request: { request_id: 'input-1', questions: [] }, reply: null }],
      },
      authorization: {
        permissions: [{ request: { request_id: 'permission-1', action: { kind: 'network_access' } } }],
      },
    })
    expect(partInteractionRequestIds(part)).toEqual(['input-1', 'permission-1'])
  })

  test('extracts explicit stdout logs and removes duplicate primary output', () => {
    const projected = operationPresentation(
      operationPart(
        { output: { payload: { text: 'raw text' } } },
        {
          title: 'Complete',
          summary: 'Done',
          blocks: [{ type: 'log', stream: 'stdout', text: '## Complete\n\n- one' }],
        },
      ),
    )
    expect(projected.stdout).toBe('## Complete\n\n- one')
    expect(projected.humanMarkdown).toBe('')
    expect(projected.blocks).toEqual([])
  })

  test('extracts command stdout while retaining command diagnostics in Output', () => {
    const projected = operationPresentation(
      operationPart(
        { output: { payload: { raw: true } } },
        {
          title: 'cargo test',
          summary: '2 passed',
          blocks: [
            {
              type: 'command',
              command: 'cargo test',
              cwd: '/workspace',
              stdout: '**2 passed**',
              stderr: 'warning',
              exit_code: 0,
            },
          ],
        },
      ),
    )
    expect(projected.stdout).toBe('**2 passed**')
    expect(projected.blocks).toEqual([
      {
        type: 'command',
        command: 'cargo test',
        cwd: '/workspace',
        stderr: 'warning',
        exit_code: 0,
      },
    ])
  })

  test('deduplicates direct stdout sources and leaves stdout-only Output empty', () => {
    const projected = operationPresentation(
      operationPart(
        { output: { payload: { text: 'raw result' } } },
        { title: 'Result', summary: 'Complete', blocks: [{ type: 'log', stream: 'stdout', text: '# Result' }] },
      ),
    )
    expect(projected.stdout).toBe('# Result')
    expect(projected.humanMarkdown).toBe('')
    expect(projected.modelOutput).toBe('')
    expect(projected.structured).toBeNull()
    expect(projected.blocks).toEqual([])
    expect(projected.displaySections).toEqual([])
    expect(projected.attachments).toEqual([])
  })

  test('keeps Input and Output folded while Stdout is expanded Markdown', () => {
    const source = readFileSync(new URL('../src/components/chat/AgenaOperationPart.vue', import.meta.url), 'utf8')
    expect(source).toContain('const inputExpanded = ref(false)')
    expect(source).toContain('const outputExpanded = ref(false)')
    expect(source).toContain('const stdoutExpanded = ref(true)')
    expect(source).toContain('<MarkdownRenderer :content="operation.stdout" mode="markdown" :stream="false" />')
  })
})
