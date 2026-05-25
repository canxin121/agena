<script setup lang="ts">
import { computed, onMounted, reactive, ref, watchEffect, type Ref } from 'vue'

import {
  listDraftProviderAdapterModels,
  listSavedProviderAdapterModels,
  listModelCatalogEntries,
  lookupModelCatalogEntries,
  patchSettings,
} from '../lib/agenaApi'
import {
  adapterModelsMatchedModels,
  adapterModelsUnmatchedModels,
  buildAdaptersPatchFromDraftSelection,
  configuredProviderModelDefinitions,
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
  ProviderAdapterModels,
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
    refreshing: false,
    entry_count: entries.length,
  }
}

const ADAPTER_OPTIONS = ['openai', 'anthropic', 'gemini', 'ollama', 'gitlab', 'amazon_bedrock'] as const
const SHARED_GATEWAY_MODEL_LIST_ADAPTERS = ['openai', 'anthropic', 'gemini'] as const
const submittingConfig = ref(false)
const catalogCopyProviderId = ref('')
const catalogCopyAdapterId = ref('openai')
const catalogCopySetDefault = ref(false)
const providerModelProviderId = ref('')
const providerModelSetDefault = ref(false)
const providerModelDraft = ref<ModelCatalogEditableDraft>(createEmptyModelCatalogDraft('openai', ''))
const draftAdapterModelLists = ref<ProviderAdapterModels[]>([])
const providerAdapterModelLists = reactive<Record<string, ProviderAdapterModels[]>>({})
const listingDraftAdapters = ref(false)
const listingSavedProviderIds = reactive<Record<string, boolean>>({})
const draftSelectedAdapterIds = ref<string[]>([])
const savedProviderSelectedAdapterIds = reactive<Record<string, string[]>>({})
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
type ProviderCreateNativeToolsProfile =
  | 'disabled'
  | 'openai_hosted_defaults'
  | 'anthropic_hosted_defaults'
  | 'gemini_hosted_defaults'
