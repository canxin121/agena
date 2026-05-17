<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type {
  ModelCatalogEntry,
  ModelCatalogEntryKind,
  ProviderModel,
  ProviderModelVariant,
  ProviderSummary,
  RuntimeStatus,
} from '@/agena/lib/agenaApi'

import {
  MODEL_LIFECYCLE_OPTIONS,
  createEmptyModelCatalogDraft,
  createEmptyModelCatalogVariantDraft,
  createModelCatalogDraftFromEntry,
  createModelCatalogDraftFromProviderSelection,
  findCatalogEntryForProviderModel,
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
const catalogKindFilter = ref<'all' | ModelCatalogEntryKind>('all')
const catalogQuery = ref('')
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

const { deleteCatalogEntryAction, refreshCatalogAction, saveCatalogEntryAction } = useRuntimeModelCatalogActions({
  actionError,
  actionMessage,
  catalogEntries: catalogEntriesState,
  load: props.load,
})

const sortedCatalogEntries = computed(() =>
  [...catalogEntriesState.value].sort((left, right) => {
    if (left.model_id !== right.model_id) return left.model_id.localeCompare(right.model_id)
    return left.kind.localeCompare(right.kind)
  }),
)

const customCatalogEntriesCount = computed(
  () => catalogEntriesState.value.filter((entry) => entry.kind === 'custom').length,
)

const officialCatalogEntriesCount = computed(
  () => catalogEntriesState.value.filter((entry) => entry.kind === 'official').length,
)

function catalogEntrySearchText(entry: ModelCatalogEntry) {
  const variantText = Object.entries(entry.variants || {})
    .flatMap(([name, variant]) => [
      name,
      variant.display_name,
      variant.description,
      variant.thinking ? JSON.stringify(variant.thinking) : '',
    ])
    .filter(Boolean)
    .join('\n')

  return [
    entry.model_id,
    entry.display_name,
    entry.description,
    entry.kind,
    entry.source,
    entry.source_label,
    entry.lifecycle,
    variantText,
  ]
    .filter(Boolean)
    .join('\n')
    .toLowerCase()
}

const filteredCatalogEntries = computed(() => {
  const query = catalogQuery.value.trim().toLowerCase()
  const kind = catalogKindFilter.value

  return sortedCatalogEntries.value.filter((entry) => {
    if (kind !== 'all' && entry.kind !== kind) return false
    return !query || catalogEntrySearchText(entry).includes(query)
  })
})

function makeEntryKey(modelId: string, kind: ModelCatalogEntry['kind']) {
  return `${modelId}/${kind}`
}

function resetEditor(adapterId = '', modelId = '') {
  draft.value = createEmptyModelCatalogDraft(adapterId, modelId)
  editingEntryKey.value = ''
}

function firstProviderAdapterId(provider: ProviderSummary) {
  return provider.default_adapter || provider.adapters?.find((adapter) => adapter.enabled)?.adapter_id || ''
}

function editEntry(entry: ModelCatalogEntry) {
  draft.value = createModelCatalogDraftFromEntry(entry)
  editingEntryKey.value = makeEntryKey(entry.model_id, entry.kind)
  actionError.value = ''
  actionMessage.value = `Loaded ${entry.model_id} into the draft editor.`
}

function clearCatalogFilters() {
  catalogQuery.value = ''
  catalogKindFilter.value = 'all'
}

type ProviderModelVariantWithDisabled = ProviderModelVariant & {
  disabled?: boolean
}

function addVariantDraft() {
  draft.value.variants.push(createEmptyModelCatalogVariantDraft())
}

function removeVariantDraft(index: number) {
  draft.value.variants.splice(index, 1)
}

function entryVariantItems(entry: ModelCatalogEntry): Array<[string, ProviderModelVariantWithDisabled]> {
  return Object.entries(entry.variants || {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, variant]) => [name, variant as ProviderModelVariantWithDisabled])
}

function formatVariantThinking(value: Record<string, unknown> | null | undefined) {
  if (!value) return ''
  const text = JSON.stringify(value)
  return text.length > 96 ? `${text.slice(0, 93)}...` : text
}

function loadProviderModelDraft(model: ProviderModel) {
  const matchingEntry = findCatalogEntryForProviderModel(catalogEntriesState.value, model)
  draft.value = createModelCatalogDraftFromProviderSelection(catalogEntriesState.value, model)
  editingEntryKey.value = matchingEntry ? makeEntryKey(matchingEntry.model_id, matchingEntry.kind) : ''
  actionError.value = ''
  actionMessage.value = `Loaded ${model.provider_id}/${model.adapter_id || 'adapter'}/${model.id} into the draft editor.`
}

