<script setup lang="ts">
import type { SettingsAgentCard } from './useSettingsAgentsState'

const props = defineProps<{
  actionError: string
  actionMessage: string
  agentCards: SettingsAgentCard[]
  load: () => void | Promise<void>
  summaryFacts: Array<{ label: string; value: string }>
  setDefaultAgent: (agentName: string) => void | Promise<void>
}>()
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Runtime</p>
          <h3 class="settings-panel-title">Agents</h3>
        </div>
        <button class="button ghost" @click="props.load">Refresh</button>
      </div>

      <p v-if="props.actionMessage" class="muted">{{ props.actionMessage }}</p>
      <p v-if="props.actionError" class="muted">{{ props.actionError }}</p>

      <div class="settings-summary">
        <div v-for="fact in props.summaryFacts" :key="fact.label" class="summary-item">
          <div class="summary-label">{{ fact.label }}</div>
          <div class="summary-value">{{ fact.value }}</div>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Runtime Agent Registry</p>
          <h3 class="settings-panel-title">Profiles</h3>
        </div>
        <span class="badge">{{ props.agentCards.length }}</span>
      </div>

      <div v-if="props.agentCards.length" class="list">
        <div v-for="agent in props.agentCards" :key="agent.name" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div class="button-row" style="align-items: center; flex-wrap: wrap">
                <strong>{{ agent.name }}</strong>
                <span v-if="agent.isDefault" class="badge success">default</span>
                <span class="badge">{{ agent.scope }}</span>
              </div>
              <div class="muted">{{ agent.description }}</div>
              <div class="muted mono">source={{ agent.sourcePath || 'runtime built-in / config' }}</div>
              <div class="muted mono">{{ agent.detailFacts.join(' · ') }}</div>
              <div class="muted mono">permission={{ agent.permissionSummary }}</div>
              <div class="muted mono">defaults={{ agent.defaultSummary }}</div>
            </div>
            <div class="button-row" style="flex-wrap: wrap">
              <button class="button primary" :disabled="agent.isDefault" @click="props.setDefaultAgent(agent.name)">
                Make Default
              </button>
            </div>
          </div>
        </div>
      </div>
      <p v-else class="muted">No agents are available in the current runtime snapshot.</p>
    </section>
  </div>
</template>
