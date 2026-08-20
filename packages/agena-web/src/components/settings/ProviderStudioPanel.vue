<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiAddLine, RiCloudLine, RiDeleteBinLine, RiEditLine, RiPlugLine, RiRefreshLine } from '@remixicon/vue'

import SettingsDisclosureRow from '@/components/settings/SettingsDisclosureRow.vue'
import SettingsSaveBar from '@/components/settings/SettingsSaveBar.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import { apiJson } from '@/lib/api'
import { useToastsStore } from '@/stores/toasts'
import type { JsonValue } from '@/types/json'
import { settingsText as st } from '@/i18n/settingsText'
import {
  normalizeProviderAdapterModels,
  type ProviderAdapterModelsRecord,
} from '@/components/settings/providerStudioModelLists'

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

type AdapterModels = ProviderAdapterModelsRecord<ProviderModel>

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

type ModelFieldKind = 'boolean' | 'text' | 'number' | 'select' | 'csv' | 'readonly' | 'textarea'

type ModelField = {
  key: string
  label: string
  kind: ModelFieldKind
  options?: Array<{ value: string; label: string; description?: string }>
  placeholder?: string
  help?: string
}

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
const manualModelAdapterId = ref('')
const authMessage = ref('')

const editingModel = ref<{ adapterId: string; modelId: string } | null>(null)
const modelValue = ref<LooseRecord | null>(null)
const modelJson = ref('')
const modelLoading = ref(false)
const modelError = ref('')
const modelConfigValues = ref<Record<string, LooseRecord>>({})
const configuredAdapterIds = ref<Set<string>>(new Set())
const configuredModelKeys = ref<Set<string>>(new Set())
const mutationBusy = ref(false)
const authPolling = ref(false)
const authRequestInFlight = ref(false)
const expandedProviderKey = ref('')
const expandedAdapterIds = ref<Set<string>>(new Set())
const savedEditorState = ref('')
const pendingDeletedAdapterIds = ref<Set<string>>(new Set())
const pendingDeletedModelKeys = ref<Set<string>>(new Set())
const NEW_PROVIDER_ROW_KEY = '__new_provider__'
let authPollTimer: ReturnType<typeof setTimeout> | null = null
let draftRequestGeneration = 0
let modelListingGeneration = 0
let authRequestGeneration = 0
let modelEditorGeneration = 0

const authModeOptions = [
  {
    value: 'unset',
    label: st('Unset'),
    description: st('Choose an authentication mode before configuring credentials.'),
  },
  { value: 'none', label: st('None'), description: st('Provider does not require credentials.') },
  { value: 'api', label: 'API', description: st('API key or compatible API authentication.') },
  { value: 'credential', label: st('Credential'), description: st('Interactive or persisted OAuth credentials.') },
]

const apiSubtypeOptions = [
  { value: 'custom', label: st('Custom API'), description: st('OpenAI-compatible, Anthropic, or Gemini endpoints.') },
  { value: 'cline_api', label: st('Cline API'), description: st('Cline-managed model service.') },
  { value: 'gitlab_api', label: st('GitLab API'), description: st('GitLab model API using an API key.') },
  { value: 'bedrock_sigv4', label: st('Bedrock SigV4'), description: st('AWS Bedrock signed requests.') },
]

const credentialSubtypeOptions = [
  { value: 'openai_chatgpt', label: st('OpenAI ChatGPT'), description: st('ChatGPT/Codex OAuth credentials.') },
  { value: 'github_copilot', label: st('GitHub Copilot'), description: st('GitHub Copilot device authentication.') },
  { value: 'gitlab', label: st('GitLab'), description: st('GitLab OAuth credentials.') },
  { value: 'google_adc', label: st('Google ADC'), description: st('Google Application Default Credentials.') },
  { value: 'sap_ai_core', label: st('SAP AI Core'), description: st('SAP AI Core service key.') },
]

const loginKindOptions = [
  { value: 'Browser', label: st('Browser'), description: st('Open a browser-based authorization flow.') },
  { value: 'Device', label: st('Device'), description: st('Use a device-code flow in the terminal or browser.') },
]

const secretSourceOptions = [
  { value: 'Inline', label: st('Inline'), description: st('Store the secret value in this provider draft.') },
  {
    value: 'Env',
    label: st('Environment'),
    description: st('Read the secret from the configured environment source.'),
  },
]

const apiKeyEnvironmentOptions = [
  'OPENAI_API_KEY',
  'ANTHROPIC_API_KEY',
  'GEMINI_API_KEY',
  'GITLAB_TOKEN',
  'GOOGLE_VERTEX_ACCESS_TOKEN',
  'SHARED_GATEWAY_API_KEY',
  'OPENCODE_API_KEY',
].map((value) => ({ value, label: value, description: st('Environment variable used by the provider runtime.') }))

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

const gitlabInstanceOptions = [{ value: 'https://gitlab.com', label: 'https://gitlab.com', description: 'GitLab.com' }]

const redirectUriOptions = [
  {
    value: 'http://localhost:1455/auth/callback',
    label: 'http://localhost:1455/auth/callback',
    description: st('Local OAuth callback used by the TUI and web runtime.'),
  },
]

const defaultRedirectUri = 'http://localhost:1455/auth/callback'
const legacyRedirectUris = new Set(['http://127.0.0.1:1455/callback', 'http://127.0.0.1:1455/auth/callback'])

const awsProfiles = ref<string[]>(['default'])

const modelToolModeOptions = [
  {
    value: 'provider_protocol',
    label: st('Provider protocol'),
    description: st('Expose Agena tools through the provider protocol.'),
  },
  { value: 'disabled', label: st('Disabled'), description: st('Do not advertise Agena tools for this model.') },
]

const modelLifecycleOptions = [
  { value: '', label: st('Default / unset') },
  { value: 'active', label: st('Active') },
  { value: 'preview', label: st('Preview') },
  { value: 'beta', label: st('Beta') },
  { value: 'alpha', label: st('Alpha') },
  { value: 'experimental', label: st('Experimental') },
  { value: 'deprecated', label: st('Deprecated') },
]

const modelFields: ModelField[] = [
  {
    key: 'model_id',
    label: st('Model ID'),
    kind: 'readonly',
    help: 'The provider model identifier is fixed by the route.',
  },
  { key: 'enabled', label: st('Enabled'), kind: 'boolean' },
  { key: 'native_compaction', label: st('Native compaction'), kind: 'boolean' },
  { key: 'agena_tools.mode', label: st('Agena tool mode'), kind: 'select', options: modelToolModeOptions },
  {
    key: 'display_name',
    label: st('Display name'),
    kind: 'text',
    placeholder: st('Friendly name shown in model pickers'),
  },
  { key: 'lifecycle', label: st('Lifecycle'), kind: 'select', options: modelLifecycleOptions },
  {
    key: 'context_window_tokens',
    label: st('Context window tokens'),
    kind: 'number',
    placeholder: st('Optional unsigned integer'),
  },
  {
    key: 'max_input_tokens',
    label: st('Max input tokens'),
    kind: 'number',
    placeholder: st('Optional unsigned integer'),
  },
  {
    key: 'max_output_tokens',
    label: st('Max output tokens'),
    kind: 'number',
    placeholder: st('Optional unsigned integer'),
  },
  { key: 'features', label: st('Features'), kind: 'csv', placeholder: st('tool_calling, reasoning, streaming') },
  { key: 'input', label: st('Input modalities'), kind: 'csv', placeholder: st('text, image, file') },
  { key: 'output_modalities', label: st('Output modalities'), kind: 'csv', placeholder: 'text' },
  {
    key: 'thinking_modes',
    label: st('Thinking modes'),
    kind: 'readonly',
    help: st('Advertised by the provider/catalog and preserved when saving.'),
  },
  {
    key: 'speed_modes',
    label: st('Speed modes'),
    kind: 'readonly',
    help: st('Advertised by the provider/catalog and preserved when saving.'),
  },
  { key: 'description', label: st('Description'), kind: 'textarea', placeholder: st('Optional model description') },
]

const modelFeatureTokens = new Set(['tool_calling', 'streaming', 'reasoning', 'structured_output', 'temperature'])
const modelInputTokens = new Set(['text', 'image', 'document', 'audio', 'video', 'file'])

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function record(value: unknown): LooseRecord {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as LooseRecord) : {}
}

function canonicalizeModelConfig(value: JsonValue): LooseRecord {
  const next = record(clone(value))
  const legacyCapabilities = record(next.capabilities)

  // ResolvedProviderModelConfig flattens ModelCapabilityPatch. Accept the
  // old nested shape when a user pastes it into the advanced editor, but
  // always write the canonical top-level fields back out.
  for (const key of ['features', 'input']) {
    if (next[key] === undefined && legacyCapabilities[key] !== undefined) {
      next[key] = clone(legacyCapabilities[key])
    }
    delete legacyCapabilities[key]
  }
  if (Object.keys(legacyCapabilities).length === 0) delete next.capabilities
  else next.capabilities = legacyCapabilities
  return next
}

function authMessageText(value: JsonValue): string {
  const labels: Record<string, string> = {
    OpenaiBrowserStarted: 'OpenAI browser authorization started.',
    OpenaiDeviceStarted: 'OpenAI device authorization started.',
    CopilotDeviceStarted: 'GitHub Copilot device authorization started.',
    GitlabBrowserStarted: 'GitLab browser authorization started.',
    OpenaiPending: 'OpenAI authorization is still pending.',
    OpenaiCredentialCaptured: 'OpenAI credentials captured.',
    CopilotPending: 'GitHub Copilot authorization is still pending.',
    CopilotCredentialCaptured: 'GitHub Copilot credentials captured.',
    GitlabCredentialCaptured: 'GitLab credentials captured.',
  }
  if (typeof value === 'string') return labels[value] || value
  const object = record(value)
  const [variant] = Object.keys(object)
  if (!variant) return ''
  const suffix = record(object[variant]).user_code
  return suffix
    ? st('{variant} Code: {suffix}', { variant: labels[variant] || variant, suffix: suffix })
    : labels[variant] || variant
}

