<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiDeleteBinLine, RiRefreshLine, RiSave3Line } from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'

type LooseRecord = Record<string, any>

type ProviderSummary = {
  provider_id: string
  defaults?: { adapter?: string | null; model?: string | null }
  adapters?: Array<{ adapter_id: string; enabled: boolean; configured_model_count: number }>
}

type ProviderModel = {
  provider_id?: string
  adapter_id?: string | null
  id: string
  display_name?: string | null
  native_compaction?: boolean
  capabilities?: LooseRecord
  metadata?: LooseRecord
  thinking_modes?: LooseRecord[]
  speed_modes?: LooseRecord
  [key: string]: any
}

type AdapterModels = {
  adapter_id: string
  enabled: boolean
  resolved_base_url?: string | null
  models: ProviderModel[]
  failure?: LooseRecord | null
}

type AdapterModelsResponse = {
  provider_id?: string
  adapters?: AdapterModels[]
}

type ProviderConfigDraft = LooseRecord & {
  source_provider_id?: string | null
  provider_id: string
  auth_kind: JsonValue
  auth: LooseRecord
  credential_drafts: LooseRecord
  default_adapter: string
  default_model: string
  request_timeout_secs: number
  connect_timeout_secs: number
}

type DraftField = {
  path: string
  label: string
  secret?: boolean
  type?: 'text' | 'number' | 'select'
  options?: Array<{ value: string; label: string; description?: string }>
  placeholder?: string
  readOnly?: boolean
  value?: string
  includeEmpty?: boolean
  emptyLabel?: string
  allowCustom?: boolean
}

type ModelField = { key: string; label: string; readOnly?: boolean }

const { t } = useI18n()
const toasts = useToastsStore()

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const providers = ref<ProviderSummary[]>([])
const selectedProviderId = ref('')
const draft = ref<ProviderConfigDraft | null>(null)
const adapterModels = ref<AdapterModels[]>([])
const selectedAdapterIds = ref<Set<string>>(new Set())
const selectedModelKeys = ref<Set<string>>(new Set())
const listingModels = ref(false)
const newModelId = ref('')
const authMessage = ref('')

const editingModel = ref<{ adapterId: string; modelId: string } | null>(null)
const modelValue = ref<LooseRecord | null>(null)
const modelJson = ref('')
const modelLoading = ref(false)
const modelSaving = ref(false)
const modelError = ref('')

const authModeOptions = [
  { value: 'unset', label: 'Unset', description: 'Choose an authentication mode before configuring credentials.' },
  { value: 'none', label: 'None', description: 'Provider does not require credentials.' },
  { value: 'api', label: 'API', description: 'API key or compatible API authentication.' },
  { value: 'credential', label: 'Credential', description: 'Interactive or persisted OAuth credentials.' },
]

const apiSubtypeOptions = [
  { value: 'custom', label: 'Custom API', description: 'OpenAI-compatible, Anthropic, or Gemini endpoints.' },
  { value: 'cline_api', label: 'Cline API', description: 'Cline-managed model service.' },
  { value: 'gitlab_api', label: 'GitLab API', description: 'GitLab model API using an API key.' },
  { value: 'bedrock_sigv4', label: 'Bedrock SigV4', description: 'AWS Bedrock signed requests.' },
]

const credentialSubtypeOptions = [
  { value: 'openai_chatgpt', label: 'OpenAI ChatGPT', description: 'ChatGPT/Codex OAuth credentials.' },
  { value: 'github_copilot', label: 'GitHub Copilot', description: 'GitHub Copilot device authentication.' },
  { value: 'gitlab', label: 'GitLab', description: 'GitLab OAuth credentials.' },
  { value: 'google_adc', label: 'Google ADC', description: 'Google Application Default Credentials.' },
  { value: 'sap_ai_core', label: 'SAP AI Core', description: 'SAP AI Core service key.' },
]

const loginKindOptions = [
  { value: 'Browser', label: 'Browser', description: 'Open a browser-based authorization flow.' },
  { value: 'Device', label: 'Device', description: 'Use a device-code flow in the terminal or browser.' },
]

const secretSourceOptions = [
  { value: 'Inline', label: 'Inline', description: 'Store the secret value in this provider draft.' },
  { value: 'Env', label: 'Environment', description: 'Read the secret from the configured environment source.' },
]

const apiKeyEnvironmentOptions = [
  'OPENAI_API_KEY',
  'ANTHROPIC_API_KEY',
  'GEMINI_API_KEY',
  'GITLAB_TOKEN',
  'GOOGLE_VERTEX_ACCESS_TOKEN',
  'SHARED_GATEWAY_API_KEY',
  'OPENCODE_API_KEY',
].map((value) => ({ value, label: value, description: 'Environment variable used by the provider runtime.' }))

const awsRegionOptions = [
  'us-east-1',
  'us-east-2',
  'us-west-1',
  'us-west-2',
  'ca-central-1',
  'sa-east-1',
  'eu-west-1',
  'eu-west-2',
  'eu-west-3',
  'eu-central-1',
  'eu-central-2',
  'eu-north-1',
  'eu-south-1',
  'eu-south-2',
  'ap-south-1',
  'ap-south-2',
  'ap-east-1',
  'ap-southeast-1',
  'ap-southeast-2',
  'ap-southeast-3',
  'ap-southeast-4',
  'ap-northeast-1',
  'ap-northeast-2',
  'ap-northeast-3',
  'me-south-1',
  'me-central-1',
  'af-south-1',
].map((value) => ({ value, label: value }))

const gitlabInstanceOptions = [
  { value: 'https://gitlab.com', label: 'https://gitlab.com', description: 'GitLab.com' },
]

