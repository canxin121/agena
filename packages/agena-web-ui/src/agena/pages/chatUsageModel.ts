import type { MessageResource } from '@/agena/lib/agenaApi'

export type ChatUsageBreakdown = {
  providerId: string
  modelId: string
  requests: number
  /** Compatibility alias; both fields count provider requests. */
  runs: number
  inputTokens: number
  outputTokens: number
  reasoningTokens: number
  cacheWriteTokens: number
  cacheWrite5mTokens: number
  cacheWrite1hTokens: number
  cacheReadTokens: number
  toolUseTokens: number
  otherTokens: number
  recordedCostUsd: number
  estimatedCostUsd: number
  unpricedRequests: number
  totalCostUsd: number
}

export type ChatUsageSummary = Omit<ChatUsageBreakdown, 'providerId' | 'modelId'> & {
  byModel: ChatUsageBreakdown[]
}

function readFiniteNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function readString(value: unknown): string {
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : 'unknown'
}

function readUsage(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function ownCost(usage: Record<string, unknown>): {
  total: number
  recorded: number
  estimated: number
  unpriced: number
} {
  const recordedAvailable = usage.recorded_cost_available === true
  const recorded = recordedAvailable
    ? readFiniteNumber(usage.recorded_cost)
    : readFiniteNumber(usage.total_cost) > 0
      ? readFiniteNumber(usage.total_cost)
      : 0
  const estimated = recordedAvailable || recorded > 0 ? 0 : readFiniteNumber(usage.estimated_cost)
  const incomplete = usage.cost_estimate_incomplete === true
  return { total: recorded + estimated, recorded, estimated, unpriced: incomplete ? 1 : 0 }
}

function hasOwnUsage(usage: Record<string, unknown>): boolean {
  const numeric = [
    'requests',
    'input_tokens',
    'output_tokens',
    'reasoning_tokens',
    'cache_write_tokens',
    'cache_write_5m_tokens',
    'cache_write_1h_tokens',
    'cache_read_tokens',
    'tool_use_tokens',
    'other_tokens',
    'total_cost',
    'recorded_cost',
    'estimated_cost',
  ]
  return (
    numeric.some((key) => readFiniteNumber(usage[key]) > 0) ||
    usage.recorded_cost_available === true ||
    usage.cost_estimate_incomplete === true ||
    (Array.isArray(usage.billable_items) && usage.billable_items.length > 0)
  )
}

function blank(providerId = 'all', modelId = 'all'): ChatUsageBreakdown {
  return {
    providerId,
    modelId,
    requests: 0,
    runs: 0,
    inputTokens: 0,
    outputTokens: 0,
    reasoningTokens: 0,
    cacheWriteTokens: 0,
    cacheWrite5mTokens: 0,
    cacheWrite1hTokens: 0,
    cacheReadTokens: 0,
    toolUseTokens: 0,
    otherTokens: 0,
    recordedCostUsd: 0,
    estimatedCostUsd: 0,
    unpricedRequests: 0,
    totalCostUsd: 0,
  }
}

function fold(target: ChatUsageBreakdown, usage: Record<string, unknown>) {
  const cost = ownCost(usage)
  const requests = Math.max(readFiniteNumber(usage.requests), 1)
  target.requests += requests
  target.runs += requests
  target.inputTokens += readFiniteNumber(usage.input_tokens)
  target.outputTokens += readFiniteNumber(usage.output_tokens)
  target.reasoningTokens += readFiniteNumber(usage.reasoning_tokens)
  target.cacheWriteTokens += readFiniteNumber(usage.cache_write_tokens)
  target.cacheWrite5mTokens += readFiniteNumber(usage.cache_write_5m_tokens)
  target.cacheWrite1hTokens += readFiniteNumber(usage.cache_write_1h_tokens)
  target.cacheReadTokens += readFiniteNumber(usage.cache_read_tokens)
  target.toolUseTokens += readFiniteNumber(usage.tool_use_tokens)
  target.otherTokens += readFiniteNumber(usage.other_tokens)
  target.recordedCostUsd += cost.recorded
  target.estimatedCostUsd += cost.estimated
  target.unpricedRequests += cost.unpriced
  target.totalCostUsd += cost.total
}

export function summarizeChatUsage(messages: MessageResource[]): ChatUsageSummary {
  const total = blank()
  const byModel = new Map<string, ChatUsageBreakdown>()

  function visit(providerId: string, modelId: string, raw: unknown) {
    const usage = readUsage(raw)
    if (!usage) return
    if (hasOwnUsage(usage)) {
      fold(total, usage)
      const key = `${providerId}::${modelId}`
      const item = byModel.get(key) || blank(providerId, modelId)
      fold(item, usage)
      byModel.set(key, item)
    }
    const attributed = Array.isArray(usage.attributed_usage) ? usage.attributed_usage : []
    for (const entry of attributed) {
      const value = readUsage(entry)
      if (!value) continue
      visit(readString(value.provider_id), readString(value.model_id), value.usage)
    }
  }

  for (const message of messages) {
    if (message.role !== 'assistant' || !message.usage) continue
    visit(readString(message.metadata?.model_provider_id), readString(message.metadata?.model_id), message.usage)
  }
  return {
    ...total,
    byModel: [...byModel.values()].sort((left, right) => right.totalCostUsd - left.totalCostUsd),
  }
}

export function formatUsageCount(value: number): string {
  return Number.isFinite(value) ? Math.round(value).toLocaleString('en-US') : '0'
}

export function formatUsageUsd(value: number): string {
  return `$${(Number.isFinite(value) ? value : 0).toFixed(4)}`
}

export function chatUsageFacts(summary: ChatUsageSummary): string[] {
  if (!summary.requests) return []
  const facts = [
    `requests ${formatUsageCount(summary.requests)}`,
    `in ${formatUsageCount(summary.inputTokens)}`,
    `out ${formatUsageCount(summary.outputTokens)}`,
  ]
  if (summary.reasoningTokens > 0) facts.push(`reasoning ${formatUsageCount(summary.reasoningTokens)}`)
  if (summary.cacheReadTokens > 0) facts.push(`cache read ${formatUsageCount(summary.cacheReadTokens)}`)
  if (summary.totalCostUsd > 0) facts.push(`cost ${formatUsageUsd(summary.totalCostUsd)}`)
  if (summary.unpricedRequests > 0) facts.push(`unpriced ${formatUsageCount(summary.unpricedRequests)}`)
  return facts
}

export function chatUsageBreakdownFacts(item: ChatUsageBreakdown): string[] {
  return chatUsageFacts({ ...item, byModel: [] })
}
