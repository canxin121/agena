<script setup lang="ts">
import { buildAuthProviderFacts } from './runtimePageModel'
import type { AuthProvider } from '@/agena/lib/agenaApi'

const props = defineProps<{
  authProviders: AuthProvider[]
  drafts: Record<string, string>
  saveApiKey: (providerId: string) => void | Promise<void>
  refreshCredential: (providerId: string) => void | Promise<void>
  clearCredential: (providerId: string) => void | Promise<void>
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
      </div>
    </div>
    <p v-else class="muted">No auth-capable providers were exposed by the runtime.</p>
  </section>
</template>