const redirectUriOptions = [
  {
    value: 'http://localhost:1455/auth/callback',
    label: 'http://localhost:1455/auth/callback',
    description: 'Local OAuth callback used by the TUI and web runtime.',
  },
]

const defaultRedirectUri = 'http://localhost:1455/auth/callback'
const legacyRedirectUris = new Set(['http://127.0.0.1:1455/callback', 'http://127.0.0.1:1455/auth/callback'])

const awsProfiles = ref<string[]>(['default'])

const modelFields: ModelField[] = [
  { key: 'model_id', label: 'Model ID', readOnly: true },
  { key: 'enabled', label: 'Enabled' },
  { key: 'native_compaction', label: 'Native compaction' },
  { key: 'agena_tools.mode', label: 'Agena tool mode' },
  { key: 'display_name', label: 'Display name' },
  { key: 'lifecycle', label: 'Lifecycle' },
  { key: 'context_window_tokens', label: 'Context window tokens' },
  { key: 'max_input_tokens', label: 'Max input tokens' },
  { key: 'max_output_tokens', label: 'Max output tokens' },
  { key: 'capabilities.features', label: 'Features' },
  { key: 'capabilities.input', label: 'Input modalities' },
  { key: 'output_modalities', label: 'Output modalities' },
  { key: 'thinking_modes', label: 'Thinking modes' },
  { key: 'speed_modes', label: 'Speed modes' },
  { key: 'description', label: 'Description' },
]

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function record(value: unknown): LooseRecord {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as LooseRecord) : {}
}

function authKindMode(value: JsonValue): 'none' | 'api' | 'credential' | 'unset' {
  if (value === 'None') return 'none'
  if (typeof value === 'string' && ['Api', 'ApiPending', 'ClineApi', 'Gitlab', 'BedrockSigv4'].includes(value)) return 'api'
  if (value && typeof value === 'object' && !Array.isArray(value) && Object.prototype.hasOwnProperty.call(value, 'Credential')) return 'credential'
  return 'unset'
}

function authKindSubtype(value: JsonValue): string {
  if (typeof value === 'string') {
    if (value === 'Api') return 'custom'
    if (value === 'ClineApi') return 'cline_api'
    if (value === 'Gitlab') return 'gitlab_api'
    if (value === 'BedrockSigv4') return 'bedrock_sigv4'
    return ''
  }
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const issuer = (value as LooseRecord).Credential
    return typeof issuer === 'string' ? issuer : ''
  }
  return ''
}

const authMode = computed(() => authKindMode(draft.value?.auth_kind))
const authSubtype = computed(() => authKindSubtype(draft.value?.auth_kind))
const authSubtypeOptions = computed(() => (authMode.value === 'credential' ? credentialSubtypeOptions : apiSubtypeOptions))

const adapterRuleMap: Record<string, string[]> = {
  none: ['ollama'],
  api: ['openai_responses', 'openai_chat_completions', 'openai_realtime', 'anthropic', 'gemini'],
  cline_api: ['openai_chat_completions'],
  gitlab_api: ['openai_responses', 'openai_chat_completions', 'anthropic'],
  bedrock_sigv4: ['amazon_bedrock'],
  openai_chatgpt: ['openai_responses'],
  github_copilot: ['openai_responses', 'openai_chat_completions', 'anthropic'],
  gitlab: ['openai_responses', 'openai_chat_completions', 'anthropic'],
  google_adc: ['openai_chat_completions'],
  sap_ai_core: ['openai_chat_completions'],
}

const adapterCandidates = computed(() => {
  const known = (draft.value?.provider_id ? providers.value.find((item) => item.provider_id === draft.value?.source_provider_id || item.provider_id === draft.value?.provider_id)?.adapters || [] : [])
    .map((item) => item.adapter_id)
  const pendingApiSubtype = draft.value?.auth_kind === 'ApiPending'
  const ruleAdapters = pendingApiSubtype ? [] : (adapterRuleMap[authSubtype.value] || adapterRuleMap[authMode.value] || [])
  return [...new Set([...ruleAdapters, ...(pendingApiSubtype ? [] : known)])].filter(Boolean)
})

const awsProfileOptions = computed(() => {
  const current = fieldValue('auth.profile')
  return [...new Set(['default', ...awsProfiles.value, current].filter(Boolean))].map((value) => ({
    value,
    label: value,
    description: value === 'default' ? 'Use the AWS SDK default profile.' : 'AWS shared credentials profile.',
  }))
})

const allModels = computed(() => adapterModels.value.flatMap((adapter) => adapter.models.map((model) => ({ adapter, model }))))
const selectedDefaultModelKey = computed(() => {
  const adapter = String(draft.value?.default_adapter || '')
  const model = String(draft.value?.default_model || '')
  return adapter && model ? `${adapter}\u001f${model}` : ''
})
const defaultModelOptions = computed(() =>
  allModels.value.map(({ adapter, model }) => ({
    value: `${adapter.adapter_id}\u001f${model.id}`,
    label: model.display_name || model.id,
    description: `${adapter.adapter_id} / ${model.id}`,
  })),
)

function providerLabel(provider: ProviderSummary): string {
  const defaultLabel = [provider.defaults?.adapter, provider.defaults?.model].filter(Boolean).join(' / ')
  return defaultLabel ? `${provider.provider_id} · ${defaultLabel}` : provider.provider_id
}

function modelKey(adapterId: string, modelId: string): string {
  return `${adapterId}\u001f${modelId}`
}

function adapterFailure(adapter: AdapterModels): string {
  return String(adapter.failure?.user?.fallback || adapter.failure?.rendered || adapter.failure?.message || '').trim()
}

