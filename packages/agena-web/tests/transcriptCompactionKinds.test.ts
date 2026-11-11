import { describe, expect, test } from 'bun:test'
import { projectLocalPart } from '../src/pages/chat/transcriptProjection'

const labels = {
  compaction: 'Compaction',
  contextCleared: 'Context cleared',
  contextClearedSummary: 'Earlier turns stay readable but are no longer sent to the model.',
}

describe('compaction and cleared context are distinct transcript parts', () => {
  test('a compaction keeps its own identity and summary', () => {
    const part = projectLocalPart(
      {
        id: '2',
        agenaKind: 'compaction',
        agenaRole: 'assistant',
        agenaSummary: 'Summarized 12 turns',
        agenaContent: { outcome: 'compacted', run_kind: 'compaction' },
      },
      'assistant',
      labels,
    )
    expect(part.kind).toBe('compaction')
    expect(part.title).toBe('Compaction')
    expect(part.summary).toBe('Summarized 12 turns')
  })

  test('a cleared context is never presented as a compaction', () => {
    const part = projectLocalPart(
      {
        id: '3',
        agenaKind: 'context_cleared',
        agenaRole: 'assistant',
        agenaContent: { cleared_through: 41, run_kind: 'clear_context' },
      },
      'assistant',
      labels,
    )
    expect(part.kind).toBe('context_cleared')
    expect(part.title).toBe('Context cleared')
    expect(part.summary).toBe(labels.contextClearedSummary)
    // The two must not share a label: a clear produced no summary.
    expect(part.title).not.toBe(labels.compaction)
  })
})
