<script setup lang="ts">
import type { ChatUsageSummary } from './chatUsageModel'

const props = defineProps<{
  selectedSessionId: number | null
  sessionUsageHeadline: string
  sessionUsageSummaryFacts: string[]
  sessionUsageSummary: ChatUsageSummary
  sessionUsageModelLines: Array<{
    key: string
    label: string
    facts: string[]
  }>
  formatUsageCount: (value: number) => string
  formatUsageUsd: (value: number) => string
  copySummary: () => void
}>()
</script>

<template>
  <section class="card">
    <div class="page-header" style="align-items: flex-start">
      <div>
        <h3 style="margin: 0">Usage</h3>
        <p class="muted">{{ props.sessionUsageHeadline }}</p>
      </div>
      <div class="button-row">
        <button class="button ghost" :disabled="!props.selectedSessionId" @click="props.copySummary">
          Copy Summary
        </button>
      </div>
    </div>
    <div v-if="props.sessionUsageSummaryFacts.length" class="stack">
      <div class="muted mono">summary={{ props.sessionUsageSummaryFacts.join(' · ') }}</div>
      <div class="grid two">
        <div class="field">
          <label class="label">Provider Requests</label>
          <div class="muted mono">{{ props.formatUsageCount(props.sessionUsageSummary.requests) }}</div>
        </div>
        <div class="field">
          <label class="label">Total Cost</label>
          <div class="muted mono">{{ props.formatUsageUsd(props.sessionUsageSummary.totalCostUsd) }}</div>
        </div>
        <div class="field">
          <label class="label">Input Tokens</label>
          <div class="muted mono">{{ props.formatUsageCount(props.sessionUsageSummary.inputTokens) }}</div>
        </div>
        <div class="field">
          <label class="label">Output Tokens</label>
          <div class="muted mono">{{ props.formatUsageCount(props.sessionUsageSummary.outputTokens) }}</div>
        </div>
        <div class="field">
          <label class="label">Reasoning Tokens</label>
          <div class="muted mono">{{ props.formatUsageCount(props.sessionUsageSummary.reasoningTokens) }}</div>
        </div>
        <div class="field">
          <label class="label">Cache Read / Write</label>
          <div class="muted mono">
            {{ props.formatUsageCount(props.sessionUsageSummary.cacheReadTokens) }} /
            {{ props.formatUsageCount(props.sessionUsageSummary.cacheWriteTokens) }}
          </div>
        </div>
      </div>
      <div v-if="props.sessionUsageModelLines.length" class="list">
        <div v-for="item in props.sessionUsageModelLines" :key="item.key" class="list-item">
          <div>
            <strong>{{ item.label }}</strong>
          </div>
          <div class="muted mono">{{ item.facts.join(' · ') }}</div>
        </div>
      </div>
    </div>
    <p v-else class="muted">No provider usage has been recorded for the active session yet.</p>
  </section>
</template>
