<script setup lang="ts">
import { computed, onMounted, reactive, ref, type Ref } from 'vue'

import {
  discoverDraftProviderAdapters,
  discoverSavedProviderAdapters,
  listModelCatalogEntries,
  lookupModelCatalogEntries,
  patchSettings,
} from '../lib/agenaApi'
import {
  buildAdaptersPatchFromDraftSelection,
  discoveryMatchedModels,
  discoveryUnmatchedModels,
  matchedCatalogModelDefinitions,
} from './providersSettingsModel'
import { buildAuthProviderFacts } from './runtimePageModel'
import {
  buildConfiguredProviderModelFromDraft,
  catalogLookupIdForProviderModel,
  createEmptyModelCatalogDraft,
  createModelCatalogDraftFromEntry,
  createModelCatalogDraftFromProviderSelection,
  type ModelCatalogEditableDraft,
} from './useRuntimeModelCatalogActions'
import type {
  AuthBrowserStartResponse,
  AuthDeviceStartResponse,
  AuthProvider,
  ModelCatalogEntry,
  ModelCatalogSummary,
  ProviderAdapterDiscovery,
  ProviderModel,
  ProviderSummary,
} from '@/agena/lib/agenaApi'

const props = defineProps<{
  actionError: Ref<string>
  actionMessage: Ref<string>
  authProviders: AuthProvider[]
  browserAuthCodeDrafts: Record<string, string>
  browserAuthInstanceDrafts: Record<string, string>
  browserAuthStartState: Record<string, AuthBrowserStartResponse | null>
  deviceAuthEnterpriseDrafts: Record<string, string>
  deviceAuthStartState: Record<string, AuthDeviceStartResponse | null>
  drafts: Record<string, string>
  catalogEntries: ModelCatalogEntry[]
  load: () => Promise<void>
  providerModels: Record<string, ProviderModel[]>
  providers: ProviderSummary[]
  finishBrowserAuth: (providerId: string) => void | Promise<void>
  pollDeviceAuth: (providerId: string) => void | Promise<void>
  saveApiKey: (providerId: string) => void | Promise<void>
  refreshCredential: (providerId: string) => void | Promise<void>
  clearCredential: (providerId: string) => void | Promise<void>
  startBrowserAuth: (providerId: string) => void | Promise<void>
  startDeviceAuth: (providerId: string) => void | Promise<void>
}>()

function summarizeCatalogEntries(entries: ModelCatalogEntry[]): ModelCatalogSummary {
  return {
    entry_count: entries.length,
    official_entry_count: entries.filter((entry) => entry.kind !== 'custom').length,
    custom_entry_count: entries.filter((entry) => entry.kind === 'custom').length,
  }
}

const ADAPTER_OPTIONS = ['openai', 'anthropic', 'gemini', 'ollama', 'gitlab', 'amazon_bedrock'] as const
const submittingConfig = ref(false)
const catalogCopyProviderId = ref('')
const catalogCopyAdapterId = ref('openai')
const catalogCopySetDefault = ref(false)
const providerModelProviderId = ref('')
const providerModelSetDefault = ref(false)
const providerModelDraft = ref<ModelCatalogEditableDraft>(createEmptyModelCatalogDraft('openai', ''))
const draftAdapterDiscoveries = ref<ProviderAdapterDiscovery[]>([])
const providerAdapterDiscoveries = reactive<Record<string, ProviderAdapterDiscovery[]>>({})
const discoveringDraftAdapters = ref(false)
const discoveringSavedProviderIds = reactive<Record<string, boolean>>({})
const draftSelectedAdapterIds = ref<string[]>([])
const catalogSearchEntries = ref<ModelCatalogEntry[]>(props.catalogEntries.map((entry) => ({ ...entry })))
const catalogLookupEntries = ref<ModelCatalogEntry[]>([])
const catalogSummary = ref<ModelCatalogSummary | null>(
  props.catalogEntries.length ? summarizeCatalogEntries(props.catalogEntries) : null,
)
const catalogSearchQuery = ref('')
const catalogSearchOffset = ref(0)
const catalogSearchLimit = ref(50)
const catalogSearchTotal = ref(props.catalogEntries.length)
const catalogSearchOrigins = ref<string[]>([])
const catalogResolvedModelIds = reactive<Record<string, boolean>>({})
const providerCreateDraft = reactive({
  provider_id: '',
  auth_mode: 'api' as 'api' | 'none',
  base_url: '',
  api_key_env: '',
  api_key: '',
  adapter_id: 'openai',
  model_id: '',
  catalog_model_id: '',
})

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
const providerConfigCount = computed(() => props.providers.length)
function mergeCatalogEntries(existing: ModelCatalogEntry[], incoming: ModelCatalogEntry[]) {
  const merged = new Map<string, ModelCatalogEntry>()
  for (const entry of existing) {
    merged.set(`${entry.model_id}/${entry.kind}`, entry)
  }
  for (const entry of incoming) {
    merged.set(`${entry.model_id}/${entry.kind}`, entry)
  }
  return [...merged.values()].sort((left, right) => {
    if (left.model_id !== right.model_id) return left.model_id.localeCompare(right.model_id)
    return left.kind.localeCompare(right.kind)
  })
}