function authKindMode(value: JsonValue): 'none' | 'api' | 'credential' | 'unset' {
  if (value === 'None') return 'none'
  if (typeof value === 'string' && ['Api', 'ApiPending', 'ClineApi', 'Gitlab', 'BedrockSigv4'].includes(value))
    return 'api'
  if (
    value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    Object.prototype.hasOwnProperty.call(value, 'Credential')
  )
    return 'credential'
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
const authSubtypeOptions = computed(() =>
  authMode.value === 'credential' ? credentialSubtypeOptions : apiSubtypeOptions,
)

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

function adapterIdsForAuthKind(value: JsonValue): Set<string> {
  const mode = authKindMode(value)
  const subtype = authKindSubtype(value)
  // ApiPending and Credential(null) are deliberately non-operational draft
  // states. TUI does not allow a route to be selected until the subtype is
  // chosen, so the Web surface must not offer listing or saving through them.
  if (value === 'ApiPending' || mode === 'unset' || (mode === 'credential' && !subtype)) {
    return new Set()
  }
  return new Set(adapterRuleMap[subtype] || adapterRuleMap[mode] || [])
}

const adapterCandidates = computed(() => {
  const known = (
    draft.value?.provider_id
      ? providers.value.find(
          (item) =>
            item.provider_id === draft.value?.source_provider_id || item.provider_id === draft.value?.provider_id,
        )?.adapters || []
      : []
  ).map((item) => item.adapter_id)
  const pendingApiSubtype = draft.value?.auth_kind === 'ApiPending'
  const ruleAdapters = pendingApiSubtype
    ? []
    : adapterRuleMap[authSubtype.value] || adapterRuleMap[authMode.value] || []
  return [...new Set([...ruleAdapters, ...(pendingApiSubtype ? [] : known)])].filter(Boolean)
})
const supportedAdapterIds = computed(() => adapterIdsForAuthKind(draft.value?.auth_kind))

const awsProfileOptions = computed(() => {
  const current = fieldValue('auth.profile')
  return [...new Set(['default', ...awsProfiles.value, current].filter(Boolean))].map((value) => ({
    value,
    label: value,
    description: value === 'default' ? st('Use the AWS SDK default profile.') : st('AWS shared credentials profile.'),
  }))
})

const allModels = computed(() =>
  adapterModels.value.flatMap((adapter) => adapter.models.map((model) => ({ adapter, model }))),
)
const selectedDefaultModelKey = computed(() => {
  const adapter = String(draft.value?.default_adapter || '')
  const model = String(draft.value?.default_model || '')
  return adapter && model ? `${adapter}\u001f${model}` : ''
})
const defaultModelOptions = computed(() =>
  allModels.value
    .filter(({ adapter }) => supportedAdapterIds.value.has(adapter.adapter_id))
    .map(({ adapter, model }) => ({
      value: `${adapter.adapter_id}\u001f${model.id}`,
      label: model.display_name || model.id,
      description: st('{adapter_id} / {id}', { adapter_id: adapter.adapter_id, id: model.id }),
    })),
)

const manualModelAdapterOptions = computed(() =>
  [...selectedAdapterIds.value]
    .map((adapterId) => ({
      value: adapterId,
      label: adapterId,
      description: adapterModels.value.find((adapter) => adapter.adapter_id === adapterId)?.resolved_base_url || '',
    }))
    .sort((left, right) => left.label.localeCompare(right.label)),
)

function providerLabel(provider: ProviderSummary): string {
  const defaultLabel = [provider.defaults?.adapter, provider.defaults?.model].filter(Boolean).join(' / ')
  return defaultLabel
    ? st('{provider_id} · {defaultLabel}', { provider_id: provider.provider_id, defaultLabel: defaultLabel })
    : provider.provider_id
}

function modelKey(adapterId: string, modelId: string): string {
  return `${adapterId}\u001f${modelId}`
}

type ProviderRow = {
  key: string
  providerId: string
  summary: ProviderSummary | null
  isNew: boolean
}

const providerRows = computed<ProviderRow[]>(() => {
  const rows = providers.value.map((provider) => ({
    key: provider.provider_id,
    providerId: provider.provider_id,
    summary: provider,
    isNew: false,
  }))
  if (draft.value && !String(draft.value.source_provider_id || '').trim()) {
    rows.unshift({ key: NEW_PROVIDER_ROW_KEY, providerId: '', summary: null, isNew: true })
  }
  return rows
})

const providerDirty = computed(() => {
  if (!draft.value) return false
  if (!String(draft.value.source_provider_id || '').trim()) return true
  return Boolean(
    pendingDeletedAdapterIds.value.size ||
    pendingDeletedModelKeys.value.size ||
    (savedEditorState.value && providerEditorStateFingerprint() !== savedEditorState.value),
  )
})

const adapterRows = computed<AdapterModels[]>(() => {
  const byId = new Map(adapterModels.value.map((adapter) => [adapter.adapter_id, adapter]))
  return [...new Set([...adapterCandidates.value, ...byId.keys()])].map((adapterId) => {
    const existing = byId.get(adapterId)
    return {
      adapter_id: adapterId,
      enabled: selectedAdapterIds.value.has(adapterId),
      resolved_base_url: existing?.resolved_base_url,
      failure: existing?.failure,
      models: existing?.models || [],
    }
  })
})

function providerRowLabel(row: ProviderRow): string {
  if (row.isNew) return String(draft.value?.provider_id || '').trim() || st('New provider')
  return row.summary ? providerLabel(row.summary) : row.providerId
}

function providerRowSummary(row: ProviderRow): string {
  if (row.isNew) return st('Not saved yet')
  const adapters = row.summary?.adapters || []
  const enabled = adapters.filter((adapter) => adapter.enabled).length
  return st('{enabled} enabled · {total} adapters', { enabled: enabled, total: adapters.length })
}

function adapterRowSummary(adapter: AdapterModels): string {
  if (adapterFailure(adapter)) return adapterFailure(adapter)
  const route = adapter.resolved_base_url ? ` · ${adapter.resolved_base_url}` : ''
  return st('{count} models{route}', { count: adapter.models.length, route: route })
}

function toggleAdapterRow(adapterId: string) {
  const next = new Set(expandedAdapterIds.value)
  if (next.has(adapterId)) next.delete(adapterId)
  else next.add(adapterId)
  expandedAdapterIds.value = next
}

function canPersistExistingProvider(): boolean {
  const source = String(draft.value?.source_provider_id || '').trim()
  const providerId = String(draft.value?.provider_id || '').trim()
  return Boolean(source && providerId && source === providerId)
}

function providerDraftIdentity(value: ProviderConfigDraft | null): string {
  if (!value) return ''
  return `${String(value.source_provider_id || '').trim()}\u001f${String(value.provider_id || '').trim()}`
}

function providerEditorStateFingerprint(
  draftValue: ProviderConfigDraft | null = draft.value,
  adapterList: AdapterModels[] = adapterModels.value,
  adapterSelection: Set<string> = selectedAdapterIds.value,
  modelSelection: Set<string> = selectedModelKeys.value,
  modelConfigs: Record<string, LooseRecord> = modelConfigValues.value,
): string {
  return JSON.stringify({
    draft: draftValue,
    adapter_list: adapterList,
    selected_adapter_ids: [...adapterSelection].sort(),
    selected_model_keys: [...modelSelection].sort(),
    model_config_values: modelConfigs,
  })
}

function resetAuthUiState() {
  ++authRequestGeneration
  clearAuthPollTimer()
  authPolling.value = false
  authRequestInFlight.value = false
  authMessage.value = ''
}

function invalidateModelListing() {
  ++modelListingGeneration
  listingModels.value = false
}

function selectedAdaptersAreSupported(): boolean {
  return [...selectedAdapterIds.value].every((adapterId) => supportedAdapterIds.value.has(adapterId))
}

function syncManualModelAdapter() {
  if (manualModelAdapterId.value && selectedAdapterIds.value.has(manualModelAdapterId.value)) return
  const preferred = String(draft.value?.default_adapter || '').trim()
  manualModelAdapterId.value = selectedAdapterIds.value.has(preferred)
    ? preferred
    : [...selectedAdapterIds.value].sort((left, right) => left.localeCompare(right))[0] || ''
}

function clearModelStudioState() {
  ++modelListingGeneration
  ++modelEditorGeneration
  listingModels.value = false
  adapterModels.value = []
  selectedModelKeys.value = new Set()
  modelConfigValues.value = {}
  manualModelAdapterId.value = ''
  editingModel.value = null
  modelLoading.value = false
  modelValue.value = null
  modelJson.value = ''
  modelError.value = ''
  configuredAdapterIds.value = new Set()
  configuredModelKeys.value = new Set()
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
    clearAuthFields(
      'base_url',
      'region',
      'profile',
      'access_key_id',
      'secret_access_key',
      'session_token',
      'service_key_env',
    )
  } else if (mode === 'api' && subtype === 'bedrock_sigv4') {
    next.auth.secret_source_kind = 'Unset'
    next.auth.secret_source_value = ''
    clearAuthFields('service_key_env')
  } else if (mode === 'api') {
    clearAuthFields('region', 'profile', 'access_key_id', 'secret_access_key', 'session_token', 'service_key_env')
    if (subtype === 'cline_api') clearAuthFields('base_url')
    if (subtype === 'gitlab_api' && !String(next.auth.instance_url || '').trim())
      next.auth.instance_url = 'https://gitlab.com'
  } else if (mode === 'credential') {
    next.auth.secret_source_kind = 'Unset'
    next.auth.secret_source_value = ''
    clearAuthFields('region', 'profile', 'access_key_id', 'secret_access_key', 'session_token')
    if (!['google_adc', 'sap_ai_core'].includes(subtype)) clearAuthFields('base_url')
    if (subtype === 'gitlab' && !String(next.auth.instance_url || '').trim())
      next.auth.instance_url = 'https://gitlab.com'
    if (subtype === 'sap_ai_core') {
      if (!String(next.auth.service_key_env || '').trim()) next.auth.service_key_env = 'AICORE_SERVICE_KEY'
    } else {
      clearAuthFields('service_key_env')
    }
  }

  const supportedAdapters = next.auth_kind === 'ApiPending' ? [] : adapterRuleMap[subtype] || adapterRuleMap[mode] || []
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
  const previousValue = fieldValue(path)
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
  if (authRequestInFlight.value || authPolling.value) resetAuthUiState()
  if (path === 'credential_drafts.openai_chatgpt.login_kind' && previousValue !== String(value)) {
    const openai = record(next.credential_drafts.openai_chatgpt)
    openai.callback_url = ''
    openai.browser = null
    openai.device = null
    next.credential_drafts.openai_chatgpt = openai
    resetAuthUiState()
  }
  const normalized = normalizeDraftShape(next)
  const changed = JSON.stringify(normalized) !== JSON.stringify(draft.value)
  draft.value = normalized
  if (changed) invalidateModelListing()
}

