import type { MessageResource } from '@/agena/lib/agenaApi'

export type ChatUsageBreakdown = {
  providerId: string
  modelId: string
  runs: number
  inputTokens: number
  outputTokens: number
  reasoningTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  totalCostUsd: number
}

export type ChatUsageSummary = {
  runs: number
  inputTokens: number
  outputTokens: number
  reasoningTokens: number
  cacheWriteTokens: number
  cacheReadTokens: number
  totalCostUsd: number
  byModel: ChatUsageBreakdown[]
}

function readFiniteNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

function readString(value: unknown): string {
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : 'unknown'
}

export function summarizeChatUsage(messages: MessageResource[]): ChatUsageSummary {
  const summary: ChatUsageSummary = {
    runs: 0,
    inputTokens: 0,
    outputTokens: 0,
    reasoningTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    totalCostUsd: 0,
    byModel: [],
  }

  const byModel = new Map<string, ChatUsageBreakdown>()

  for (const message of messages) {
    if (message.role !== 'assistant' || !message.usage) continue

    const usage = message.usage as Record<string, unknown>
    const providerId = readString(message.metadata?.model_provider_id)
    const modelId = readString(message.metadata?.model_id)
    const key = `${providerId}::${modelId}`

    const inputTokens = readFiniteNumber(usage.input_tokens)
    const outputTokens = readFiniteNumber(usage.output_tokens)
    const reasoningTokens = readFiniteNumber(usage.reasoning_tokens)
    const cacheWriteTokens = readFiniteNumber(usage.cache_write_tokens)
    const cacheReadTokens = readFiniteNumber(usage.cache_read_tokens)
    const totalCostUsd = readFiniteNumber(usage.total_cost)

    summary.runs += 1
    summary.inputTokens += inputTokens
    summary.outputTokens += outputTokens
    summary.reasoningTokens += reasoningTokens
    summary.cacheWriteTokens += cacheWriteTokens
    summary.cacheReadTokens += cacheReadTokens
    summary.totalCostUsd += totalCostUsd

    const item = byModel.get(key) || {
      providerId,
      modelId,
      runs: 0,
      inputTokens: 0,
      outputTokens: 0,
      reasoningTokens: 0,
      cacheWriteTokens: 0,
      cacheReadTokens: 0,
      totalCostUsd: 0,
    }

    item.runs += 1
    item.inputTokens += inputTokens
    item.outputTokens += outputTokens
    item.reasoningTokens += reasoningTokens
    item.cacheWriteTokens += cacheWriteTokens
    item.cacheReadTokens += cacheReadTokens
    item.totalCostUsd += totalCostUsd
    byModel.set(key, item)
  }

  summary.byModel = [...byModel.values()].sort((left, right) => right.totalCostUsd - left.totalCostUsd)
  return summary
}

export function formatUsageCount(value: number): string {
  return Number.isFinite(value) ? Math.round(value).toLocaleString('en-US') : '0'
}

export function formatUsageUsd(value: number): string {
  return `$${(Number.isFinite(value) ? value : 0).toFixed(4)}`
}

export function chatUsageFacts(summary: ChatUsageSummary): string[] {
  if (!summary.runs) return []

  const facts = [
    `runs ${formatUsageCount(summary.runs)}`,
    `in ${formatUsageCount(summary.inputTokens)}`,
    `out ${formatUsageCount(summary.outputTokens)}`,
  ]

  if (summary.reasoningTokens > 0) {
    facts.push(`reasoning ${formatUsageCount(summary.reasoningTokens)}`)
  }
  if (summary.totalCostUsd > 0) {
    facts.push(`cost ${formatUsageUsd(summary.totalCostUsd)}`)
  }

  return facts
}

export function chatUsageBreakdownFacts(item: ChatUsageBreakdown): string[] {
  const facts = [
    `runs ${formatUsageCount(item.runs)}`,
    `in ${formatUsageCount(item.inputTokens)}`,
    `out ${formatUsageCount(item.outputTokens)}`,
  ]

  if (item.reasoningTokens > 0) {
    facts.push(`reasoning ${formatUsageCount(item.reasoningTokens)}`)
  }
  if (item.totalCostUsd > 0) {
    facts.push(`cost ${formatUsageUsd(item.totalCostUsd)}`)
  }

  return facts
}
