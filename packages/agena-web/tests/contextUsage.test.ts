import { test, expect } from 'bun:test'
import { sessionContextUsage } from '../src/pages/chat/contextUsage'

test('the chip uses the admitted request count and input budget', () => {
  const result = sessionContextUsage({
    current_tokens: 616_620,
    projected_tokens: 320_000,
    input_limit_tokens: 616_000,
    limit_tokens: 596_000,
  })
  expect(result.percentUsed).toBe(100)
  expect(result.tokensValue).toBe(616_620)
  expect(result.capacityLabel).toBe('617k / 616k tokens')
})

test('the session 126 window uses its actual per-request output reservation', () => {
  expect(sessionContextUsage({ current_tokens: 616_620, input_limit_tokens: 968_000 }).percentUsed).toBe(64)
  expect(sessionContextUsage({ current_tokens: 320_001, input_limit_tokens: 968_000 }).percentUsed).toBe(33)
})

test('small windows have no hidden 12k baseline', () => {
  expect(sessionContextUsage({ current_tokens: 1_000, input_limit_tokens: 8_000 }).percentUsed).toBe(13)
  expect(sessionContextUsage({ current_tokens: 0, input_limit_tokens: 8_000 }).percentUsed).toBe(0)
})

test('missing or invalid budgets never invent a percentage', () => {
  expect(sessionContextUsage({ current_tokens: 320_000 }).percentUsed).toBeNull()
  expect(sessionContextUsage({ current_tokens: 320_000, limit_tokens: 512_000 }).percentUsed).toBeNull()
  expect(sessionContextUsage({ current_tokens: NaN, input_limit_tokens: 968_000 }).percentUsed).toBeNull()
  expect(sessionContextUsage({ current_tokens: 320_000, input_limit_tokens: 0 }).percentUsed).toBeNull()
})
