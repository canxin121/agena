<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  RiAddLine,
  RiArrowRightUpLine,
  RiChat3Line,
  RiCheckLine,
  RiErrorWarningLine,
  RiGroupLine,
  RiRefreshLine,
  RiSendPlane2Line,
  RiSettings3Line,
  RiShieldCheckLine,
} from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import BotEditor from './BotEditor.vue'
import RoomEditor from './RoomEditor.vue'
import BindingDetail from './BindingDetail.vue'
import { useTelegramCopy } from './copy'
import { telegramApi } from './api'
import {
  botConfig,
  filterTelegramChats,
  matchesTelegramBot,
  newBotConfig,
  telegramBindingRequest,
  telegramChatRows,
  type TelegramBot,
  type TelegramBotConfig,
  type TelegramCatalog,
  type TelegramChatRow,
  type TelegramInbox,
  type TelegramRoomDraft,
  type TelegramView,
} from './model'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { confirmAction } from '@/lib/appConfirm'
import { apiJson } from '@/lib/api'
import { integrationApi } from '@/lib/integrationApi'
import type {
  CreateIntegration,
  IntegrationBinding,
  IntegrationControlAction,
  IntegrationDetail,
  IntegrationsSnapshot,
} from '@/generated/agenaState'
import type { PluginRuntimeStatus } from '@/features/pluginWeb/hub'

const props = defineProps<{ source: number | null; status?: PluginRuntimeStatus; botId?: string; view?: string }>()
const emit = defineEmits<{
  sourceChange: [value: string]
  selectBot: [value: string]
  selectView: [value: TelegramView]
  refresh: []
  manage: []
  integrations: []
  openSession: [value: number]
}>()
const { tx, kindLabel, stateLabel, date } = useTelegramCopy()
const chat = useChatStore(),
  toasts = useToastsStore(),
  visible = usePaneVisibility()
const catalog = ref<TelegramCatalog>({ revision: 0, bots: [], max_bots: 32 })
const loaded = ref(false),
  loading = ref(false),
  error = ref(''),
  scopeError = ref(''),
  mutationError = ref('')
const fallbackBot = ref(''),
  localView = ref<TelegramView>('overview')
const botId = computed(() => props.botId || fallbackBot.value || catalog.value.bots[0]?.id || '')
const bot = computed(() => catalog.value.bots.find((entry) => entry.id === botId.value))
const view = computed<TelegramView>(() =>
  ['overview', 'chats', 'activity', 'settings'].includes(props.view || '')
    ? (props.view as TelegramView)
    : localView.value,
)
const inventory = ref<TelegramInbox | null>(null),
  snapshot = ref<IntegrationsSnapshot | null>(null),
  detail = ref<IntegrationDetail | null>(null)
const scopedLoading = ref(false),
  busy = ref(false),
  verifying = ref(false),
  syncing = ref(false),
  accessEditing = ref(false)
const selectedRoom = ref(''),
  selectedBinding = ref('')
const provider = computed(() => snapshot.value?.providers.find((entry) => entry.plugin_id === 'agena.telegram'))
const bindings = computed(() =>
  bot.value ? (snapshot.value?.bindings || []).filter((entry) => matchesTelegramBot(entry, bot.value!)) : [],
)
const rows = computed(() => (bot.value ? telegramChatRows(bot.value, inventory.value, bindings.value) : []))
const query = ref(''),
  kind = ref(''),
  state = ref(''),
  page = ref(1),
  activityQuery = ref('')
const filtered = computed(() => filterTelegramChats(rows.value, query.value, kind.value, state.value))
const pageCount = computed(() => Math.max(1, Math.ceil(filtered.value.length / 15)))
const visibleRows = computed(() => filtered.value.slice((page.value - 1) * 15, page.value * 15))
const activeRow = computed(() => rows.value.find((row) => row.id === selectedRoom.value))
const recent = computed(() =>
  (inventory.value?.recent || []).filter(
    (entry) =>
      !activityQuery.value ||
      `${entry.text} ${entry.sender_id || ''} ${entry.chat_id || ''}`
        .toLowerCase()
        .includes(activityQuery.value.toLowerCase()),
  ),
)
const sourceOptions = computed(() => {
  const options = chat.sessions.map((session) => ({
    id: String(session.id),
    title: session.title || String(session.id),
  }))
  if (props.source && !options.some((entry) => entry.id === String(props.source)))
    options.unshift({ id: String(props.source), title: `#${props.source}` })
  return options
})
const editor = ref<{ initial: TelegramBotConfig; existing: boolean; hasToken: boolean; revision: number } | null>(null)
const roomDraft = ref<TelegramRoomDraft | null>(null)
const pendingCreate = ref<{ source: number; botId: string; request: CreateIntegration } | null>(null)
const inspectId = ref(''),
  inspectBusy = ref(false)
const logs = ref<Array<{ seq: number; timestamp_ms: number; level: string; message: string }>>([])
const logsError = ref('')
const field =
  'h-10 w-full min-w-0 rounded-lg border border-border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'
let disposed = false,
  catalogRevision = 0,
  epoch = 0,
  detailRevision = 0