function authDetailFields(): DraftField[] {
  const mode = authMode.value
  const subtype = authSubtype.value
  const secretSourceKind = fieldValue('auth.secret_source_kind').trim().toLowerCase()
  const apiSecretField: DraftField =
    secretSourceKind === 'env'
      ? {
          path: 'auth.secret_source_value',
          label: st('API key environment variable'),
          type: 'select',
          options: apiKeyEnvironmentOptions,
          includeEmpty: true,
          emptyLabel: st('No environment variable'),
          allowCustom: true,
        }
      : { path: 'auth.secret_source_value', label: st('API key value'), secret: true }
  if (mode === 'api') {
    if (subtype === 'custom')
      return [
        { path: 'auth.base_url', label: st('Base URL'), placeholder: 'https://api.example.com/v1' },
        {
          path: 'auth.secret_source_kind',
          label: st('API key source'),
          type: 'select',
          options: secretSourceOptions,
          includeEmpty: true,
          emptyLabel: st('No API key source'),
        },
        apiSecretField,
      ]
    if (subtype === 'cline_api')
      return [
        {
          path: 'auth.secret_source_kind',
          label: st('API key source'),
          type: 'select',
          options: secretSourceOptions,
          includeEmpty: true,
          emptyLabel: st('No API key source'),
        },
        apiSecretField,
      ]
    if (subtype === 'gitlab_api')
      return [
        {
          path: 'auth.instance_url',
          label: st('Instance URL'),
          type: 'select',
          options: gitlabInstanceOptions,
          includeEmpty: true,
          emptyLabel: st('No GitLab instance'),
          allowCustom: true,
        },
        {
          path: 'auth.secret_source_kind',
          label: st('API key source'),
          type: 'select',
          options: secretSourceOptions,
          includeEmpty: true,
          emptyLabel: st('No API key source'),
        },
        apiSecretField,
      ]
    if (subtype === 'bedrock_sigv4')
      return [
        { path: 'auth.base_url', label: st('Base URL') },
        {
          path: 'auth.region',
          label: st('Region'),
          type: 'select',
          options: awsRegionOptions,
          includeEmpty: true,
          emptyLabel: st('No AWS region'),
          allowCustom: true,
        },
        {
          path: 'auth.profile',
          label: st('AWS profile'),
          type: 'select',
          options: awsProfileOptions.value,
          includeEmpty: true,
          emptyLabel: st('No AWS profile'),
          allowCustom: true,
        },
        { path: 'auth.access_key_id', label: st('Access key ID'), secret: true },
        { path: 'auth.secret_access_key', label: st('Secret access key'), secret: true },
        { path: 'auth.session_token', label: st('Session token'), secret: true },
      ]
  }
  if (mode !== 'credential') return []
  const base =
    subtype === 'openai_chatgpt'
      ? 'credential_drafts.openai_chatgpt'
      : subtype === 'github_copilot'
        ? 'credential_drafts.github_copilot'
        : 'credential_drafts.gitlab'
  if (subtype === 'openai_chatgpt') {
    const fields: DraftField[] = [
      { path: `${base}.login_kind`, label: st('Auth login method'), type: 'select', options: loginKindOptions },
    ]
    const loginKind = fieldValue(`${base}.login_kind`).trim().toLowerCase()
    if (loginKind === 'browser') {
      fields.push(
        {
          path: `${base}.redirect_uri`,
          label: st('Redirect URI'),
          type: 'select',
          options: redirectUriOptions,
          includeEmpty: true,
          emptyLabel: st('No redirect URI'),
          allowCustom: true,
        },
        { path: `${base}.callback_url`, label: st('Callback URL') },
      )
    }
    fields.push(
      { path: `${base}.tokens.refresh_token`, label: st('Refresh token'), secret: true },
      { path: `${base}.tokens.access_token`, label: st('Access token'), secret: true },
      { path: `${base}.tokens.expires_at_ms`, label: st('Expires at (ms)') },
      { path: `${base}.account_id`, label: st('Account ID') },
    )
    return fields
  }
  if (subtype === 'github_copilot')
    return [
      { path: '__auth_login_method', value: 'Device', label: st('Auth login method'), readOnly: true },
      { path: `${base}.enterprise_domain`, label: st('Enterprise domain') },
      { path: `${base}.tokens.refresh_token`, label: st('Refresh token'), secret: true },
      { path: `${base}.tokens.access_token`, label: st('Access token'), secret: true },
      { path: `${base}.tokens.expires_at_ms`, label: st('Expires at (ms)') },
    ]
  if (subtype === 'gitlab')
    return [
      { path: '__auth_login_method', value: 'Browser', label: st('Auth login method'), readOnly: true },
      {
        path: 'auth.instance_url',
        label: st('Instance URL'),
        type: 'select',
        options: gitlabInstanceOptions,
        includeEmpty: true,
        emptyLabel: st('No GitLab instance'),
        allowCustom: true,
      },
      {
        path: `${base}.redirect_uri`,
        label: st('Redirect URI'),
        type: 'select',
        options: redirectUriOptions,
        includeEmpty: true,
        emptyLabel: st('No redirect URI'),
        allowCustom: true,
      },
      { path: `${base}.callback_url`, label: st('Callback URL') },
      { path: `${base}.tokens.refresh_token`, label: st('Refresh token'), secret: true },
      { path: `${base}.tokens.access_token`, label: st('Access token'), secret: true },
      { path: `${base}.tokens.expires_at_ms`, label: st('Expires at (ms)') },
    ]
  if (subtype === 'google_adc') return [{ path: 'auth.base_url', label: st('Base URL') }]
  if (subtype === 'sap_ai_core')
    return [
      { path: 'auth.base_url', label: st('Base URL') },
      {
        path: 'auth.service_key_env',
        label: st('Service key env'),
        type: 'select',
        options: [{ value: 'AICORE_SERVICE_KEY', label: 'AICORE_SERVICE_KEY' }],
        includeEmpty: true,
        emptyLabel: st('No service key env'),
        allowCustom: true,
      },
    ]
  return []
}

const visibleAuthFields = computed(() => authDetailFields())
const interactiveAuthAvailable = computed(() =>
  ['openai_chatgpt', 'github_copilot', 'gitlab'].includes(authSubtype.value),
)

const pendingDeviceAuth = computed(() => {
  const credentials = record(draft.value?.credential_drafts)
  const openai = record(credentials.openai_chatgpt)
  const openaiLoginKind = String(openai.login_kind || '')
    .trim()
    .toLowerCase()
  const candidates =
    authSubtype.value === 'openai_chatgpt'
      ? [openaiLoginKind === 'browser' ? null : openai.device]
      : authSubtype.value === 'github_copilot'
        ? [record(credentials.github_copilot).device]
        : []
  return candidates.find((value) => String(value?.device_code || '').trim()) || null
})

const pendingBrowserAuth = computed(() => {
  const credentials = record(draft.value?.credential_drafts)
  const openai = record(credentials.openai_chatgpt)
  const openaiLoginKind = String(openai.login_kind || '')
    .trim()
    .toLowerCase()
  const candidates =
    authSubtype.value === 'openai_chatgpt'
      ? [openaiLoginKind === 'browser' ? openai.browser : null]
      : authSubtype.value === 'gitlab'
        ? [record(credentials.gitlab).browser]
        : []
  return candidates.find((value) => String(value?.state || '').trim()) || null
})

const canListDraftModels = computed(() => {
  if (!draft.value || selectedAdapterIds.value.size === 0) return false
  if (supportedAdapterIds.value.size === 0) return false
  return selectedAdaptersAreSupported()
})

const canListSavedModels = computed(() => {
  if (!draft.value?.source_provider_id || selectedAdapterIds.value.size === 0) return false
  return supportedAdapterIds.value.size > 0 && selectedAdaptersAreSupported()
})

const modelListingKind = computed(() => {
  if (canListDraftModels.value) return 'live'
  if (canListSavedModels.value) return 'saved'
  return 'unavailable'
})