async function saveDraft() {
  submitting.value = true
  try {
    await saveCatalogEntryAction(draft.value)
    editingEntryKey.value = makeEntryKey(draft.value.model_id.trim(), 'custom')
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
    await deleteCatalogEntryAction(entry.model_id)
    if (editingEntryKey.value === makeEntryKey(entry.model_id, entry.kind)) {
      resetEditor()
    }
  } finally {
    submitting.value = false
  }
}

function isEntrySelected(entry: ModelCatalogEntry) {
  return editingEntryKey.value === makeEntryKey(entry.model_id, entry.kind)
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
          <div>
            <strong>Reload:</strong> {{ props.runtime.reload.enabled ? 'enabled' : 'disabled' }} ({{
              props.runtime.reload.interval_secs
            }}s)
          </div>
          <div>
            <strong>Janitor:</strong> {{ props.runtime.janitor.enabled ? 'enabled' : 'disabled' }} ({{
              props.runtime.janitor.interval_secs
            }}s)
          </div>
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
                <div>
                  <strong>{{ job.kind }}</strong> <span class="muted mono">{{ job.id }}</span>
                </div>
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
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Provider Defaults</h3>
            <p class="muted">Provider defaults keep adapter and model as separate runtime fields.</p>
          </div>
        </div>
        <div v-if="props.providers.length" class="list">
          <div v-for="provider in props.providers" :key="provider.provider_id" class="list-item">
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ provider.provider_id }}</strong>
                </div>
                <div class="muted">Default adapter: {{ provider.default_adapter || 'auto' }}</div>
                <div class="muted">Default model: {{ provider.default_model || 'unset' }}</div>
                <div class="muted" style="margin-top: 8px">Live models:</div>
                <div
                  v-if="(props.providerModels[provider.provider_id] || []).length"
                  class="button-row"
                  style="margin-top: 8px; flex-wrap: wrap"
                >
                  <button
                    v-for="model in props.providerModels[provider.provider_id] || []"
                    :key="model.id"
                    class="button"
                    :disabled="submitting"
                    :title="`${model.provider_id}/${model.id}`"
                    @click="loadProviderModelDraft(model)"
                  >
                    Bring to Draft: {{ props.formatProviderModel(model) }}
                  </button>
                </div>
                <div v-else class="muted">No live models loaded.</div>
              </div>
              <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
                <button class="button" :disabled="submitting" @click="resetEditor(firstProviderAdapterId(provider))">
                  Blank Draft
                </button>
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
            <p class="muted">
              Refresh the runtime catalog, pull a live provider model into the draft editor, save custom entries from
              official or live metadata, and delete custom entries.
            </p>
          </div>
          <div class="button-row" style="flex-wrap: wrap; justify-content: flex-end">
            <button class="button" :disabled="submitting" @click="resetEditor()">Blank Custom Entry</button>
            <button class="button primary" :disabled="submitting" @click="refreshCatalog">Refresh Catalog</button>
          </div>
        </div>

        <div v-if="props.runtime?.model_catalog" class="stack" style="margin-top: 12px">
          <div>
            <strong>Remote:</strong> <span class="mono">{{ props.runtime.model_catalog.remote_url }}</span>
          </div>
          <div>
            <strong>Fallback:</strong> <span class="mono">{{ props.runtime.model_catalog.fallback_url }}</span>
          </div>
          <div><strong>Last Source:</strong> {{ props.runtime.model_catalog.last_successful_source || 'none' }}</div>
          <div><strong>Last Refresh:</strong> {{ props.runtime.model_catalog.last_refresh_at || 'never' }}</div>
          <div v-if="props.runtime.model_catalog.last_error" class="muted">
            {{ props.runtime.model_catalog.last_error }}
          </div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">Model catalog is not available in the runtime snapshot yet.</p>

        <p v-if="actionMessage" class="muted" style="margin-top: 12px">{{ actionMessage }}</p>
        <p v-if="actionError" class="muted" style="margin-top: 8px">{{ actionError }}</p>
        <p class="muted" style="margin-top: 12px">
          Use the live model buttons above for the fastest draft path, then adjust any fields below before saving.
        </p>

        <div class="grid two" style="margin-top: 16px">
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
            <label class="label" for="catalog-lifecycle">Lifecycle</label>
            <select id="catalog-lifecycle" v-model="draft.lifecycle" class="select">
              <option value="">Unset</option>
              <option v-for="lifecycle in MODEL_LIFECYCLE_OPTIONS" :key="lifecycle" :value="lifecycle">
                {{ lifecycle }}
              </option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="catalog-context-window">Context Window Tokens</label>
            <input
              id="catalog-context-window"
              v-model="draft.context_window_tokens"
              class="input mono"
              inputmode="numeric"
              placeholder="128000"
            />
          </div>

          <div class="field">
            <label class="label" for="catalog-max-output">Max Output Tokens</label>
            <input
              id="catalog-max-output"
              v-model="draft.max_output_tokens"
              class="input mono"
              inputmode="numeric"
              placeholder="8192"
            />
          </div>
        </div>

        <div class="field" style="margin-top: 12px">
          <label class="label" for="catalog-description">Description</label>
          <textarea
            id="catalog-description"
            v-model="draft.description"
            class="input"
            rows="3"
            placeholder="Optional model notes or behavior summary."
          />
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
          <label
            class="muted"
            for="catalog-capability-temperature"
            style="display: flex; gap: 8px; align-items: center"
          >
            <input id="catalog-capability-temperature" v-model="draft.temperature_supported" type="checkbox" />
            Temperature supported
          </label>
        </div>

        <div class="page-header" style="margin-top: 16px; align-items: flex-start">
          <div>
            <h4 style="margin: 0">Variants</h4>
            <p class="muted" style="margin: 4px 0 0">
              Add a few provider/model variants with optional labels, descriptions, disabled state, and raw thinking
              JSON.
            </p>
          </div>
          <button class="button" :disabled="submitting" @click="addVariantDraft">Add Variant</button>
        </div>

        <div v-if="draft.variants.length" class="stack" style="margin-top: 12px">
          <div
            v-for="(variant, index) in draft.variants"
            :key="`${variant.name || 'variant'}-${index}`"
            class="list-item"
            style="padding: 12px"
          >
            <div class="page-header" style="align-items: flex-start">
              <strong>{{ variant.name.trim() || `Variant ${index + 1}` }}</strong>
              <button class="button danger" :disabled="submitting" @click="removeVariantDraft(index)">Remove</button>
            </div>

            <div class="grid two" style="margin-top: 12px">
              <div class="field">
                <label class="label" :for="`catalog-variant-name-${index}`">Variant Name</label>
                <input
                  :id="`catalog-variant-name-${index}`"
                  v-model="variant.name"
                  class="input mono"
                  placeholder="deep"
                />
              </div>
              <div class="field">
                <label class="label" :for="`catalog-variant-display-name-${index}`">Display Name</label>
                <input
                  :id="`catalog-variant-display-name-${index}`"
                  v-model="variant.display_name"
                  class="input"
                  placeholder="Deep Thinking"
                />
              </div>
            </div>

            <div class="field" style="margin-top: 12px">
              <label class="label" :for="`catalog-variant-description-${index}`">Description</label>
              <textarea
                :id="`catalog-variant-description-${index}`"
                v-model="variant.description"
                class="input"
                rows="2"
                placeholder="Optional behavior notes for this variant."
              />
            </div>

            <div class="field" style="margin-top: 12px">
              <label class="label" :for="`catalog-variant-thinking-${index}`">Thinking JSON</label>
              <textarea
                :id="`catalog-variant-thinking-${index}`"
                v-model="variant.thinking_json"
                class="input mono"
                rows="4"
                placeholder='{"type":"budget","budget_tokens":20000}'
              />
              <div class="muted" style="margin-top: 6px">
                Accepts the backend thinking payload, for example
                <span class="mono">{"type":"budget","budget_tokens":20000}</span>
                or
                <span class="mono">{"type":"effort","effort":"medium"}</span>.
              </div>
            </div>

            <label
              class="muted"
              :for="`catalog-variant-disabled-${index}`"
              style="display: flex; gap: 8px; align-items: center; margin-top: 12px"
            >
              <input :id="`catalog-variant-disabled-${index}`" v-model="variant.disabled" type="checkbox" />
              Disable this variant
            </label>
          </div>
        </div>
        <p v-else class="muted" style="margin-top: 12px">No variants configured for this draft.</p>

        <div class="button-row" style="margin-top: 16px; flex-wrap: wrap">
          <button class="button primary" :disabled="submitting" @click="saveDraft">
            {{ editingEntryKey ? 'Save Custom Entry' : 'Create Custom Entry' }}
          </button>
          <button class="button" :disabled="submitting" @click="resetEditor()">Reset Form</button>
        </div>
      </section>

      <section class="card">
        <div class="page-header" style="align-items: flex-start">
          <div>
            <h3>Catalog Entries</h3>
            <p class="muted">
              Official entries are runtime catalog metadata. Custom entries are local overrides layered on top and are
              the only entries you can delete.
            </p>
          </div>
          <span class="badge">{{ filteredCatalogEntries.length }}/{{ sortedCatalogEntries.length }}</span>
        </div>

        <div class="settings-summary" style="margin-top: 12px">
          <div class="summary-item">
            <div class="summary-label">Official</div>
            <div class="summary-value">{{ officialCatalogEntriesCount }}</div>
          </div>
          <div class="summary-item">
            <div class="summary-label">Custom</div>
            <div class="summary-value">{{ customCatalogEntriesCount }}</div>
          </div>
          <div class="summary-item">
            <div class="summary-label">Models</div>
            <div class="summary-value">{{ sortedCatalogEntries.length }}</div>
          </div>
          <div class="summary-item">
            <div class="summary-label">Showing</div>
            <div class="summary-value">{{ filteredCatalogEntries.length }}</div>
          </div>
        </div>

        <div class="grid three" style="margin-top: 12px">
          <div class="field">
            <label class="label" for="catalog-entry-search">Find Entries</label>
            <input
              id="catalog-entry-search"
              v-model="catalogQuery"
              class="input mono"
              placeholder="model, variant, description"
            />
          </div>
          <div class="field">
            <label class="label" for="catalog-entry-kind-filter">Kind</label>
            <select id="catalog-entry-kind-filter" v-model="catalogKindFilter" class="select">
              <option value="all">All entries</option>
              <option value="official">Official only</option>
              <option value="custom">Custom only</option>
            </select>
          </div>
        </div>

        <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
          <button class="button" :disabled="!catalogQuery && catalogKindFilter === 'all'" @click="clearCatalogFilters">
            Clear Filters
          </button>
        </div>

        <div v-if="filteredCatalogEntries.length" class="list" style="margin-top: 12px">
          <div
            v-for="entry in filteredCatalogEntries"
            :key="makeEntryKey(entry.model_id, entry.kind)"
            class="list-item"
            :style="isEntrySelected(entry) ? 'border-color: var(--accent-color, #444);' : ''"
          >
            <div class="page-header" style="align-items: flex-start">
              <div>
                <div>
                  <strong>{{ entry.model_id }}</strong>
                </div>
                <div class="muted">
                  {{ entry.display_name || 'Unnamed model' }} · {{ entry.kind }} ·
                  {{ entry.source_label || entry.source }}
                </div>
                <div v-if="entry.kind === 'custom'" class="muted">Custom entry saved for this model.</div>
                <div v-if="entry.lifecycle" class="muted">{{ entry.lifecycle }}</div>
                <div v-if="entry.description" class="muted">{{ entry.description }}</div>
                <div v-if="entry.context_window_tokens || entry.max_output_tokens" class="muted mono">
                  ctx={{ entry.context_window_tokens ?? 'n/a' }} · max_out={{ entry.max_output_tokens ?? 'n/a' }}
                </div>
                <div v-if="entryVariantItems(entry).length" class="stack" style="margin-top: 8px">
                  <div class="muted">Variants:</div>
                  <div
                    v-for="[variantName, variant] in entryVariantItems(entry)"
                    :key="variantName"
                    class="list-item"
                    style="padding: 8px 10px"
                  >
                    <div>
                      <strong>{{ variantName }}</strong>
                      <span v-if="variant.display_name" class="muted"> · {{ variant.display_name }}</span>
                      <span v-if="variant.disabled" class="badge" style="margin-left: 8px">disabled</span>
                    </div>
                    <div v-if="variant.description" class="muted">{{ variant.description }}</div>
                    <div v-if="variant.thinking" class="muted mono">
                      thinking {{ formatVariantThinking(variant.thinking) }}
                    </div>
                  </div>
                </div>
              </div>
              <span v-if="entry.kind === 'custom'" class="badge">custom</span>
            </div>

            <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button class="button" :disabled="submitting" @click="editEntry(entry)">
                {{ entry.kind === 'custom' ? 'Edit Custom Entry' : 'Create Custom Entry' }}
              </button>
              <button
                v-if="entry.kind === 'custom'"
                class="button danger"
                :disabled="submitting"
                @click="deleteEntry(entry)"
              >
                Delete Custom Entry
              </button>
            </div>
          </div>
        </div>
        <p v-else-if="sortedCatalogEntries.length" class="muted" style="margin-top: 12px">
          No catalog entries match the current filters.
        </p>
        <p v-else class="muted" style="margin-top: 12px">No catalog entries loaded.</p>
      </section>
    </div>
  </div>
</template>
