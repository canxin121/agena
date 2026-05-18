import { describe, expect, test } from 'bun:test'

import type { UsageStats } from '@/agena/lib/agenaApi'
import {
  formatUsageCost,
  formatUsageInteger,
  formatUsagePercent,
  hasUsage,
  usageFactLine,
  usageHeadline,
} from './usageStatsModel'

const stats: UsageStats = {
  generated_at: '2026-05-18T00:00:00Z',
  period: 'last_7_days',
  period_label: 'last_7_days',
  from: '2026-05-11T00:00:00Z',
  to: '2026-05-18T00:00:00Z',
  totals: {
    turns: 3,
    sessions: 2,
    input_tokens: 1234,
    output_tokens: 567,
    reasoning_tokens: 12,
    cache_write_tokens: 50,
    cache_read_tokens: 150,
    total_tokens: 1813,
    cache_input_tokens: 1434,
    cache_hit_rate: 150 / 1434,
    total_cost_usd: 0.12345,
    recorded_cost_usd: 0.01,
    estimated_cost_usd: 0.11345,
    unpriced_turns: 0,
  },
  by_day: [],
  by_provider: [],
  by_model: [],
  by_session: [],
}

describe('usageStatsModel', () => {
  test('formats usage numbers and percentages', () => {
    expect(formatUsageInteger(12345.4)).toBe('12,345')
    expect(formatUsageCost(1.234)).toBe('$1.23')
    expect(formatUsageCost(0.1234)).toBe('$0.123')
    expect(formatUsagePercent(0.1234)).toBe('12.3%')
  })

  test('builds headline and compact fact line', () => {
    expect(usageHeadline(stats)).toContain('5/11/2026')
    expect(usageFactLine(stats.totals)).toEqual([
      'turns 3',
      'sessions 2',
      'in 1,234',
      'out 567',
      'cache 10.5%',
      'cost $0.123',
    ])
    expect(hasUsage(stats)).toBe(true)
  })
})