const modelListingDescription = computed(() => {
  if (modelListingKind.value === 'live')
    return st('Fetch current models from the provider using the draft credentials.')
  if (modelListingKind.value === 'saved')
    return st('List models using the saved provider configuration; this may contact the provider.')
  return st('Choose an authentication mode and adapter before listing models.')
})

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
  const requestGeneration = ++draftRequestGeneration
  loading.value = true
  error.value = ''
  clearModelStudioState()
  resetAuthUiState()
  try {
    const query = providerId ? `?provider_id=${encodeURIComponent(providerId)}` : ''
    const nextDraft = normalizeDraftShape(await apiJson<ProviderConfigDraft>(`/api/v1/provider-studio/draft${query}`))
    if (requestGeneration !== draftRequestGeneration) return
    draft.value = nextDraft
    selectedProviderId.value = providerId || ''
    const summary = providers.value.find((item) => item.provider_id === providerId)
    let configuredAdapters: AdapterModels[] = []
    let configuredModelsError = ''
    if (providerId) {
      try {
        const configuredResponse = await apiJson<unknown>(
          `/api/v1/providers/${encodeURIComponent(providerId)}/configured-models`,
        )
        configuredAdapters = normalizeProviderAdapterModels<ProviderModel>(configuredResponse)
      } catch (reason) {
        configuredModelsError = reason instanceof Error ? reason.message : String(reason)
      }
    }
    if (requestGeneration !== draftRequestGeneration) return
    const enabled = new Set(
      configuredAdapters.length
        ? configuredAdapters.filter((item) => item.enabled).map((item) => item.adapter_id)
        : (summary?.adapters || []).filter((item) => item.enabled).map((item) => item.adapter_id),
    )
    const supported = adapterIdsForAuthKind(nextDraft.auth_kind)
    // Match the TUI restore semantics: only adapters that are actually
    // enabled in the saved provider are selected. An empty configured set is
    // meaningful; silently selecting every candidate would make a later
    // Save provider create routes the user never chose.
    selectedAdapterIds.value = new Set([...enabled].filter((adapterId) => supported.has(adapterId)))
    adapterModels.value = configuredAdapters
    configuredAdapterIds.value = new Set(configuredAdapters.map((adapter) => adapter.adapter_id))
    configuredModelKeys.value = new Set(
      configuredAdapters.flatMap((adapter) => adapter.models.map((model) => modelKey(adapter.adapter_id, model.id))),
    )
    selectedModelKeys.value = new Set(
      configuredAdapters
        .filter((adapter) => adapter.enabled)
        .flatMap((adapter) => adapter.models.map((model) => modelKey(adapter.adapter_id, model.id))),
    )
    syncManualModelAdapter()
    expandedAdapterIds.value = new Set([...selectedAdapterIds.value].slice(0, 1))
    pendingDeletedAdapterIds.value = new Set()
    pendingDeletedModelKeys.value = new Set()
    savedEditorState.value = providerEditorStateFingerprint()
    expandedProviderKey.value = providerId || NEW_PROVIDER_ROW_KEY
    if (configuredModelsError) error.value = configuredModelsError
  } catch (reason) {
    if (requestGeneration !== draftRequestGeneration) return
    error.value = reason instanceof Error ? reason.message : String(reason)
    draft.value = null
    savedEditorState.value = ''
  } finally {
    if (requestGeneration === draftRequestGeneration) loading.value = false
  }
}

async function listDraftModels() {
  if (listingModels.value || mutationBusy.value) return
  if (!draft.value || selectedAdapterIds.value.size === 0) {
    error.value = st('Select at least one adapter before listing models.')
    return
  }
  if (modelListingKind.value === 'unavailable') {
    error.value = modelListingDescription.value
    return
  }
  if (!selectedAdaptersAreSupported()) {
    error.value = st('One or more selected adapters are not supported by the current authentication subtype.')
    return
  }
  const requestGeneration = ++modelListingGeneration
  listingModels.value = true
  mutationBusy.value = true
  error.value = ''
  try {
    const adapterIds = [...selectedAdapterIds.value]
    const draftSnapshot = clone(draft.value)
    const response =
      modelListingKind.value === 'live'
        ? await apiJson<AdapterModelsResponse>('/api/v1/provider-studio/draft/models', {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify({ draft: draftSnapshot, adapter_ids: adapterIds }),
          })
        : await apiJson<AdapterModelsResponse>(
            `/api/v1/providers/${encodeURIComponent(draftSnapshot.source_provider_id || '')}/models`,
            {
              method: 'POST',
              headers: { 'content-type': 'application/json' },
              body: JSON.stringify({ adapter_ids: adapterIds }),
            },
          )
    if (requestGeneration !== modelListingGeneration) return
    const refreshed = normalizeProviderAdapterModels<ProviderModel>(response)
      .filter((adapter) => !pendingDeletedAdapterIds.value.has(adapter.adapter_id))
      .map((adapter) => ({
        ...adapter,
        models: adapter.models.filter(
          (model) => !pendingDeletedModelKeys.value.has(modelKey(adapter.adapter_id, model.id)),
        ),
      }))
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
    if (requestGeneration !== modelListingGeneration) return
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    if (requestGeneration === modelListingGeneration) listingModels.value = false
    mutationBusy.value = false
  }
}

function toggleAdapter(adapterId: string) {
  if (mutationBusy.value || listingModels.value || saving.value || !supportedAdapterIds.value.has(adapterId)) return
  const next = new Set(selectedAdapterIds.value)
  if (next.has(adapterId)) next.delete(adapterId)
  else {
    next.add(adapterId)
    const deleted = new Set(pendingDeletedAdapterIds.value)
    deleted.delete(adapterId)
    pendingDeletedAdapterIds.value = deleted
  }
  selectedAdapterIds.value = next
  syncManualModelAdapter()
  // Adapter selection is a local draft edit. Model discovery is an explicit
  // action, matching the TUI and avoiding a provider request for every click.
  ++modelListingGeneration
  listingModels.value = false
  error.value = ''
}

function toggleModel(adapterId: string, modelId: string) {
  if (mutationBusy.value || listingModels.value || saving.value) return
  const key = modelKey(adapterId, modelId)
  const next = new Set(selectedModelKeys.value)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  selectedModelKeys.value = next
}

function setAuthMode(value: string) {
  if (!draft.value) return
  const previouslySelected = selectedAdapterIds.value
  const next = clone(draft.value)
  if (value === 'unset') next.auth_kind = 'Unset'
  else if (value === 'none') next.auth_kind = 'None'
  else if (value === 'api') next.auth_kind = 'ApiPending'
  else if (value === 'credential') next.auth_kind = { Credential: null }
  draft.value = normalizeDraftShape(next)
  resetAuthUiState()
  clearModelStudioState()
  const supported = adapterIdsForAuthKind(draft.value.auth_kind)
  selectedAdapterIds.value = new Set([...previouslySelected].filter((adapterId) => supported.has(adapterId)))
  syncManualModelAdapter()
}

function setAuthSubtype(value: string) {
  if (!draft.value) return
  const previouslySelected = selectedAdapterIds.value
  const next = clone(draft.value)
  if (authMode.value === 'credential') next.auth_kind = { Credential: value }
  else if (value === 'custom') next.auth_kind = 'Api'
  else if (value === 'cline_api') next.auth_kind = 'ClineApi'
  else if (value === 'gitlab_api') next.auth_kind = 'Gitlab'
  else if (value === 'bedrock_sigv4') next.auth_kind = 'BedrockSigv4'
  draft.value = normalizeDraftShape(next)
  resetAuthUiState()
  clearModelStudioState()
  const supported = adapterIdsForAuthKind(draft.value.auth_kind)
  selectedAdapterIds.value = new Set([...previouslySelected].filter((adapterId) => supported.has(adapterId)))
  syncManualModelAdapter()
}

function setDefaultAdapter(value: string) {
  if (!draft.value) return
  const next = clone(draft.value)
  const adapter = String(value || '').trim()
  if (next.default_adapter !== adapter) next.default_model = ''
  next.default_adapter = adapter
  const normalized = normalizeDraftShape(next)
  if (JSON.stringify(normalized) !== JSON.stringify(draft.value)) invalidateModelListing()
  draft.value = normalized
}

function setDefaultModel(value: string) {
  if (!draft.value) return
  const [adapter, model] = value.split('\u001f')
  const next = clone(draft.value)
  next.default_adapter = adapter || ''
  next.default_model = model || ''
  const normalized = normalizeDraftShape(next)
  if (JSON.stringify(normalized) !== JSON.stringify(draft.value)) invalidateModelListing()
  draft.value = normalized
}

async function saveDraft() {
  if (!draft.value || saving.value || mutationBusy.value || listingModels.value) return
  const draftSnapshot = clone(draft.value)
  const submittedDraftJson = JSON.stringify(draftSnapshot)
  const submittedDraftGeneration = draftRequestGeneration
  const adapterModelListsSnapshot = clone(adapterModels.value)
  const selectedAdapterIdsSnapshot = [...selectedAdapterIds.value]
  const selectedModelKeysSnapshot = [...selectedModelKeys.value]
  const modelConfigValuesSnapshot = clone(modelConfigValues.value)
  const deletedAdapterIdsSnapshot = [...pendingDeletedAdapterIds.value]
  const deletedModelKeysSnapshot = [...pendingDeletedModelKeys.value]
  const submittedEditorState = providerEditorStateFingerprint(
    draftSnapshot,
    adapterModelListsSnapshot,
    new Set(selectedAdapterIdsSnapshot),
    new Set(selectedModelKeysSnapshot),
    modelConfigValuesSnapshot,
  )
  saving.value = true
  mutationBusy.value = true
  error.value = ''
  try {
    const response = await apiJson<JsonValue>('/api/v1/provider-studio/save', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        draft: draftSnapshot,
        adapter_model_lists: adapterModelListsSnapshot,
        selected_adapter_ids: selectedAdapterIdsSnapshot,
        selected_model_keys: selectedModelKeysSnapshot,
        model_config_values: modelConfigValuesSnapshot,
      }),
    })
    const responseRecord = record(response)
    const savedResult = record(responseRecord.ProviderDraftSaved ?? responseRecord.provider_draft_saved)
    const savedId = String(savedResult.provider_id || draftSnapshot.provider_id || '').trim()
    const persistedDraftSnapshot = {
      ...draftSnapshot,
      provider_id: savedId,
      source_provider_id: savedId,
    }
    for (const key of deletedModelKeysSnapshot) {
      const [adapterId, modelId] = key.split('\u001f')
      if (!adapterId || !modelId || deletedAdapterIdsSnapshot.includes(adapterId)) continue
      await apiJson('/api/v1/provider-studio/delete-model', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ draft: persistedDraftSnapshot, adapter_id: adapterId, model_id: modelId }),
      })
    }
    for (const adapterId of deletedAdapterIdsSnapshot) {
      await apiJson('/api/v1/provider-studio/delete-adapter', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ draft: persistedDraftSnapshot, adapter_id: adapterId }),
      })
    }
    toasts.push('success', st('Provider configuration saved'))
    await loadProviders()
    // Keep edits made while the request was in flight in the editor. A late
    // save response must not replace a newer local draft with the old server
    // snapshot.
    if (
      submittedDraftGeneration !== draftRequestGeneration ||
      providerDraftIdentity(draft.value) !== providerDraftIdentity(draftSnapshot) ||
      JSON.stringify(draft.value) !== submittedDraftJson ||
      providerEditorStateFingerprint() !== submittedEditorState
    ) {
      return
    }
    await loadDraft(savedId || selectedProviderId.value || undefined)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
    toasts.push('error', error.value)
  } finally {
    saving.value = false
    mutationBusy.value = false
  }
}