function normalizeDraftShape(value: ProviderConfigDraft): ProviderConfigDraft {
  const next = clone(value)
  next.auth = record(next.auth)
  next.credential_drafts = record(next.credential_drafts)

  const openai = (next.credential_drafts.openai_chatgpt = record(next.credential_drafts.openai_chatgpt))
  next.credential_drafts.github_copilot = record(next.credential_drafts.github_copilot)
  const gitlab = (next.credential_drafts.gitlab = record(next.credential_drafts.gitlab))

  const openaiRedirect = String(openai.redirect_uri || '').trim()
  if (!openaiRedirect || legacyRedirectUris.has(openaiRedirect)) openai.redirect_uri = defaultRedirectUri
  if (!String(gitlab.redirect_uri || '').trim()) gitlab.redirect_uri = defaultRedirectUri

  const mode = authKindMode(next.auth_kind)
  const subtype = authKindSubtype(next.auth_kind)
  next.auth.credential_issuer = mode === 'credential' ? subtype : ''

  const clearAuthFields = (...keys: string[]) => {
    for (const key of keys) next.auth[key] = ''
  }

  if (mode === 'unset' || mode === 'none') {
    next.auth.secret_source_kind = 'Unset'
    next.auth.secret_source_value = ''
    clearAuthFields('base_url', 'region', 'profile', 'access_key_id', 'secret_access_key', 'session_token', 'service_key_env')
  } else if (mode === 'api' && subtype === 'bedrock_sigv4') {
    next.auth.secret_source_kind = 'Unset'
    next.auth.secret_source_value = ''
    clearAuthFields('service_key_env')
  } else if (mode === 'api') {
    clearAuthFields('region', 'profile', 'access_key_id', 'secret_access_key', 'session_token', 'service_key_env')
    if (subtype === 'cline_api') clearAuthFields('base_url')
    if (subtype === 'gitlab_api' && !String(next.auth.instance_url || '').trim()) next.auth.instance_url = 'https://gitlab.com'
  } else if (mode === 'credential') {
    next.auth.secret_source_kind = 'Unset'
    next.auth.secret_source_value = ''
    clearAuthFields('region', 'profile', 'access_key_id', 'secret_access_key', 'session_token')
    if (!['google_adc', 'sap_ai_core'].includes(subtype)) clearAuthFields('base_url')
    if (subtype === 'gitlab' && !String(next.auth.instance_url || '').trim()) next.auth.instance_url = 'https://gitlab.com'
    if (subtype === 'sap_ai_core') {
      if (!String(next.auth.service_key_env || '').trim()) next.auth.service_key_env = 'AICORE_SERVICE_KEY'
    } else {
      clearAuthFields('service_key_env')
    }
  }

  const supportedAdapters = next.auth_kind === 'ApiPending'
    ? []
    : adapterRuleMap[subtype] || adapterRuleMap[mode] || []
  next.default_adapter = String(next.default_adapter || '').trim()
  next.default_model = String(next.default_model || '').trim()
  if (next.default_adapter && !supportedAdapters.includes(next.default_adapter)) next.default_adapter = ''
  if (!next.default_adapter) next.default_model = ''
  return next
}

function fieldValue(path: string): string {
  const parts = path.split('.')
  let current: any = draft.value
  for (const part of parts) current = current?.[part]
  if (current === undefined || current === null) return ''
  if (path === 'auth.secret_source_kind' && current === 'Unset') return ''
  return String(current)
}

function setFieldValue(path: string, value: string | number) {
  if (!draft.value) return
  const next = clone(draft.value)
  const parts = path.split('.')
  let target: LooseRecord = next
  for (const part of parts.slice(0, -1)) {
    if (!target[part] || typeof target[part] !== 'object') target[part] = {}
    target = target[part] as LooseRecord
  }
  if (path === 'auth.secret_source_kind') {
    const normalized = String(value).trim().toLowerCase()
    target[parts[parts.length - 1]] = normalized === 'inline' ? 'Inline' : normalized === 'env' ? 'Env' : 'Unset'
  } else {
    target[parts[parts.length - 1]] = typeof value === 'number' ? value : String(value)
  }
  draft.value = normalizeDraftShape(next)
}

