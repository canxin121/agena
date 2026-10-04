import { describe, expect, test } from 'bun:test'
import { planDocument, readPlanSnapshot } from '../src/pages/chat/planSnapshot'

describe('session plan projection', () => {
  test('summary uses durable steps including skipped work and the server-selected current step', () => {
    const response = {
      payload: {
        plan: {
          title: 'Ship the change',
          phase: 'active',
          autorun: true,
          steps: [{ status: 'completed' }, { status: 'skipped' }, { status: 'in_progress' }, { status: 'pending' }],
        },
        current_step: { title: 'Verify 中文' },
      },
    }
    expect(readPlanSnapshot(response)).toEqual({
      title: 'Ship the change',
      phase: 'active',
      progress: '▶ 2/4 ↻',
      completed: 2,
      total: 4,
      currentStep: 'Verify 中文',
      autorun: true,
    })
  })

  test('terminal plans do not claim that an old current step is still running', () => {
    for (const phase of ['completed', 'cancelled']) {
      const snapshot = readPlanSnapshot({ payload: { plan: { phase }, current_step: { title: 'old step' } } })
      expect(snapshot?.currentStep).toBe('')
    }
    expect(readPlanSnapshot({ payload: { plan: { phase: 'blocked', steps: [] } } })?.progress).toBe('⚠')
  })

  test('missing plans stay absent and transport revisions stay out of the document', () => {
    for (const payload of [null, {}, { plan: null }, { plan: [] }]) expect(readPlanSnapshot({ payload })).toBeNull()
    expect(planDocument({ output_text: 'Revision: a1\n# Plan\n\n- **check**' })).toBe('# Plan\n\n- **check**')
    expect(planDocument({ output_text: '# Plan\n\nRevision: part of the document' })).toContain(
      'Revision: part of the document',
    )
  })
})
