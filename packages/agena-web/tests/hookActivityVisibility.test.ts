import { describe, expect, test } from 'bun:test'
import { projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'

describe('hook transcript activity', () => {
  test('hides routine persisted hook parts without hiding real outcomes', () => {
    const [message] = projectTranscriptBlocks([
      {
        info: { id: 'reply-1', role: 'assistant' },
        parts: [
          {
            id: 'system',
            agenaKind: 'hook',
            agenaContent: { hook: 'chat.system', summary: 'chat.system hook ran', plugin_id: 'agena.companion' },
          },
          {
            id: 'session-end',
            agenaKind: 'hook',
            agenaContent: { hook: 'session.end', summary: 'session.end hook notified' },
          },
          {
            id: 'blocked',
            agenaKind: 'hook',
            agenaContent: {
              hook: 'command.before',
              status: 'applied',
              summary: 'command.before hook aborted command',
              detail: 'blocked by policy',
            },
          },
          {
            id: 'continued',
            agenaKind: 'hook',
            agenaContent: {
              hook: 'agent.stop',
              summary: 'agent.stop hook blocked stop',
              message: 'Continue the approved plan',
            },
          },
          {
            id: 'failed',
            agenaKind: 'hook',
            agenaContent: { hook: 'chat.system', status: 'failed', summary: 'chat.system hook failed' },
          },
          {
            id: 'answer',
            agenaKind: 'text',
            text: 'Done',
          },
        ],
      },
    ])

    expect(message?.displayParts.map((part) => part.id)).toEqual(['answer', 'blocked', 'continued', 'failed'])
  })

  test('filters successful hook status from new and historical transcripts', () => {
    const [message] = projectTranscriptBlocks([
      {
        info: { id: 'reply-2', role: 'assistant' },
        parts: [
          {
            id: 'applied',
            agenaKind: 'hook',
            agenaContent: { hook: 'chat.params', status: 'applied', summary: 'chat.params hook ran' },
          },
          {
            id: 'skipped',
            agenaKind: 'hook',
            agenaContent: { hook: 'chat.headers', status: 'skipped', summary: 'chat.headers hook skipped' },
          },
          {
            id: 'timeout',
            agenaKind: 'hook',
            agenaContent: { hook: 'session.end', status: 'timed_out', summary: 'session.end hook timed out' },
          },
        ],
      },
    ])

    expect(message?.displayParts.map((part) => part.id)).toEqual(['timeout'])
  })
})
