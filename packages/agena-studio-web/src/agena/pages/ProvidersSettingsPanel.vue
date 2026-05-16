<script setup lang="ts">
import { computed } from 'vue'

import { buildAuthProviderFacts } from './runtimePageModel'
import type { AuthBrowserStartResponse, AuthDeviceStartResponse, AuthProvider } from '@/agena/lib/agenaApi'

const props = defineProps<{
  authProviders: AuthProvider[]
  browserAuthCodeDrafts: Record<string, string>
  browserAuthInstanceDrafts: Record<string, string>
  browserAuthStartState: Record<string, AuthBrowserStartResponse | null>
  deviceAuthEnterpriseDrafts: Record<string, string>
  deviceAuthStartState: Record<string, AuthDeviceStartResponse | null>
  drafts: Record<string, string>
  finishBrowserAuth: (providerId: string) => void | Promise<void>
  pollDeviceAuth: (providerId: string) => void | Promise<void>
  saveApiKey: (providerId: string) => void | Promise<void>
  refreshCredential: (providerId: string) => void | Promise<void>
  clearCredential: (providerId: string) => void | Promise<void>
  startBrowserAuth: (providerId: string) => void | Promise<void>
  startDeviceAuth: (providerId: string) => void | Promise<void>
}>()

const connectedCount = computed(
  () => props.authProviders.filter((provider) => provider.credential_present && !provider.expired).length,
)
const expiredCount = computed(() => props.authProviders.filter((provider) => provider.expired).length)
const browserFlowCount = computed(
  () => props.authProviders.filter((provider) => supportsBrowserLogin(provider.provider_id)).length,
)
const deviceFlowCount = computed(
  () => props.authProviders.filter((provider) => supportsDeviceLogin(provider.provider_id)).length,
)

function providerName(providerId: string) {
  return providerId
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
}

function credentialBadgeClass(provider: AuthProvider) {
  if (provider.expired) return 'danger'
  if (provider.credential_present) return 'success'
  if (provider.configured) return 'warn'
  return 'neutral'
}

function credentialLabel(provider: AuthProvider) {
  if (provider.expired) return 'expired'
  if (provider.credential_present) return 'connected'
  if (provider.configured) return 'configured'
  return 'not configured'
}

function supportsBrowserLogin(providerId: string) {
  return providerId === 'openai' || providerId === 'gitlab'
}

