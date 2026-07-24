import type { UsageStats, UsageTotals } from '@/agena/lib/agenaApi'

export type UsagePeriodOption = {
  id: UsageStats['period']
  label: string
}

export const usagePeriodOptions: UsagePeriodOption[] = [
  { id: 'today', label: 'Today' },
  { id: 'yesterday', label: 'Yesterday' },
  { id: 'last_7_days', label: '7 Days' },
  { id: 'last_14_days', label: '14 Days' },
  { id: 'last_30_days', label: '30 Days' },
  { id: 'last_90_days', label: '90 Days' },
  { id: 'month_to_date', label: 'Month' },
  { id: 'year_to_date', label: 'Year' },
  { id: 'all_time', label: 'All' },
]

export type UsageCommandPlan = { kind: 'open'; query: Record<string, string> } | { kind: 'error'; message: string }

const usagePeriodAliases: Record<string, UsageStats['period']> = {
  today: 'today',
  '1d': 'today',
  yesterday: 'yesterday',
  yd: 'yesterday',
  week: 'last_7_days',
  '7d': 'last_7_days',
  '2w': 'last_14_days',
  '14d': 'last_14_days',
  '30d': 'last_30_days',
  '90d': 'last_90_days',
  month: 'month_to_date',
  mtd: 'month_to_date',
  year: 'year_to_date',
  ytd: 'year_to_date',
  all: 'all_time',
  'all-time': 'all_time',
}

const usageCommandHelp =
  'Usage: /usage [today|yesterday|7d|14d|30d|90d|month|year|all] [--provider ID] [--model ID] [--no-subagents]'

export function parseUsageCommandArgs(args: string[]): UsageCommandPlan {
  const query: Record<string, string> = { period: 'last_7_days' }
  let index = 0
  while (index < args.length) {
    const token = (args[index] || '').toLowerCase()
    const period = usagePeriodAliases[token]
    if (period) {
      query.period = period
      index += 1
      continue
    }
    if (token === '--no-subagents') {
      query.include_subagents = 'false'
      index += 1
      continue
    }
    if (token === '--provider' || token === '-p' || token === '--model' || token === '-m') {
      const value = args[index + 1]?.trim()
      if (!value) return { kind: 'error', message: usageCommandHelp }
      query[token === '--provider' || token === '-p' ? 'provider' : 'model'] = value
      index += 2
      continue
    }
    return { kind: 'error', message: usageCommandHelp }
  }
  return { kind: 'open', query }
}

export function isUsagePeriod(value: string): value is UsageStats['period'] {
  return usagePeriodOptions.some((option) => option.id === value)
}

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
