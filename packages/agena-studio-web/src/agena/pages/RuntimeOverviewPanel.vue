<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type { ModelCatalogEntry, ProviderModel, ProviderSummary, RuntimeStatus } from '@/agena/lib/agenaApi'

import {
  MODEL_FAMILY_OPTIONS,
  MODEL_LIFECYCLE_OPTIONS,
  createEmptyModelCatalogDraft,
  createModelCatalogDraftFromEntry,
  useRuntimeModelCatalogActions,
  type ModelCatalogEditableDraft,
} from './useRuntimeModelCatalogActions'

const props = defineProps<{
  catalogEntries: ModelCatalogEntry[]
  operatorCards: Array<{ label: string; value: string | number }>
  runtimeSnapshotFacts: Array<{ label: string; value: string; mono?: boolean }>
  runtime: RuntimeStatus | null
  providers: ProviderSummary[]
  providerModels: Record<string, ProviderModel[]>
  sessionCacheFacts: Array<{ label: string; value: string; mono?: boolean }>
  formatProviderModel: (model: ProviderModel) => string
  load: () => Promise<void>
}>()

const actionError = ref('')
const actionMessage = ref('')
const catalogEntriesState = ref<ModelCatalogEntry[]>([])
const draft = ref<ModelCatalogEditableDraft>(createEmptyModelCatalogDraft())
const editingEntryKey = ref('')
const submitting = ref(false)

watch(
  () => props.catalogEntries,
  (entries) => {
    catalogEntriesState.value = entries.map((entry) => ({ ...entry }))
  },
  { immediate: true, deep: true },
)

const { deleteCatalogEntryAction, refreshCatalogAction, saveCatalogEntryAction, setCatalogDefaultModelAction } =
  useRuntimeModelCatalogActions({
    actionError,
    actionMessage,
    catalogEntries: catalogEntriesState,
    load: props.load,
  })

const sortedCatalogEntries = computed(() =>
  [...catalogEntriesState.value].sort((left, right) => {
    if (left.provider_id !== right.provider_id) return left.provider_id.localeCompare(right.provider_id)
    if (left.model_id !== right.model_id) return left.model_id.localeCompare(right.model_id)
    return left.kind.localeCompare(right.kind)
  }),
)

function makeEntryKey(providerId: string, modelId: string) {
  return `${providerId}/${modelId}`
}

function resetEditor(providerId = '', modelId = '') {
  draft.value = createEmptyModelCatalogDraft(providerId, modelId)
  editingEntryKey.value = ''
}

function editEntry(entry: ModelCatalogEntry) {
  draft.value = createModelCatalogDraftFromEntry(entry)
  editingEntryKey.value = makeEntryKey(entry.provider_id, entry.model_id)
}

async function saveDraft() {
  submitting.value = true
  try {
    await saveCatalogEntryAction(draft.value)
    editingEntryKey.value = makeEntryKey(draft.value.provider_id.trim(), draft.value.model_id.trim())
  } finally {
    submitting.value = false
  }
}

async function refreshCatalog() {
  submitting.value = true
  try {
    await refreshCatalogAction()
  } finally {
    submitting.value = false
  }
}

async function deleteEntry(entry: ModelCatalogEntry) {
  submitting.value = true
  try {
    await deleteCatalogEntryAction(entry.provider_id, entry.model_id)
    if (editingEntryKey.value === makeEntryKey(entry.provider_id, entry.model_id)) {
      resetEditor(entry.provider_id)
    }
  } finally {
    submitting.value = false
  }
}

async function setDefault(entry: ModelCatalogEntry) {
  submitting.value = true
  try {
    await setCatalogDefaultModelAction(entry.provider_id, entry.model_id)
  } finally {
    submitting.value = false
  }
}

