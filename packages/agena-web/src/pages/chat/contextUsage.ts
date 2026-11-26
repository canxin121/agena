import { formatUsageCount } from '@/lib/usageFormatting'

type Usage = {
  current_tokens?: number
  projected_tokens?: number | null
  input_limit_tokens?: number | null
  limit_tokens?: number | null
  limit_basis?: string | null
}

function finiteTokens(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

/** The server owns request admission; its current usage and input budget also own the chip. */
export function sessionContextUsage(usage: Usage) {
  const tokens = finiteTokens(usage.current_tokens) ?? finiteTokens(usage.projected_tokens)
  const limit =
    finiteTokens(usage.input_limit_tokens) ??
    (usage.limit_basis === 'context_window' ? finiteTokens(usage.limit_tokens) : null)
  const percentUsed =
    tokens !== null && limit !== null && limit > 0
      ? Math.max(0, Math.min(100, Math.round((tokens / limit) * 100)))
      : null
  return {
    tokensValue: tokens,
    tokensLabel: tokens !== null ? `${formatUsageCount(tokens)} used` : '--',
    percentUsed,
    capacityLabel:
      tokens !== null && limit !== null && limit > 0
        ? `${formatUsageCount(tokens)} / ${formatUsageCount(limit)} tokens`
        : undefined,
  }
}