function authDetailFields(): DraftField[] {
  const mode = authMode.value
  const subtype = authSubtype.value
  const secretSourceKind = fieldValue('auth.secret_source_kind').trim().toLowerCase()
  const apiSecretField: DraftField = secretSourceKind === 'env'
    ? {
        path: 'auth.secret_source_value',
        label: 'API key environment variable',
        type: 'select',
        options: apiKeyEnvironmentOptions,
        includeEmpty: true,
        emptyLabel: 'No environment variable',
        allowCustom: true,
      }
    : { path: 'auth.secret_source_value', label: 'API key value', secret: true }
  if (mode === 'api') {
    if (subtype === 'custom') return [
      { path: 'auth.base_url', label: 'Base URL', placeholder: 'https://api.example.com/v1' },
      { path: 'auth.secret_source_kind', label: 'API key source', type: 'select', options: secretSourceOptions, includeEmpty: true, emptyLabel: 'No API key source' },
      apiSecretField,
    ]
    if (subtype === 'cline_api') return [
      { path: 'auth.secret_source_kind', label: 'API key source', type: 'select', options: secretSourceOptions, includeEmpty: true, emptyLabel: 'No API key source' },
      apiSecretField,
    ]
    if (subtype === 'gitlab_api') return [
      { path: 'auth.instance_url', label: 'Instance URL', type: 'select', options: gitlabInstanceOptions, includeEmpty: true, emptyLabel: 'No GitLab instance' },
      { path: 'auth.secret_source_kind', label: 'API key source', type: 'select', options: secretSourceOptions, includeEmpty: true, emptyLabel: 'No API key source' },
      apiSecretField,
    ]
    if (subtype === 'bedrock_sigv4') return [
      { path: 'auth.base_url', label: 'Base URL' },
      { path: 'auth.region', label: 'Region', type: 'select', options: awsRegionOptions, includeEmpty: true, emptyLabel: 'No AWS region', allowCustom: true },
      { path: 'auth.profile', label: 'AWS profile', type: 'select', options: awsProfileOptions.value, includeEmpty: true, emptyLabel: 'No AWS profile', allowCustom: true },
      { path: 'auth.access_key_id', label: 'Access key ID', secret: true },
      { path: 'auth.secret_access_key', label: 'Secret access key', secret: true },
      { path: 'auth.session_token', label: 'Session token', secret: true },
    ]
  }
  if (mode !== 'credential') return []
  const base = subtype === 'openai_chatgpt' ? 'credential_drafts.openai_chatgpt' : subtype === 'github_copilot' ? 'credential_drafts.github_copilot' : 'credential_drafts.gitlab'
  if (subtype === 'openai_chatgpt') {
    const fields: DraftField[] = [
      { path: `${base}.login_kind`, label: 'Auth login method', type: 'select', options: loginKindOptions },
    ]
    const loginKind = fieldValue(`${base}.login_kind`).trim().toLowerCase()
    if (loginKind === 'browser') {
      fields.push(
        { path: `${base}.redirect_uri`, label: 'Redirect URI', type: 'select', options: redirectUriOptions, includeEmpty: true, emptyLabel: 'No redirect URI' },
        { path: `${base}.callback_url`, label: 'Callback URL' },
      )
    }
    fields.push(
      { path: `${base}.tokens.refresh_token`, label: 'Refresh token', secret: true },
      { path: `${base}.tokens.access_token`, label: 'Access token', secret: true },
      { path: `${base}.tokens.expires_at_ms`, label: 'Expires at (ms)' },
      { path: `${base}.account_id`, label: 'Account ID' },
    )
    return fields
  }
  if (subtype === 'github_copilot') return [
    { path: '__auth_login_method', value: 'Device', label: 'Auth login method', readOnly: true },
    { path: `${base}.enterprise_domain`, label: 'Enterprise domain' },
    { path: `${base}.tokens.refresh_token`, label: 'Refresh token', secret: true },
    { path: `${base}.tokens.access_token`, label: 'Access token', secret: true },
    { path: `${base}.tokens.expires_at_ms`, label: 'Expires at (ms)' },
  ]
  if (subtype === 'gitlab') return [
    { path: '__auth_login_method', value: 'Browser', label: 'Auth login method', readOnly: true },
    { path: 'auth.instance_url', label: 'Instance URL', type: 'select', options: gitlabInstanceOptions, includeEmpty: true, emptyLabel: 'No GitLab instance' },
    { path: `${base}.redirect_uri`, label: 'Redirect URI', type: 'select', options: redirectUriOptions, includeEmpty: true, emptyLabel: 'No redirect URI' },
    { path: `${base}.callback_url`, label: 'Callback URL' },
    { path: `${base}.tokens.refresh_token`, label: 'Refresh token', secret: true },
    { path: `${base}.tokens.access_token`, label: 'Access token', secret: true },
    { path: `${base}.tokens.expires_at_ms`, label: 'Expires at (ms)' },
  ]
  if (subtype === 'google_adc') return [{ path: 'auth.base_url', label: 'Base URL' }]
  if (subtype === 'sap_ai_core') return [
    { path: 'auth.base_url', label: 'Base URL' },
    { path: 'auth.service_key_env', label: 'Service key env', type: 'select', options: [{ value: 'AICORE_SERVICE_KEY', label: 'AICORE_SERVICE_KEY' }], includeEmpty: true, emptyLabel: 'No service key env' },
  ]
  return []
}

const visibleAuthFields = computed(() => authDetailFields())
const interactiveAuthAvailable = computed(() => ['openai_chatgpt', 'github_copilot', 'gitlab'].includes(authSubtype.value))

async function loadProviders() {
  const response = await apiJson<ProviderSummary[]>('/api/v1/providers')
  providers.value = Array.isArray(response) ? response : []
}

async function loadAwsProfiles() {
  try {
    const response = await apiJson<{ profiles?: unknown }>('/api/v1/providers/aws-profiles')
    const profiles = Array.isArray(response?.profiles)
      ? response.profiles.map((value) => String(value).trim()).filter(Boolean)
      : []
    awsProfiles.value = [...new Set(['default', ...profiles])]
  } catch {
    // The profile catalog is an optional convenience. Keep the TUI's default
    // profile available when the server cannot inspect its AWS credentials.
    awsProfiles.value = ['default']
  }
}

async function loadDraft(providerId?: string) {
  loading.value = true
  error.value = ''
  adapterModels.value = []
  selectedModelKeys.value = new Set()
  try {
    const query = providerId ? `?provider_id=${encodeURIComponent(providerId)}` : ''
    draft.value = normalizeDraftShape(await apiJson<ProviderConfigDraft>(`/api/v1/provider-studio/draft${query}`))
    selectedProviderId.value = providerId || ''
    const summary = providers.value.find((item) => item.provider_id === providerId)
    const configuredAdapters = providerId
      ? await apiJson<AdapterModels[]>(`/api/v1/providers/${encodeURIComponent(providerId)}/configured-models`)
      : []
    const enabled = new Set(
      (configuredAdapters.length
        ? configuredAdapters.filter((item) => item.enabled).map((item) => item.adapter_id)
        : (summary?.adapters || []).filter((item) => item.enabled).map((item) => item.adapter_id)),
    )
    selectedAdapterIds.value = enabled.size ? enabled : new Set(adapterCandidates.value)
    adapterModels.value = configuredAdapters
    selectedModelKeys.value = new Set(
      configuredAdapters
        .filter((adapter) => adapter.enabled)
        .flatMap((adapter) => adapter.models.map((model) => modelKey(adapter.adapter_id, model.id))),
    )
    await listDraftModels()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    draft.value = null
  } finally {
    loading.value = false
  }
}

