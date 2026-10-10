type Usage = {
  current_tokens?: number
  projected_tokens?: number | null
  input_limit_tokens?: number | null
  limit_tokens?: number | null
}

function finiteTokens(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

export function formatTokensK(tokens: number): string {
  if (tokens <= 0) return '0k'
  const value = tokens / 1_000
  return value < 10 ? `${value.toFixed(1)}k` : `${value.toFixed(0)}k`
}

/** The server owns request admission; its current usage and input budget also own the chip. */
export function sessionContextUsage(usage: Usage) {
  const tokens = finiteTokens(usage.current_tokens) ?? finiteTokens(usage.projected_tokens)
  const limit = finiteTokens(usage.input_limit_tokens)
  const percentUsed =
    tokens !== null && limit !== null && limit > 0
      ? Math.max(0, Math.min(100, Math.round((tokens / limit) * 100)))
      : null
  return {
    tokensValue: tokens,
    tokensLabel: tokens !== null ? `${formatTokensK(tokens)} used` : '--',
    percentUsed,
    capacityLabel:
      tokens !== null && limit !== null && limit > 0
        ? `${formatTokensK(tokens)} / ${formatTokensK(limit)} tokens`
        : undefined,
  }
}
