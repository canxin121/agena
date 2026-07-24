<script setup lang="ts">
import type { useSettingsAgentsState } from './useSettingsAgentsState'

const props = defineProps<{
  agents: ReturnType<typeof useSettingsAgentsState>
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
        <button class="button ghost" @click="props.agents.load">Refresh</button>
      </div>

      <p v-if="props.agents.actionMessage.value" class="muted">{{ props.agents.actionMessage.value }}</p>
      <p v-if="props.agents.actionError.value" class="muted">{{ props.agents.actionError.value }}</p>

      <div class="settings-summary">
        <div v-for="fact in props.agents.summaryFacts.value" :key="fact.label" class="summary-item">
          <div class="summary-label">{{ fact.label }}</div>
          <div class="summary-value">{{ fact.value }}</div>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Config-backed profile</p>
          <h3 class="settings-panel-title">Create Agent</h3>
          <p class="record-subtitle">
            Create an editable agent entry in the Agena config, then define its prompt, defaults, and permission policy.
          </p>
        </div>
      </div>
      <div class="inline-fields">
        <div class="field">
          <label class="label" for="new-agent-name">Agent name</label>
          <input
            id="new-agent-name"
            v-model="props.agents.newAgentName.value"
            class="input mono"
            placeholder="reviewer"
            @keyup.enter="props.agents.createConfigAgent"
          />
        </div>
        <button
          class="button primary"
          :disabled="props.agents.agentSaving.value || !props.agents.newAgentName.value.trim()"
          @click="props.agents.createConfigAgent"
        >
          Create Agent
        </button>
      </div>
    </section>

    <section v-if="props.agents.editor.open" id="agent-config-editor" class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agent studio</p>
          <h3 class="settings-panel-title">{{ props.agents.editor.name }}</h3>
        </div>
        <button class="button ghost" @click="props.agents.closeAgentEditor">Close</button>
      </div>
      <div class="form-grid">
        <div class="field">
          <label class="label" for="agent-description">Description</label>
          <input id="agent-description" v-model="props.agents.editor.description" class="input" />
        </div>
        <div class="field">
          <label class="label" for="agent-provider">Default provider</label>
          <input id="agent-provider" v-model="props.agents.editor.provider" class="input mono" placeholder="inherit" />
        </div>
        <div class="field">
          <label class="label" for="agent-adapter">Default adapter</label>
          <input id="agent-adapter" v-model="props.agents.editor.adapter" class="input mono" placeholder="inherit" />
        </div>
        <div class="field">
          <label class="label" for="agent-model">Default model</label>
          <input id="agent-model" v-model="props.agents.editor.model" class="input mono" placeholder="inherit" />
        </div>
        <div class="field full">
          <label class="label" for="agent-prompt">System prompt</label>
          <textarea id="agent-prompt" v-model="props.agents.editor.prompt" class="textarea" rows="10" />
        </div>
        <div class="field full">
          <label class="label" for="agent-permission">Permission policy (JSON; empty inherits)</label>
          <textarea
            id="agent-permission"
            v-model="props.agents.editor.permissionJson"
            class="textarea mono"
            rows="10"
            placeholder="{}"
          />
        </div>
      </div>
      <div class="button-row">
        <button class="button primary" :disabled="props.agents.agentSaving.value" @click="props.agents.saveAgentEditor">
          {{ props.agents.agentSaving.value ? 'Saving…' : 'Save Agent' }}
        </button>
        <button class="button" @click="props.agents.closeAgentEditor">Cancel</button>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Runtime Agent Registry</p>
          <h3 class="settings-panel-title">Profiles</h3>
        </div>
        <span class="badge">{{ props.agents.agentCards.value.length }}</span>
      </div>

      <div v-if="props.agents.agentCards.value.length" class="list">
        <div v-for="agent in props.agents.agentCards.value" :key="agent.name" class="list-item">
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
              <button
                class="button primary"
                :disabled="agent.isDefault"
                @click="props.agents.setDefaultAgent(agent.name)"
              >
                Make Default
              </button>
              <button
                class="button"
                :disabled="!props.agents.isConfigAgent(agent.name)"
                @click="props.agents.openAgentEditor(agent.name)"
              >
                {{ props.agents.isConfigAgent(agent.name) ? 'Edit Config Agent' : 'Read-only Source' }}
              </button>
            </div>
          </div>
        </div>
      </div>
      <p v-else class="muted">No agents are available in the current runtime snapshot.</p>
    </section>
  </div>
</template>
