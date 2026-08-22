import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'

import type { TranscriptDisplayPart } from '../src/components/chat/messageList.types'
import {
  decodeStructuredValue,
  interactionPresentationFromAttention,
  operationPresentation,
  partInteractionRequestIds,
  partStatusPresentation,
  pendingInteractionPresentationFromAttention,
  permissionPresentationFromAttention,
  structuredValueMarkdown,
} from '../src/pages/chat/transcriptPartPresentation'
import { projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'

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
          input: { file_path: 'README.md' },
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
    expect(projected.structured).toEqual({ preview: 'raw value' })
    expect(projected.blocks).toEqual([{ type: 'json', value: { preview: '**README**' } }])
    expect(projected.metadata).toEqual({ cache: true })
    expect(projected.durationMs).toBe(25)
  })

  test('projects operation attachments without losing source-specific media facts', () => {
    const projected = operationPresentation(
      operationPart({
        name: 'image.generate',
        output: {
          attachments: [
            {
              kind: 'image',
              mime: 'image/png',
              source: { source: 'local_path', path: 'artifacts/chart.png' },
              filename: 'chart.png',
              size_bytes: 2048,
              width: 640,
              height: 480,
            },
            {
              kind: 'audio',
              mime: 'audio/mpeg',
              source: { source: 'url', url: 'https://example.test/audio.mp3' },
              title: 'Audio preview',
              duration_ms: 2500,
            },
            {
              kind: 'image',
              mime: 'image/png',
              source: { source: 'base64', data: 'aGVsbG8=' },
              filename: 'inline.png',
            },
          ],
        },
      }),
    )
    expect(projected.attachments).toHaveLength(3)
    expect(projected.attachments[0]).toMatchObject({
      label: 'chart.png',
      path: 'artifacts/chart.png',
      url: 'artifacts/chart.png',
      width: 640,
      height: 480,
    })
    expect(projected.attachments[1]).toMatchObject({
      label: 'Audio preview',
      url: 'https://example.test/audio.mp3',
      durationMs: 2500,
    })
    expect(projected.attachments[2]?.url).toBe('data:image/png;base64,aGVsbG8=')
  })

  test('keeps completed permission decisions and their reply reason in the part projection', () => {
    const projected = operationPresentation(
      operationPart({
        authorization: {
          permissions: [
            {
              request: {
                request_id: 'permission-1',
                action: { kind: 'path_access', access_kind: 'read', target_path: 'notes.md' },
              },
              reply: { kind: 'deny_once', reason: 'The file contains private notes.' },
            },
            {
              request: {
                request_id: 'permission-2',
                action: { kind: 'network_access', host: 'example.test' },
              },
              reply: { kind: 'auto_approve' },
            },
            {
              request: {
                request_id: 'permission-3',
                action: { kind: 'tool', tool_name: 'legacy.tool' },
              },
              reply: { kind: 'legacy_decision' },
            },
          ],
        },
      }),
    )
    expect(projected.permissions).toMatchObject([
      { pending: false, status: 'Denied once', replyReason: 'The file contains private notes.' },
      { pending: false, status: 'Approved automatically', replyReason: '' },
      { pending: false, status: 'Replied (legacy_decision)', replyReason: '' },
    ])

    const pending = operationPresentation(
      operationPart({
        authorization: {
          permissions: [
            {
              request: { request_id: 'permission-pending', action: { kind: 'path_access', target_path: 'notes.md' } },
              reply: null,
            },
          ],
        },
      }),
    )
    expect(pending.permissions[0]).toMatchObject({ pending: true, status: 'Awaiting user approval' })
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

  test('keeps review body, input kind, question ids, and completed decisions', () => {
    const projected = operationPresentation(
      operationPart({
        name: 'plan.review',
        user_input: {
          requests: [
            {
              request: {
                request_id: 'review-1',
                title: 'Review proposed plan',
                input_kind: 'review',
                body_markdown: '## Plan\n\n1. Update the renderer.\n2. Run tests.',
                questions: [
                  {
                    question_id: 'decision',
                    question: 'How should this plan proceed?',
                    options: [
                      { label: 'Approve', description: 'Run the plan' },
                      { label: 'Request changes', description: 'Send feedback' },
                    ],
                    allow_custom: true,
                  },
                ],
              },
              reply: { kind: 'submit', answers: { decision: ['Approve'] } },
            },
          ],
        },
      }),
    )
    const review = projected.userInputs[0]
    expect(review).toMatchObject({
      requestId: 'review-1',
      title: 'Review proposed plan',
      kind: 'review',
      bodyMarkdown: '## Plan\n\n1. Update the renderer.\n2. Run tests.',
      pending: false,
    })
    expect(review?.questions[0]).toMatchObject({
      questionId: 'decision',
      allowCustom: true,
    })
    expect(review?.reply).toEqual({ kind: 'submit', answers: { decision: ['Approve'] } })
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
    expect(projected.structured).toEqual(structured)
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

  test('projects live review attention with the same shape as a durable interaction part', () => {
    const reviewAttention = {
      kind: 'question',
      payload: {
        type: 'question.asked',
        properties: {
          id: 'review-live',
          request: {
            kind: 'user_input',
            request_id: 'review-live',
            input_kind: 'review',
            title: 'Review proposed plan',
            body_markdown: '## Plan\n\n1. Update the renderer.',
            questions: [
              {
                question: 'How should this plan proceed?',
                options: [{ label: 'Approve', description: 'Run the plan' }],
                allow_custom: true,
              },
            ],
          },
        },
      },
    } as const
    const review = interactionPresentationFromAttention(reviewAttention)
    expect(review).toMatchObject({
      requestId: 'review-live',
      kind: 'review',
      pending: true,
      bodyMarkdown: '## Plan\n\n1. Update the renderer.',
    })
    expect(review?.questions[0]).toMatchObject({
      question: 'How should this plan proceed?',
      allowCustom: true,
    })
    expect(pendingInteractionPresentationFromAttention(reviewAttention, new Set())).toMatchObject({
      requestId: 'review-live',
    })
    expect(pendingInteractionPresentationFromAttention(reviewAttention, new Set(['review-live']))).toBeNull()
  })

  test('projects live permission attention with full action context', () => {
    const permission = permissionPresentationFromAttention({
      kind: 'permission',
      payload: {
        type: 'permission.asked',
        properties: {
          id: 'permission-live',
          request: {
            request_id: 'permission-live',
            action: { kind: 'path_access', access_kind: 'read', target_path: 'notes.md' },
            reason: 'The operation needs to inspect the note.',
            explanation: 'The file is read-only.',
            source: 'tool',
            scope: 'session',
          },
        },
      },
    })
    expect(permission).toMatchObject({
      requestId: 'permission-live',
      pending: true,
      status: 'Awaiting user approval',
      action: 'read notes.md',
      reason: 'The operation needs to inspect the note.',
      explanation: 'The file is read-only.',
      provenance: 'tool · session',
    })
  })

  test('auto-expands a pending interaction operation as one transcript Part', () => {
    const [block] = projectTranscriptBlocks(
      [
        {
          info: { id: 'message-1', role: 'assistant' },
          parts: [
            {
              id: 'operation-1',
              type: 'tool',
              partState: 'in_progress',
              agenaKind: 'tool_call',
              agenaRole: 'assistant',
              agenaContent: {
                name: 'plan.review',
                user_input: {
                  requests: [{ request: { request_id: 'review-1', kind: 'review', questions: [] }, reply: null }],
                },
              },
            },
          ],
        },
      ],
      { showReasoning: true, revert: null },
    )
    expect(block?.kind).toBe('message')
    if (block?.kind === 'message') expect(block.displayParts[0]?.defaultExpanded).toBe(true)
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
    expect(projected.structured).toBeNull()
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
    expect(projected.structured).toBeNull()
    expect(projected.blocks).toEqual([])
    expect(projected.attachments).toEqual([])
  })

  test('keeps non-presentation sections folded and presents the five tool sections in order', () => {
    const source = readFileSync(new URL('../src/components/chat/AgenaOperationPart.vue', import.meta.url), 'utf8')
    expect(source).toContain('const metadataExpanded = ref(false)')
    expect(source).toContain('const inputExpanded = ref(false)')
    expect(source).toContain('const outputExpanded = ref(false)')
    expect(source).toContain('const outputMetadataExpanded = ref(false)')
    expect(source).toContain('const presentationExpanded = ref(true)')
    expect(source).toContain(
      "const toolDetailSections: ToolDetailSection[] = ['metadata', 'input', 'output', 'output_metadata', 'presentation']",
    )
    expect(source).toContain('getToolPartDetail')
    expect(source).toContain('data-tool-detail-section')
    expect(source).not.toContain('stdoutExpanded')
    expect(source).toContain('AgenaInteractionPart')
    expect(source).not.toContain('AttentionPanel')

    const interactionSource = readFileSync(
      new URL('../src/components/chat/AgenaInteractionPart.vue', import.meta.url),
      'utf8',
    )
    expect(interactionSource).toContain('data-transcript-interaction-part="true"')
    expect(interactionSource).toContain('data-transcript-chrome="true"')
    expect(interactionSource).toContain('@keydown.stop="handleKeydown"')
    expect(interactionSource).toContain('ArrowUp')
    expect(interactionSource).toContain('ArrowDown')
    expect(interactionSource).toContain("lowerKey === 'k'")
    expect(interactionSource).toContain("lowerKey === 'j'")
    expect(interactionSource).toContain("key === 'Tab'")
    expect(interactionSource).toContain("key === 'Enter'")
    expect(interactionSource).toContain('focusNextIncompleteQuestion(index)')
    expect(interactionSource).toContain("key === 'Escape'")
    expect(interactionSource).toContain("lowerKey === 'd'")
    expect(interactionSource).toContain("lowerKey === 'x'")
    expect(interactionSource).toContain('nextTick')
    expect(interactionSource).toContain('v-for="(question, questionIndex) in questions"')
    expect(interactionSource).not.toContain('questionPage')
    expect(interactionSource).toContain('permission.replyReason')
    expect(interactionSource).toContain('isReviewDecision')

    const planViewerSource = readFileSync(
      new URL('../src/components/chat/PlanViewerDialog.vue', import.meta.url),
      'utf8',
    )
    expect(planViewerSource).toContain('data-plan-state-viewer="true"')
    expect(planViewerSource).toContain('Plan approval decisions')
    expect(planViewerSource).not.toContain('replyQuestion')
    expect(planViewerSource).not.toContain('replyPermission')

    const chatPageViewSource = readFileSync(new URL('../src/pages/chat/ChatPageView.vue', import.meta.url), 'utf8')
    const messageListSource = readFileSync(new URL('../src/components/chat/MessageList.vue', import.meta.url), 'utf8')
    expect(chatPageViewSource).not.toContain('AttentionPanel')
    expect(messageListSource).not.toContain('AttentionPanel')
    expect(messageListSource).toContain('pendingInteractionFallback')
    expect(messageListSource).toContain('partInteractionRequestIds')

    const attachmentSource = readFileSync(
      new URL('../src/components/chat/AgenaAttachmentPreview.vue', import.meta.url),
      'utf8',
    )
    expect(attachmentSource).toContain('buildWorkspaceRawFileUrl')
    expect(attachmentSource).toContain('data:')

    const headerSource = readFileSync(new URL('../src/components/chat/ChatHeader.vue', import.meta.url), 'utf8')
    expect(headerSource).not.toContain('AttentionPanel')
  })

  test('keeps raw output and output metadata independent in section projections', () => {
    const projected = operationPresentation(
      operationPart(
        {
          name: 'shell.run',
          input: { script: 'echo done' },
          metadata: { source: 'test' },
          state: 'completed',
        },
        {
          title: 'shell.run',
          summary: 'Command finished',
          blocks: [{ type: 'text', text: 'Human summary' }],
        },
      ),
      {
        input: { script: 'private input' },
        output: { text: 'done', payload: { exit_code: 0 }, metadata: { exit_code: 0 } },
        output_metadata: { exit_code: 0 },
      },
    )

    expect(projected.input).toEqual({ script: 'private input' })
    expect(projected.metadata).toEqual({ source: 'test' })
    expect(projected.outputText).toBe('done')
    expect(projected.rawOutput).toEqual({ text: 'done', payload: { exit_code: 0 } })
    expect(projected.outputMetadata).toEqual({ exit_code: 0 })
    expect(projected.presentationBlocks).toEqual([{ type: 'text', text: 'Human summary' }])
  })
})