async function deleteProvider() {
  if (!draft.value || mutationBusy.value) return
  const draftSnapshot = clone(draft.value)
  const providerId = String(draftSnapshot.source_provider_id || draftSnapshot.provider_id || '').trim()
  if (!providerId || !window.confirm(st('Delete provider {providerId}?', { providerId: providerId }))) return
  const submittedDraftGeneration = draftRequestGeneration
  const submittedDraftIdentity = providerDraftIdentity(draftSnapshot)
  const submittedEditorState = providerEditorStateFingerprint(
    draftSnapshot,
    clone(adapterModels.value),
    new Set(selectedAdapterIds.value),
    new Set(selectedModelKeys.value),
    clone(modelConfigValues.value),
  )
  mutationBusy.value = true
  try {
    await apiJson('/api/v1/provider-studio/delete-provider', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ provider_id: providerId }),
    })
    if (submittedDraftGeneration !== draftRequestGeneration) return
    toasts.push('success', st('Provider deleted'))
    await loadProviders()
    // A provider deletion can race with a local edit or a provider switch.
    // Refresh the editor only when the exact state submitted for deletion is
    // still on screen; otherwise the user's newer draft must remain intact.
    if (
      submittedDraftGeneration === draftRequestGeneration &&
      providerDraftIdentity(draft.value) === submittedDraftIdentity &&
      providerEditorStateFingerprint() === submittedEditorState
    ) {
      const nextProviderId = providers.value[0]?.provider_id
      if (nextProviderId) await loadDraft(nextProviderId)
      else {
        draft.value = null
        expandedProviderKey.value = ''
        savedEditorState.value = ''
      }
    }
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  } finally {
    mutationBusy.value = false
  }
}

function clearAuthPollTimer() {
  if (authPollTimer !== null) {
    clearTimeout(authPollTimer)
    authPollTimer = null
  }
}

function scheduleDeviceAuthPoll() {
  clearAuthPollTimer()
  if (!pendingDeviceAuth.value || authPolling.value) return
  const intervalSeconds = Math.max(1, Number(pendingDeviceAuth.value.interval_seconds) || 2)
  authPollTimer = setTimeout(() => {
    authPollTimer = null
    void startAuth('continue', true)
  }, intervalSeconds * 1000)
}