let catalogRead: AbortController | null = null,
  scopeRead: AbortController | null = null,
  detailRead: AbortController | null = null,
  verificationRead: AbortController | null = null
const message = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause))
const scopeKey = () => `${botId.value}:${props.source || ''}`

function fenceScopeReads() {
  epoch++
  detailRevision++
  scopeRead?.abort()
  detailRead?.abort()
  scopedLoading.value = false
}
async function refreshCatalog() {
  const revision = ++catalogRevision
  catalogRead?.abort()
  const controller = new AbortController()
  catalogRead = controller
  loading.value = true
  try {
    const next = await telegramApi.list(controller.signal)
    if (disposed || revision !== catalogRevision) return
    if (!next || !Array.isArray(next.bots) || !Number.isSafeInteger(next.revision))
      throw new Error('Invalid Telegram bot catalog')
    catalog.value = next
    loaded.value = true
    error.value = ''
  } catch (cause) {
    if (!disposed && revision === catalogRevision && !controller.signal.aborted) error.value = message(cause)
  } finally {
    if (!disposed && revision === catalogRevision) loading.value = false
  }
}
async function refreshScope() {
  if (!bot.value || disposed || scopedLoading.value || busy.value || accessEditing.value) return
  const current = epoch,
    key = scopeKey(),
    owner = bot.value.id,
    source = props.source
  scopeRead?.abort()
  const controller = new AbortController()
  scopeRead = controller
  scopedLoading.value = true
  try {
    const results = await Promise.allSettled([
      bot.value.connection.account ? telegramApi.inventory(owner, controller.signal) : Promise.resolve(null),
      source ? integrationApi.list(source, controller.signal) : Promise.resolve(null),
    ])
    if (disposed || current !== epoch || key !== scopeKey()) return
    const failures: string[] = []
    if (results[0].status === 'fulfilled') inventory.value = results[0].value
    else if (!controller.signal.aborted) failures.push(message(results[0].reason))
    if (results[1].status === 'fulfilled') snapshot.value = results[1].value
    else if (!controller.signal.aborted) failures.push(message(results[1].reason))
    scopeError.value = failures.join(' · ')
    if (selectedBinding.value) await loadDetail(selectedBinding.value)
  } finally {
    if (!disposed && current === epoch) scopedLoading.value = false
  }
}
function resetScope() {
  epoch++
  detailRevision++
  scopeRead?.abort()
  detailRead?.abort()
  verificationRead?.abort()
  scopedLoading.value = false
  verifying.value = false
  inventory.value = null
  snapshot.value = null
  detail.value = null
  selectedRoom.value = ''
  selectedBinding.value = ''
  scopeError.value = ''
  mutationError.value = ''
  roomDraft.value = null
  pendingCreate.value = null
  page.value = 1
  void refreshScope()
}
async function loadDetail(id: string) {
  const source = props.source
  if (!source || !id || !bindings.value.some((entry) => entry.id === id) || accessEditing.value) return
  const current = ++detailRevision,
    key = scopeKey()
  detailRead?.abort()
  const controller = new AbortController()
  detailRead = controller
  try {
    const result = await integrationApi.detail(source, id, controller.signal)
    if (!disposed && current === detailRevision && key === scopeKey() && selectedBinding.value === id)
      detail.value = result
  } catch (cause) {
    if (!disposed && current === detailRevision && !controller.signal.aborted) scopeError.value = message(cause)
  }
}
function chooseBot(id: string) {
  if (busy.value || editor.value || pendingCreate.value) return
  fallbackBot.value = id
  emit('selectBot', id)
}
function chooseView(value: TelegramView) {
  localView.value = value
  emit('selectView', value)
  if (value === 'activity') void loadLogs()
}
function chooseKind(value: string) {
  kind.value = value
  chooseView('chats')
}
function chooseBinding(id: string) {
  selectedBinding.value = id
  void loadDetail(id)
}
function openRecentRoom(id: string | null) {
  selectedRoom.value = id || ''
  chooseView('chats')
}
function chooseRoom(row: TelegramChatRow) {
  selectedRoom.value = row.id
  selectedBinding.value = row.bindings[0]?.id || ''
  detail.value = null
  if (selectedBinding.value) void loadDetail(selectedBinding.value)
}
function editBot(target?: TelegramBot) {
  mutationError.value = ''
  editor.value = {
    initial: target ? botConfig(target) : newBotConfig(),
    existing: target?.managed === true,
    hasToken: target?.credential_kind === 'vault' && target.has_token === true,
    revision: catalog.value.revision,
  }
}
async function saveBot(config: TelegramBotConfig, token: string) {
  if (!editor.value || busy.value) return
  const revision = editor.value.revision
  busy.value = true
  mutationError.value = ''
  try {
    await telegramApi.save(revision, config, config.credential_kind === 'vault' ? token : undefined)
    if (disposed) return
    editor.value = null
    fallbackBot.value = config.id
    emit('selectBot', config.id)
    await refreshCatalog()
    toasts.push('success', tx('saved'))
    emit('refresh')
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed) await refreshScope()
}
async function toggleBot(target: TelegramBot) {
  const current = botConfig(target),
    revision = catalog.value.revision
  if (
    busy.value ||
    !(await confirmAction({
      title: tx(target.enabled ? 'disable' : 'enable'),
      description: tx('disableHint'),
      confirmText: tx('confirm'),
      cancelText: tx('cancel'),
      variant: 'default',
    }))
  )
    return
  if (disposed || catalog.value.revision !== revision) {
    mutationError.value = 'Settings changed. Refresh and try again.'
    return
  }
  busy.value = true
  mutationError.value = ''
  try {
    await telegramApi.save(revision, { ...current, enabled: !current.enabled })
    if (!disposed) await refreshCatalog()
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
}
async function removeBot(target: TelegramBot) {
  const revision = catalog.value.revision
  if (
    target.enabled ||
    busy.value ||
    !(await confirmAction({
      title: tx('delete'),
      description: tx('deleteHint'),
      confirmText: tx('delete'),
      cancelText: tx('cancel'),
    }))
  )
    return
  if (disposed) return
  busy.value = true
  try {
    await telegramApi.remove(revision, target.id)
    if (disposed) return
    await refreshCatalog()
    const id = catalog.value.bots[0]?.id || ''
    fallbackBot.value = id
    emit('selectBot', id)
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
}
async function verify() {
  if (!bot.value || verifying.value) return
  const id = bot.value.id,
    current = epoch
  verificationRead?.abort()
  const controller = new AbortController()
  verificationRead = controller
  verifying.value = true
  mutationError.value = ''
  try {
    const result = await telegramApi.verify(id, controller.signal)
    if (!disposed && current === epoch) {
      await refreshCatalog()
      toasts.push('success', `@${result.username} · ${result.bot_id}`)
      await refreshScope()
    }
  } catch (cause) {
    if (!disposed && current === epoch && !controller.signal.aborted) {
      mutationError.value = message(cause)
      await refreshCatalog()
    }
  } finally {
    if (!disposed && current === epoch) verifying.value = false
  }
}
async function synchronize() {
  if (!bot.value || syncing.value || !bot.value.enabled) return
  const id = bot.value.id,
    current = epoch
  syncing.value = true
  mutationError.value = ''
  try {
    const result = await telegramApi.sync(id)
    if (!disposed && current === epoch) {
      inventory.value = result
      await refreshCatalog()
    }
  } catch (cause) {
    if (!disposed && current === epoch) mutationError.value = message(cause)
  } finally {
    if (!disposed) syncing.value = false
  }
}
async function inspect() {
  if (!bot.value || inspectBusy.value || !inspectId.value.trim()) return
  const id = bot.value.id,
    current = epoch
  inspectBusy.value = true
  mutationError.value = ''
  try {
    const result = await telegramApi.inspect(id, inspectId.value.trim())
    if (!disposed && current === epoch) {
      await refreshScope()
      selectedRoom.value = result.room.chat_id
      inspectId.value = ''
    }
  } catch (cause) {
    if (!disposed && current === epoch) mutationError.value = message(cause)
  } finally {
    if (!disposed) inspectBusy.value = false
  }
}
function newBinding(row?: TelegramChatRow) {
  if (!bot.value || !props.source || !provider.value) return
  mutationError.value = ''
  pendingCreate.value = null
  const type =
    row && ['direct', 'group', 'broadcast'].includes(row.kind)
      ? (row.kind as 'direct' | 'group' | 'broadcast')
      : 'direct'
  roomDraft.value = {
    botId: bot.value.id,
    title: row?.title || '',
    chatId: row?.id || '',
    kind: type,
    ownerId: type === 'direct' ? row?.room?.last_sender_id || '' : '',
    members: '',
    useSourcePermissions: false,
    trigger: type === 'group' ? 'direct_or_mention' : 'every_message',
    sessionMode: 'per_conversation',
    threadMode: type === 'group' ? 'per_thread' : 'exact',
    threadId: '',
    dailyLimit: 20,
    totalLimit: 1000,
    perUserLimit: 20,
    expiresDays: 30,
    executionSeconds: 300,
    pollSeconds: 2,
    allowUnlisted: false,
  }
}
async function createBinding(draft: TelegramRoomDraft) {
  if (!bot.value || !props.source || !provider.value || busy.value) return
  try {
    pendingCreate.value ||= {
      source: props.source,
      botId: bot.value.id,
      request: telegramBindingRequest(provider.value, bot.value.connection, draft),
    }
  } catch (cause) {
    mutationError.value = message(cause)
    return
  }
  fenceScopeReads()
  const owner = pendingCreate.value,
    current = epoch
  if (owner.source !== props.source || owner.botId !== bot.value.id) return
  busy.value = true
  mutationError.value = ''
  try {
    const result = await integrationApi.create(owner.source, owner.request)
    if (disposed || current !== epoch) return
    pendingCreate.value = null
    roomDraft.value = null
    selectedRoom.value = String(result.spec.resource)
    selectedBinding.value = result.id
  } catch (cause) {
    if (!disposed && current === epoch) {
      mutationError.value = message(cause)
      const status = cause && typeof cause === 'object' && 'status' in cause ? cause.status : null
      if (status === 400 || status === 422) pendingCreate.value = null
    }
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed && current === epoch) await refreshScope()
}
async function control(binding: IntegrationBinding, action: IntegrationControlAction) {
  const source = props.source,
    current = epoch
  if (!source || busy.value || !bindings.value.some((entry) => entry.id === binding.id)) return
  if (
    action !== 'poll_now' &&
    !(await confirmAction({
      title: tx(action === 'enable' ? 'enableBinding' : 'pauseBinding'),
      description: tx('bindingConfirm'),
      confirmText: tx('confirm'),
      cancelText: tx('cancel'),
      variant: 'default',
    }))
  )
    return
  if (disposed || current !== epoch) return
  busy.value = true
  mutationError.value = ''
  try {
    await integrationApi.control(source, binding.id, { action, expected_revision: binding.revision })
  } catch (cause) {
    if (!disposed && current === epoch) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed && current === epoch) await refreshScope()
}
async function loadLogs() {
  try {
    const result = await apiJson<{ logs?: typeof logs.value }>('/api/v1/plugins/agena.telegram/logs?limit=80')
    if (!disposed) {
      logs.value = result.logs || []
      logsError.value = ''
    }
  } catch (cause) {
    if (!disposed) logsError.value = message(cause)
  }
}
function accessChanged(binding: IntegrationBinding) {
  if (detail.value?.binding.id === binding.id) detail.value.binding = binding
  void refreshScope()
}
async function refreshAll() {
  await refreshCatalog()
  await refreshScope()
}
watch(() => [botId.value, props.source], resetScope)
watch([query, kind, state], () => {
  page.value = 1
})
watch(pageCount, (count) => {
  page.value = Math.min(count, page.value)
})
onMounted(() => {
  void chat.refreshSessions().catch(() => {})
  void refreshAll()
})
const timer = setInterval(() => {
  if (visible.value && !busy.value && !editor.value && !roomDraft.value && !accessEditing.value) void refreshAll()
}, 15000)
onBeforeUnmount(() => {
  disposed = true
  epoch++
  catalogRevision++
  detailRevision++
  catalogRead?.abort()
  scopeRead?.abort()
  detailRead?.abort()
  verificationRead?.abort()
  clearInterval(timer)
  editor.value = null
})
</script>

<template>
  <main class="mx-auto w-full max-w-[1500px] space-y-6 px-4 py-6 sm:px-7 sm:py-8" data-testid="telegram-manager">
    <header class="flex flex-wrap items-center justify-between gap-4">
      <div class="flex min-w-0 items-start gap-3">
        <span class="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-sky-500/10 text-sky-500"
          ><RiSendPlane2Line class="h-6 w-6" aria-hidden="true"
        /></span>
        <div class="min-w-0">
          <h2 class="text-2xl font-semibold tracking-tight">{{ tx('title') }}</h2>
          <p class="mt-1 max-w-2xl text-xs leading-relaxed text-muted-foreground">{{ tx('subtitle') }}</p>
        </div>
      </div>
      <div class="flex gap-2">
        <Button variant="ghost" size="sm" @click="emit('manage')"
          ><RiSettings3Line class="h-4 w-4" />{{ tx('fullSettings') }}</Button
        ><Button variant="outline" size="sm" :disabled="loading || busy" @click="refreshAll"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />{{ tx('refresh') }}</Button
        ><Button
          size="sm"
          :disabled="busy || catalog.bots.length >= catalog.max_bots"
          data-testid="telegram-add-bot"
          @click="editBot()"
          ><RiAddLine class="h-4 w-4" />{{ tx('addBot') }}</Button
        >
      </div>
    </header>
    <p v-if="error" role="alert" class="break-words rounded-xl bg-destructive/10 p-4 text-sm text-destructive">
      {{ error }}
    </p>
    <div class="grid gap-5 xl:grid-cols-[220px_minmax(0,1fr)]">
      <aside class="min-w-0 xl:sticky xl:top-3 xl:self-start">
        <div class="mb-3 flex items-center justify-between">
          <h3 class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">{{ tx('bots') }}</h3>
          <span class="rounded-md bg-muted px-2 text-xs text-muted-foreground"
            >{{ catalog.bots.length }}/{{ catalog.max_bots }}</span
          >
        </div>
        <div
          class="flex gap-2 overflow-x-auto pb-2 sm:grid sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-1"
          data-testid="telegram-bot-list"
        >
          <button
            v-for="entry in catalog.bots"
            :key="entry.id"
            type="button"
            :data-telegram-bot="entry.id"
            :aria-pressed="entry.id === botId"
            class="w-52 min-w-0 shrink-0 rounded-xl border p-3 text-left transition-colors sm:w-full hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring"
            :class="entry.id === botId ? 'border-sky-500/50 bg-sky-500/5' : 'border-border/60 bg-card'"
            :disabled="busy || !!pendingCreate"
            @click="chooseBot(entry.id)"
          >
            <div class="flex items-center justify-between gap-2">
              <span class="truncate text-sm font-semibold">{{ entry.name }}</span
              ><span
                class="h-2 w-2 shrink-0 rounded-full"
                :class="
                  !entry.enabled
                    ? 'bg-muted-foreground/40'
                    : entry.last_error
                      ? 'bg-destructive'
                      : entry.verification
                        ? 'bg-emerald-500'
                        : 'bg-amber-500'
                "
              />
            </div>
            <p class="mt-1 truncate font-mono text-[11px] text-muted-foreground">
              {{ entry.connection.account ? '@' + entry.connection.account : tx('missingToken') }}
            </p>
            <div class="mt-3 flex justify-between gap-2 text-[10px] text-muted-foreground">
              <span>{{
                entry.enabled ? (entry.verification ? tx('connected') : tx('notVerified')) : tx('disabled')
              }}</span
              ><span>{{ entry.room_count }} {{ tx('chats') }}</span>
            </div>
          </button>
        </div>
      </aside>
      <section v-if="bot" class="min-w-0 space-y-5">
        <div class="flex flex-wrap items-start justify-between gap-3 rounded-2xl border border-border/70 bg-card p-5">
          <div class="min-w-0">
            <h3 class="text-lg font-semibold">
              {{ bot.name }}
              <span v-if="bot.connection.account" class="ml-2 text-sm font-normal text-muted-foreground"
                >@{{ bot.connection.account }}</span
              >
            </h3>
            <div class="mt-2 flex flex-wrap gap-2 text-[11px]">
              <span class="rounded bg-muted px-2 py-1">{{ bot.enabled ? tx('enabled') : tx('disabled') }}</span
              ><span class="rounded bg-muted px-2 py-1">{{
                bot.credential_kind === 'vault' ? tx('vault') : tx('environment')
              }}</span
              ><span class="rounded bg-muted px-2 py-1">{{
                bot.has_token ? tx('configured') : tx('missingToken')
              }}</span
              ><span v-if="!bot.managed" class="rounded bg-muted px-2 py-1">{{ tx('legacy') }}</span>
            </div>
          </div>
          <div class="flex flex-wrap gap-2">
            <Button
              variant="outline"
              size="sm"
              :disabled="busy"
              data-testid="telegram-edit-bot"
              @click="editBot(bot)"
              >{{ tx('edit') }}</Button
            ><Button
              variant="outline"
              size="sm"
              :disabled="busy || verifying || !bot.connection.account"
              data-testid="telegram-verify"
              @click="verify"
              ><RiCheckLine class="h-4 w-4" />{{ verifying ? tx('verifying') : tx('verify') }}</Button
            >
          </div>
        </div>
        <p
          v-if="mutationError && !editor && !roomDraft"
          role="alert"
          class="break-words rounded-xl border border-destructive/20 bg-destructive/5 p-3 text-sm text-destructive"
        >
          {{ mutationError }}
        </p>
        <p v-if="scopeError" role="alert" class="break-words rounded-xl bg-destructive/5 p-3 text-sm text-destructive">
          {{ scopeError }}
        </p>
        <nav class="flex gap-1 overflow-x-auto border-b border-border pb-2" :aria-label="tx('title')">
          <Button
            v-for="tab in ['overview', 'chats', 'activity', 'settings'] as const"
            :key="tab"
            size="sm"
            :variant="view === tab ? 'secondary' : 'ghost'"
            :data-telegram-tab="tab"
            :aria-pressed="view === tab"
            @click="chooseView(tab)"
            >{{ tx(tab) }}</Button
          >
        </nav>

        <template v-if="view === 'overview'">
          <div class="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <button
              v-for="type in ['direct', 'group', 'broadcast'] as const"
              :key="type"
              type="button"
              class="rounded-2xl border border-border/70 bg-card p-4 text-left hover:bg-accent/30"
              @click="chooseKind(type)"
            >
              <span class="text-xs text-muted-foreground">{{ kindLabel(type) }}</span
              ><strong class="mt-2 block text-2xl font-semibold tabular-nums">{{
                rows.filter((row) => row.kind === type).length
              }}</strong>
            </button>
            <div class="rounded-2xl border border-border/70 bg-card p-4">
              <span class="text-xs text-muted-foreground">{{ tx('authorized') }}</span
              ><strong class="mt-2 block text-2xl font-semibold tabular-nums">{{
                source ? bindings.length : '—'
              }}</strong>
            </div>
          </div>
          <div class="grid gap-4 lg:grid-cols-2">
            <section
              class="space-y-4 rounded-2xl border border-border/70 bg-card p-5"
              data-testid="telegram-connection"
            >
              <h4 class="flex items-center gap-2 font-semibold">
                <RiShieldCheckLine class="h-4 w-4 text-sky-500" />{{ tx('connected') }}
              </h4>
              <dl class="space-y-3 text-xs">
                <div class="flex justify-between gap-3">
                  <dt class="text-muted-foreground">{{ tx('botId') }}</dt>
                  <dd class="font-mono">{{ bot.verification?.bot_id || '—' }}</dd>
                </div>
                <div class="flex justify-between gap-3">
                  <dt class="text-muted-foreground">{{ tx('lastVerify') }}</dt>
                  <dd>{{ date(bot.verification?.checked_at_ms) }}</dd>
                </div>
                <div>
                  <dt class="mb-1 text-muted-foreground">{{ tx('webhook') }}</dt>
                  <dd :class="bot.verification?.webhook_active ? 'text-amber-600 dark:text-amber-400' : ''">
                    {{
                      !bot.verification
                        ? tx('notVerified')
                        : bot.verification.webhook_warning
                          ? tx('webhookUnknown')
                          : bot.verification.webhook_active
                            ? tx('webhookActive')
                            : tx('webhookClear')
                    }}
                  </dd>
                </div>
              </dl>
              <p v-if="bot.last_error" class="break-words text-xs text-destructive">{{ bot.last_error }}</p>
            </section>
            <section class="space-y-4 rounded-2xl border border-border/70 bg-card p-5">
              <h4 class="flex items-center gap-2 font-semibold">
                <RiChat3Line class="h-4 w-4 text-sky-500" />{{ tx('inbox') }}
              </h4>
              <dl class="space-y-3 text-xs">
                <div class="flex justify-between">
                  <dt class="text-muted-foreground">{{ tx('inbox') }}</dt>
                  <dd>{{ inventory?.buffered_updates ?? bot.buffered_updates }}</dd>
                </div>
                <div class="flex justify-between gap-3">
                  <dt class="text-muted-foreground">{{ tx('lastSync') }}</dt>
                  <dd>{{ date(inventory?.last_poll_ms || bot.last_poll_ms) }}</dd>
                </div>
              </dl>
              <p class="text-xs leading-relaxed text-muted-foreground">{{ tx('retention') }}</p>
              <Button
                variant="outline"
                size="sm"
                :disabled="syncing || !bot.enabled || !bot.connection.account"
                data-testid="telegram-sync"
                @click="synchronize"
                ><RiRefreshLine class="h-4 w-4" :class="syncing ? 'animate-spin' : ''" />{{
                  syncing ? tx('syncing') : tx('sync')
                }}</Button
              >
            </section>
          </div>
          <div class="rounded-2xl border border-border/70 bg-card p-5">
            <label class="block max-w-xl text-sm font-medium"
              >{{ tx('source')
              }}<select
                :value="source ? String(source) : ''"
                :class="field + ' mt-2'"
                :disabled="busy || !!pendingCreate"
                data-testid="telegram-source"
                @change="emit('sourceChange', ($event.target as HTMLSelectElement).value)"
              >
                <option value="">—</option>
                <option v-for="item in sourceOptions" :key="item.id" :value="item.id">
                  {{ item.title }} ({{ item.id }})
                </option>
              </select></label
            >
            <p class="mt-2 text-xs leading-relaxed text-muted-foreground">
              {{ source ? tx('sourceHint') : tx('noSource') }}
            </p>
          </div>
        </template>

        <template v-else-if="view === 'chats'">
          <div class="flex flex-wrap items-end justify-between gap-3">
            <label class="block min-w-0 flex-1 text-xs font-medium sm:max-w-sm"
              >{{ tx('source')
              }}<select
                :value="source ? String(source) : ''"
                :class="field + ' mt-1'"
                :disabled="busy || !!pendingCreate"
                data-testid="telegram-source"
                @change="emit('sourceChange', ($event.target as HTMLSelectElement).value)"
              >
                <option value="">—</option>
                <option v-for="item in sourceOptions" :key="item.id" :value="item.id">
                  {{ item.title }} ({{ item.id }})
                </option>
              </select></label
            >
            <div class="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                :disabled="syncing || !bot.enabled || !bot.connection.account"
                data-testid="telegram-sync"
                @click="synchronize"
                >{{ syncing ? tx('syncing') : tx('sync') }}</Button
              ><Button
                size="sm"
                :disabled="!source || !provider || busy || !bot.connection.account"
                data-testid="telegram-add-chat"
                @click="newBinding()"
                ><RiAddLine class="h-4 w-4" />{{ tx('addChat') }}</Button
              >
            </div>
          </div>
          <p class="text-xs leading-relaxed text-muted-foreground">
            {{ tx('discoverHint') }} <span v-if="!source">{{ tx('noSource') }}</span>
          </p>
          <form class="flex flex-wrap gap-2" @submit.prevent="inspect">
            <input
              v-model="inspectId"
              :class="field + ' flex-1 sm:max-w-sm'"
              :aria-label="tx('inspect')"
              placeholder="Chat ID / @username"
              data-testid="telegram-inspect-id"
            /><Button
              type="submit"
              variant="outline"
              :disabled="inspectBusy || !inspectId.trim() || !bot.connection.account"
              data-testid="telegram-inspect"
              >{{ tx('inspect') }}</Button
            >
          </form>
          <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_140px_150px]">
            <input
              v-model="query"
              type="search"
              :class="field"
              :placeholder="tx('search')"
              :aria-label="tx('search')"
              data-testid="telegram-chat-search"
            /><select v-model="kind" :class="field" :aria-label="tx('allKinds')" data-testid="telegram-kind-filter">
              <option value="">{{ tx('allKinds') }}</option>
              <option v-for="item in ['direct', 'group', 'broadcast']" :key="item" :value="item">
                {{ kindLabel(item) }}
              </option></select
            ><select v-model="state" :class="field" :aria-label="tx('allStates')">
              <option value="">{{ tx('allStates') }}</option>
              <option
                v-for="item in ['unbound', 'enabled', 'disabled', 'paused', 'expired', 'faulted']"
                :key="item"
                :value="item"
              >
                {{ stateLabel(item) }}
              </option>
            </select>
          </div>
          <div class="overflow-hidden rounded-2xl border border-border/70 bg-card" data-testid="telegram-chat-list">
            <p v-if="!visibleRows.length" class="px-5 py-10 text-center text-sm text-muted-foreground">
              {{ tx('noRooms')
              }}<span class="mx-auto mt-2 block max-w-lg text-xs leading-relaxed">{{ tx('noRoomsHint') }}</span>
            </p>
            <button
              v-for="row in visibleRows"
              :key="row.id"
              type="button"
              :data-telegram-chat="row.id"
              class="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] items-start gap-3 border-b border-border/50 px-4 py-4 text-left last:border-b-0 hover:bg-accent/30"
              :class="selectedRoom === row.id ? 'bg-accent/50' : ''"
              @click="chooseRoom(row)"
            >
              <span class="min-w-0"
                ><span class="flex items-center gap-2"
                  ><RiGroupLine v-if="row.kind === 'group'" class="h-4 w-4 shrink-0 text-sky-500" /><RiSendPlane2Line
                    v-else-if="row.kind === 'broadcast'"
                    class="h-4 w-4 shrink-0 text-sky-500"
                  /><RiChat3Line v-else class="h-4 w-4 shrink-0 text-sky-500" /><strong class="truncate text-sm">{{
                    row.title
                  }}</strong></span
                ><span class="mt-1 block truncate font-mono text-[10px] text-muted-foreground"
                  >{{ row.id }} {{ row.username ? '@' + row.username : '' }}</span
                ><span v-if="row.room?.last_text" class="mt-2 line-clamp-1 block text-xs text-muted-foreground">{{
                  row.room.last_text
                }}</span></span
              ><span class="space-y-2 text-right"
                ><span class="block text-[10px] text-muted-foreground">{{ kindLabel(row.kind) }}</span
                ><span
                  class="block rounded-md bg-muted px-2 py-1 text-[10px]"
                  :class="
                    row.state === 'enabled'
                      ? 'text-emerald-600 dark:text-emerald-400'
                      : row.state === 'faulted'
                        ? 'text-destructive'
                        : 'text-muted-foreground'
                  "
                  >{{ stateLabel(row.state) }}</span
                ></span
              >
            </button>
          </div>
          <div v-if="pageCount > 1" class="flex items-center justify-between text-xs text-muted-foreground">
            <span>{{ page }} / {{ pageCount }} · {{ filtered.length }}</span>
            <div class="flex gap-2">
              <Button size="sm" variant="outline" :disabled="page <= 1" @click="page--">{{ tx('previous') }}</Button
              ><Button size="sm" variant="outline" :disabled="page >= pageCount" @click="page++">{{
                tx('next')
              }}</Button>
            </div>
          </div>
          <section
            v-if="activeRow"
            class="space-y-4 rounded-2xl border border-border/70 bg-card p-5"
            data-testid="telegram-chat-inspector"
          >
            <div class="flex flex-wrap justify-between gap-3">
              <div>
                <h4 class="font-semibold">{{ activeRow.title }}</h4>
                <p class="mt-1 break-all font-mono text-xs text-muted-foreground">{{ activeRow.id }}</p>
              </div>
              <Button
                size="sm"
                variant="outline"
                :disabled="!source || !provider || busy"
                @click="newBinding(activeRow)"
                >{{ tx('authorize') }}</Button
              >
            </div>
            <p v-if="activeRow.room?.description" class="whitespace-pre-wrap text-sm text-muted-foreground">
              {{ activeRow.room.description }}
            </p>
            <dl v-if="activeRow.room" class="grid gap-3 text-xs sm:grid-cols-2">
              <div>
                <dt class="text-muted-foreground">{{ tx('lastMessage') }}</dt>
                <dd class="mt-1">{{ date(activeRow.room.last_seen_ms) }}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">{{ tx('membership') }}</dt>
                <dd class="mt-1">{{ activeRow.room.bot_membership?.status || '—' }}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">{{ tx('seenMessages') }}</dt>
                <dd class="mt-1">{{ activeRow.room.messages_seen }}</dd>
              </div>
              <div>
                <dt class="text-muted-foreground">{{ tx('topics') }}</dt>
                <dd class="mt-1 break-words">
                  {{
                    Object.entries(activeRow.room.topics)
                      .map(([id, title]) => `${title} (${id})`)
                      .join(', ') || '—'
                  }}
                </dd>
              </div>
            </dl>
            <div v-if="activeRow.bindings.length" class="flex flex-wrap gap-2">
              <Button
                v-for="binding in activeRow.bindings"
                :key="binding.id"
                size="sm"
                :variant="binding.id === selectedBinding ? 'secondary' : 'ghost'"
                :data-telegram-binding="binding.id"
                @click="chooseBinding(binding.id)"
                >{{ binding.spec.title }}</Button
              >
            </div>
            <p v-else class="text-xs text-muted-foreground">{{ tx('noBindings') }}</p>
          </section>
          <BindingDetail
            v-if="detail && source"
            :key="`${botId}:${source}:${detail.binding.id}`"
            :detail="detail"
            :source="source"
            :busy="busy"
            :bot-enabled="bot.enabled"
            @control="control"
            @changed="accessChanged"
            @editing="accessEditing = $event"
            @open-session="emit('openSession', $event)"
          />
        </template>

        <template v-else-if="view === 'activity'">
          <p class="text-xs leading-relaxed text-muted-foreground">{{ tx('knownOnly') }}</p>
          <input
            v-model="activityQuery"
            :class="field"
            type="search"
            :placeholder="tx('messageFilter')"
            :aria-label="tx('messageFilter')"
          />
          <div class="space-y-3" data-testid="telegram-recent-messages">
            <p
              v-if="!recent.length"
              class="rounded-xl border border-dashed border-border px-4 py-10 text-center text-sm text-muted-foreground"
            >
              {{ tx('none') }}
            </p>
            <article
              v-for="entry in recent"
              :key="entry.sequence"
              class="space-y-2 rounded-xl border border-border/70 bg-card p-4"
            >
              <div class="flex flex-wrap justify-between gap-2 font-mono text-[10px] text-muted-foreground">
                <button type="button" class="underline" @click="openRecentRoom(entry.chat_id)">
                  {{ entry.chat_id }}</button
                ><span>{{ entry.sender_id }} · {{ date(entry.received_at_ms) }}</span>
              </div>
              <p class="whitespace-pre-wrap break-words text-sm">{{ entry.text }}</p>
            </article>
          </div>
          <details
            class="rounded-xl border border-border/70 p-4"
            data-testid="telegram-logs"
            @toggle="($event.target as HTMLDetailsElement).open && loadLogs()"
          >
            <summary class="cursor-pointer text-sm font-semibold">{{ tx('logs') }}</summary>
            <p v-if="logsError" role="alert" class="mt-3 text-sm text-destructive">{{ logsError }}</p>
            <p v-if="!logs.length" class="mt-3 text-xs text-muted-foreground">{{ tx('none') }}</p>
            <article v-for="entry in logs" :key="entry.seq" class="mt-3 border-t border-border/50 pt-3 text-xs">
              <span class="text-muted-foreground">{{ entry.level }} · {{ date(entry.timestamp_ms) }}</span>
              <p class="mt-1 break-words">{{ entry.message }}</p>
            </article>
          </details>
        </template>

        <section v-else class="space-y-5 rounded-2xl border border-border/70 bg-card p-5">
          <h4 class="font-semibold">{{ tx('settings') }}</h4>
          <dl class="grid gap-4 text-sm sm:grid-cols-2">
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('account') }}</dt>
              <dd class="mt-1 font-mono">{{ bot.connection.account || '—' }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('credential') }}</dt>
              <dd class="mt-1">{{ bot.credential_kind === 'vault' ? tx('vault') : bot.connection.bot_token_env }}</dd>
            </div>
            <div class="sm:col-span-2">
              <dt class="text-xs text-muted-foreground">{{ tx('apiBase') }}</dt>
              <dd class="mt-1 break-all font-mono text-xs">{{ bot.connection.api_base }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('longPoll') }}</dt>
              <dd class="mt-1">{{ bot.connection.long_poll_seconds }}</dd>
            </div>
          </dl>
          <p class="text-xs leading-relaxed text-muted-foreground">{{ tx('savedIdentity') }}</p>
          <div class="flex flex-wrap gap-2">
            <Button size="sm" :disabled="busy" @click="editBot(bot)">{{ tx('edit') }}</Button
            ><Button
              size="sm"
              variant="outline"
              :disabled="busy || !bot.connection.account"
              data-testid="telegram-toggle-bot"
              @click="toggleBot(bot)"
              >{{ bot.enabled ? tx('disable') : tx('enable') }}</Button
            ><Button
              size="sm"
              variant="outline"
              :disabled="busy || bot.enabled || !bot.managed"
              data-testid="telegram-remove-bot"
              @click="removeBot(bot)"
              >{{ tx('delete') }}</Button
            >
          </div>
        </section>
      </section>
      <div
        v-else
        class="flex min-h-60 items-center justify-center rounded-2xl border border-dashed border-border p-6 text-sm text-muted-foreground"
      >
        {{ loading || !loaded ? tx('refresh') : tx('addBot') }}
      </div>
    </div>
    <BotEditor
      v-if="editor"
      :key="editor.initial.id"
      :initial="editor.initial"
      :existing="editor.existing"
      :has-token="editor.hasToken"
      :busy="busy"
      :error="mutationError"
      @close="editor = null"
      @save="saveBot"
    />
    <RoomEditor
      v-if="roomDraft"
      :key="`${botId}:${source}`"
      :initial="roomDraft"
      :busy="busy"
      :pending="!!pendingCreate"
      :error="mutationError"
      @close="roomDraft = null"
      @submit="createBinding"
    />
  </main>
</template>
