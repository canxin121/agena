import { describe, expect, test } from 'bun:test'

import { parseUsageCommandArgs } from './usageStatsModel'

describe('parseUsageCommandArgs', () => {
  test('maps TUI periods and filters into usage route query values', () => {
    expect(parseUsageCommandArgs(['14d', '--provider', 'openai', '--model', 'gpt-5', '--no-subagents'])).toEqual({
      kind: 'open',
      query: {
        period: 'last_14_days',
        provider: 'openai',
        model: 'gpt-5',
        include_subagents: 'false',
      },
    })
    expect(parseUsageCommandArgs(['year'])).toEqual({ kind: 'open', query: { period: 'year_to_date' } })
  })

  test('rejects unsupported tokens and missing option values', () => {
    expect(parseUsageCommandArgs(['banana']).kind).toBe('error')
    expect(parseUsageCommandArgs(['--provider']).kind).toBe('error')
  })
})