const providerCreateNativeToolsTouched = ref(false)
const providerCreateDraft = reactive({
  provider_id: '',
  auth_mode: 'api' as 'api' | 'none',
  base_url: '',
  api_key_env: '',
  api_key: '',
  adapter_id: 'openai',
  model_id: '',
  catalog_model_id: '',
  native_tools_profile: 'disabled' as ProviderCreateNativeToolsProfile,
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
    merged.set(entry.model_id, entry)
  }
  for (const entry of incoming) {
    merged.set(entry.model_id, entry)
  }
  return [...merged.values()].sort((left, right) => left.model_id.localeCompare(right.model_id))
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

function providerCreateBaseUrlHost(value: string) {
  const normalized = String(value || '').trim()
  if (!normalized) return ''
  try {
    return new URL(normalized).hostname.toLowerCase()
  } catch {
    return ''
  }
}

function suggestProviderCreateNativeToolsProfile(): ProviderCreateNativeToolsProfile | null {
  if (providerCreateDraft.auth_mode !== 'api') return null
  const host = providerCreateBaseUrlHost(providerCreateDraft.base_url)
  const adapterId = String(providerCreateDraft.adapter_id || '').trim()
  if (host === 'api.openai.com' && adapterId === 'openai') return 'openai_hosted_defaults'
  if ((host === 'api.anthropic.com' || host === 'api-staging.anthropic.com') && adapterId === 'anthropic') {
    return 'anthropic_hosted_defaults'
  }
  if (host === 'generativelanguage.googleapis.com' && adapterId === 'gemini') {
    return 'gemini_hosted_defaults'
  }
  return null
}

function availableProviderCreateNativeToolsProfile(): ProviderCreateNativeToolsProfile | null {
  const adapterId = String(providerCreateDraft.adapter_id || '').trim()
  if (adapterId === 'openai') return 'openai_hosted_defaults'
  if (adapterId === 'anthropic') return 'anthropic_hosted_defaults'
  if (adapterId === 'gemini') return 'gemini_hosted_defaults'
  return null
}

const providerCreateNativeToolsOptions = computed(() => {
  const options: Array<{ value: ProviderCreateNativeToolsProfile; label: string; detail: string }> = [
    {
      value: 'disabled',
      label: 'disabled',
      detail: 'Write providers.<id>.adapters.<adapter>.models.<model>.native_tools.enabled = false for the default model.',
    },
  ]
  const available = availableProviderCreateNativeToolsProfile()
  if (available === 'openai_hosted_defaults') {
    options.push({
      value: available,
      label: 'openai hosted defaults',
      detail: 'Write explicit hosted routes for web_search, file_search, and code_execution on the default model.',
    })
  } else if (available === 'anthropic_hosted_defaults') {
    options.push({
      value: available,
      label: 'anthropic hosted defaults',
      detail: 'Write an explicit hosted route for web_search on the default model.',
    })
  } else if (available === 'gemini_hosted_defaults') {
    options.push({
      value: available,
      label: 'gemini hosted defaults',
      detail: 'Write explicit hosted routes for web_search, url_context, and code_execution on the default model.',
    })
  }
  return options
})

const providerCreateNativeToolsDetail = computed(
  () =>
    providerCreateNativeToolsOptions.value.find((option) => option.value === providerCreateDraft.native_tools_profile)
      ?.detail || 'Remote native tools are written explicitly into providers.<id>.adapters.<adapter>.models.<model>.native_tools.',
)

watchEffect(() => {
  const suggested = suggestProviderCreateNativeToolsProfile() ?? 'disabled'
  const allowed = new Set(providerCreateNativeToolsOptions.value.map((option) => option.value))
  if (!allowed.has(providerCreateDraft.native_tools_profile)) {
    providerCreateDraft.native_tools_profile = suggested
    return
  }
  if (!providerCreateNativeToolsTouched.value) {
    providerCreateDraft.native_tools_profile = suggested
  }
})

function onProviderCreateNativeToolsProfileChange() {
  providerCreateNativeToolsTouched.value = true
}

function buildProviderCreateNativeToolsPatch(profile: ProviderCreateNativeToolsProfile) {
  if (profile === 'disabled') {
    return { enabled: false }
  }
  if (profile === 'openai_hosted_defaults') {
    return {
      enabled: true,
      routes: {
        web_search: 'provider_hosted',
        file_search: 'provider_hosted',
        code_execution: 'provider_hosted',
      },
    }
  }
  if (profile === 'anthropic_hosted_defaults') {
    return {
      enabled: true,
      routes: {
        web_search: 'provider_hosted',
      },
    }
  }
  return {
    enabled: true,
    routes: {
      web_search: 'provider_hosted',
      url_context: 'provider_hosted',
      code_execution: 'provider_hosted',
    },
  }
}

function applyNativeToolsToAdaptersPatch(
  adaptersPatch: Record<string, { enabled: boolean; models?: Record<string, Record<string, unknown>> }>,
  adapterId: string,
  modelId: string,
  nativeTools: Record<string, unknown>,
) {
  const existingAdapter = adaptersPatch[adapterId] || { enabled: true, models: {} }
  const existingModels = existingAdapter.models || {}
  const existingModel = existingModels[modelId] || {}
  adaptersPatch[adapterId] = {
    ...existingAdapter,
    enabled: true,
    models: {
      ...existingModels,
      [modelId]: {
        ...existingModel,
        native_tools: nativeTools,
      },
    },
  }
}

function providerNativeToolLabel(value: string) {
  return String(value || '')
    .split('_')
    .filter(Boolean)
    .join(' ')
}

function isSharedGatewayModelListAdapter(adapterId: string) {
  return SHARED_GATEWAY_MODEL_LIST_ADAPTERS.includes(adapterId as (typeof SHARED_GATEWAY_MODEL_LIST_ADAPTERS)[number])
}

function adapterModelListOptionsForSavedProvider(providerId: string): string[] {
  const provider = props.providers.find((item) => item.provider_id === providerId)
  const configured = new Set(
    (provider?.adapters || []).map((adapter) => String(adapter.adapter_id || '').trim()).filter(Boolean),
  )
  const hasSharedGatewayAdapter = [...configured].some((adapterId) => isSharedGatewayModelListAdapter(adapterId))
  if (configured.size === 0 || hasSharedGatewayAdapter) {
    for (const adapterId of SHARED_GATEWAY_MODEL_LIST_ADAPTERS) configured.add(adapterId)
  }
  return [...configured].sort((left, right) => left.localeCompare(right))
}

watchEffect(() => {
  for (const provider of props.providers) {
    if (!savedProviderSelectedAdapterIds[provider.provider_id]) {
      savedProviderSelectedAdapterIds[provider.provider_id] = []
    }
  }
})

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

async function listCreateProviderAdapterModels() {
  if (providerCreateDraft.auth_mode !== 'api') {
    setConfigError('Draft adapter model listing currently requires api auth.')
    return
  }
  if (!providerCreateDraft.base_url.trim()) {
    setConfigError('Listing adapter models requires a base URL.')
    return
  }
  const adapterIds = [
    ...new Set(draftSelectedAdapterIds.value.map((value) => String(value || '').trim()).filter(Boolean)),
  ]
  if (!adapterIds.length) {
    setConfigError('Listing adapter models requires at least one explicit adapter selection.')
    return
  }
  listingDraftAdapters.value = true
  try {
    draftAdapterModelLists.value = await listDraftProviderAdapterModels({
      providerId: providerCreateDraft.provider_id,
      baseUrl: providerCreateDraft.base_url,
      apiKey: providerCreateDraft.api_key,
      apiKeyEnv: providerCreateDraft.api_key_env,
      adapterIds,
    })
    await ensureCatalogEntriesForModelIds(
      draftAdapterModelLists.value.flatMap((adapterModels) =>
        adapterModels.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
      ),
    )
    setConfigMessage(`Listed adapter models for ${draftAdapterModelLists.value.length} draft adapters.`)
  } catch (err) {
    draftAdapterModelLists.value = []
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    listingDraftAdapters.value = false
  }
}

function useListedCreateModel(adapterId: string, model: ProviderModel) {
  providerCreateDraft.adapter_id = adapterId
  providerCreateDraft.model_id = model.id
  providerCreateDraft.catalog_model_id = catalogLookupIdForProviderModel(model)
  setConfigMessage(`Loaded ${adapterId}/${model.id} into provider create draft.`)
}

async function listExistingProviderAdapterModels(providerId: string) {
  const adapterIds = [
    ...new Set(
      (savedProviderSelectedAdapterIds[providerId] || []).map((value) => String(value || '').trim()).filter(Boolean),
    ),
  ]
  if (!adapterIds.length) {
    setConfigError(`Listing adapter models requires at least one explicit adapter selection for ${providerId}.`)
    return
  }
  listingSavedProviderIds[providerId] = true
  try {
    providerAdapterModelLists[providerId] = await listSavedProviderAdapterModels(providerId, { adapterIds })
    await ensureCatalogEntriesForModelIds(
      providerAdapterModelLists[providerId].flatMap((adapterModels) =>
        adapterModels.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
      ),
    )
    setConfigMessage(`Listed adapter models for ${providerId}.`)
  } catch (err) {
    providerAdapterModelLists[providerId] = []
    setConfigError(err instanceof Error ? err.message : String(err))
  } finally {
    listingSavedProviderIds[providerId] = false
  }
}

function loadListedProviderModel(providerId: string, adapterId: string, model: ProviderModel) {
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

async function saveListedAdapterModels(providerId: string, adapterModels: ProviderAdapterModels) {
  const matchedModels = matchedCatalogModelDefinitions(cachedCatalogEntries.value, adapterModels.models)
  const configuredModels = configuredProviderModelDefinitions(cachedCatalogEntries.value, adapterModels.models)
  const providerPatch: Record<string, unknown> = {
    adapters: {
      [adapterModels.adapter_id]: {
        enabled: true,
        models: configuredModels,
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
    setConfigMessage(
      `Saved ${providerId}/${adapterModels.adapter_id} with ${adapterModels.models.length} listed model(s); ${Object.keys(matchedModels).length} catalog matched.`,
    )
    await props.load()
    const refreshAdapterIds = [
      ...new Set(
        (savedProviderSelectedAdapterIds[providerId] || []).map((value) => String(value || '').trim()).filter(Boolean),
      ),
    ]
    providerAdapterModelLists[providerId] = refreshAdapterIds.length
      ? await listSavedProviderAdapterModels(providerId, { adapterIds: refreshAdapterIds })
      : []
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
    providerPatch.defaults = {
      adapter: adapterId,
      model: modelId,
    }
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
  const nativeTools = buildProviderCreateNativeToolsPatch(providerCreateDraft.native_tools_profile)

  await ensureCatalogEntriesForModelIds([
    providerCreateDraft.catalog_model_id.trim() || modelId,
    ...draftAdapterModelLists.value.flatMap((adapterModels) =>
      adapterModels.models.map((model) => catalogLookupIdForProviderModel(model) || model.id),
    ),
  ])

  const adaptersPatch = buildAdaptersPatchFromDraftSelection({
    catalogEntries: cachedCatalogEntries.value,
    adapterModelLists: draftAdapterModelLists.value,
    selectedAdapterIds: draftSelectedAdapterIds.value,
    defaultAdapterId: adapterId,
    defaultModelId: modelId,
    defaultCatalogModelId: providerCreateDraft.catalog_model_id.trim(),
  })
  applyNativeToolsToAdaptersPatch(adaptersPatch, adapterId, modelId, nativeTools)

  submittingConfig.value = true
  try {
    await patchSettings({
      path: 'providers',
      changes: {
        [providerId]: {
          enabled: true,
          defaults: {
            adapter: adapterId,
            model: modelId,
          },
          auth,
          adapters: adaptersPatch,
        },
      },
      validate: true,
      reload: true,
    })
    const addedAdapterCount = Object.keys(adaptersPatch).length
    setConfigMessage(
      `Created provider ${providerId} with ${addedAdapterCount} adapter(s); default ${adapterId}/${modelId}.`,
    )
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
            :disabled="submittingConfig || listingDraftAdapters || !draftSelectedAdapterIds.length"
            @click="listCreateProviderAdapterModels"
          >
            {{ listingDraftAdapters ? 'Listing…' : 'List Adapter Models' }}
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
        <div class="field">
          <label class="label" for="provider-create-native-tools">Remote Native Tools</label>
          <select
            id="provider-create-native-tools"
            v-model="providerCreateDraft.native_tools_profile"
            class="select"
            @change="onProviderCreateNativeToolsProfileChange"
          >
            <option v-for="option in providerCreateNativeToolsOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
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
        Provider auth keeps one base URL and one credential. HTTP adapters derive their own protocol endpoints from that
        root.
      </p>
      <p class="muted" style="margin-top: 8px">
        {{ providerCreateNativeToolsDetail }}
      </p>
      <div class="field full" style="margin-top: 12px">
        <label class="label">Adapters To List</label>
        <div class="button-row" style="flex-wrap: wrap">
          <label
            v-for="adapterId in SHARED_GATEWAY_MODEL_LIST_ADAPTERS"
            :key="`draft-adapter-model-list-${adapterId}`"
            class="muted"
            style="display: flex; gap: 8px; align-items: center"
          >
            <input v-model="draftSelectedAdapterIds" type="checkbox" :value="adapterId" />
            {{ adapterId }}
          </label>
        </div>
      </div>

      <div v-if="draftAdapterModelLists.length" class="record-list" style="margin-top: 12px">
        <article
          v-for="adapterModels in draftAdapterModelLists"
          :key="`draft-${adapterModels.adapter_id}`"
          class="record-card"
        >
          <div class="record-header">
            <div>
              <p class="settings-panel-kicker">Draft Adapter</p>
              <h3 class="record-title">{{ adapterModels.adapter_id }}</h3>
              <div class="record-subtitle mono">
                {{ adapterModels.resolved_base_url || 'no resolved base URL' }}
              </div>
            </div>
            <div class="record-meta">
              <span class="badge" :class="adapterModels.error ? 'danger' : 'success'">
                {{ adapterModels.error ? 'error' : 'loaded' }}
              </span>
            </div>
          </div>
          <label v-if="!adapterModels.error" class="muted" style="display: flex; gap: 8px; align-items: center">
            <input v-model="draftSelectedAdapterIds" type="checkbox" :value="adapterModels.adapter_id" />
            Add this adapter when creating the provider
          </label>
          <p v-if="adapterModels.error" class="muted">{{ adapterModels.error }}</p>
          <p v-if="!adapterModels.error" class="muted">
            Catalog matched: {{ adapterModelsMatchedModels(cachedCatalogEntries, adapterModels).length }} · unmatched:
            {{ adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels).length }}
          </p>
          <div v-if="adapterModels.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <button
              v-for="model in adapterModels.models"
              :key="`${adapterModels.adapter_id}-${model.id}`"
              class="button"
              :disabled="submittingConfig"
              @click="useListedCreateModel(adapterModels.adapter_id, model)"
            >
              {{ model.display_name || model.id }}
            </button>
          </div>
          <div v-if="adapterModels.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
            <span
              v-for="model in adapterModels.models"
              :key="`${adapterModels.adapter_id}-${model.id}-catalog`"
              class="badge"
              :class="modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'success' : 'warn'"
            >
              {{ model.id }} ·
              {{ modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'catalog' : 'unmatched' }}
            </span>
          </div>
          <p
            v-if="adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels).length"
            class="muted"
            style="margin-top: 10px"
          >
            Unmatched:
            {{
              adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels)
                .map((model) => model.id)
                .join(', ')
            }}
          </p>
          <p v-else-if="!adapterModels.error" class="muted" style="margin-top: 10px">No models returned.</p>
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
              {{ provider.defaults.adapter || 'auto' }} · {{ provider.defaults.model || 'default unset' }}
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
        <p v-if="provider.native_tools" class="muted" style="margin-top: 10px">
          Remote tools:
          {{
            provider.native_tools.enabled ? 'default model enabled' : 'default model disabled'
          }}
          · {{ provider.native_tools.model_count }} model(s) configured
          <template v-if="provider.native_tools.bindings?.length">
            ·
            {{
              provider.native_tools.bindings
                .map((binding) => `${providerNativeToolLabel(binding.tool)} (${binding.route})`)
                .join(', ')
            }}
          </template>
        </p>

        <div class="field full" style="margin-top: 12px">
          <label class="label">Adapters To List</label>
          <div class="button-row" style="flex-wrap: wrap">
            <label
              v-for="adapterId in adapterModelListOptionsForSavedProvider(provider.provider_id)"
              :key="`${provider.provider_id}-adapter-model-list-${adapterId}`"
              class="muted"
              style="display: flex; gap: 8px; align-items: center"
            >
              <input
                v-model="savedProviderSelectedAdapterIds[provider.provider_id]"
                type="checkbox"
                :value="adapterId"
              />
              {{ adapterId }}
            </label>
          </div>
        </div>

        <div class="button-row" style="margin-top: 12px">
          <button
            class="button"
            :disabled="
              submittingConfig ||
              !!listingSavedProviderIds[provider.provider_id] ||
              !(savedProviderSelectedAdapterIds[provider.provider_id] || []).length
            "
            @click="listExistingProviderAdapterModels(provider.provider_id)"
          >
            {{ listingSavedProviderIds[provider.provider_id] ? 'Listing…' : 'List Adapter Models' }}
          </button>
        </div>

        <div
          v-if="(providerAdapterModelLists[provider.provider_id] || []).length"
          class="record-list"
          style="margin-top: 12px"
        >
          <article
            v-for="adapterModels in providerAdapterModelLists[provider.provider_id] || []"
            :key="`${provider.provider_id}-${adapterModels.adapter_id}`"
            class="record-card"
          >
            <div class="record-header">
              <div>
                <p class="settings-panel-kicker">Listed Adapter</p>
                <h3 class="record-title">{{ adapterModels.adapter_id }}</h3>
                <div class="record-subtitle mono">
                  {{ adapterModels.resolved_base_url || 'no resolved base URL' }}
                </div>
              </div>
              <div class="record-meta">
                <span class="badge" :class="adapterModels.error ? 'danger' : 'success'">
                  {{ adapterModels.error ? 'error' : 'loaded' }}
                </span>
              </div>
            </div>
            <p v-if="adapterModels.error" class="muted">{{ adapterModels.error }}</p>
            <p v-if="!adapterModels.error" class="muted">
              Catalog matched: {{ adapterModelsMatchedModels(cachedCatalogEntries, adapterModels).length }} · unmatched:
              {{ adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels).length }}
            </p>
            <div class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button
                class="button"
                :disabled="submittingConfig || !adapterModels.models.length"
                @click="saveListedAdapterModels(provider.provider_id, adapterModels)"
              >
                Save Listed Models
              </button>
            </div>
            <div v-if="adapterModels.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <button
                v-for="model in adapterModels.models"
                :key="`${provider.provider_id}-${adapterModels.adapter_id}-${model.id}`"
                class="button"
                :disabled="submittingConfig"
                @click="loadListedProviderModel(provider.provider_id, adapterModels.adapter_id, model)"
              >
                {{ model.display_name || model.id }}
              </button>
            </div>
            <div v-if="adapterModels.models.length" class="button-row" style="margin-top: 10px; flex-wrap: wrap">
              <span
                v-for="model in adapterModels.models"
                :key="`${provider.provider_id}-${adapterModels.adapter_id}-${model.id}-catalog`"
                class="badge"
                :class="modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'success' : 'warn'"
              >
                {{ model.id }} ·
                {{ modelIsCatalogMatched(model.id, model.catalog_model_id || '') ? 'catalog' : 'unmatched' }}
              </span>
            </div>
            <p
              v-if="adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels).length"
              class="muted"
              style="margin-top: 10px"
            >
              Unmatched:
              {{
                adapterModelsUnmatchedModels(cachedCatalogEntries, adapterModels)
                  .map((model) => model.id)
                  .join(', ')
              }}
            </p>
            <p v-else-if="!adapterModels.error" class="muted" style="margin-top: 10px">No models returned.</p>
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
          <label class="label" for="provider-model-input">Max Input</label>
          <input
            id="provider-model-input"
            v-model="providerModelDraft.max_input_tokens"
            class="input mono"
            inputmode="numeric"
            placeholder="200000"
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
        <article v-for="entry in sortedCatalogEntries" :key="entry.model_id" class="record-card">
          <div class="record-header">
            <div>
              <p class="settings-panel-kicker">{{ entry.source_label || entry.source }}</p>
              <h3 class="record-title">{{ entry.display_name || entry.model_id }}</h3>
              <div class="record-subtitle mono">{{ entry.model_id }}</div>
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
