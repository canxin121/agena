<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  loading: boolean
  desktopEnabled: boolean
  desktopSaving: boolean
  desktopUpdateRunning: boolean
  desktopBackendUrl: string
  desktopNotice: string
  desktopConfig: unknown
  desktopRuntimeFacts: Array<{ label: string; value: string; mono?: boolean }>
  desktopStatusFacts: Array<{ label: string; value: string; mono?: boolean }>
  desktopBackendErrorFacts: Array<{ label: string; value: string; mono?: boolean }>
  desktopUpdateFacts: Array<{ label: string; value: string; mono?: boolean }>
  desktopConfigFacts: Array<{ label: string; value: string; mono?: boolean }>
  desktopUpdateProgressPercent: string
  desktopServiceUpdateUrl: string
  desktopInstallerUpdateUrl: string
  desktopInstallerAssetName: string
  desktopForm: {
    autostart_on_boot: boolean
    host: string
    port: string
    workspace_root: string
    agena_config_path: string
    database_path: string
    database_url: string
    backend_log_level: string
    ui_cookie_samesite: string
  }
  loadDesktopPanel: () => void | Promise<void>
  restartDesktopBackendAction: () => void | Promise<void>
  openDesktopBackendUrlAction: () => void | Promise<void>
  openDesktopConfigAction: () => void | Promise<void>
  refreshDesktopUpdateProgressAction: () => void | Promise<void>
  runDesktopServiceUpdateAction: () => void | Promise<void>
  runDesktopInstallerUpdateAction: () => void | Promise<void>
  saveDesktopConfigAction: () => void | Promise<void>
}>()

const emit = defineEmits<{
  'update:desktopServiceUpdateUrl': [value: string]
  'update:desktopInstallerUpdateUrl': [value: string]
  'update:desktopInstallerAssetName': [value: string]
}>()

const updateProgressWidth = computed(() => props.desktopUpdateProgressPercent || '0%')
const backendState = computed(() => {
  const runningFact = props.desktopStatusFacts.find((fact) => fact.label.toLowerCase() === 'running')
  if (!runningFact) return props.desktopBackendUrl ? 'available' : 'unknown'
  return runningFact.value === 'yes' || runningFact.value === 'true' ? 'running' : 'stopped'
})
</script>