function supportsDeviceLogin(providerId: string) {
  return providerId === 'openai' || providerId === 'github-copilot'
}
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Agena Runtime</p>
          <h3 class="settings-panel-title">Provider Auth</h3>
        </div>
      </div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Providers</div>
          <div class="summary-value">{{ props.authProviders.length }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Connected</div>
          <div class="summary-value">{{ connectedCount }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Expired</div>
          <div class="summary-value">{{ expiredCount }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Login Flows</div>
          <div class="summary-value">{{ browserFlowCount }} browser · {{ deviceFlowCount }} device</div>
        </div>
      </div>
    </section>

    <section v-if="props.authProviders.length" class="record-list">
      <article v-for="provider in props.authProviders" :key="provider.provider_id" class="record-card">
        <div class="record-header">
          <div>
            <p class="settings-panel-kicker">{{ provider.provider_id }}</p>
            <h3 class="record-title">{{ providerName(provider.provider_id) }}</h3>
            <div class="record-subtitle">
              {{ provider.key_preview || provider.account_id || provider.enterprise_url || 'Credential not saved' }}
            </div>
          </div>
          <div class="record-meta">
            <span class="badge" :class="credentialBadgeClass(provider)">
              <span class="status-dot" :class="credentialBadgeClass(provider)" />
              {{ credentialLabel(provider) }}
            </span>
            <span class="badge neutral">{{ provider.credential_type || 'credential' }}</span>
          </div>
        </div>

        <div class="facts-grid">
          <div v-for="fact in buildAuthProviderFacts(provider)" :key="fact.label" class="fact-row">
            <div class="fact-label">{{ fact.label }}</div>
            <div class="fact-value" :class="{ mono: fact.mono }">{{ fact.value }}</div>
          </div>
        </div>

        <div class="inline-fields">
          <div class="field">
            <label class="label" :for="`api-key-${provider.provider_id}`">API Key</label>
            <input
              :id="`api-key-${provider.provider_id}`"
              v-model="props.drafts[provider.provider_id]"
              class="input mono"
              type="password"
              placeholder="sk-..."
            />
          </div>
          <div class="button-row">
            <button class="button primary" @click="props.saveApiKey(provider.provider_id)">Save Key</button>
            <button class="button" @click="props.refreshCredential(provider.provider_id)">Refresh</button>
            <button class="button danger" @click="props.clearCredential(provider.provider_id)">Delete</button>
          </div>
        </div>

        <div v-if="supportsBrowserLogin(provider.provider_id)" class="record-section">
          <div class="settings-panel-header">
            <div>
              <p class="settings-panel-kicker">Browser Login</p>
              <h4 class="settings-panel-title">
                {{ provider.provider_id === 'gitlab' ? 'GitLab OAuth' : 'OAuth Redirect' }}
              </h4>
            </div>
            <button class="button" @click="props.startBrowserAuth(provider.provider_id)">Start Browser Login</button>
          </div>

          <div v-if="provider.provider_id === 'gitlab'" class="field">
            <label class="label" :for="`browser-instance-${provider.provider_id}`">GitLab Instance URL</label>
            <input
              :id="`browser-instance-${provider.provider_id}`"
              v-model="props.browserAuthInstanceDrafts[provider.provider_id]"
              class="input mono"
              placeholder="https://gitlab.com"
            />
          </div>

          <div v-if="props.browserAuthStartState[provider.provider_id]" class="form-grid">
            <div class="field">
              <label class="label" :for="`browser-code-${provider.provider_id}`">Authorization Code</label>
              <input
                :id="`browser-code-${provider.provider_id}`"
                v-model="props.browserAuthCodeDrafts[provider.provider_id]"
                class="input mono"
                placeholder="paste callback code"
              />
            </div>
            <div class="field">
              <label class="label">State</label>
              <div class="input mono">{{ props.browserAuthStartState[provider.provider_id]?.state }}</div>
            </div>
            <div class="button-row full">
              <button class="button primary" @click="props.finishBrowserAuth(provider.provider_id)">
                Finish Browser Login
              </button>
            </div>
          </div>
        </div>

        <div v-if="supportsDeviceLogin(provider.provider_id)" class="record-section">
          <div class="settings-panel-header">
            <div>
              <p class="settings-panel-kicker">Device Login</p>
              <h4 class="settings-panel-title">
                {{ provider.provider_id === 'github-copilot' ? 'GitHub Copilot' : 'Device Code' }}
              </h4>
            </div>
            <button class="button" @click="props.startDeviceAuth(provider.provider_id)">Start Device Login</button>
          </div>

          <div v-if="provider.provider_id === 'github-copilot'" class="field">
            <label class="label" :for="`device-enterprise-${provider.provider_id}`">Enterprise Domain</label>
            <input
              :id="`device-enterprise-${provider.provider_id}`"
              v-model="props.deviceAuthEnterpriseDrafts[provider.provider_id]"
              class="input mono"
              placeholder="github.example.com"
            />
          </div>

          <div v-if="props.deviceAuthStartState[provider.provider_id]" class="facts-grid">
            <div class="fact-row">
              <div class="fact-label">Verification URL</div>
              <div class="fact-value mono">
                {{ props.deviceAuthStartState[provider.provider_id]?.verification_url }}
              </div>
            </div>
            <div class="fact-row">
              <div class="fact-label">User Code</div>
              <div class="fact-value mono">{{ props.deviceAuthStartState[provider.provider_id]?.user_code }}</div>
            </div>
            <div class="fact-row">
              <div class="fact-label">Interval</div>
              <div class="fact-value">{{ props.deviceAuthStartState[provider.provider_id]?.interval_seconds }}s</div>
            </div>
            <div class="button-row">
              <button class="button primary" @click="props.pollDeviceAuth(provider.provider_id)">
                Poll Device Login
              </button>
            </div>
          </div>
        </div>
      </article>
    </section>

    <div v-else class="empty-state">No auth-capable providers were exposed by the Agena runtime.</div>
  </div>
</template>