async function startAuth(action: 'start' | 'continue', silent = false) {
  if (!draft.value || authRequestInFlight.value) return
  const requestGeneration = ++authRequestGeneration
  const draftSnapshot = clone(draft.value)
  authRequestInFlight.value = true
  if (silent) authPolling.value = true
  if (action === 'start') {
    clearAuthPollTimer()
    authMessage.value = ''
  }
  try {
    const response = await apiJson<{
      draft?: ProviderConfigDraft
      message?: JsonValue
      clipboard_text?: string | null
    }>(`/api/v1/provider-studio/auth/${action}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draftSnapshot }),
    })
    if (requestGeneration !== authRequestGeneration) return
    if (response?.draft) draft.value = normalizeDraftShape(response.draft)
    authMessage.value = response?.message ? authMessageText(response.message) : ''
    if (response?.clipboard_text && navigator.clipboard) {
      try {
        await navigator.clipboard.writeText(response.clipboard_text)
      } catch {
        // Clipboard permission is optional; a successful auth action must not
        // be reported as failed just because the browser refused the copy.
      }
    }
    scheduleDeviceAuthPoll()
  } catch (reason) {
    if (requestGeneration !== authRequestGeneration) return
    const message = reason instanceof Error ? reason.message : String(reason)
    if (!silent) error.value = message
    else authMessage.value = message
    if (pendingDeviceAuth.value) scheduleDeviceAuthPoll()
  } finally {
    if (requestGeneration === authRequestGeneration) {
      authRequestInFlight.value = false
      if (silent) authPolling.value = false
      if (pendingDeviceAuth.value) scheduleDeviceAuthPoll()
    }
  }
}

function addManualModel() {
  if (mutationBusy.value || listingModels.value || saving.value) return
  const adapterId = String(manualModelAdapterId.value || draft.value?.default_adapter || '').trim()
  const modelId = newModelId.value.trim()
  if (!draft.value || !adapterId || !modelId || !selectedAdapterIds.value.has(adapterId)) {
    error.value = st('Select an adapter before adding a model.')
    return
  }
  const adapter = adapterModels.value.find((item) => item.adapter_id === adapterId)
  if (adapter?.models.some((item) => item.id === modelId)) {
    error.value = st('Model {adapterId}/{modelId} is already in the draft.', { adapterId: adapterId, modelId: modelId })
    return
  }
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
  const key = modelKey(adapterId, modelId)
  selectedModelKeys.value = new Set([...selectedModelKeys.value, key])
  const deleted = new Set(pendingDeletedModelKeys.value)
  deleted.delete(key)
  pendingDeletedModelKeys.value = deleted
  newModelId.value = ''
}

async function openModelEditor(adapterId: string, model: ProviderModel) {
  if (!draft.value || mutationBusy.value || listingModels.value) return
  const requestGeneration = ++modelEditorGeneration
  editingModel.value = { adapterId, modelId: model.id }
  modelLoading.value = true
  modelError.value = ''
  modelValue.value = null
  modelJson.value = ''
  const staged = modelConfigValues.value[modelKey(adapterId, model.id)]
  if (staged) {
    modelValue.value = canonicalizeModelConfig(staged)
    modelJson.value = JSON.stringify(modelValue.value, null, 2)
    modelLoading.value = false
    return
  }
  try {
    const response = await apiJson<{ value?: JsonValue }>('/api/v1/provider-studio/draft/model', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ draft: draft.value, adapter_id: adapterId, model_id: model.id, provider_model: model }),
    })
    if (requestGeneration !== modelEditorGeneration) return
    modelValue.value = canonicalizeModelConfig(response?.value ?? {})
    modelJson.value = JSON.stringify(modelValue.value, null, 2)
  } catch (reason) {
    if (requestGeneration !== modelEditorGeneration) return
    modelError.value = reason instanceof Error ? reason.message : String(reason)
    modelValue.value = null
    modelJson.value = ''
  } finally {
    if (requestGeneration === modelEditorGeneration) modelLoading.value = false
  }
}

function closeModelEditor() {
  ++modelEditorGeneration
  editingModel.value = null
  modelLoading.value = false
  modelValue.value = null
  modelJson.value = ''
  modelError.value = ''
}

function modelPathValue(key: string): any {
  if (!modelValue.value) return undefined
  const parts = key.split('.')
  let current: any = modelValue.value
  for (const part of parts) current = current?.[part]
  return current
}

function modelCapabilityValue(key: 'features' | 'input'): { supported: string[]; unsupported: string[] } {
  const direct = modelValue.value?.[key]
  const legacy = modelValue.value?.capabilities?.[key]
  const value = direct ?? legacy
  if (Array.isArray(value)) return { supported: value.map(String), unsupported: [] }
  const object = record(value)
  return {
    supported: Array.isArray(object.supported) ? object.supported.map(String) : [],
    unsupported: Array.isArray(object.unsupported) ? object.unsupported.map(String) : [],
  }
}

function modelFieldValue(key: string): string {
  if (key === 'model_id') return editingModel.value?.modelId || ''
  if (key === 'agena_tools.mode' && modelPathValue(key) === undefined) return 'disabled'
  if (key === 'features' || key === 'input') return modelCapabilityValue(key).supported.join(', ')
  if (key === 'thinking_modes' || key === 'speed_modes') {
    const value = modelPathValue(key)
    if (!value || typeof value !== 'object' || Array.isArray(value)) return ''
    const object = record(value)
    const names = Object.keys(object).filter((name) => name !== 'default')
    if (names.length > 0) return names.join(', ')
    return typeof object.default === 'string' ? object.default : ''
  }
  const current = modelPathValue(key)
  if (current === undefined || current === null) return ''
  if (typeof current === 'boolean') return current ? 'true' : 'false'
  if (Array.isArray(current)) return current.map(String).join(', ')
  return typeof current === 'string' ? current : JSON.stringify(current)
}

function modelFieldBooleanValue(key: string): boolean {
  return modelPathValue(key) !== false
}

function csvTokens(value: string): string[] {
  return [
    ...new Set(
      value
        .split(',')
        .map((token) => token.trim())
        .filter(Boolean),
    ),
  ]
}

function deleteModelPath(target: LooseRecord, key: string) {
  const parts = key.split('.')
  let cursor: any = target
  for (const part of parts.slice(0, -1)) {
    if (!cursor || typeof cursor !== 'object') return
    cursor = cursor[part]
  }
  if (cursor && typeof cursor === 'object') delete cursor[parts[parts.length - 1]]
}

function setModelPath(target: LooseRecord, key: string, value: unknown) {
  const parts = key.split('.')
  let cursor: LooseRecord = target
  for (const part of parts.slice(0, -1)) {
    if (!cursor[part] || typeof cursor[part] !== 'object' || Array.isArray(cursor[part])) cursor[part] = {}
    cursor = cursor[part] as LooseRecord
  }
  cursor[parts[parts.length - 1]] = value
}

function setModelFieldValue(key: string, value: string | number | boolean) {
  if (!modelValue.value || key === 'model_id' || key === 'thinking_modes' || key === 'speed_modes') return
  const field = modelFields.find((item) => item.key === key)
  if (!field) return
  let base: LooseRecord
  try {
    base = syncModelValueFromJson()
  } catch (reason) {
    modelError.value = reason instanceof Error ? reason.message : String(reason)
    return
  }
  const next = clone(base)
  if (field.kind === 'boolean') {
    setModelPath(next, key, Boolean(value))
  } else if (field.kind === 'number') {
    const text = String(value).trim()
    if (!text) deleteModelPath(next, key)
    else {
      const parsed = Number(text)
      if (!Number.isSafeInteger(parsed) || parsed < 0) {
        modelError.value = st('{label} must be an unsigned integer.', { label: field.label })
        return
      }
      setModelPath(next, key, parsed)
    }
  } else if (field.kind === 'csv') {
    const tokens = csvTokens(String(value))
    if (key === 'features' || key === 'input') {
      const allowed = key === 'features' ? modelFeatureTokens : modelInputTokens
      const invalid = tokens.find((token) => !allowed.has(token))
      if (invalid) {
        modelError.value = st('Unsupported {field} token `{invalid}`.', {
          field: field.label.toLowerCase(),
          invalid: invalid,
        })
        return
      }
      const previous = modelCapabilityValue(key)
      const unsupported = previous.unsupported.filter((token) => !tokens.includes(token))
      if (tokens.length || unsupported.length) {
        setModelPath(next, key, unsupported.length ? { supported: tokens, unsupported } : tokens)
      } else {
        deleteModelPath(next, key)
      }
      // The configured model schema keeps these fields at the top level. A
      // legacy capabilities wrapper is accepted on read, but never emitted.
      if (next.capabilities && typeof next.capabilities === 'object') delete next.capabilities[key]
    } else if (tokens.length) {
      setModelPath(next, key, tokens)
    } else {
      deleteModelPath(next, key)
    }
  } else if (field.kind === 'select' || field.kind === 'text' || field.kind === 'textarea') {
    const text = String(value)
    if (!text.trim() && field.kind !== 'select') deleteModelPath(next, key)
    else if (!text.trim() && key === 'lifecycle') deleteModelPath(next, key)
    else setModelPath(next, key, text)
  }
  modelValue.value = next
  modelJson.value = JSON.stringify(next, null, 2)
  stageCurrentModelValue(next)
  modelError.value = ''
}

function syncModelValueFromJson(): LooseRecord {
  const parsed = JSON.parse(modelJson.value) as JsonValue
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
    throw new Error(st('Model config must be a JSON object.'))
  const next = canonicalizeModelConfig(parsed)
  modelValue.value = next
  return next
}

function applyModelJson() {
  try {
    const next = syncModelValueFromJson()
    modelJson.value = JSON.stringify(next, null, 2)
    stageCurrentModelValue(next)
    modelError.value = ''
  } catch (reason) {
    modelError.value = reason instanceof Error ? reason.message : String(reason)
  }
}

function stageCurrentModelValue(value: LooseRecord) {
  const editing = editingModel.value
  if (!editing) return
  const key = modelKey(editing.adapterId, editing.modelId)
  modelConfigValues.value = { ...modelConfigValues.value, [key]: clone(value) }
  updateModelRowFromValue(editing.adapterId, editing.modelId, value)
}

function updateModelRowFromValue(adapterId: string, modelId: string, value: LooseRecord) {
  adapterModels.value = adapterModels.value.map((adapter) => {
    if (adapter.adapter_id !== adapterId) return adapter
    return {
      ...adapter,
      models: adapter.models.map((model) => {
        if (model.id !== modelId) return model
        const next = { ...model }
        if (typeof value.display_name === 'string') next.display_name = value.display_name
        else if (value.display_name === null) next.display_name = null
        if (typeof value.native_compaction === 'boolean') next.native_compaction = value.native_compaction
        return next
      }),
    }
  })
}

async function deleteModel(adapterId: string, modelId: string) {
  if (
    !draft.value ||
    mutationBusy.value ||
    !window.confirm(st('Delete model {adapterId}/{modelId}?', { adapterId: adapterId, modelId: modelId }))
  )
    return
  const key = modelKey(adapterId, modelId)
  if (configuredModelKeys.value.has(key)) {
    pendingDeletedModelKeys.value = new Set([...pendingDeletedModelKeys.value, key])
  }
  if (editingModel.value?.adapterId === adapterId && editingModel.value?.modelId === modelId) closeModelEditor()
  const nextValues = { ...modelConfigValues.value }
  delete nextValues[key]
  modelConfigValues.value = nextValues
  adapterModels.value = adapterModels.value.map((adapter) =>
    adapter.adapter_id === adapterId
      ? { ...adapter, models: adapter.models.filter((model) => model.id !== modelId) }
      : adapter,
  )
  const next = new Set(selectedModelKeys.value)
  next.delete(key)
  selectedModelKeys.value = next
  if (draft.value.default_adapter === adapterId && draft.value.default_model === modelId) {
    const nextDraft = clone(draft.value)
    nextDraft.default_model = ''
    draft.value = nextDraft
  }
  toasts.push('success', st('Model removal staged; save the Provider to apply it'))
}
async function deleteAdapter(adapterId: string) {
  if (
    !draft.value ||
    mutationBusy.value ||
    !window.confirm(st('Delete adapter {adapterId}?', { adapterId: adapterId }))
  )
    return
  if (configuredAdapterIds.value.has(adapterId)) {
    pendingDeletedAdapterIds.value = new Set([...pendingDeletedAdapterIds.value, adapterId])
  }
  if (editingModel.value?.adapterId === adapterId) closeModelEditor()
  selectedAdapterIds.value = new Set([...selectedAdapterIds.value].filter((id) => id !== adapterId))
  if (manualModelAdapterId.value === adapterId) syncManualModelAdapter()
  adapterModels.value = adapterModels.value.filter((adapter) => adapter.adapter_id !== adapterId)
  const nextValues = { ...modelConfigValues.value }
  for (const key of Object.keys(nextValues)) if (key.startsWith(`${adapterId}\u001f`)) delete nextValues[key]
  modelConfigValues.value = nextValues
  selectedModelKeys.value = new Set([...selectedModelKeys.value].filter((key) => !key.startsWith(`${adapterId}\u001f`)))
  if (draft.value.default_adapter === adapterId) {
    const nextDraft = clone(draft.value)
    nextDraft.default_adapter = ''
    nextDraft.default_model = ''
    draft.value = nextDraft
  }
  const nextExpanded = new Set(expandedAdapterIds.value)
  nextExpanded.delete(adapterId)
  expandedAdapterIds.value = nextExpanded
  toasts.push('success', st('Adapter removal staged; save the Provider to apply it'))
}
async function openProviderRow(row: ProviderRow) {
  if (mutationBusy.value || loading.value) return
  if (expandedProviderKey.value === row.key) {
    expandedProviderKey.value = ''
    return
  }
  const currentKey = expandedProviderKey.value
  if (providerDirty.value && currentKey && currentKey !== row.key) {
    const discard = window.confirm(st('Discard unsaved provider changes and open another provider?'))
    if (!discard) return
  }
  expandedProviderKey.value = row.key
  if (row.isNew) return
  if (selectedProviderId.value !== row.providerId || !draft.value) await loadDraft(row.providerId)
}

async function discardProviderChanges() {
  if (!draft.value || mutationBusy.value) return
  const sourceProviderId = String(draft.value.source_provider_id || '').trim()
  if (sourceProviderId) {
    await loadDraft(sourceProviderId)
    return
  }
  const firstProviderId = providers.value[0]?.provider_id
  if (firstProviderId) await loadDraft(firstProviderId)
  else {
    draft.value = null
    expandedProviderKey.value = ''
    savedEditorState.value = ''
  }
}

async function refreshActiveProvider() {
  if (providerDirty.value && !window.confirm(st('Discard unsaved provider changes and refresh from the server?')))
    return
  const sourceProviderId = String(draft.value?.source_provider_id || selectedProviderId.value || '').trim()
  await loadDraft(sourceProviderId || undefined)
}

async function deleteProviderRow(row: ProviderRow) {
  if (row.isNew) {
    await discardProviderChanges()
    return
  }
  if (selectedProviderId.value !== row.providerId || !draft.value) {
    if (!window.confirm(st('Delete provider {providerId}?', { providerId: row.providerId }))) return
    mutationBusy.value = true
    try {
      await apiJson('/api/v1/provider-studio/delete-provider', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ provider_id: row.providerId }),
      })
      toasts.push('success', st('Provider deleted'))
      await loadProviders()
      if (expandedProviderKey.value === row.key) expandedProviderKey.value = ''
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : String(reason)
    } finally {
      mutationBusy.value = false
    }
    return
  }
  await deleteProvider()
}

async function createProvider() {
  if (mutationBusy.value || loading.value) return
  if (providerDirty.value && !window.confirm(st('Discard unsaved provider changes and create a new provider?'))) return
  await loadDraft()
  if (draft.value) {
    const next = clone(draft.value)
    next.provider_id = ''
    next.source_provider_id = null
    draft.value = normalizeDraftShape(next)
    selectedProviderId.value = ''
    expandedProviderKey.value = NEW_PROVIDER_ROW_KEY
    expandedAdapterIds.value = new Set()
    savedEditorState.value = ''
  }
}

onMounted(async () => {
  try {
    await Promise.all([loadProviders(), loadAwsProfiles()])
    const firstProviderId = providers.value[0]?.provider_id
    if (firstProviderId) await loadDraft(firstProviderId)
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : String(reason)
  }
})

onBeforeUnmount(() => {
  resetAuthUiState()
  ++draftRequestGeneration
  ++modelListingGeneration
  ++modelEditorGeneration
})
</script>

<template>
  <section class="grid gap-4 rounded-lg border border-border/60 bg-background/30 p-4 lg:p-5">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h2 class="text-base font-medium">{{ $st('Provider Studio') }}</h2>
        <p class="mt-1 text-xs text-muted-foreground">
          {{
            $st('Edit the same provider draft, authentication fields, adapters, and model policies exposed by the TUI.')
          }}
        </p>
      </div>
      <div class="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" :disabled="loading || mutationBusy" @click="createProvider">
          <RiAddLine class="mr-2 h-4 w-4" />
          {{ $st('New provider') }}
        </Button>
      </div>
    </div>

    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div class="grid gap-2">
      <SettingsDisclosureRow
        v-for="row in providerRows"
        :key="row.key"
        :open="expandedProviderKey === row.key"
        :label="providerRowLabel(row)"
        :summary="providerRowSummary(row)"
        :tone="expandedProviderKey === row.key && providerDirty ? 'dirty' : 'default'"
        @toggle="openProviderRow(row)"
      >
        <template #leading>
          <RiCloudLine class="h-4 w-4 shrink-0 text-primary" />
        </template>
        <template #badges>
          <span
            v-if="expandedProviderKey === row.key && providerDirty"
            class="rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-700 dark:text-amber-300"
            >{{ $st('Unsaved') }}</span
          >
          <span v-else-if="row.isNew" class="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">{{
            $st('New')
          }}</span>
        </template>
        <template #actions>
          <IconButton
            v-if="expandedProviderKey === row.key"
            variant="ghost"
            size="sm"
            :tooltip="$st('Refresh provider')"
            :aria-label="$st('Refresh provider')"
            :disabled="loading || mutationBusy"
            @click="refreshActiveProvider"
          >
            <RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
          </IconButton>
          <IconButton
            variant="ghost"
            size="sm"
            :tooltip="row.isNew ? $st('Discard new provider') : $st('Delete provider')"
            :aria-label="row.isNew ? $st('Discard new provider') : $st('Delete provider')"
            :disabled="mutationBusy"
            @click="deleteProviderRow(row)"
          >
            <RiDeleteBinLine class="h-4 w-4 text-destructive" />
          </IconButton>
        </template>

        <div v-if="draft && expandedProviderKey === row.key" class="grid min-w-0 gap-5">
          <section class="grid gap-3">
            <div>
              <div class="text-sm font-medium">{{ $st('Provider configuration') }}</div>
              <div class="mt-1 text-xs text-muted-foreground">
                {{
                  $st(
                    'Edit provider identity, authentication, defaults, adapters, and models, then save them together.',
                  )
                }}
              </div>
            </div>
            <div class="grid gap-3 sm:grid-cols-2">
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Provider ID') }}</span>
                <Input
                  :value="draft.provider_id"
                  class="font-mono"
                  placeholder="openai"
                  @input="setFieldValue('provider_id', ($event.target as HTMLInputElement).value)"
                />
              </label>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Auth mode') }}</span>
                <OptionPicker
                  :model-value="authMode"
                  :options="authModeOptions"
                  :include-empty="false"
                  :title="$st('Auth mode')"
                  @update:model-value="setAuthMode"
                />
              </label>
              <label v-if="authMode !== 'none' && authMode !== 'unset'" class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Auth subtype') }}</span>
                <OptionPicker
                  :model-value="authSubtype"
                  :options="authSubtypeOptions"
                  :include-empty="false"
                  :title="$st('Auth subtype')"
                  @update:model-value="setAuthSubtype"
                />
              </label>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Default adapter') }}</span>
                <OptionPicker
                  :model-value="draft.default_adapter"
                  :options="adapterCandidates.map((value) => ({ value, label: value }))"
                  :include-empty="true"
                  :empty-label="$st('No default adapter')"
                  :title="$st('Default adapter')"
                  monospace
                  @update:model-value="setDefaultAdapter"
                />
              </label>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Default model') }}</span>
                <OptionPicker
                  :model-value="selectedDefaultModelKey"
                  :options="defaultModelOptions"
                  :include-empty="true"
                  :empty-label="$st('No default model')"
                  :title="$st('Default model')"
                  monospace
                  :disabled="defaultModelOptions.length === 0"
                  @update:model-value="setDefaultModel"
                />
              </label>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Request timeout (seconds)') }}</span>
                <Input
                  :value="draft.request_timeout_secs"
                  type="number"
                  @input="setFieldValue('request_timeout_secs', Number(($event.target as HTMLInputElement).value))"
                />
              </label>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Connect timeout (seconds)') }}</span>
                <Input
                  :value="draft.connect_timeout_secs"
                  type="number"
                  @input="setFieldValue('connect_timeout_secs', Number(($event.target as HTMLInputElement).value))"
                />
              </label>
            </div>
          </section>

          <section v-if="authMode !== 'none' && authMode !== 'unset'" class="grid gap-3 border-t border-border/60 pt-4">
            <div class="flex flex-wrap items-center justify-between gap-2">
              <div>
                <div class="text-sm font-medium">{{ $st('Authentication details') }}</div>
                <div class="mt-1 text-xs text-muted-foreground">
                  {{ $st('Secret values are sent only to the provider-studio endpoint and are masked in the form.') }}
                </div>
              </div>
              <div v-if="interactiveAuthAvailable" class="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  :disabled="mutationBusy || authRequestInFlight || authPolling"
                  @click="startAuth('start')"
                  >{{ authRequestInFlight ? $st('Working…') : $st('Start auth') }}</Button
                >
                <Button
                  variant="outline"
                  size="sm"
                  :disabled="mutationBusy || authRequestInFlight || authPolling"
                  @click="startAuth('continue')"
                  >{{ authPolling ? $st('Waiting…') : $st('Continue auth') }}</Button
                >
              </div>
            </div>
            <div v-if="authMessage" class="rounded-md border border-border/60 bg-muted/20 px-3 py-2 text-xs">
              {{ authMessage }}
            </div>
            <div
              v-if="pendingDeviceAuth"
              class="grid gap-1 rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs"
            >
              <div class="font-medium">{{ $st('Device authorization is pending') }}</div>
              <a
                v-if="pendingDeviceAuth.verification_url"
                class="break-all text-primary underline underline-offset-2"
                :href="pendingDeviceAuth.verification_url"
                target="_blank"
                rel="noreferrer"
              >
                {{ $st('Open verification page') }}
              </a>
              <div
                v-if="pendingDeviceAuth.verification_url"
                class="break-all font-mono text-[11px] text-muted-foreground"
              >
                {{ pendingDeviceAuth.verification_url }}
              </div>
              <div v-if="pendingDeviceAuth.user_code" class="font-mono text-sm">
                {{ $st('Code:') }} {{ pendingDeviceAuth.user_code }}
              </div>
              <div class="text-muted-foreground">
                {{
                  $st(
                    'The Web client polls this device flow using the provider interval. You can also press Continue auth.',
                  )
                }}
              </div>
            </div>
            <div
              v-if="pendingBrowserAuth"
              class="grid gap-1 rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs text-muted-foreground"
            >
              <div class="font-medium text-foreground">{{ $st('Browser authorization is pending') }}</div>
              <a
                v-if="pendingBrowserAuth.display_url || pendingBrowserAuth.authorize_url"
                class="break-all text-primary underline underline-offset-2"
                :href="pendingBrowserAuth.display_url || pendingBrowserAuth.authorize_url"
                target="_blank"
                rel="noreferrer"
              >
                {{ $st('Open authorization page') }}
              </a>
              <div
                v-if="pendingBrowserAuth.display_url || pendingBrowserAuth.authorize_url"
                class="break-all font-mono text-[11px]"
              >
                {{ pendingBrowserAuth.display_url || pendingBrowserAuth.authorize_url }}
              </div>
              <div>
                {{
                  $st('Finish the browser flow, paste the callback URL into the field above, then press Continue auth.')
                }}
              </div>
            </div>
            <div class="grid gap-3 sm:grid-cols-2">
              <label v-for="field in visibleAuthFields" :key="field.path" class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ field.label }}</span>
                <div
                  v-if="field.readOnly"
                  class="flex h-9 items-center rounded-md border border-input bg-muted/20 px-3 font-mono text-sm text-muted-foreground"
                >
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
                <div class="text-sm font-medium">{{ $st('Adapter model lists') }}</div>
                <div class="mt-1 text-xs text-muted-foreground">
                  {{ modelListingDescription }} {{ $st('Selected routes are persisted when you save the Provider.') }}
                </div>
              </div>
              <Button
                variant="outline"
                size="sm"
                :disabled="mutationBusy || listingModels || modelListingKind === 'unavailable'"
                @click="listDraftModels"
              >
                <RiRefreshLine class="mr-1.5 h-4 w-4" :class="listingModels ? 'animate-spin' : ''" />
                {{ modelListingKind === 'saved' ? $st('Refresh saved models') : $st('List provider models') }}
              </Button>
            </div>
            <div v-if="adapterRows.length === 0 && !listingModels" class="text-xs text-muted-foreground">
              {{ $st('No adapter models loaded. Select an adapter and list live models.') }}
            </div>
            <SettingsDisclosureRow
              v-for="adapter in adapterRows"
              :key="adapter.adapter_id"
              :open="expandedAdapterIds.has(adapter.adapter_id)"
              :label="adapter.adapter_id"
              :summary="adapterRowSummary(adapter)"
              :tone="
                adapterFailure(adapter) ? 'error' : selectedAdapterIds.has(adapter.adapter_id) ? 'default' : 'disabled'
              "
              nested
              @toggle="toggleAdapterRow(adapter.adapter_id)"
            >
              <template #leading>
                <RiPlugLine class="h-4 w-4 shrink-0 text-muted-foreground" />
              </template>
              <template #badges>
                <span
                  v-if="!supportedAdapterIds.has(adapter.adapter_id)"
                  class="rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                  >{{ $st('unsupported') }}</span
                >
                <span
                  v-else-if="selectedAdapterIds.has(adapter.adapter_id)"
                  class="rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] text-emerald-700 dark:text-emerald-300"
                  >{{ $st('Enabled') }}</span
                >
              </template>
              <template #actions>
                <label
                  class="mr-1 inline-flex items-center gap-1.5 rounded-md px-1.5 py-1 text-[10px] text-muted-foreground hover:bg-muted/40"
                  :title="selectedAdapterIds.has(adapter.adapter_id) ? $st('Disable adapter') : $st('Enable adapter')"
                >
                  <input
                    type="checkbox"
                    :checked="selectedAdapterIds.has(adapter.adapter_id)"
                    :disabled="mutationBusy || listingModels || !supportedAdapterIds.has(adapter.adapter_id)"
                    @change="toggleAdapter(adapter.adapter_id)"
                  />
                  <span>{{ selectedAdapterIds.has(adapter.adapter_id) ? $st('On') : $st('Off') }}</span>
                </label>
                <IconButton
                  variant="ghost"
                  size="sm"
                  :tooltip="$st('Delete adapter')"
                  :aria-label="$st('Delete adapter')"
                  :disabled="mutationBusy"
                  @click="deleteAdapter(adapter.adapter_id)"
                >
                  <RiDeleteBinLine class="h-4 w-4 text-destructive" />
                </IconButton>
              </template>

              <div v-if="adapterFailure(adapter)" class="mb-2 text-xs text-destructive">
                {{ adapterFailure(adapter) }}
              </div>
              <div class="grid gap-1">
                <div
                  v-for="model in adapter.models"
                  :key="model.id"
                  class="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/40"
                >
                  <input
                    type="checkbox"
                    :checked="selectedModelKeys.has(modelKey(adapter.adapter_id, model.id))"
                    :disabled="mutationBusy || listingModels || !selectedAdapterIds.has(adapter.adapter_id)"
                    @change="toggleModel(adapter.adapter_id, model.id)"
                  />
                  <button
                    type="button"
                    class="min-w-0 flex-1 truncate text-left"
                    :disabled="mutationBusy || listingModels"
                    @click="openModelEditor(adapter.adapter_id, model)"
                  >
                    <span>{{ model.display_name || model.id }}</span>
                    <code v-if="model.display_name" class="ml-2 font-mono text-[10px] text-muted-foreground">{{
                      model.id
                    }}</code>
                  </button>
                  <IconButton
                    variant="ghost"
                    size="sm"
                    :tooltip="$st('Edit model')"
                    :aria-label="$st('Edit model')"
                    :disabled="mutationBusy || listingModels"
                    @click="openModelEditor(adapter.adapter_id, model)"
                  >
                    <RiEditLine class="h-3.5 w-3.5" />
                  </IconButton>
                  <IconButton
                    variant="ghost"
                    size="sm"
                    :tooltip="$st('Delete model')"
                    :aria-label="$st('Delete model')"
                    :disabled="mutationBusy"
                    @click="deleteModel(adapter.adapter_id, model.id)"
                  >
                    <RiDeleteBinLine class="h-3.5 w-3.5 text-destructive" />
                  </IconButton>
                </div>
                <div v-if="adapter.models.length === 0" class="px-2 py-3 text-xs text-muted-foreground">
                  {{ $st('No models listed.') }}
                </div>
              </div>
            </SettingsDisclosureRow>
            <div class="flex flex-wrap items-end gap-2">
              <label class="grid min-w-[12rem] gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Adapter') }}</span>
                <OptionPicker
                  :model-value="manualModelAdapterId"
                  :options="manualModelAdapterOptions"
                  :include-empty="false"
                  :placeholder="$st('Select adapter')"
                  :title="$st('Adapter for manual model')"
                  monospace
                  :disabled="mutationBusy || listingModels || manualModelAdapterOptions.length === 0"
                  @update:model-value="manualModelAdapterId = $event"
                />
              </label>
              <label class="grid min-w-[14rem] gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Add model id') }}</span>
                <Input
                  v-model="newModelId"
                  class="font-mono"
                  placeholder="model-name"
                  @keydown.enter="addManualModel"
                />
              </label>
              <Button
                variant="outline"
                size="sm"
                :disabled="mutationBusy || listingModels || !newModelId.trim() || !manualModelAdapterId"
                @click="addManualModel"
                ><RiAddLine class="mr-1.5 h-4 w-4" /> {{ $st('Add model') }}</Button
              >
            </div>
          </section>

          <section v-if="editingModel" class="grid gap-3 border-t border-border/60 pt-4">
            <div class="flex flex-wrap items-start justify-between gap-2">
              <div>
                <div class="text-sm font-medium">
                  {{ $st('Model ·') }} {{ editingModel.adapterId }}/{{ editingModel.modelId }}
                </div>
                <div class="mt-1 text-xs text-muted-foreground">
                  {{ $st('The TUI exposes these 15 model configuration fields through its persisted JSON editor.') }}
                </div>
              </div>
              <Button variant="ghost" size="sm" @click="closeModelEditor">{{ $st('Close') }}</Button>
            </div>
            <div v-if="modelLoading" class="text-sm text-muted-foreground">
              {{ $st('Loading model configuration…') }}
            </div>
            <div v-else-if="modelError" class="text-sm text-destructive">{{ modelError }}</div>
            <template v-else>
              <div class="grid gap-2 sm:grid-cols-2">
                <div v-for="field in modelFields" :key="field.key" class="rounded border border-border/50 px-3 py-2">
                  <div class="grid gap-1.5">
                    <div class="flex items-center justify-between gap-2">
                      <label
                        :for="`provider-model-${field.key.replaceAll('.', '-')}`"
                        class="text-[10px] uppercase tracking-wide text-muted-foreground"
                        >{{ field.label }}</label
                      >
                      <span v-if="field.kind === 'readonly'" class="text-[10px] text-muted-foreground">{{
                        $st('read-only')
                      }}</span>
                    </div>
                    <div
                      v-if="field.kind === 'readonly'"
                      :id="`provider-model-${field.key.replaceAll('.', '-')}`"
                      class="break-all font-mono text-xs"
                    >
                      {{ modelFieldValue(field.key) || '—' }}
                    </div>
                    <input
                      v-else-if="field.kind === 'boolean'"
                      :id="`provider-model-${field.key.replaceAll('.', '-')}`"
                      type="checkbox"
                      class="h-4 w-4 justify-self-start accent-primary"
                      :checked="modelFieldBooleanValue(field.key)"
                      @change="setModelFieldValue(field.key, ($event.target as HTMLInputElement).checked)"
                    />
                    <OptionPicker
                      v-else-if="field.kind === 'select'"
                      :id="`provider-model-${field.key.replaceAll('.', '-')}`"
                      :model-value="modelFieldValue(field.key)"
                      :options="field.options || []"
                      :include-empty="field.key === 'lifecycle'"
                      :empty-label="field.key === 'lifecycle' ? $st('Default / unset') : undefined"
                      :title="field.label"
                      @update:model-value="setModelFieldValue(field.key, $event)"
                    />
                    <textarea
                      v-else-if="field.kind === 'textarea'"
                      :id="`provider-model-${field.key.replaceAll('.', '-')}`"
                      :value="modelFieldValue(field.key)"
                      :placeholder="field.placeholder"
                      rows="3"
                      class="w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm outline-none focus:border-ring"
                      @input="setModelFieldValue(field.key, ($event.target as HTMLTextAreaElement).value)"
                    />
                    <Input
                      v-else
                      :id="`provider-model-${field.key.replaceAll('.', '-')}`"
                      :value="modelFieldValue(field.key)"
                      :type="field.kind === 'number' ? 'number' : 'text'"
                      :placeholder="field.placeholder"
                      :class="field.kind === 'csv' || field.kind === 'number' ? 'font-mono' : ''"
                      @input="
                        field.kind === 'csv'
                          ? undefined
                          : setModelFieldValue(field.key, ($event.target as HTMLInputElement).value)
                      "
                      @change="
                        field.kind === 'csv'
                          ? setModelFieldValue(field.key, ($event.target as HTMLInputElement).value)
                          : undefined
                      "
                    />
                    <div v-if="field.help" class="text-[10px] text-muted-foreground">{{ field.help }}</div>
                  </div>
                </div>
              </div>
              <label class="grid gap-1.5">
                <span class="text-xs text-muted-foreground">{{ $st('Persisted model JSON') }}</span>
                <textarea
                  v-model="modelJson"
                  rows="16"
                  spellcheck="false"
                  class="w-full rounded-md border border-input bg-transparent p-3 font-mono text-xs outline-none focus:border-ring"
                />
              </label>
              <div v-if="modelError" class="text-xs text-destructive">{{ modelError }}</div>
              <div class="flex flex-wrap items-center gap-2">
                <Button variant="outline" :disabled="mutationBusy" @click="applyModelJson">{{
                  $st('Apply JSON to provider draft')
                }}</Button>
                <span class="text-xs text-muted-foreground">{{
                  $st('Model field changes are staged automatically and saved with the Provider.')
                }}</span>
              </div>
            </template>
          </section>
          <SettingsSaveBar
            :dirty="providerDirty"
            :saving="saving"
            :disabled="mutationBusy || listingModels || !draft.provider_id.trim()"
            :error="error"
            :save-label="$st('Save provider changes')"
            sticky
            @save="saveDraft"
            @discard="discardProviderChanges"
          />
        </div>
        <div v-else-if="loading" class="py-6 text-center text-sm text-muted-foreground">
          {{ $st('Loading provider draft…') }}
        </div>
      </SettingsDisclosureRow>

      <div
        v-if="providerRows.length === 0"
        class="rounded-lg border border-dashed border-border/60 px-4 py-8 text-center text-sm text-muted-foreground"
      >
        {{ $st('No providers configured. Create one to get started.') }}
      </div>
    </div>
  </section>
</template>
