<script setup lang="ts">
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
    agena_mode: string
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
</script>

<template>
  <section class="card" v-if="!props.desktopEnabled">
    <h3>Desktop Runtime</h3>
    <p class="muted">Desktop features are only available in the desktop runtime.</p>
  </section>

  <template v-else>
    <section class="card">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Desktop Runtime</h3>
          <p class="muted">Inspect desktop installer/runtime status and edit backend-facing desktop config values.</p>
        </div>
        <div class="button-row" style="flex-wrap: wrap">
          <button class="button ghost" :disabled="props.loading || props.desktopSaving || props.desktopUpdateRunning" @click="props.loadDesktopPanel">Refresh</button>
          <button class="button" :disabled="props.desktopSaving || props.desktopUpdateRunning" @click="props.restartDesktopBackendAction">Restart Backend</button>
          <button class="button" :disabled="props.desktopSaving || props.desktopUpdateRunning || !props.desktopBackendUrl" @click="props.openDesktopBackendUrlAction">Open Backend URL</button>
          <button class="button" :disabled="props.desktopSaving || props.desktopUpdateRunning" @click="props.openDesktopConfigAction">Open Config</button>
          <button class="button" :disabled="props.desktopSaving" @click="props.refreshDesktopUpdateProgressAction">Refresh Update</button>
        </div>
      </div>
      <div v-if="props.desktopNotice" class="notice">{{ props.desktopNotice }}</div>

      <div class="grid two" style="margin-top: 12px">
        <section class="card">
          <h3>Runtime Facts</h3>
          <div v-if="props.desktopRuntimeFacts.length" class="stack">
            <div v-for="fact in props.desktopRuntimeFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else class="muted">Desktop runtime info is not available.</p>
        </section>

        <section class="card">
          <h3>Backend Status</h3>
          <div v-if="props.desktopStatusFacts.length" class="stack">
            <div v-for="fact in props.desktopStatusFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <div v-if="props.desktopBackendUrl" class="muted mono" style="margin-top: 12px">url={{ props.desktopBackendUrl }}</div>
          <div v-if="props.desktopBackendErrorFacts.length" class="stack" style="margin-top: 12px">
            <div><strong>Last Error</strong></div>
            <div v-for="fact in props.desktopBackendErrorFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else-if="!props.desktopStatusFacts.length" class="muted">Desktop backend status is not available.</p>
        </section>

        <section class="card">
          <h3>Update Progress</h3>
          <div v-if="props.desktopUpdateFacts.length" class="stack">
            <div v-if="props.desktopUpdateProgressPercent" class="muted mono">progress={{ props.desktopUpdateProgressPercent }}</div>
            <div v-for="fact in props.desktopUpdateFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else class="muted">No desktop update activity is currently reported.</p>
        </section>

        <section class="card">
          <h3>Resolved Config</h3>
          <div v-if="props.desktopConfigFacts.length" class="stack">
            <div v-for="fact in props.desktopConfigFacts" :key="fact.label">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <p v-else class="muted">Desktop config is not available yet.</p>
        </section>
      </div>
    </section>

    <section class="card" style="margin-top: 16px">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Desktop Update Actions</h3>
          <p class="muted">Trigger service or installer updates from explicit asset URLs and inspect the reported progress above.</p>
        </div>
      </div>

      <div class="grid two" style="margin-top: 12px">
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
        <div class="field" style="display: flex; align-items: end">
          <button class="button primary" :disabled="props.desktopUpdateRunning || !props.desktopServiceUpdateUrl.trim()" @click="props.runDesktopServiceUpdateAction">
            Start Service Update
          </button>
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

      <div class="button-row" style="margin-top: 12px">
        <button class="button primary" :disabled="props.desktopUpdateRunning || !props.desktopInstallerUpdateUrl.trim()" @click="props.runDesktopInstallerUpdateAction">
          Start Installer Update
        </button>
      </div>
    </section>

    <section class="card" style="margin-top: 16px">
      <div class="page-header" style="align-items: flex-start">
        <div>
          <h3>Edit Desktop Config</h3>
          <p class="muted">This is a first-pass editor for the most important backend and workspace fields.</p>
        </div>
        <button class="button primary" :disabled="props.desktopSaving || !props.desktopConfig || props.desktopUpdateRunning" @click="props.saveDesktopConfigAction">
          Save Desktop Config
        </button>
      </div>

      <div class="grid two" style="margin-top: 12px">
        <div class="field">
          <label class="label" for="desktop-autostart">Autostart</label>
          <select id="desktop-autostart" v-model="props.desktopForm.autostart_on_boot" class="select">
            <option :value="true">enabled</option>
            <option :value="false">disabled</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="desktop-port">Port</label>
          <input id="desktop-port" v-model="props.desktopForm.port" class="input mono" inputmode="numeric" placeholder="3210" />
        </div>
        <div class="field">
          <label class="label" for="desktop-host">Host</label>
          <input id="desktop-host" v-model="props.desktopForm.host" class="input mono" placeholder="127.0.0.1" />
        </div>
        <div class="field">
          <label class="label" for="desktop-mode">Mode</label>
          <input id="desktop-mode" v-model="props.desktopForm.agena_mode" class="input mono" placeholder="default" />
        </div>
        <div class="field">
          <label class="label" for="desktop-workspace-root">Workspace Root</label>
          <input id="desktop-workspace-root" v-model="props.desktopForm.workspace_root" class="input mono" placeholder="/workspace" />
        </div>
        <div class="field">
          <label class="label" for="desktop-config-path">Agena Config Path</label>
          <input id="desktop-config-path" v-model="props.desktopForm.agena_config_path" class="input mono" placeholder="/workspace/.agena/config.toml" />
        </div>
        <div class="field">
          <label class="label" for="desktop-database-path">Database Path</label>
          <input id="desktop-database-path" v-model="props.desktopForm.database_path" class="input mono" placeholder="/path/to/agena.db" />
        </div>
        <div class="field">
          <label class="label" for="desktop-database-url">Database URL</label>
          <input id="desktop-database-url" v-model="props.desktopForm.database_url" class="input mono" placeholder="sqlite:///..." />
        </div>
        <div class="field">
          <label class="label" for="desktop-log-level">Log Level</label>
          <input id="desktop-log-level" v-model="props.desktopForm.backend_log_level" class="input mono" placeholder="info" />
        </div>
        <div class="field">
          <label class="label" for="desktop-cookie-samesite">UI Cookie SameSite</label>
          <input id="desktop-cookie-samesite" v-model="props.desktopForm.ui_cookie_samesite" class="input mono" placeholder="lax" />
        </div>
      </div>
    </section>
  </template>
</template>
