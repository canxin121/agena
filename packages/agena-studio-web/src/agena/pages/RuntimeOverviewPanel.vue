<script setup lang="ts">
import type { ProviderModel, ProviderSummary, RuntimeStatus } from '@/agena/lib/agenaApi'

const props = defineProps<{
  operatorCards: Array<{ label: string; value: string | number }>
  runtimeSnapshotFacts: Array<{ label: string; value: string; mono?: boolean }>
  runtime: RuntimeStatus | null
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  sessionCacheFacts: Array<{ label: string; value: string; mono?: boolean }>
  formatProviderModel: (model: ProviderModel) => string
}>()
</script>

<template>
  <div>
    <div class="grid three">
      <section v-for="card in props.operatorCards" :key="card.label" class="card">
        <div class="muted">{{ card.label }}</div>
        <div style="font-size: 1.5rem; font-weight: 600">{{ card.value }}</div>
      </section>
    </div>

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <h3>Runtime Snapshot</h3>
        <div v-if="props.runtimeSnapshotFacts.length" class="stack">
          <div v-for="fact in props.runtimeSnapshotFacts" :key="fact.label">
            <strong>{{ fact.label }}:</strong>
            <span :class="{ mono: fact.mono }">{{ fact.value }}</span>
          </div>
        </div>
        <p v-else class="muted">Loading runtime snapshot…</p>
      </section>

      <section class="card">
        <h3>Maintenance</h3>
        <div v-if="props.runtime" class="stack">
          <div><strong>Reload:</strong> {{ props.runtime.reload.enabled ? 'enabled' : 'disabled' }} ({{ props.runtime.reload.interval_secs }}s)</div>
          <div><strong>Janitor:</strong> {{ props.runtime.janitor.enabled ? 'enabled' : 'disabled' }} ({{ props.runtime.janitor.interval_secs }}s)</div>
          <div><strong>Watch Paths:</strong></div>
          <div v-if="props.runtime.watch_paths.length" class="list">
            <div v-for="path in props.runtime.watch_paths" :key="path" class="list-item mono">{{ path }}</div>
          </div>
          <div v-else class="muted">No watch paths configured.</div>
        </div>
        <p v-else class="muted">Loading maintenance state…</p>
      </section>
    </div>

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <h3>Recent Automation</h3>
        <div v-if="props.runtime?.automation.recent_jobs.length" class="list">
          <div v-for="job in props.runtime.automation.recent_jobs" :key="job.id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div><strong>{{ job.kind }}</strong> <span class="muted mono">{{ job.id }}</span></div>
                <div class="muted">session {{ job.owner_session_id ?? 'n/a' }}</div>
                <div v-if="job.last_run" class="muted">
                  {{ job.last_run.status }} · triggered {{ job.last_run.triggered_at }}
                </div>
                <div v-else-if="job.next_fire_at" class="muted">next {{ job.next_fire_at }}</div>
                <div v-if="job.last_run?.error_message" class="muted">{{ job.last_run.error_message }}</div>
              </div>
              <span class="badge">{{ job.expression || job.at || 'scheduled' }}</span>
            </div>
          </div>
        </div>
        <p v-else class="muted">No scheduled jobs visible yet.</p>
      </section>

      <section class="card">
        <h3>Provider Defaults</h3>
        <div v-if="props.providers.length" class="list">
          <div v-for="provider in props.providers" :key="provider.provider_id" class="list-item">
            <div><strong>{{ provider.provider_id }}</strong></div>
            <div class="muted">Default model: {{ provider.default_model }}</div>
            <div class="muted mono">{{ provider.default_model_ref }}</div>
            <div class="muted">
              Models:
              {{ (props.providerModels[provider.provider_id] || []).map((model) => props.formatProviderModel(model)).join(', ') || 'none' }}
            </div>
          </div>
        </div>
        <p v-else class="muted">No providers loaded.</p>
      </section>

      <section class="card">
        <h3>Session Cache</h3>
        <div v-if="props.sessionCacheFacts.length" class="stack">
          <div v-for="fact in props.sessionCacheFacts" :key="fact.label">
            <strong>{{ fact.label }}:</strong> {{ fact.value }}
          </div>
        </div>
        <p v-else class="muted">Session cache is not available.</p>
      </section>
    </div>
  </div>
</template>