async function listDraftModels() {
  if (!draft.value || selectedAdapterIds.value.size === 0) {
    adapterModels.value = []
    selectedModelKeys.value = new Set()
    return
  }
  listingModels.value = true
  try {
    const response = await apiJson<AdapterModelsResponse>('/api/v1/provider-studio/draft/models', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_ids: [...selectedAdapterIds.value] }),
    })
    const refreshed = Array.isArray(response?.adapters) ? response.adapters : []
    const byAdapter = new Map(adapterModels.value.map((adapter) => [adapter.adapter_id, adapter]))
    for (const adapter of refreshed) byAdapter.set(adapter.adapter_id, adapter)
    adapterModels.value = [...byAdapter.values()]

    // Match the TUI restore behavior: keep configured routes, discard routes
    // that are no longer available, and select every model only when a
    // selected adapter has no surviving route at all.
    const availableKeys = new Set(
      adapterModels.value.flatMap((adapter) => adapter.models.map((model) => modelKey(adapter.adapter_id, model.id))),
    )
    const nextKeys = new Set([...selectedModelKeys.value].filter((key) => availableKeys.has(key)))
    for (const adapter of adapterModels.value) {
      if (!selectedAdapterIds.value.has(adapter.adapter_id) || adapter.failure) continue
      const hasSelectedModel = adapter.models.some((model) => nextKeys.has(modelKey(adapter.adapter_id, model.id)))
      if (!hasSelectedModel) {
        for (const model of adapter.models) nextKeys.add(modelKey(adapter.adapter_id, model.id))
      }
    }
    selectedModelKeys.value = nextKeys
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    listingModels.value = false
  }
}

function toggleAdapter(adapterId: string) {
  const next = new Set(selectedAdapterIds.value)
  if (next.has(adapterId)) next.delete(adapterId)
  else next.add(adapterId)
  selectedAdapterIds.value = next
  void listDraftModels()
}

function toggleModel(adapterId: string, modelId: string) {
  const key = modelKey(adapterId, modelId)
  const next = new Set(selectedModelKeys.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  selectedModelKeys.value = next
}

function setAuthMode(value: string) {
  if (!draft.value) return
  const next = clone(draft.value)
  if (value === 'unset') next.auth_kind = 'Unset'
  else if (value === 'none') next.auth_kind = 'None'
  else if (value === 'api') next.auth_kind = 'ApiPending'
  else if (value === 'credential') next.auth_kind = { Credential: null }
  draft.value = normalizeDraftShape(next)
  selectedAdapterIds.value = new Set()
  adapterModels.value = []
  selectedModelKeys.value = new Set()
}

function setAuthSubtype(value: string) {
  if (!draft.value) return
  const next = clone(draft.value)
  if (authMode.value === 'credential') next.auth_kind = { Credential: value }
  else if (value === 'custom') next.auth_kind = 'Api'
  else if (value === 'cline_api') next.auth_kind = 'ClineApi'
  else if (value === 'gitlab_api') next.auth_kind = 'Gitlab'
  else if (value === 'bedrock_sigv4') next.auth_kind = 'BedrockSigv4'
  draft.value = normalizeDraftShape(next)
  selectedAdapterIds.value = new Set(adapterRuleMap[value] || [])
  adapterModels.value = []
  selectedModelKeys.value = new Set()
  void listDraftModels()
}

function setDefaultAdapter(value: string) {
  if (!draft.value) return
  const next = clone(draft.value)
  const adapter = String(value || '').trim()
  if (next.default_adapter !== adapter) next.default_model = ''
  next.default_adapter = adapter
  draft.value = normalizeDraftShape(next)
}

function setDefaultModel(value: string) {
  if (!draft.value) return
  const [adapter, model] = value.split('\u001f')
  const next = clone(draft.value)
  next.default_adapter = adapter || ''
  next.default_model = model || ''
  draft.value = normalizeDraftShape(next)
}

async function saveDraft() {
  if (!draft.value || saving.value) return
  saving.value = true
  error.value = ''
  try {
    const response = await apiJson<JsonValue>('/api/v1/provider-studio/save', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        draft: draft.value,
        adapter_model_lists: adapterModels.value,
        selected_adapter_ids: [...selectedAdapterIds.value],
        selected_model_keys: [...selectedModelKeys.value],
      }),
    })
    const savedId = String(record(record(response).ProviderDraftSaved).provider_id || draft.value.provider_id || '').trim()
    toasts.push('success', 'Provider configuration saved')
    await loadProviders()
    await loadDraft(savedId || selectedProviderId.value || undefined)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    toasts.push('error', error.value)
  } finally {
    saving.value = false
  }
}