const cachedCatalogEntries = computed(() => mergeCatalogEntries(catalogLookupEntries.value, catalogSearchEntries.value))
const sortedCatalogEntries = computed(() => catalogSearchEntries.value)
const catalogCopyAdapterOptions = computed(() => {
  const adapterIds = new Set<string>(ADAPTER_OPTIONS)
  const provider = props.providers.find((provider) => provider.provider_id === catalogCopyProviderId.value)
  for (const adapter of provider?.adapters || []) adapterIds.add(adapter.adapter_id)
  return [...adapterIds].filter(Boolean).sort((left, right) => left.localeCompare(right))
})

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
  return providerId === 'openai' || providerId === 'gitlab' || isAtomGitProvider(providerId)
}

function supportsDeviceLogin(providerId: string) {
  return providerId === 'openai' || providerId === 'github-copilot'
}

function isAtomGitProvider(providerId: string) {
  const normalized = providerId.toLowerCase()
  return normalized === 'atomgit' || normalized.startsWith('atomgit-')
}

function optionalText(value: string) {
  const normalized = String(value || '').trim()
  return normalized || undefined
}

function setConfigMessage(message: string) {
  props.actionError.value = ''
  props.actionMessage.value = message
}

function setConfigError(message: string) {
  props.actionMessage.value = ''
  props.actionError.value = message
}

function modelDefinitionFromEntry(entry: ModelCatalogEntry) {
  return buildConfiguredProviderModelFromDraft(createModelCatalogDraftFromEntry(entry))
}

function modelIsCatalogMatched(modelId: string, catalogModelId = '') {
  const lookupIds = [...new Set([String(modelId || '').trim(), String(catalogModelId || '').trim()].filter(Boolean))]
  return cachedCatalogEntries.value.some((entry) => lookupIds.includes(entry.model_id))
}

async function loadCatalogPage(offset = 0) {
  const response = await listModelCatalogEntries({
    q: catalogSearchQuery.value,
    offset,
    limit: catalogSearchLimit.value,
  })
  catalogSearchEntries.value = response.items ?? []
  catalogSummary.value = response.summary
  catalogSearchOffset.value = response.offset ?? offset
  catalogSearchLimit.value = response.limit ?? catalogSearchLimit.value
  catalogSearchTotal.value = response.total ?? 0
  catalogSearchOrigins.value = response.available_origins ?? []
}

function previousCatalogPage() {
  if (catalogSearchOffset.value <= 0) return
  void loadCatalogPage(Math.max(0, catalogSearchOffset.value - catalogSearchLimit.value))
}

function nextCatalogPage() {
  if (catalogSearchOffset.value + catalogSearchEntries.value.length >= catalogSearchTotal.value) return
  void loadCatalogPage(catalogSearchOffset.value + catalogSearchLimit.value)
}

async function searchCatalogPage() {
  await loadCatalogPage(0)
}

async function ensureCatalogEntriesForModelIds(modelIds: string[]) {
  const requested = [...new Set(modelIds.map((value) => String(value || '').trim()).filter(Boolean))]
  if (!requested.length) return

  for (const entry of cachedCatalogEntries.value) {
    catalogResolvedModelIds[entry.model_id] = true
  }

  const missing = requested.filter((modelId) => !catalogResolvedModelIds[modelId])
  if (!missing.length) return

  const items = await lookupModelCatalogEntries(missing)
  catalogLookupEntries.value = mergeCatalogEntries(catalogLookupEntries.value, items)
  for (const modelId of missing) {
    catalogResolvedModelIds[modelId] = true
  }
}

