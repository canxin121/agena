<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'

import {
  deleteProviderCredential,
  fetchRuntimeStatus,
  listAuthProviders,
  listProviders,
  refreshProviderCredential,
  reloadRuntime,
  setProviderApiKey,
  type AuthProvider,
  type ProviderSummary,
  type RuntimeStatus,
} from '@/agena/lib/agenaApi'

const runtime = ref<RuntimeStatus | null>(null)
const providers = ref<ProviderSummary[]>([])
const authProviders = ref<AuthProvider[]>([])
const loading = ref(false)
const actionError = ref('')
const actionMessage = ref('')
const drafts = reactive<Record<string, string>>({})

async function load() {
  loading.value = true
  actionError.value = ''
  try {
    const [runtimeData, providerData, authData] = await Promise.all([
      fetchRuntimeStatus(),
      listProviders(),
      listAuthProviders(),
    ])
    runtime.value = runtimeData
    providers.value = providerData
    authProviders.value = authData
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    loading.value = false
  }
}

async function triggerReload() {
  actionMessage.value = ''
  actionError.value = ''
  try {
    const result = await reloadRuntime()
    actionMessage.value = `Runtime reloaded to generation ${result.generation}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function saveApiKey(providerId: string) {
  const apiKey = String(drafts[providerId] || '').trim()
  if (!apiKey) return
  actionMessage.value = ''
  actionError.value = ''
  try {
    await setProviderApiKey(providerId, apiKey)
    drafts[providerId] = ''
    actionMessage.value = `Saved API key for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function clearCredential(providerId: string) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await deleteProviderCredential(providerId)
    actionMessage.value = `Cleared credential for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

async function refreshCredential(providerId: string) {
  actionMessage.value = ''
  actionError.value = ''
  try {
    await refreshProviderCredential(providerId)
    actionMessage.value = `Requested credential refresh for ${providerId}.`
    await load()
  } catch (err) {
    actionError.value = err instanceof Error ? err.message : String(err)
  }
}

onMounted(() => {
  void load()
})
</script>

<template>
  <section class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Runtime</h1>
        <p class="page-description">Inspect the loaded agena config, providers, and credential state.</p>
      </div>
      <div class="button-row">
        <button class="button ghost" :disabled="loading" @click="load">Refresh</button>
        <button class="button primary" :disabled="loading" @click="triggerReload">Reload Runtime</button>
      </div>
    </header>

    <div v-if="actionError" class="notice">{{ actionError }}</div>
    <div v-else-if="actionMessage" class="notice">{{ actionMessage }}</div>

    <div class="grid two">
      <section class="card">
        <h3>Runtime Snapshot</h3>
        <div v-if="runtime" class="stack">
          <div><strong>Generation:</strong> {{ runtime.generation }}</div>
          <div><strong>Loaded At:</strong> {{ runtime.loaded_at }}</div>
          <div><strong>Workspace Root:</strong> <span class="mono">{{ runtime.workspace_root }}</span></div>
          <div><strong>Config Path:</strong> <span class="mono">{{ runtime.config_path }}</span></div>
          <div><strong>Active Mode:</strong> {{ runtime.active_mode || 'default' }}</div>
          <div><strong>Providers:</strong> {{ runtime.provider_ids.join(', ') || 'none' }}</div>
          <div><strong>Plugin Count:</strong> {{ runtime.plugin_count }}</div>
          <div><strong>Session Runtime:</strong> {{ runtime.session_runtime_available ? 'enabled' : 'disabled' }}</div>
        </div>
        <p v-else class="muted">Loading runtime snapshot…</p>
      </section>

      <section class="card">
        <h3>Provider Defaults</h3>
        <div v-if="providers.length" class="list">
          <div v-for="provider in providers" :key="provider.provider_id" class="list-item">
            <div><strong>{{ provider.provider_id }}</strong></div>
            <div class="muted">Default model: {{ provider.default_model }}</div>
            <div class="muted mono">{{ provider.default_model_ref }}</div>
          </div>
        </div>
        <p v-else class="muted">No providers loaded.</p>
      </section>
    </div>

    <section class="card">
      <h3>Credentials</h3>
      <div v-if="authProviders.length" class="list">
        <div v-for="provider in authProviders" :key="provider.provider_id" class="list-item">
          <div class="page-header" style="align-items: flex-start">
            <div>
              <div><strong>{{ provider.provider_id }}</strong></div>
              <div class="muted">
                configured={{ provider.configured ? 'yes' : 'no' }}, credential={{
                  provider.credential_present ? 'present' : 'missing'
                }}
              </div>
              <div v-if="provider.key_preview" class="muted mono">{{ provider.key_preview }}</div>
              <div v-if="provider.expires_at" class="muted">expires {{ provider.expires_at }}</div>
            </div>
            <span class="badge">{{ provider.credential_type || 'unknown' }}</span>
          </div>

          <div class="field" style="margin-top: 12px">
            <label class="label" :for="`api-key-${provider.provider_id}`">API Key</label>
            <input
              :id="`api-key-${provider.provider_id}`"
              v-model="drafts[provider.provider_id]"
              class="input mono"
              type="password"
              placeholder="sk-..."
            />
          </div>

          <div class="button-row" style="margin-top: 12px">
            <button class="button primary" @click="saveApiKey(provider.provider_id)">Save API Key</button>
            <button class="button" @click="refreshCredential(provider.provider_id)">Refresh</button>
            <button class="button danger" @click="clearCredential(provider.provider_id)">Delete</button>
          </div>
        </div>
      </div>
      <p v-else class="muted">No auth-capable providers were exposed by the runtime.</p>
    </section>
  </section>
</template>