async function deleteProvider() {
  const providerId = String(draft.value?.source_provider_id || draft.value?.provider_id || '').trim()
  if (!providerId || !window.confirm(`Delete provider ${providerId}?`)) return
  try {
    await apiJson('/api/v1/provider-studio/delete-provider', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ provider_id: providerId }),
    })
    toasts.push('success', 'Provider deleted')
    await loadProviders()
    await loadDraft()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function startAuth(action: 'start' | 'continue') {
  if (!draft.value) return
  try {
    const response = await apiJson<{ draft?: ProviderConfigDraft; message?: JsonValue; clipboard_text?: string | null }>(`/api/v1/provider-studio/auth/${action}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value }),
    })
    if (response?.draft) draft.value = normalizeDraftShape(response.draft)
    authMessage.value = response?.message ? JSON.stringify(response.message) : ''
    if (response?.clipboard_text && navigator.clipboard) await navigator.clipboard.writeText(response.clipboard_text)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function addManualModel() {
  const adapterId = String(draft.value?.default_adapter || [...selectedAdapterIds.value][0] || '').trim()
  const modelId = newModelId.value.trim()
  if (!draft.value || !adapterId || !modelId) return
  const adapter = adapterModels.value.find((item) => item.adapter_id === adapterId)
  const model: ProviderModel = {
    provider_id: draft.value.provider_id,
    adapter_id: adapterId,
    id: modelId,
    native_compaction: true,
    capabilities: {},
    metadata: {},
    thinking_modes: [],
    speed_modes: {},
  }
  if (adapter) adapter.models = [...adapter.models.filter((item) => item.id !== modelId), model]
  else adapterModels.value = [...adapterModels.value, { adapter_id: adapterId, enabled: true, models: [model] }]
  selectedAdapterIds.value = new Set([...selectedAdapterIds.value, adapterId])
  selectedModelKeys.value = new Set([...selectedModelKeys.value, modelKey(adapterId, modelId)])
  newModelId.value = ''
}

async function openModelEditor(adapterId: string, model: ProviderModel) {
  if (!draft.value) return
  editingModel.value = { adapterId, modelId: model.id }
  modelLoading.value = true
  modelError.value = ''
  try {
    const response = await apiJson<{ value?: JsonValue }>('/api/v1/provider-studio/draft/model', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_id: adapterId, model_id: model.id, provider_model: model }),
    })
    modelValue.value = record(response?.value)
    modelJson.value = JSON.stringify(modelValue.value, null, 2)
  } catch (reason) {
    modelError.value = reason instanceof Error ? reason.message : String(reason)
    modelValue.value = null
    modelJson.value = ''
  } finally {
    modelLoading.value = false
  }
}

function modelFieldValue(key: string): string {
  if (key === 'model_id') return editingModel.value?.modelId || ''
  const parts = key.split('.')
  let current: any = modelValue.value
  for (const part of parts) current = current?.[part]
  if (current === undefined || current === null) return ''
  return typeof current === 'string' ? current : JSON.stringify(current)
}

async function saveModel() {
  if (!draft.value || !editingModel.value || modelSaving.value) return
  modelSaving.value = true
  modelError.value = ''
  try {
    const parsed = JSON.parse(modelJson.value) as JsonValue
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('Model config must be a JSON object.')
    await apiJson('/api/v1/provider-studio/save-model', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        draft: draft.value,
        adapter_id: editingModel.value.adapterId,
        model_id: editingModel.value.modelId,
        model_value: parsed,
      }),
    })
    modelValue.value = record(parsed)
    modelJson.value = JSON.stringify(modelValue.value, null, 2)
    toasts.push('success', 'Provider model configuration saved')
    await loadProviders()
  } catch (reason) {
    modelError.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    modelSaving.value = false
  }
}

async function deleteModel(adapterId: string, modelId: string) {
  if (!draft.value || !window.confirm(`Delete model ${adapterId}/${modelId}?`)) return
  const providerId = draft.value.provider_id
  try {
    await apiJson('/api/v1/provider-studio/delete-model', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_id: adapterId, model_id: modelId }),
    })
    adapterModels.value = adapterModels.value.map((adapter) => adapter.adapter_id === adapterId ? { ...adapter, models: adapter.models.filter((model) => model.id !== modelId) } : adapter)
    const next = new Set(selectedModelKeys.value)
    next.delete(modelKey(adapterId, modelId))
    selectedModelKeys.value = next
    if (draft.value.default_adapter === adapterId && draft.value.default_model === modelId) {
      const nextDraft = clone(draft.value)
      nextDraft.default_model = ''
      draft.value = nextDraft
    }
    if (editingModel.value?.adapterId === adapterId && editingModel.value?.modelId === modelId) editingModel.value = null
    toasts.push('success', 'Provider model deleted')
    await loadProviders()
    await loadDraft(providerId)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function deleteAdapter(adapterId: string) {
  if (!draft.value || !window.confirm(`Delete adapter ${adapterId}?`)) return
  const providerId = draft.value.provider_id
  try {
    await apiJson('/api/v1/provider-studio/delete-adapter', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_id: adapterId }),
    })
    selectedAdapterIds.value = new Set([...selectedAdapterIds.value].filter((id) => id !== adapterId))
    adapterModels.value = adapterModels.value.filter((adapter) => adapter.adapter_id !== adapterId)
    if (draft.value.default_adapter === adapterId) {
      const nextDraft = clone(draft.value)
      nextDraft.default_adapter = ''
      nextDraft.default_model = ''
      draft.value = nextDraft
    }
    toasts.push('success', 'Provider adapter deleted')
    await loadProviders()
    await loadDraft(providers.value.some((provider) => provider.provider_id === providerId) ? providerId : undefined)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function saveAdapter(adapter: AdapterModels) {
  if (!draft.value) return
  try {
    const adapterForSave: AdapterModels = {
      ...adapter,
      models: adapter.models.filter((model) => selectedModelKeys.value.has(modelKey(adapter.adapter_id, model.id))),
    }
    await apiJson('/api/v1/provider-studio/save-adapter', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_models: adapterForSave }),
    })
    toasts.push('success', `Saved ${adapter.adapter_id} adapter matches`)
    await loadProviders()
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
}

async function createProvider() {
  await loadDraft()
  if (draft.value) {
    const next = clone(draft.value)
    next.provider_id = ''
    next.source_provider_id = null
    draft.value = normalizeDraftShape(next)
  }
}

onMounted(async () => {
  try {
    await Promise.all([loadProviders(), loadAwsProfiles()])
    await loadDraft(providers.value[0]?.provider_id)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
})
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 bg-background/30 p-4 lg:p-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-medium">Provider Studio</h2>
        <p class="mt-1 text-xs text-muted-foreground">Edit the same provider draft, authentication fields, adapters, and model policies exposed by the TUI.</p>
      </div>
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" :disabled="loading" @click="loadDraft(selectedProviderId || undefined)">
          <RiRefreshLine class="mr-2 h-4 w-4" :class="loading ? 'animate-spin' : ''" />
          Refresh draft
        </Button>
        <Button variant="outline" size="sm" @click="createProvider">
          <RiAddLine class="mr-2 h-4 w-4" />
          New provider
        </Button>
      </div>
    </div>

    <div v-if="error" class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">{{ error }}</div>

    <div class="grid gap-4 lg:grid-cols-[minmax(13rem,0.7fr)_minmax(0,2fr)]">
      <div class="grid content-start gap-2">
        <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Providers</div>
        <button
          v-for="provider in providers"
          :key="provider.provider_id"
          type="button"
          class="rounded-md border px-3 py-2 text-left text-xs transition-colors"
          :class="selectedProviderId === provider.provider_id ? 'border-primary bg-primary/10 text-foreground' : 'border-border/60 hover:bg-muted/40'"
          @click="loadDraft(provider.provider_id)"
        >
          <span class="block truncate font-mono font-semibold">{{ providerLabel(provider) }}</span>
          <span class="mt-1 block text-[10px] text-muted-foreground">{{ provider.adapters?.length || 0 }} adapters</span>
        </button>
        <div v-if="providers.length === 0" class="rounded-md border border-dashed border-border/60 px-3 py-4 text-center text-xs text-muted-foreground">No providers configured.</div>
      </div>

      <div v-if="draft" class="grid min-w-0 gap-5">
        <section class="grid gap-3">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">Draft</div>
            <div class="flex gap-2">
              <Button variant="ghost" size="sm" class="text-destructive" :disabled="saving || !draft.source_provider_id" @click="deleteProvider">
                <RiDeleteBinLine class="mr-1.5 h-4 w-4" /> Delete provider
              </Button>
              <Button size="sm" :disabled="saving || !draft.provider_id" @click="saveDraft">
                <RiSave3Line class="mr-1.5 h-4 w-4" /> {{ saving ? 'Saving…' : 'Save provider' }}
              </Button>
            </div>
          </div>
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Provider ID</span>
              <Input :value="draft.provider_id" class="font-mono" placeholder="openai" @input="setFieldValue('provider_id', ($event.target as HTMLInputElement).value)" />
            </label>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Auth mode</span>
              <OptionPicker :model-value="authMode" :options="authModeOptions" :include-empty="false" title="Auth mode" @update:model-value="setAuthMode" />
            </label>
            <label v-if="authMode !== 'none' && authMode !== 'unset'" class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Auth subtype</span>
              <OptionPicker :model-value="authSubtype" :options="authSubtypeOptions" :include-empty="false" title="Auth subtype" @update:model-value="setAuthSubtype" />
            </label>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Default adapter</span>
              <OptionPicker :model-value="draft.default_adapter" :options="adapterCandidates.map((value) => ({ value, label: value }))" :include-empty="true" empty-label="No default adapter" title="Default adapter" monospace @update:model-value="setDefaultAdapter" />
            </label>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Default model</span>
              <OptionPicker :model-value="selectedDefaultModelKey" :options="defaultModelOptions" :include-empty="true" empty-label="No default model" title="Default model" monospace :disabled="defaultModelOptions.length === 0" @update:model-value="setDefaultModel" />
            </label>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Request timeout (seconds)</span>
              <Input :value="draft.request_timeout_secs" type="number" @input="setFieldValue('request_timeout_secs', Number(($event.target as HTMLInputElement).value))" />
            </label>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Connect timeout (seconds)</span>
              <Input :value="draft.connect_timeout_secs" type="number" @input="setFieldValue('connect_timeout_secs', Number(($event.target as HTMLInputElement).value))" />
            </label>
          </div>
        </section>

        <section v-if="authMode !== 'none' && authMode !== 'unset'" class="grid gap-3 border-t border-border/60 pt-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">Authentication details</div>
              <div class="mt-1 text-xs text-muted-foreground">Secret values are sent only to the provider-studio endpoint and are masked in the form.</div>
            </div>
            <div v-if="interactiveAuthAvailable" class="flex gap-2">
              <Button variant="outline" size="sm" @click="startAuth('start')">Start auth</Button>
              <Button variant="outline" size="sm" @click="startAuth('continue')">Continue auth</Button>
            </div>
          </div>
          <div v-if="authMessage" class="rounded-md border border-border/60 bg-muted/20 px-3 py-2 text-xs">{{ authMessage }}</div>
          <div class="grid gap-3 sm:grid-cols-2">
            <label v-for="field in visibleAuthFields" :key="field.path" class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">{{ field.label }}</span>
              <div v-if="field.readOnly" class="flex h-9 items-center rounded-md border border-input bg-muted/20 px-3 font-mono text-sm text-muted-foreground">
                {{ field.value || '—' }}
              </div>
              <OptionPicker
                v-else-if="field.type === 'select'"
                :model-value="fieldValue(field.path)"
                :options="field.options || []"
                :title="field.label"
                :include-empty="field.includeEmpty ?? false"
                :empty-label="field.emptyLabel"
                :allow-custom="field.allowCustom ?? false"
                :monospace="field.path.includes('source_kind')"
                @update:model-value="setFieldValue(field.path, $event)"
              />
              <Input
                v-else
                :value="fieldValue(field.path)"
                :type="field.secret ? 'password' : field.type || 'text'"
                :placeholder="field.placeholder"
                :class="field.secret || field.path.includes('url') || field.path.includes('token') ? 'font-mono' : ''"
                @input="setFieldValue(field.path, ($event.target as HTMLInputElement).value)"
              />
            </label>
          </div>
        </section>

        <section class="grid gap-3 border-t border-border/60 pt-4">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div class="text-sm font-medium">Adapter model lists</div>
              <div class="mt-1 text-xs text-muted-foreground">Select adapters, load their live models, then select the model routes to persist.</div>
            </div>
            <Button variant="outline" size="sm" :disabled="listingModels" @click="listDraftModels">
              <RiRefreshLine class="mr-1.5 h-4 w-4" :class="listingModels ? 'animate-spin' : ''" /> List live models
            </Button>
          </div>
          <div class="grid gap-2 sm:grid-cols-2">
            <label v-for="adapterId in adapterCandidates" :key="adapterId" class="flex items-center gap-2 rounded-md border border-border/60 px-3 py-2 text-xs">
              <input type="checkbox" :checked="selectedAdapterIds.has(adapterId)" @change="toggleAdapter(adapterId)" />
              <span class="font-mono">{{ adapterId }}</span>
            </label>
          </div>
          <div v-if="adapterModels.length === 0 && !listingModels" class="text-xs text-muted-foreground">No adapter models loaded. Select an adapter and list live models.</div>
          <div v-for="adapter in adapterModels" :key="adapter.adapter_id" class="rounded-md border border-border/60">
            <div class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 px-3 py-2">
              <div class="flex items-center gap-2 text-xs">
                <span class="font-mono font-semibold">{{ adapter.adapter_id }}</span>
                <span v-if="adapter.resolved_base_url" class="break-all text-muted-foreground">{{ adapter.resolved_base_url }}</span>
                <span v-if="adapterFailure(adapter)" class="text-destructive">{{ adapterFailure(adapter) }}</span>
              </div>
              <div class="flex gap-1">
                <Button variant="ghost" size="sm" @click="saveAdapter(adapter)">Save adapter</Button>
                <IconButton variant="ghost" size="sm" tooltip="Delete adapter" aria-label="Delete adapter" @click="deleteAdapter(adapter.adapter_id)"><RiDeleteBinLine class="h-4 w-4 text-destructive" /></IconButton>
              </div>
            </div>
            <div class="grid gap-1 p-2">
              <label v-for="model in adapter.models" :key="model.id" class="flex min-w-0 items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-muted/40">
                <input type="checkbox" :checked="selectedModelKeys.has(modelKey(adapter.adapter_id, model.id))" @change="toggleModel(adapter.adapter_id, model.id)" />
                <button type="button" class="min-w-0 flex-1 truncate text-left" @click="openModelEditor(adapter.adapter_id, model)">
                  <span>{{ model.display_name || model.id }}</span>
                  <code v-if="model.display_name" class="ml-2 font-mono text-[10px] text-muted-foreground">{{ model.id }}</code>
                </button>
                <IconButton variant="ghost" size="sm" tooltip="Delete model" aria-label="Delete model" @click.stop="deleteModel(adapter.adapter_id, model.id)"><RiDeleteBinLine class="h-3.5 w-3.5 text-destructive" /></IconButton>
              </label>
              <div v-if="adapter.models.length === 0" class="px-2 py-3 text-xs text-muted-foreground">No models listed.</div>
            </div>
          </div>
          <div class="flex flex-wrap items-end gap-2">
            <label class="grid min-w-[14rem] gap-1.5">
              <span class="text-xs text-muted-foreground">Add model id</span>
              <Input v-model="newModelId" class="font-mono" placeholder="model-name" @keydown.enter="addManualModel" />
            </label>
            <Button variant="outline" size="sm" :disabled="!newModelId.trim() || selectedAdapterIds.size === 0" @click="addManualModel"><RiAddLine class="mr-1.5 h-4 w-4" /> Add model</Button>
          </div>
        </section>

        <section v-if="editingModel" class="grid gap-3 border-t border-border/60 pt-4">
          <div class="flex flex-wrap items-start justify-between gap-2">
            <div>
              <div class="text-sm font-medium">Model · {{ editingModel.adapterId }}/{{ editingModel.modelId }}</div>
              <div class="mt-1 text-xs text-muted-foreground">The TUI exposes these 15 model configuration fields through its persisted JSON editor.</div>
            </div>
            <Button variant="ghost" size="sm" @click="editingModel = null">Close</Button>
          </div>
          <div v-if="modelLoading" class="text-sm text-muted-foreground">Loading model configuration…</div>
          <div v-else-if="modelError" class="text-sm text-destructive">{{ modelError }}</div>
          <template v-else>
            <div class="grid gap-2 sm:grid-cols-2">
              <div v-for="field in modelFields" :key="field.key" class="rounded border border-border/50 px-3 py-2">
                <div class="text-[10px] uppercase tracking-wide text-muted-foreground">{{ field.label }}</div>
                <code class="mt-1 block break-all text-xs">{{ modelFieldValue(field.key) || '—' }}</code>
              </div>
            </div>
            <label class="grid gap-1.5">
              <span class="text-xs text-muted-foreground">Persisted model JSON</span>
              <textarea v-model="modelJson" rows="16" spellcheck="false" class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring" />
            </label>
            <div v-if="modelError" class="text-xs text-destructive">{{ modelError }}</div>
            <Button class="justify-self-start" :disabled="modelSaving" @click="saveModel"><RiSave3Line class="mr-1.5 h-4 w-4" /> {{ modelSaving ? 'Saving…' : 'Save model config' }}</Button>
          </template>
        </section>
      </div>
      <div v-else class="rounded-md border border-dashed border-border/60 px-4 py-8 text-sm text-muted-foreground">Loading provider draft…</div>
    </div>
  </section>
</template>