async function discoverCreateProviderAdapters() {
  if (providerCreateDraft.auth_mode !== 'api') {
    setConfigError('Draft adapter discovery currently requires api auth.')
    return
  }
  if (!providerCreateDraft.base_url.trim()) {
    setConfigError('Adapter discovery requires a base URL.')
    return
  }
  discoveringDraftAdapters.value = true
  try {
    draftAdapterDiscoveries.value = await discoverDraftProviderAdapters({
      providerId: providerCreateDraft.provider_id,
      baseUrl: providerCreateDraft.base_url,
      apiKey: providerCreateDraft.api_key,
      apiKeyEnv: providerCreateDraft.api_key_env,
    })
    draftSelectedAdapterIds.value = draftAdapterDiscoveries.value
      .filter((discovery) => discovery.supported)
      .map((discovery) => discovery.adapter_id)
    await ensureCatalogEntriesForModelIds(
      draftAdapterDiscoveries.value.flatMap((discovery) =>
        discovery.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
      ),
    )
    setConfigMessage(`Discovered ${draftAdapterDiscoveries.value.length} draft adapters.`)
  } catch (err) {
    draftAdapterDiscoveries.value = []
    draftSelectedAdapterIds.value = []
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    discoveringDraftAdapters.value = false
  }
}

function useDiscoveredCreateModel(adapterId: string, model: ProviderModel) {
  providerCreateDraft.adapter_id = adapterId
  providerCreateDraft.model_id = model.id
  providerCreateDraft.catalog_model_id = catalogLookupIdForProviderModel(model)
  setConfigMessage(`Loaded ${adapterId}/${model.id} into provider create draft.`)
}

async function discoverExistingProviderAdapters(providerId: string) {
  discoveringSavedProviderIds[providerId] = true
  try {
    providerAdapterDiscoveries[providerId] = await discoverSavedProviderAdapters(providerId)
    await ensureCatalogEntriesForModelIds(
      providerAdapterDiscoveries[providerId].flatMap((discovery) =>
        discovery.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
      ),
    )
    setConfigMessage(`Refreshed adapter discovery for ${providerId}.`)
  } catch (err) {
    providerAdapterDiscoveries[providerId] = []
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    discoveringSavedProviderIds[providerId] = false
  }
}

function loadDiscoveredProviderModel(providerId: string, adapterId: string, model: ProviderModel) {
  providerModelProviderId.value = providerId
  providerModelDraft.value = createModelCatalogDraftFromProviderSelection(cachedCatalogEntries.value, {
    ...model,
    provider_id: providerId,
    adapter_id: adapterId,
  })
  providerModelDraft.value.adapter_id = adapterId
  providerModelSetDefault.value = false
  catalogCopyProviderId.value = providerId
  catalogCopyAdapterId.value = adapterId
  setConfigMessage(`Loaded ${providerId}/${adapterId}/${model.id} into provider model draft.`)
}

async function addDiscoveredAdapterCatalogMatches(providerId: string, discovery: ProviderAdapterDiscovery) {
  const matchedModels = matchedCatalogModelDefinitions(cachedCatalogEntries.value, discovery.models)
  const providerPatch: Record<string, unknown> = {
    adapters: {
      [discovery.adapter_id]: {
        enabled: true,
        ...(Object.keys(matchedModels).length ? { models: matchedModels } : {}),
      },
    },
  }

  submittingConfig.value = true
  try {
    await patchSettings({
      path: 'providers',
      changes: {
        [providerId]: providerPatch,
      },
      validate: true,
      reload: true,
    })
    setConfigMessage(`Saved ${providerId}/${discovery.adapter_id} with ${Object.keys(matchedModels).length} catalog-matched model(s).`)
    await props.load()
    providerAdapterDiscoveries[providerId] = await discoverSavedProviderAdapters(providerId)
  } catch (err) {
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    submittingConfig.value = false
  }
}

