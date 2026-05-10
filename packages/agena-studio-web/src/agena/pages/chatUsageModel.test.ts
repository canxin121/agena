import { describe, expect, test } from 'bun:test'

import type { MessageResource } from '@/agena/lib/agenaApi'
import { chatUsageBreakdownFacts, chatUsageFacts, formatUsageCount, formatUsageUsd, summarizeChatUsage } from './chatUsageModel'

function assistantMessage(id: number, overrides?: Partial<MessageResource>): MessageResource {
  return {
    id,
    session_id: 1,
    role: 'assistant',
    state: 'complete',
    created_at: '2026-05-10T00:00:00Z',
    updated_at: '2026-05-10T00:00:00Z',
    metadata: {
      model_provider_id: 'anthropic',
      model_id: 'claude-sonnet',
    },
    usage: {
      input_tokens: 100,
      output_tokens: 50,
      reasoning_tokens: 10,
      cache_write_tokens: 5,
      cache_read_tokens: 7,
      total_cost: 0.0123,
    },
    finish: null,
    part_count: 0,
    ...overrides,
  }
}

describe('chatUsageModel', () => {
  test('summarizes assistant usage totals and breakdowns', () => {
    const summary = summarizeChatUsage([
      assistantMessage(1),
      assistantMessage(2, {
        metadata: { model_provider_id: 'openai', model_id: 'gpt-5' },
        usage: {
          input_tokens: 20,
          output_tokens: 30,
          reasoning_tokens: 0,
          cache_write_tokens: 0,
          cache_read_tokens: 0,
          total_cost: 0.004,
        },
      }),
      {
        ...assistantMessage(3),
        role: 'user',
        usage: {
          input_tokens: 999,
          output_tokens: 999,
          reasoning_tokens: 999,
          cache_write_tokens: 999,
          cache_read_tokens: 999,
          total_cost: 999,
        },
      },
    ])

    expect(summary.turns).toBe(2)
    expect(summary.inputTokens).toBe(120)
    expect(summary.outputTokens).toBe(80)
    expect(summary.reasoningTokens).toBe(10)
    expect(summary.cacheWriteTokens).toBe(5)
    expect(summary.cacheReadTokens).toBe(7)
    expect(Math.abs(summary.totalCostUsd - 0.0163) < 0.0001).toBe(true)
    expect(summary.byModel.map((item) => `${item.providerId}/${item.modelId}`)).toEqual([
      'anthropic/claude-sonnet',
      'openai/gpt-5',
    ])
  })

  test('formats summary facts for chat header', () => {
    const facts = chatUsageFacts(
      summarizeChatUsage([
        assistantMessage(1),
      ]),
    )

    expect(facts).toEqual([
      'turns 1',
      'in 100',
      'out 50',
      'reasoning 10',
      'cache read 7',
      'cache write 5',
      'cost $0.0123',
    ])
  })

  test('formats usage helpers and per-model facts', () => {
    expect(formatUsageCount(12345)).toBe('12,345')
    expect(formatUsageUsd(1.23456)).toBe('$1.2346')

    const summary = summarizeChatUsage([
      assistantMessage(1),
    ])

    expect(chatUsageBreakdownFacts(summary.byModel[0]!)).toEqual([
      'turns 1',
      'in 100',
      'out 50',
      'reasoning 10',
      'cache read 7',
      'cache write 5',
      'cost $0.0123',
    ])
  })
})