function isEntrySelected(entry: ModelCatalogEntry) {
  return editingEntryKey.value === makeEntryKey(entry.provider_id, entry.model_id)
}
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
                <div v-if="job.last_run" class="muted">{{ job.last_run.status }} · triggered {{ job.last_run.triggered_at }}</div>
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
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Provider Defaults</h3>
            <p class="muted">Provider-visible models usually follow the runtime adapter routing, often as <span class="mono">&lt;adapter&gt;/&lt;model&gt;</span>.</p>
          </div>
        </div>
        <div v-if="props.providers.length" class="list">
          <div v-for="provider in props.providers" :key="provider.provider_id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div><strong>{{ provider.provider_id }}</strong></div>
                <div class="muted">Default model: {{ provider.default_model || 'unset' }}</div>
                <div v-if="provider.catalog_default_model" class="muted">Catalog default: {{ provider.catalog_default_model }}</div>
                <div class="muted mono">{{ provider.default_model_ref || 'n/a' }}</div>
                <div class="muted">
                  Models:
                  {{ (props.providerModels[provider.provider_id] || []).map((model) => props.formatProviderModel(model)).join(', ') || 'none' }}
                </div>
              </div>
              <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
                <button class="button" :disabled="submitting" @click="resetEditor(provider.provider_id)">Add Override</button>
              </div>
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

    <div class="grid two" style="margin-top: 16px">
      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Model Catalog</h3>
            <p class="muted">Refresh the runtime catalog, create or edit local overrides, delete local overrides, and set the provider default model.</p>
          </div>
          <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
            <button class="button" :disabled="submitting" @click="resetEditor()">New Override</button>
            <button class="button primary" :disabled="submitting" @click="refreshCatalog">Refresh Catalog</button>
          </div>
        </div>

        <div v-if="props.runtime?.model_catalog" class="stack" style="margin-top: 12px">
          <div><strong>Remote:</strong> <span class="mono">{{ props.runtime.model_catalog.remote_url }}</span></div>
          <div><strong>Fallback:</strong> <span class="mono">{{ props.runtime.model_catalog.fallback_url }}</span></div>
          <div><strong>Last Source:</strong> {{ props.runtime.model_catalog.last_successful_source || 'none' }}</div>
          <div><strong>Last Refresh:</strong> {{ props.runtime.model_catalog.last_refresh_at || 'never' }}</div>
          <div v-if="props.runtime.model_catalog.last_error" class="muted">{{ props.runtime.model_catalog.last_error }}</div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">Model catalog is not available in the runtime snapshot yet.</p>

        <p v-if="actionMessage" class="muted" style="margin-top: 12px">{{ actionMessage }}</p>
        <p v-if="actionError" class="muted" style="margin-top: 8px">{{ actionError }}</p>

        <div class="grid two" style="margin-top: 16px">
          <div class="field">
            <label class="label" for="catalog-provider-id">Provider ID</label>
            <input id="catalog-provider-id" v-model="draft.provider_id" class="input mono" placeholder="shared" />
          </div>
          <div class="field">
            <label class="label" for="catalog-model-id">Model ID</label>
            <input
              id="catalog-model-id"
              v-model="draft.model_id"
              class="input mono"
              placeholder="openai/gpt-4.1-mini"
            />
          </div>

          <div class="field">
            <label class="label" for="catalog-display-name">Display Name</label>
            <input id="catalog-display-name" v-model="draft.display_name" class="input" placeholder="Acme Reasoner" />
          </div>
          <div class="field">
            <label class="label" for="catalog-family">Family</label>
            <select id="catalog-family" v-model="draft.family" class="select">
              <option value="">Unset</option>
              <option v-for="family in MODEL_FAMILY_OPTIONS" :key="family" :value="family">{{ family }}</option>
            </select>
          </div>

          <div class="field">
            <label class="label" for="catalog-lifecycle">Lifecycle</label>
            <select id="catalog-lifecycle" v-model="draft.lifecycle" class="select">
              <option value="">Unset</option>
              <option v-for="lifecycle in MODEL_LIFECYCLE_OPTIONS" :key="lifecycle" :value="lifecycle">{{ lifecycle }}</option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="catalog-context-window">Context Window Tokens</label>
            <input id="catalog-context-window" v-model="draft.context_window_tokens" class="input mono" inputmode="numeric" placeholder="128000" />
          </div>

          <div class="field">
            <label class="label" for="catalog-max-output">Max Output Tokens</label>
            <input id="catalog-max-output" v-model="draft.max_output_tokens" class="input mono" inputmode="numeric" placeholder="8192" />
          </div>
          <div class="field" style="display: flex; align-items: end">
            <label class="muted" for="catalog-default-toggle" style="display: flex; gap: 8px; align-items: center">
              <input id="catalog-default-toggle" v-model="draft.set_default_for_provider" type="checkbox" />
              Set default for provider on save
            </label>
          </div>
        </div>

        <div class="field" style="margin-top: 12px">
          <label class="label" for="catalog-description">Description</label>
          <textarea id="catalog-description" v-model="draft.description" class="input" rows="3" placeholder="Optional model notes or behavior summary." />
        </div>

        <div class="grid two" style="margin-top: 12px">
          <label class="muted" for="catalog-capability-tool" style="display: flex; gap: 8px; align-items: center">
            <input id="catalog-capability-tool" v-model="draft.tool_calling" type="checkbox" />
            Tool calling
          </label>
          <label class="muted" for="catalog-capability-streaming" style="display: flex; gap: 8px; align-items: center">
            <input id="catalog-capability-streaming" v-model="draft.streaming" type="checkbox" />
            Streaming
          </label>
          <label class="muted" for="catalog-capability-reasoning" style="display: flex; gap: 8px; align-items: center">
            <input id="catalog-capability-reasoning" v-model="draft.reasoning" type="checkbox" />
            Reasoning
          </label>
          <label class="muted" for="catalog-capability-structured" style="display: flex; gap: 8px; align-items: center">
            <input id="catalog-capability-structured" v-model="draft.structured_output" type="checkbox" />
            Structured output
          </label>
          <label class="muted" for="catalog-capability-temperature" style="display: flex; gap: 8px; align-items: center">
            <input id="catalog-capability-temperature" v-model="draft.temperature_supported" type="checkbox" />
            Temperature supported
          </label>
        </div>

        <div class="button-row" style="margin-top: 16px; flex-wrap: wrap">
          <button class="button primary" :disabled="submitting" @click="saveDraft">
            {{ editingEntryKey ? 'Save Override' : 'Create Override' }}
          </button>
          <button class="button" :disabled="submitting" @click="resetEditor(draft.provider_id)">Reset Form</button>
        </div>
      </section>

      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Catalog Entries</h3>
            <p class="muted">Official entries can be edited into a local override. Delete only removes local custom entries.</p>
          </div>
          <span class="badge">{{ sortedCatalogEntries.length }}</span>
        </div>

        <div v-if="sortedCatalogEntries.length" class="list" style="margin-top: 12px">
          <div
            v-for="entry in sortedCatalogEntries"
            :key="`${entry.provider_id}/${entry.model_id}/${entry.kind}`"
            class="list-item"
            :style="isEntrySelected(entry) ? 'border-color: var(--accent-color, #444);' : ''"
          >
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div><strong>{{ entry.provider_id }}/{{ entry.model_id }}</strong></div>
                <div class="muted">{{ entry.display_name || 'Unnamed model' }} · {{ entry.kind }} · {{ entry.source }}</div>
                <div v-if="entry.family || entry.lifecycle" class="muted">
                  {{ entry.family || 'family unset' }} · {{ entry.lifecycle || 'lifecycle unset' }}
                </div>
                <div v-if="entry.default_model_for_provider" class="muted">Provider default: {{ entry.default_model_for_provider }}</div>
                <div v-if="entry.description" class="muted">{{ entry.description }}</div>
                <div v-if="entry.context_window_tokens || entry.max_output_tokens" class="muted mono">
                  ctx={{ entry.context_window_tokens ?? 'n/a' }} · max_out={{ entry.max_output_tokens ?? 'n/a' }}
                </div>
              </div>
              <span class="badge">{{ entry.source_label || entry.kind }}</span>
            </div>

            <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button class="button" :disabled="submitting" @click="editEntry(entry)">
                {{ entry.kind === 'custom' ? 'Edit Override' : 'Create Override' }}
              </button>
              <button class="button" :disabled="submitting" @click="setDefault(entry)">Set Default</button>
              <button
                v-if="entry.kind === 'custom'"
                class="button danger"
                :disabled="submitting"
                @click="deleteEntry(entry)"
              >
                Delete Override
              </button>
            </div>
          </div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">No catalog entries loaded.</p>
      </section>
    </div>
  </div>
</template>
