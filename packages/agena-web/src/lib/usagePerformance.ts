import type { RequestPerformance } from '@/types/usage'

function finiteNonNegative(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null
}

function tokenRate(tokens: number | null, durationMs: number | null): number | null {
  if (tokens === null || tokens <= 0 || durationMs === null || durationMs <= 0) return null
  return finiteNonNegative((tokens / durationMs) * 1000)
}

export function requestPerformanceMetrics(performance: RequestPerformance | null | undefined) {
  const durationMs = finiteNonNegative(performance?.duration_ms)
  const first = finiteNonNegative(performance?.ttft_ms)
  const ttftMs = first !== null && durationMs !== null && first <= durationMs ? first : null
  const generation = finiteNonNegative(performance?.generation_ms)
  const generationMs =
    generation !== null && ttftMs !== null && durationMs !== null && generation <= durationMs - ttftMs
      ? generation
      : null
  const tokens = finiteNonNegative(performance?.generated_tokens)
  return {
    durationMs,
    ttftMs,
    generationMs,
    tokensPerSecond: tokens !== null && tokens > 1 ? tokenRate(tokens, generationMs) : null,
    endToEndTokensPerSecond: tokenRate(tokens, durationMs),
  }
}

export function formatDurationMs(value: unknown, locale?: string): string {
  const ms = finiteNonNegative(value)
  if (ms === null) return '—'
  const seconds = ms >= 1000
  const formatted = new Intl.NumberFormat(locale, {
    maximumFractionDigits: seconds ? 2 : 0,
  }).format(seconds ? ms / 1000 : ms)
  return `${formatted} ${seconds ? 's' : 'ms'}`
}

export function formatTokenRate(value: unknown, locale?: string): string {
  const rate = finiteNonNegative(value)
  if (rate === null) return '—'
  const formatted = new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(rate)
  return `${formatted} tok/s`
}