async function patchProviderAdapterModel(input: {
  providerId: string
  adapterId: string
  modelId: string
  definition: Record<string, unknown>
  setDefault: boolean
}) {
  const providerId = input.providerId.trim()
  const adapterId = input.adapterId.trim()
  const modelId = input.modelId.trim()
  if (!providerId || !adapterId || !modelId) {
    setConfigError('provider_id, adapter_id and model_id are required.')
    return
  }

  const providerPatch: Record<string, unknown> = {
    adapters: {
      [adapterId]: {
        enabled: true,
        models: {
          [modelId]: input.definition,
        },
      },
    },
  }
  if (input.setDefault) {
    providerPatch.default_adapter = adapterId
    providerPatch.default_model = modelId
  }

  submittingConfig.value = true
  try {
    await patchSettings({
      path: 'providers',
      changes: {
        [providerId]: providerPatch,
      },
      validate: true,
      reload: true,
    })
    setConfigMessage(`Saved ${providerId}/${adapterId}/${modelId}.`)
    await props.load()
  } catch (err) {
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    submittingConfig.value = false
  }
}

async function createProvider() {
  const providerId = providerCreateDraft.provider_id.trim()
  const adapterId = providerCreateDraft.adapter_id.trim()
  const modelId = providerCreateDraft.model_id.trim()
  if (!providerId || !adapterId || !modelId) {
    setConfigError('Provider ID, adapter ID and model ID are required.')
    return
  }
  if (providerCreateDraft.auth_mode === 'api' && !providerCreateDraft.base_url.trim()) {
    setConfigError('API auth providers require a base URL.')
    return
  }

  const auth =
    providerCreateDraft.auth_mode === 'none'
      ? { mode: 'none' }
      : {
          mode: 'api',
          base_url: providerCreateDraft.base_url.trim(),
          ...(optionalText(providerCreateDraft.api_key_env)
            ? { api_key_env: optionalText(providerCreateDraft.api_key_env) }
            : {}),
          ...(optionalText(providerCreateDraft.api_key) ? { api_key: optionalText(providerCreateDraft.api_key) } : {}),
  }

  await ensureCatalogEntriesForModelIds([
    providerCreateDraft.catalog_model_id.trim() || modelId,
    ...draftAdapterDiscoveries.value.flatMap((discovery) =>
      discovery.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
    ),
  ])

  const adaptersPatch = buildAdaptersPatchFromDraftSelection({
    catalogEntries: cachedCatalogEntries.value,
    discoveries: draftAdapterDiscoveries.value,
    selectedAdapterIds: draftSelectedAdapterIds.value,
    defaultAdapterId: adapterId,
    defaultModelId: modelId,
    defaultCatalogModelId: providerCreateDraft.catalog_model_id.trim(),
  })

  submittingConfig.value = true
  try {
    await patchSettings({
      path: 'providers',
      changes: {
        [providerId]: {
          enabled: true,
          default_adapter: adapterId,
          default_model: modelId,
          auth,
          adapters: adaptersPatch,
        },
      },
      validate: true,
      reload: true,
    })
    const addedAdapterCount = Object.keys(adaptersPatch).length
    setConfigMessage(`Created provider ${providerId} with ${addedAdapterCount} adapter(s); default ${adapterId}/${modelId}.`)
    await props.load()
  } catch (err) {
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    submittingConfig.value = false
  }
}

function loadCatalogEntryIntoProviderDraft(entry: ModelCatalogEntry) {
  providerModelDraft.value = createModelCatalogDraftFromEntry(entry)
  providerModelDraft.value.adapter_id = catalogCopyAdapterId.value || providerModelDraft.value.adapter_id
  providerModelProviderId.value = catalogCopyProviderId.value || providerModelProviderId.value
  providerModelSetDefault.value = catalogCopySetDefault.value
  setConfigMessage(`Loaded ${entry.model_id} into provider model draft.`)
}

async function loadLiveModelIntoProviderDraft(model: ProviderModel) {
  await ensureCatalogEntriesForModelIds([catalogLookupIdForProviderModel(model) || model.id])
  providerModelProviderId.value = model.provider_id
  providerModelDraft.value = createModelCatalogDraftFromProviderSelection(cachedCatalogEntries.value, model)
  providerModelDraft.value.adapter_id = providerModelDraft.value.adapter_id || String(model.adapter_id || '')
  providerModelSetDefault.value = false
  setConfigMessage(
    `Loaded ${model.provider_id}/${model.adapter_id || 'adapter'}/${model.id} into provider model draft.`,
  )
}

async function saveProviderModelDraft() {
  await patchProviderAdapterModel({
    providerId: providerModelProviderId.value,
    adapterId: providerModelDraft.value.adapter_id,
    modelId: providerModelDraft.value.model_id,
    definition: buildConfiguredProviderModelFromDraft(providerModelDraft.value),
    setDefault: providerModelSetDefault.value,
  })
}