<template>
  <section v-if="!props.desktopEnabled" class="settings-panel">
    <div>
      <p class="settings-panel-kicker">Agena Desktop</p>
      <h3 class="settings-panel-title">Desktop Runtime</h3>
    </div>
    <div class="empty-state">Desktop controls are available only inside the Agena desktop runtime.</div>
  </section>

  <div v-else class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Desktop</p>
          <h3 class="settings-panel-title">Runtime Control</h3>
        </div>
        <div class="button-row">
          <button
            class="button ghost"
            :disabled="props.loading || props.desktopSaving || props.desktopUpdateRunning"
            @click="props.loadDesktopPanel"
          >
            Refresh
          </button>
          <button
            class="button"
            :disabled="props.desktopSaving || props.desktopUpdateRunning"
            @click="props.restartDesktopBackendAction"
          >
            Restart Backend
          </button>
          <button
            class="button"
            :disabled="props.desktopSaving || props.desktopUpdateRunning || !props.desktopBackendUrl"
            @click="props.openDesktopBackendUrlAction"
          >
            Open Backend
          </button>
          <button
            class="button"
            :disabled="props.desktopSaving || props.desktopUpdateRunning"
            @click="props.openDesktopConfigAction"
          >
            Open Config
          </button>
        </div>
      </div>

      <div v-if="props.desktopNotice" class="notice">{{ props.desktopNotice }}</div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Backend</div>
          <div class="summary-value">{{ backendState }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">URL</div>
          <div class="summary-value mono">{{ props.desktopBackendUrl || 'n/a' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Config</div>
          <div class="summary-value">{{ props.desktopConfig ? 'loaded' : 'missing' }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Update</div>
          <div class="summary-value">
            {{ props.desktopUpdateRunning ? 'running' : props.desktopUpdateProgressPercent || 'idle' }}
          </div>
        </div>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Status</p>
          <h3 class="settings-panel-title">Runtime Facts</h3>
        </div>
        <button class="button" :disabled="props.desktopSaving" @click="props.refreshDesktopUpdateProgressAction">
          Refresh Update
        </button>
      </div>

      <div class="facts-grid">
        <div v-for="fact in props.desktopRuntimeFacts" :key="`runtime-${fact.label}`" class="fact-row">
          <div class="fact-label">{{ fact.label }}</div>
          <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
        </div>
        <div v-for="fact in props.desktopStatusFacts" :key="`status-${fact.label}`" class="fact-row">
          <div class="fact-label">{{ fact.label }}</div>
          <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
        </div>
        <div v-for="fact in props.desktopBackendErrorFacts" :key="`error-${fact.label}`" class="fact-row danger-zone">
          <div class="fact-label">{{ fact.label }}</div>
          <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
        </div>
      </div>

      <div
        v-if="
          !props.desktopRuntimeFacts.length &&
          !props.desktopStatusFacts.length &&
          !props.desktopBackendErrorFacts.length
        "
        class="empty-state"
      >
        Desktop runtime status is not available yet.
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Updates</p>
          <h3 class="settings-panel-title">Service and Installer</h3>
        </div>
      </div>

      <div class="progress-track">
        <div class="progress-fill" :style="{ width: updateProgressWidth }" />
      </div>

      <div v-if="props.desktopUpdateFacts.length" class="facts-grid">
        <div v-for="fact in props.desktopUpdateFacts" :key="fact.label" class="fact-row">
          <div class="fact-label">{{ fact.label }}</div>
          <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
        </div>
      </div>

      <div class="form-grid">
        <div class="field">
          <label class="label" for="desktop-service-update-url">Service Asset URL</label>
          <input
            id="desktop-service-update-url"
            :value="props.desktopServiceUpdateUrl"
            class="input mono"
            placeholder="https://example.com/agena-service.tgz"
            @input="emit('update:desktopServiceUpdateUrl', ($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-installer-update-url">Installer Asset URL</label>
          <input
            id="desktop-installer-update-url"
            :value="props.desktopInstallerUpdateUrl"
            class="input mono"
            placeholder="https://example.com/agena-installer.AppImage"
            @input="emit('update:desktopInstallerUpdateUrl', ($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-installer-asset-name">Installer Asset Name</label>
          <input
            id="desktop-installer-asset-name"
            :value="props.desktopInstallerAssetName"
            class="input mono"
            placeholder="agena-installer.AppImage"
            @input="emit('update:desktopInstallerAssetName', ($event.target as HTMLInputElement).value)"
          />
        </div>
      </div>

      <div class="button-row">
        <button
          class="button primary"
          :disabled="props.desktopUpdateRunning || !props.desktopServiceUpdateUrl.trim()"
          @click="props.runDesktopServiceUpdateAction"
        >
          Update Service
        </button>
        <button
          class="button primary"
          :disabled="props.desktopUpdateRunning || !props.desktopInstallerUpdateUrl.trim()"
          @click="props.runDesktopInstallerUpdateAction"
        >
          Update Installer
        </button>
      </div>
    </section>

    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Configuration</p>
          <h3 class="settings-panel-title">Backend Settings</h3>
        </div>
        <button
          class="button primary"
          :disabled="props.desktopSaving || !props.desktopConfig || props.desktopUpdateRunning"
          @click="props.saveDesktopConfigAction"
        >
          Save Config
        </button>
      </div>

      <div v-if="props.desktopConfigFacts.length" class="facts-grid">
        <div v-for="fact in props.desktopConfigFacts" :key="fact.label" class="fact-row">
          <div class="fact-label">{{ fact.label }}</div>
          <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
        </div>
      </div>

      <div class="form-grid three">
        <div class="field">
          <label class="label" for="desktop-autostart">Autostart</label>
          <select id="desktop-autostart" v-model="props.desktopForm.autostart_on_boot" class="select">
            <option :value="true">Enabled</option>
            <option :value="false">Disabled</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="desktop-host">Host</label>
          <input id="desktop-host" v-model="props.desktopForm.host" class="input mono" placeholder="127.0.0.1" />
        </div>
        <div class="field">
          <label class="label" for="desktop-port">Port</label>
          <input
            id="desktop-port"
            v-model="props.desktopForm.port"
            class="input mono"
            inputmode="numeric"
            placeholder="3210"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-workspace-root">Workspace Root</label>
          <input
            id="desktop-workspace-root"
            v-model="props.desktopForm.workspace_root"
            class="input mono"
            placeholder="/workspace"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-config-path">Agena Config Path</label>
          <input
            id="desktop-config-path"
            v-model="props.desktopForm.agena_config_path"
            class="input mono"
            placeholder="/workspace/.agena/config.toml"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-database-path">Database Path</label>
          <input
            id="desktop-database-path"
            v-model="props.desktopForm.database_path"
            class="input mono"
            placeholder="/workspace/agena.db"
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-database-url">Database URL</label>
          <input
            id="desktop-database-url"
            v-model="props.desktopForm.database_url"
            class="input mono"
            placeholder="sqlite:///..."
          />
        </div>
        <div class="field">
          <label class="label" for="desktop-log-level">Log Level</label>
          <select id="desktop-log-level" v-model="props.desktopForm.backend_log_level" class="select">
            <option value="trace">trace</option>
            <option value="debug">debug</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="desktop-cookie-samesite">UI Cookie SameSite</label>
          <select id="desktop-cookie-samesite" v-model="props.desktopForm.ui_cookie_samesite" class="select">
            <option value="lax">lax</option>
            <option value="strict">strict</option>
            <option value="none">none</option>
          </select>
        </div>
      </div>
    </section>
  </div>
</template>
