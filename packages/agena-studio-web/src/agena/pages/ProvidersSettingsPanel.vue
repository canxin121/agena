<script setup lang="ts">
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
</script>

<template>
  <section class="card">
    <h3>Credentials</h3>
    <div v-if="props.authProviders.length" class="list">
      <div v-for="provider in props.authProviders" :key="provider.provider_id" class="list-item">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <div><strong>{{ provider.provider_id }}</strong></div>
            <div v-for="fact in buildAuthProviderFacts(provider)" :key="fact.label" class="muted">
              <strong>{{ fact.label }}:</strong>
              <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
            </div>
          </div>
          <span class="badge">{{ provider.credential_type || 'unknown' }}</span>
        </div>

        <div class="field" style="margin-top: 12px">
          <label class="label" :for="`api-key-${provider.provider_id}`">API Key</label>
          <input
            :id="`api-key-${provider.provider_id}`"
            v-model="props.drafts[provider.provider_id]"
            class="input mono"
            type="password"
            placeholder="sk-..."
          />
        </div>

        <div class="button-row" style="margin-top: 12px">
          <button class="button primary" @click="props.saveApiKey(provider.provider_id)">Save API Key</button>
          <button class="button" @click="props.refreshCredential(provider.provider_id)">Refresh</button>
          <button class="button danger" @click="props.clearCredential(provider.provider_id)">Delete</button>
        </div>

        <template v-if="provider.provider_id === 'openai' || provider.provider_id === 'gitlab'">
          <div class="field" style="margin-top: 16px">
            <label class="label" :for="`browser-instance-${provider.provider_id}`">
              {{ provider.provider_id === 'gitlab' ? 'GitLab Instance URL' : 'Browser Login' }}
            </label>
            <input
              v-if="provider.provider_id === 'gitlab'"
              :id="`browser-instance-${provider.provider_id}`"
              v-model="props.browserAuthInstanceDrafts[provider.provider_id]"
              class="input mono"
              placeholder="https://gitlab.com"
            />
            <div v-else class="muted">Open a browser login flow and paste the returned authorization code below.</div>
          </div>
          <div class="button-row" style="margin-top: 12px">
            <button class="button" @click="props.startBrowserAuth(provider.provider_id)">Start Browser Login</button>
          </div>
          <div v-if="props.browserAuthStartState[provider.provider_id]" class="stack" style="margin-top: 12px">
            <div class="muted mono">
              state={{ props.browserAuthStartState[provider.provider_id]?.state }} · pkce={{ props.browserAuthStartState[provider.provider_id]?.pkce_verifier }}
            </div>
            <div class="field">
              <label class="label" :for="`browser-code-${provider.provider_id}`">Authorization Code</label>
              <input
                :id="`browser-code-${provider.provider_id}`"
                v-model="props.browserAuthCodeDrafts[provider.provider_id]"
                class="input mono"
                placeholder="paste code from callback"
              />
            </div>
            <div class="button-row">
              <button class="button primary" @click="props.finishBrowserAuth(provider.provider_id)">Finish Browser Login</button>
            </div>
          </div>
        </template>

        <template v-if="provider.provider_id === 'openai' || provider.provider_id === 'github-copilot'">
          <div class="field" style="margin-top: 16px">
            <label class="label" :for="`device-enterprise-${provider.provider_id}`">
              {{ provider.provider_id === 'github-copilot' ? 'Enterprise Domain (Optional)' : 'Device Login' }}
            </label>
            <input
              v-if="provider.provider_id === 'github-copilot'"
              :id="`device-enterprise-${provider.provider_id}`"
              v-model="props.deviceAuthEnterpriseDrafts[provider.provider_id]"
              class="input mono"
              placeholder="github.example.com"
            />
            <div v-else class="muted">Start device login if you prefer a code-based flow instead of browser redirect.</div>
          </div>
          <div class="button-row" style="margin-top: 12px">
            <button class="button" @click="props.startDeviceAuth(provider.provider_id)">Start Device Login</button>
          </div>
          <div v-if="props.deviceAuthStartState[provider.provider_id]" class="stack" style="margin-top: 12px">
            <div class="muted mono">
              verification_url={{ props.deviceAuthStartState[provider.provider_id]?.verification_url }}
            </div>
            <div class="muted mono">
              user_code={{ props.deviceAuthStartState[provider.provider_id]?.user_code }} · interval={{
                props.deviceAuthStartState[provider.provider_id]?.interval_seconds
              }}s
            </div>
            <div class="button-row">
              <button class="button primary" @click="props.pollDeviceAuth(provider.provider_id)">Poll Device Login</button>
            </div>
          </div>
        </template>
      </div>
    </div>
    <p v-else class="muted">No auth-capable providers were exposed by the runtime.</p>
  </section>
</template>
