<script setup lang="ts">
import type { PluginInspect, PluginLogEntry, PluginStatus } from '@/agena/lib/agenaApi'

const props = defineProps<{
  plugins: PluginStatus[]
  selectedPlugin: PluginInspect | null
  pluginLogs: PluginLogEntry[]
  pluginLoading: boolean
  loadPluginDetails: (pluginId: string) => void | Promise<void>
  openPluginManifestInWorkspace: () => void
  openPluginLogsWorkspacePath: () => void
}>()
</script>

<template>
  <div class="grid two">
    <section class="card">
      <h3>Plugins</h3>
      <div v-if="props.plugins.length" class="list">
        <button
          v-for="plugin in props.plugins"
          :key="plugin.plugin_id"
          class="list-item"
          style="width: 100%; text-align: left"
          @click="props.loadPluginDetails(plugin.plugin_id)"
        >
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ plugin.plugin_id }}</strong></div>
              <div class="muted">{{ plugin.kind }} · {{ plugin.state }}</div>
              <div v-if="plugin.last_error" class="muted">{{ plugin.last_error }}</div>
            </div>
            <span class="badge">restarts {{ plugin.restart_count }}</span>
          </div>
        </button>
      </div>
      <p v-else class="muted">No configured plugins.</p>
    </section>

    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Plugin Detail</h3>
          <p class="muted">Inspect manifest and recent logs, then jump to the corresponding workspace config areas.</p>
        </div>
        <div class="button-row" style="flex-wrap: wrap">
          <button class="button" :disabled="!props.selectedPlugin" @click="props.openPluginManifestInWorkspace">Open Plugin Dir</button>
          <button class="button" :disabled="!props.selectedPlugin" @click="props.openPluginLogsWorkspacePath">Open Logs Dir</button>
        </div>
      </div>
      <div v-if="props.selectedPlugin" class="stack">
        <div><strong>ID:</strong> {{ props.selectedPlugin.status.plugin_id }}</div>
        <div><strong>Kind:</strong> {{ props.selectedPlugin.status.kind }}</div>
        <div><strong>State:</strong> {{ props.selectedPlugin.status.state }}</div>
        <div><strong>PID:</strong> {{ props.selectedPlugin.status.pid ?? 'n/a' }}</div>
        <div><strong>Last Exit:</strong> {{ props.selectedPlugin.status.last_exit_code ?? 'n/a' }}</div>
        <div><strong>Last Restart:</strong> {{ props.selectedPlugin.status.last_restart_at_ms ?? 'n/a' }}</div>
        <div><strong>Manifest:</strong></div>
        <pre class="mono" style="white-space: pre-wrap">{{ JSON.stringify(props.selectedPlugin.manifest ?? {}, null, 2) }}</pre>
        <div><strong>Recent Logs:</strong></div>
        <div v-if="props.pluginLogs.length" class="list">
          <div v-for="entry in props.pluginLogs" :key="entry.seq" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <strong>#{{ entry.seq }}</strong>
              <span class="badge">{{ entry.level }}</span>
            </div>
            <div class="muted">{{ entry.target || 'plugin' }} · {{ entry.timestamp_ms }}</div>
            <div class="muted">{{ entry.message }}</div>
          </div>
        </div>
        <div v-else class="muted">No retained logs.</div>
      </div>
      <p v-else-if="props.pluginLoading" class="muted">Loading plugin detail…</p>
      <p v-else class="muted">Select a plugin to inspect.</p>
    </section>
  </div>
</template>
