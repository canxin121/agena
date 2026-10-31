import { describe, expect, test } from 'bun:test'
import {
  interactionQuestionOptions,
  interactionPresentationFromAttention,
  permissionPresentationFromAttention,
  pendingInteractionPresentationFromAttention,
  type InteractionQuestionPresentation,
} from '../src/pages/chat/transcriptPartPresentation'

const question: InteractionQuestionPresentation = {
  questionId: '0',
  header: 'Choice',
  question: 'Which option?',
  multiple: false,
  allowCustom: true,
  options: [
    { label: 'Custom', description: 'A literal named choice' },
    { label: 'Type your own answer', description: 'Another literal choice' },
  ],
}

describe('current interaction contracts', () => {
  test('literal custom labels remain ordinary choices and the generated editor has a distinct identity', () => {
    const options = interactionQuestionOptions(question)
    expect(options.filter((option) => !option.synthetic).map((option) => option.custom)).toEqual([false, false])
    expect(options.filter((option) => option.custom)).toHaveLength(1)
    expect(new Set(options.map((option) => option.label)).size).toBe(options.length)
    expect(interactionQuestionOptions({ ...question, allowCustom: false }).some((option) => option.custom)).toBe(false)
    const explicit = interactionQuestionOptions({
      ...question,
      options: [{ label: 'Feedback', description: '', custom: true }],
    })
    expect(explicit).toEqual([{ label: 'Feedback', description: '', custom: true, synthetic: false }])
  })

  test('pending request envelopes preserve input kind and current custom flags', () => {
    const attention = {
      kind: 'question' as const,
      payload: {
        properties: {
          id: 'same-request',
          request: {
            request_id: 'same-request',
            kind: 'user_input',
            input_kind: 'review',
            questions: [
              {
                header: 'Review',
                question: 'Proceed?',
                options: [{ label: 'Feedback', description: '', custom: true }],
                allow_custom: true,
              },
            ],
          },
        },
      },
    }
    const view = interactionPresentationFromAttention(attention)!
    expect(view.kind).toBe('review')
    expect(view.requestId).toBe('same-request')
    expect(view.questions[0]?.options[0]?.custom).toBe(true)
    expect(pendingInteractionPresentationFromAttention(attention, new Set(['same-request']))).toBeNull()
    expect(pendingInteractionPresentationFromAttention(attention, new Set())?.interaction).toEqual(view)
    expect(
      permissionPresentationFromAttention({
        kind: 'permission',
        payload: {
          properties: {
            request: {
              request_id: 'permission-id',
              kind: 'permission',
              action: { kind: 'network_access', target: 'example.test' },
            },
          },
        },
      }),
    ).toMatchObject({ requestId: 'permission-id', action: 'network example.test' })
  })

  test('retired unwrapped attention ids do not enable interactive controls', () => {
    for (const payload of [
      { id: 'old' },
      { request_id: 'old' },
      { properties: { id: 'old', questions: [{ question: 'Old' }] } },
      { request: { request_id: 'old' } },
    ]) {
      expect(pendingInteractionPresentationFromAttention({ kind: 'question', payload }, new Set())).toBeNull()
      expect(pendingInteractionPresentationFromAttention({ kind: 'permission', payload }, new Set())).toBeNull()
    }
  })
})