async function copyCatalogEntryToProvider(entry: ModelCatalogEntry) {
  await patchProviderAdapterModel({
    providerId: catalogCopyProviderId.value,
    adapterId: catalogCopyAdapterId.value,
    modelId: entry.model_id,
    definition: modelDefinitionFromEntry(entry),
    setDefault: catalogCopySetDefault.value,
  })
}

onMounted(() => {
  void loadCatalogPage(0)
})
</script>

<template>
  <div class="settings-page">
    <section class="settings-panel">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Provider Config</p>
          <h3 class="settings-panel-title">Adapters and Models</h3>
        </div>
      </div>

      <div class="settings-summary">
        <div class="summary-item">
          <div class="summary-label">Configured Providers</div>
          <div class="summary-value">{{ providerConfigCount }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Catalog Entries</div>
          <div class="summary-value">{{ catalogSummary?.entry_count ?? 0 }}</div>
        </div>
        <div class="summary-item">
          <div class="summary-label">Copy Target</div>
          <div class="summary-value mono">{{ catalogCopyProviderId || 'unset' }}</div>
        </div>
      </div>

      <p v-if="props.actionMessage.value" class="muted" style="margin-top: 12px">{{ props.actionMessage.value }}</p>
      <p v-if="props.actionError.value" class="muted" style="margin-top: 8px">{{ props.actionError.value }}</p>
    </section>

    <section class="record-card">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Create Provider</p>
          <h3 class="settings-panel-title">New Runtime Provider</h3>
        </div>
        <div class="button-row">
          <button
            class="button"
            :disabled="submittingConfig || discoveringDraftAdapters"
            @click="discoverCreateProviderAdapters"
          >
            {{ discoveringDraftAdapters ? 'Detecting…' : 'Detect Adapters' }}
          </button>
          <button class="button primary" :disabled="submittingConfig" @click="createProvider">Create Provider</button>
        </div>
      </div>

      <div class="form-grid">
        <div class="field">
          <label class="label" for="provider-create-id">Provider ID</label>
          <input
            id="provider-create-id"
            v-model="providerCreateDraft.provider_id"
            class="input mono"
            placeholder="shared-gateway"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-create-auth">Auth Mode</label>
          <select id="provider-create-auth" v-model="providerCreateDraft.auth_mode" class="select">
            <option value="api">api</option>
            <option value="none">none</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="provider-create-base">Base URL</label>
          <input
            id="provider-create-base"
            v-model="providerCreateDraft.base_url"
            class="input mono"
            placeholder="https://api.example.com"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-create-key-env">API Key Env</label>
          <input
            id="provider-create-key-env"
            v-model="providerCreateDraft.api_key_env"
            class="input mono"
            placeholder="OPENAI_API_KEY"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-create-adapter">Adapter</label>
          <select id="provider-create-adapter" v-model="providerCreateDraft.adapter_id" class="select">
            <option v-for="adapterId in ADAPTER_OPTIONS" :key="adapterId" :value="adapterId">{{ adapterId }}</option>
          </select>
        </div>
        <div class="field">
          <label class="label" for="provider-create-model">Initial Model</label>
          <input
            id="provider-create-model"
            v-model="providerCreateDraft.model_id"
            @input="providerCreateDraft.catalog_model_id = ''"
            class="input mono"
            placeholder="gpt-4.1-mini"
          />
        </div>
        <div class="field full">
          <label class="label" for="provider-create-key">Inline API Key</label>
          <input
            id="provider-create-key"
            v-model="providerCreateDraft.api_key"
            class="input mono"
            type="password"
            placeholder="optional"
          />
        </div>
      </div>
      <p class="muted" style="margin-top: 12px">
        Provider auth keeps one base URL and one credential. HTTP adapters derive their own protocol endpoints from that root.
      </p>

      <div v-if="draftAdapterDiscoveries.length" class="record-list" style="margin-top: 12px">
        <article
          v-for="adapter in draftAdapterDiscoveries"
          :key="`draft-${adapter.adapter_id}`"
          class="record-card"
        >
          <div class="record-header">
            <div>
              <p class="settings-panel-kicker">Draft Adapter</p>
              <h3 class="record-title">{{ adapter.adapter_id }}</h3>
              <div class="record-subtitle mono">
                {{ adapter.resolved_base_url || 'no resolved base URL' }}
              </div>
            </div>
            <div class="record-meta">
              <span class="badge" :class="adapter.supported ? 'success' : 'danger'">
                {{ adapter.supported ? 'supported' : 'unsupported' }}
              </span>
            </div>
          </div>
          <label v-if="adapter.supported" class="muted" style="display: flex; gap: 8px; align-items: center">
            <input v-model="draftSelectedAdapterIds" type="checkbox" :value="adapter.adapter_id" />
            Add this adapter when creating the provider
          </label>
          <p v-if="adapter.error" class="muted">{{ adapter.error }}</p>
          <p v-if="adapter.supported" class="muted">
            Catalog matched: {{ discoveryMatchedModels(cachedCatalogEntries, adapter).length }} · unmatched:
            {{ discoveryUnmatchedModels(cachedCatalogEntries, adapter).length }}
          </p>
          <div v-if="adapter.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button
              v-for="model in adapter.models"
              :key="`${adapter.adapter_id}-${model.id}`"
              class="button"
              :disabled="submittingConfig"
              @click="useDiscoveredCreateModel(adapter.adapter_id, model)"
            >
              {{ model.display_name || model.id }}
            </button>
          </div>
          <div v-if="adapter.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <span
              v-for="model in adapter.models"
              :key="`${adapter.adapter_id}-${model.id}-catalog`"
              class="badge"
              :class="modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'success' : 'warn'"
            >
              {{ model.id }} · {{ modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'catalog' : 'unmatched' }}
            </span>
          </div>
          <p v-if="discoveryUnmatchedModels(cachedCatalogEntries, adapter).length" class="muted" style="margin-top: 10px">
            Unmatched: {{ discoveryUnmatchedModels(cachedCatalogEntries, adapter).map((model) => model.id).join(', ') }}
          </p>
          <p v-else-if="adapter.supported" class="muted" style="margin-top: 10px">No models returned.</p>
        </article>
      </div>
    </section>

    <section v-if="props.providers.length" class="record-list">
      <article v-for="provider in props.providers" :key="provider.provider_id" class="record-card">
        <div class="record-header">
          <div>
            <p class="settings-panel-kicker">{{ provider.provider_id }}</p>
            <h3 class="record-title">{{ provider.provider_id }}</h3>
            <div class="record-subtitle mono">
              {{ provider.default_adapter || 'auto' }} · {{ provider.default_model || 'default unset' }}
            </div>
          </div>
          <div class="record-meta">
            <span
              v-for="adapter in provider.adapters || []"
              :key="adapter.adapter_id"
              class="badge"
              :class="adapter.enabled ? 'success' : 'neutral'"
            >
              {{ adapter.adapter_id }} · {{ adapter.configured_model_count }}
            </span>
          </div>
        </div>

        <div class="button-row" style="margin-top: 12px">
          <button
            class="button"
            :disabled="submittingConfig || !!discoveringSavedProviderIds[provider.provider_id]"
            @click="discoverExistingProviderAdapters(provider.provider_id)"
          >
            {{ discoveringSavedProviderIds[provider.provider_id] ? 'Inspecting…' : 'Inspect Adapters' }}
          </button>
        </div>

        <div
          v-if="(props.providerModels[provider.provider_id] || []).length"
          class="button-row"
          style="margin-top: 12px; flex-wrap: wrap"
        >
          <button
            v-for="model in props.providerModels[provider.provider_id] || []"
            :key="model.id"
            class="button"
            :disabled="submittingConfig"
            @click="loadLiveModelIntoProviderDraft(model)"
          >
            Edit {{ model.display_name || model.id }}
          </button>
        </div>
        <p v-else class="muted" style="margin-top: 12px">No live models loaded for this provider.</p>

        <div
          v-if="(providerAdapterDiscoveries[provider.provider_id] || []).length"
          class="record-list"
          style="margin-top: 12px"
        >
          <article
            v-for="adapter in providerAdapterDiscoveries[provider.provider_id] || []"
            :key="`${provider.provider_id}-${adapter.adapter_id}`"
            class="record-card"
          >
            <div class="record-header">
              <div>
                <p class="settings-panel-kicker">Detected Adapter</p>
                <h3 class="record-title">{{ adapter.adapter_id }}</h3>
                <div class="record-subtitle mono">
                  {{ adapter.resolved_base_url || 'no resolved base URL' }}
                </div>
              </div>
              <div class="record-meta">
                <span class="badge" :class="adapter.supported ? 'success' : 'danger'">
                  {{ adapter.supported ? 'supported' : 'unsupported' }}
                </span>
              </div>
            </div>
            <p v-if="adapter.error" class="muted">{{ adapter.error }}</p>
            <p v-if="adapter.supported" class="muted">
              Catalog matched: {{ discoveryMatchedModels(cachedCatalogEntries, adapter).length }} · unmatched:
              {{ discoveryUnmatchedModels(cachedCatalogEntries, adapter).length }}
            </p>
            <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button
                class="button"
                :disabled="submittingConfig || !discoveryMatchedModels(cachedCatalogEntries, adapter).length"
                @click="addDiscoveredAdapterCatalogMatches(provider.provider_id, adapter)"
              >
                Add Catalog Matches
              </button>
            </div>
            <div v-if="adapter.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button
                v-for="model in adapter.models"
                :key="`${provider.provider_id}-${adapter.adapter_id}-${model.id}`"
                class="button"
                :disabled="submittingConfig"
                @click="loadDiscoveredProviderModel(provider.provider_id, adapter.adapter_id, model)"
              >
                {{ model.display_name || model.id }}
              </button>
            </div>
            <div v-if="adapter.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <span
                v-for="model in adapter.models"
                :key="`${provider.provider_id}-${adapter.adapter_id}-${model.id}-catalog`"
                class="badge"
                :class="modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'success' : 'warn'"
              >
                {{ model.id }} · {{ modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'catalog' : 'unmatched' }}
              </span>
            </div>
            <p
              v-if="discoveryUnmatchedModels(cachedCatalogEntries, adapter).length"
              class="muted"
              style="margin-top: 10px"
            >
              Unmatched: {{ discoveryUnmatchedModels(cachedCatalogEntries, adapter).map((model) => model.id).join(', ') }}
            </p>
            <p v-else-if="adapter.supported" class="muted" style="margin-top: 10px">No models returned.</p>
          </article>
        </div>
      </article>
    </section>

    <section class="record-card">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Provider Model Draft</p>
          <h3 class="settings-panel-title">Add or Update Adapter Model</h3>
        </div>
        <button class="button primary" :disabled="submittingConfig" @click="saveProviderModelDraft">Save Model</button>
      </div>

      <div class="form-grid">
        <div class="field">
          <label class="label" for="provider-model-provider">Provider ID</label>
          <input
            id="provider-model-provider"
            v-model="providerModelProviderId"
            class="input mono"
            placeholder="shared-gateway"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-adapter">Adapter ID</label>
          <input
            id="provider-model-adapter"
            v-model="providerModelDraft.adapter_id"
            class="input mono"
            placeholder="openai"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-id">Model ID</label>
          <input
            id="provider-model-id"
            v-model="providerModelDraft.model_id"
            class="input mono"
            placeholder="gpt-4.1-mini"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-display">Display Name</label>
          <input
            id="provider-model-display"
            v-model="providerModelDraft.display_name"
            class="input"
            placeholder="GPT-4.1 Mini"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-lifecycle">Lifecycle</label>
          <input
            id="provider-model-lifecycle"
            v-model="providerModelDraft.lifecycle"
            class="input mono"
            placeholder="active"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-context">Context Window</label>
          <input
            id="provider-model-context"
            v-model="providerModelDraft.context_window_tokens"
            class="input mono"
            inputmode="numeric"
            placeholder="128000"
          />
        </div>
        <div class="field">
          <label class="label" for="provider-model-output">Max Output</label>
          <input
            id="provider-model-output"
            v-model="providerModelDraft.max_output_tokens"
            class="input mono"
            inputmode="numeric"
            placeholder="8192"
          />
        </div>
        <div class="field full">
          <label class="label" for="provider-model-description">Description</label>
          <textarea id="provider-model-description" v-model="providerModelDraft.description" class="input" rows="2" />
        </div>
      </div>
      <div class="button-row" style="margin-top: 12px; flex-wrap: wrap">
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelDraft.tool_calling" type="checkbox" />
          Tool calling
        </label>
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelDraft.streaming" type="checkbox" />
          Streaming
        </label>
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelDraft.reasoning" type="checkbox" />
          Reasoning
        </label>
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelDraft.structured_output" type="checkbox" />
          Structured output
        </label>
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelDraft.temperature_supported" type="checkbox" />
          Temperature
        </label>
        <label class="muted" style="display: flex; gap: 8px; align-items: center">
          <input v-model="providerModelSetDefault" type="checkbox" />
          Set provider default
        </label>
      </div>
    </section>

    <section class="record-card">
      <div class="settings-panel-header">
        <div>
          <p class="settings-panel-kicker">Model Catalog</p>
          <h3 class="settings-panel-title">Copy Models to Provider Adapter</h3>
        </div>
        <div class="inline-fields" style="align-items: end">
          <div class="field">
            <label class="label" for="catalog-copy-target">Target Provider</label>
            <select id="catalog-copy-target" v-model="catalogCopyProviderId" class="select">
              <option value="">Select provider</option>
              <option v-for="provider in props.providers" :key="provider.provider_id" :value="provider.provider_id">
                {{ provider.provider_id }}
              </option>
            </select>
          </div>
          <div class="field">
            <label class="label" for="catalog-copy-adapter">Target Adapter</label>
            <select id="catalog-copy-adapter" v-model="catalogCopyAdapterId" class="select">
              <option v-for="adapterId in catalogCopyAdapterOptions" :key="adapterId" :value="adapterId">
                {{ adapterId }}
              </option>
            </select>
          </div>
          <label class="muted" style="display: flex; gap: 8px; align-items: center">
            <input v-model="catalogCopySetDefault" type="checkbox" />
            Set default
          </label>
        </div>
      </div>

      <div class="inline-fields" style="margin-top: 12px; align-items: end">
        <div class="field" style="flex: 1 1 320px">
          <label class="label" for="catalog-search">Search Catalog</label>
          <input
            id="catalog-search"
            v-model="catalogSearchQuery"
            class="input mono"
            placeholder="model id, display name, origin"
          />
        </div>
        <div class="button-row">
          <button class="button primary" :disabled="submittingConfig" @click="searchCatalogPage">Search</button>
          <button class="button" :disabled="submittingConfig || catalogSearchOffset <= 0" @click="previousCatalogPage">
            Previous
          </button>
          <button
            class="button"
            :disabled="submittingConfig || catalogSearchOffset + sortedCatalogEntries.length >= catalogSearchTotal"
            @click="nextCatalogPage"
          >
            Next
          </button>
        </div>
      </div>
      <p class="muted" style="margin-top: 10px">
        Showing {{ sortedCatalogEntries.length }} of {{ catalogSearchTotal }} catalog entries.
      </p>

      <div v-if="sortedCatalogEntries.length" class="record-list">
        <article v-for="entry in sortedCatalogEntries" :key="`${entry.model_id}/${entry.kind}`" class="record-card">
          <div class="record-header">
            <div>
              <p class="settings-panel-kicker">{{ entry.source_label || entry.source || entry.kind }}</p>
              <h3 class="record-title">{{ entry.display_name || entry.model_id }}</h3>
              <div class="record-subtitle mono">{{ entry.model_id }}</div>
            </div>
            <div class="record-meta">
              <span class="badge neutral">{{ entry.kind }}</span>
            </div>
          </div>
          <p v-if="entry.origin" class="muted">Origin: {{ entry.origin }}</p>
          <p v-if="entry.description" class="muted">{{ entry.description }}</p>
          <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button class="button" :disabled="submittingConfig" @click="loadCatalogEntryIntoProviderDraft(entry)">
              Load Draft
            </button>
            <button
              class="button primary"
              :disabled="submittingConfig || !catalogCopyProviderId || !catalogCopyAdapterId"
              @click="copyCatalogEntryToProvider(entry)"
            >
              Copy to Provider
            </button>
          </div>
        </article>
      </div>
      <p v-else class="muted">No model catalog entries loaded.</p>
    </section>

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
                {{
                  provider.provider_id === 'gitlab'
                    ? 'GitLab OAuth'
                    : isAtomGitProvider(provider.provider_id)
                      ? 'AtomGit OAuth'
                      : 'OAuth Redirect'
                }}
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
            <div v-if="!isAtomGitProvider(provider.provider_id)" class="field">
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
                {{ isAtomGitProvider(provider.provider_id) ? 'Poll Browser Login' : 'Finish Browser Login' }}
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
