import type { UsageStats, UsageTotals } from '@/agena/lib/agenaApi'

export type UsagePeriodOption = {
  id: UsageStats['period']
  label: string
}

export const usagePeriodOptions: UsagePeriodOption[] = [
  { id: 'today', label: 'Today' },
  { id: 'last_7_days', label: '7 Days' },
  { id: 'last_30_days', label: '30 Days' },
  { id: 'month_to_date', label: 'Month' },
  { id: 'all_time', label: 'All' },
]

function finite(value: number): number {
  return Number.isFinite(value) ? value : 0
}

export function formatUsageInteger(value: number): string {
  return Math.round(finite(value)).toLocaleString('en-US')
}

export function formatUsageCost(value: number): string {
  const normalized = finite(value)
  if (normalized >= 1) return `$${normalized.toFixed(2)}`
  if (normalized >= 0.01) return `$${normalized.toFixed(3)}`
  return `$${normalized.toFixed(4)}`
}

export function formatUsagePercent(value: number): string {
  return `${(finite(value) * 100).toFixed(1)}%`
}

export function usageHeadline(stats: UsageStats | null): string {
  if (!stats) return 'No usage data loaded'
  const from = stats.from ? new Date(stats.from).toLocaleDateString() : 'first record'
  const to = stats.to ? new Date(stats.to).toLocaleDateString() : 'now'
  return `${from} to ${to}`
}

export function usageFactLine(totals: UsageTotals): string[] {
  return [
    `runs ${formatUsageInteger(totals.runs)}`,
    `sessions ${formatUsageInteger(totals.sessions)}`,
    `in ${formatUsageInteger(totals.input_tokens)}`,
    `out ${formatUsageInteger(totals.output_tokens)}`,
    `cache ${formatUsagePercent(totals.cache_hit_rate)}`,
    `cost ${formatUsageCost(totals.total_cost_usd)}`,
  ]
}

export function hasUsage(stats: UsageStats | null): boolean {
  return Boolean(stats && stats.totals.runs > 0)
}
