<script setup lang="ts">
import type { ToolDescriptionMode } from './runtimePageLoaders'
import type { SettingsPluginEntrySnapshot } from './runtimePageLoaders'

const props = defineProps<{
  actionError: string
  actionMessage: string
  defaultMode: ToolDescriptionMode
  enabled: boolean
  load: () => void | Promise<void>
  modeOptions: Array<{ label: string; value: ToolDescriptionMode; description: string }>
  summaryFacts: Array<{ label: string; value: string }>
  pluginEntries: SettingsPluginEntrySnapshot[]
  pluginEntrySummary: (entry: SettingsPluginEntrySnapshot) => string
  setDefaultToolDescriptionMode: (mode: ToolDescriptionMode) => void | Promise<void>
  togglePluginEntryDisabled: (entry: SettingsPluginEntrySnapshot) => void | Promise<void>
  togglePluginsEnabled: () => void | Promise<void>
}>()
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Runtime</p>
          <h3 class="settings-panel-title">Plugins</h3>
        </div>
        <button class="button ghost" @click="props.load">Refresh</button>
      </div>

      <p class="muted">
        These settings control global plugin loading and the model-visible tool description mode.
      </p>

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
          <p class="settings-panel-kicker">Controls</p>
          <h3 class="settings-panel-title">Tool Presentation</h3>
        </div>
      </div>

      <div class="button-row" style="flex-wrap: wrap">
        <button class="button primary" @click="props.togglePluginsEnabled">
          {{ props.enabled ? 'Disable Plugins' : 'Enable Plugins' }}
        </button>
      </div>

      <div class="list" style="margin-top: 16px">
        <div v-for="mode in props.modeOptions" :key="mode.value" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div class="button-row" style="align-items: center; flex-wrap: wrap">
                <strong>{{ mode.label }}</strong>
                <span v-if="props.defaultMode === mode.value" class="badge success">default</span>
              </div>
              <div class="muted">{{ mode.description }}</div>
            </div>
            <button
              class="button"
              :disabled="props.defaultMode === mode.value"
              @click="props.setDefaultToolDescriptionMode(mode.value)"
            >
              Use {{ mode.label }}
            </button>
          </div>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Plugin Entries</p>
          <h3 class="settings-panel-title">Enabled States</h3>
        </div>
      </div>

      <p class="muted">
        These are the individual plugin config entries known to the runtime or saved in the config file.
        Toggling one rewrites the full entry, keeps the config, and reloads the runtime.
      </p>

      <div v-if="props.pluginEntries.length === 0" class="muted" style="margin-top: 12px">
        No plugin entries are currently available.
      </div>

      <div v-else class="list" style="margin-top: 16px">
        <div v-for="entry in props.pluginEntries" :key="entry.pluginId" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div class="button-row" style="align-items: center; flex-wrap: wrap">
                <strong>{{ entry.pluginId }}</strong>
                <span class="badge">{{ entry.kind }}</span>
                <span class="badge">{{ entry.source }}</span>
                <span v-if="entry.disabled" class="badge warn">disabled</span>
                <span v-else class="badge success">enabled</span>
              </div>
              <div class="muted">{{ props.pluginEntrySummary(entry) }}</div>
            </div>
            <button class="button" @click="props.togglePluginEntryDisabled(entry)">
              {{ entry.disabled ? 'Enable' : 'Disable' }} Entry
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
