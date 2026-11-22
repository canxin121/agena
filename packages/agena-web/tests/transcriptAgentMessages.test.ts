import { expect, test } from 'bun:test'
import { normalizeAgenaPart } from '../src/stores/chat/api'
import { projectTranscriptBlocks } from '../src/pages/chat/transcriptProjection'

test('a message delivered by a child remains a collapsible notice beside the parent answer', () => {
  const detail = 'Found the files.\n\nThe full report follows.'
  const agentMessage = normalizeAgenaPart('13', '7', '5', {
    kind: 'system_notification',
    role: 'assistant',
    state: 'completed',
    content: {
      operation_kind: 'agent_message',
      operation_id: '91',
      sender_name: 'worker',
      summary: 'Found the files.',
      detail,
      body: `Message from worker (91):\n${detail}`,
    },
  })!
  const [message] = projectTranscriptBlocks(
    [
      {
        info: { id: '5', role: 'assistant' },
        parts: [{ id: '12', agenaKind: 'text', text: 'Done.' }, agentMessage],
      },
    ],
    { showReasoning: true, labels: { agentMessage: '子会话消息' } },
  )

  expect(message?.displayParts.map((part) => part.kind)).toEqual(['answer', 'notice'])
  expect(message?.displayParts[1]).toMatchObject({
    title: '子会话消息 · worker',
    summary: 'Found the files.',
    toggleable: true,
    defaultExpanded: false,
    copyText: `子会话消息 · worker\n${detail}`,
  })
})
