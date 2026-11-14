<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  RiAddLine,
  RiArrowRightUpLine,
  RiChat3Line,
  RiCheckLine,
  RiRefreshLine,
  RiSendPlane2Line,
  RiSettings3Line,
} from '@remixicon/vue'
import Button from '@/components/ui/Button.vue'
import BotEditor from './BotEditor.vue'
import ManualChatEditor from './ManualChatEditor.vue'
import BindingDetail from './BindingDetail.vue'
import { useTelegramCopy } from './copy'
import { telegramApi } from './api'
import {
  botConfig,
  filterTelegramChats,
  matchesTelegramBot,
  newBotConfig,
  telegramChatAllowed,
  telegramChatRows,
  copyTelegramChatAccess,
  type TelegramAiBot,
  type TelegramBot,
  type TelegramBotConfig,
  type TelegramCatalog,
  type TelegramChatRow,
  type TelegramInbox,
  type TelegramView,
} from './model'
import { usePaneVisibility } from '@/composables/usePaneVisibility'
import { useToastsStore } from '@/stores/toasts'
import { confirmAction } from '@/lib/appConfirm'
import { apiJson } from '@/lib/api'
import { integrationApi } from '@/lib/integrationApi'
import type {
  IntegrationBinding,
  IntegrationControlAction,
  IntegrationDetail,
  IntegrationsSnapshot,
} from '@/generated/agenaState'
import type { PluginRuntimeStatus } from '@/features/pluginWeb/hub'

const props = defineProps<{ status?: PluginRuntimeStatus; botId?: string; view?: string }>()
const emit = defineEmits<{
  selectBot: [value: string]
  selectView: [value: TelegramView]
  refresh: []
  manage: []
  openSession: [value: number]
}>()
const { tx, kindLabel, stateLabel, date } = useTelegramCopy()
const toasts = useToastsStore(),
  visible = usePaneVisibility()
const catalog = ref<TelegramCatalog>({ revision: 0, bots: [], max_bots: 32 })
const aiBots = ref<TelegramAiBot[]>([])
const loading = ref(false),
  loaded = ref(false),
  aiLoading = ref(false)
const error = ref(''),
  aiError = ref(''),
  scopeError = ref(''),
  mutationError = ref('')
const fallbackBot = ref(''),
  localView = ref<TelegramView>('chats')
const botId = computed(() => {
  const requested = props.botId || fallbackBot.value
  return catalog.value.bots.some((entry) => entry.id === requested) ? requested : catalog.value.bots[0]?.id || ''
})
const bot = computed(() => catalog.value.bots.find((entry) => entry.id === botId.value))
const source = computed(() => bot.value?.source_session_id || null)
const view = computed<TelegramView>(() =>
  ['activity', 'settings'].includes(props.view || '')
    ? (props.view as TelegramView)
    : props.view
      ? 'chats'
      : localView.value,
)
const inventory = ref<TelegramInbox | null>(null)
const snapshot = ref<IntegrationsSnapshot | null>(null),
  detail = ref<IntegrationDetail | null>(null)
const busy = ref(false),
  scopedLoading = ref(false),
  syncing = ref(false),
  verifying = ref(false)
const accessEditing = ref(false),
  selectedRoom = ref(''),
  selectedBinding = ref('')
const bindings = computed(() =>
  bot.value ? (snapshot.value?.bindings || []).filter((entry) => matchesTelegramBot(entry, bot.value!)) : [],
)
const rows = computed(() => (bot.value ? telegramChatRows(bot.value, inventory.value, bindings.value) : []))
const draft = ref<TelegramBotConfig | null>(null)
const draftBaseline = ref(''),
  draftRevision = ref(0)
const dirty = computed(() => !!draft.value && JSON.stringify(draft.value) !== draftBaseline.value)
const query = ref(''),
  kind = ref(''),
  accessFilter = ref(''),
  page = ref(1)
const filtered = computed(() =>
  filterTelegramChats(rows.value, query.value, kind.value, '').filter(
    (row) => !accessFilter.value || (accessFilter.value === 'allowed' ? allowed(row) : !allowed(row)),
  ),
)
const pageCount = computed(() => Math.max(1, Math.ceil(filtered.value.length / 20)))
const visibleRows = computed(() => filtered.value.slice((page.value - 1) * 20, page.value * 20))
const activeRow = computed(() => rows.value.find((row) => row.id === selectedRoom.value))
const activityQuery = ref('')
const recent = computed(() =>
  (inventory.value?.recent || []).filter(
    (entry) =>
      !activityQuery.value ||
      [entry.text, entry.sender_id, entry.chat_id].join(' ').toLowerCase().includes(activityQuery.value.toLowerCase()),
  ),
)
const editor = ref<{ initial: TelegramBotConfig; existing: boolean; hasToken: boolean; revision: number } | null>(null)
const manual = ref(false)
const logs = ref<Array<{ seq: number; timestamp_ms: number; level: string; message: string }>>([])
const logsError = ref('')
const field =
  'h-10 w-full min-w-0 rounded-lg border border-border bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring'
let disposed = false,
  catalogGeneration = 0,
  aiGeneration = 0,
  epoch = 0,
  detailGeneration = 0
let catalogRead: AbortController | null = null,
  aiRead: AbortController | null = null
let scopeRead: AbortController | null = null,
  detailRead: AbortController | null = null
const message = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause))
const scopeKey = () => botId.value + ':' + (source.value || '')

function resetDraft() {
  draft.value = bot.value ? botConfig(bot.value) : null
  draftBaseline.value = JSON.stringify(draft.value)
  draftRevision.value = catalog.value.revision
  mutationError.value = ''
}
watch(bot, (next, previous) => {
  if (!draft.value || next?.id !== previous?.id || !dirty.value) resetDraft()
})
function stopScopeReads() {
  epoch++
  detailGeneration++
  scopeRead?.abort()
  detailRead?.abort()
  scopedLoading.value = false
}
async function refreshCatalog() {
  const generation = ++catalogGeneration
  catalogRead?.abort()
  const controller = new AbortController()
  catalogRead = controller
  loading.value = true
  try {
    const next = await telegramApi.list(controller.signal)
    if (disposed || generation !== catalogGeneration) return
    if (!next || !Array.isArray(next.bots) || !Number.isSafeInteger(next.revision))
      throw new Error('Invalid Telegram bot catalog')
    catalog.value = {
      ...next,
      bots: next.bots.map((entry) => ({ ...entry, chat_access: copyTelegramChatAccess(entry.chat_access) })),
    }
    loaded.value = true
    error.value = ''
  } catch (cause) {
    if (!disposed && generation === catalogGeneration && !controller.signal.aborted) error.value = message(cause)
  } finally {
    if (!disposed && generation === catalogGeneration) loading.value = false
  }
}
async function refreshAiBots() {
  const generation = ++aiGeneration
  aiRead?.abort()
  const controller = new AbortController()
  aiRead = controller
  aiLoading.value = true
  try {
    const result = await telegramApi.aiBots(controller.signal)
    if (disposed || generation !== aiGeneration) return
    if (!Array.isArray(result.bots)) throw new Error('Invalid AI bot catalog')
    aiBots.value = result.bots.filter((entry) => !entry.archived)
    aiError.value = ''
  } catch (cause) {
    if (!disposed && generation === aiGeneration && !controller.signal.aborted) aiError.value = message(cause)
  } finally {
    if (!disposed && generation === aiGeneration) aiLoading.value = false
  }
}
async function refreshScope() {
  if (!bot.value || disposed || scopedLoading.value || busy.value || accessEditing.value) return
  const current = epoch,
    key = scopeKey(),
    owner = bot.value.id,
    sourceId = source.value
  scopeRead?.abort()
  const controller = new AbortController()
  scopeRead = controller
  scopedLoading.value = true
  try {
    const results = await Promise.allSettled([
      telegramApi.inventory(owner, controller.signal),
      sourceId ? integrationApi.list(sourceId, controller.signal) : Promise.resolve(null),
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
  stopScopeReads()
  inventory.value = null
  snapshot.value = null
  detail.value = null
  selectedRoom.value = ''
  selectedBinding.value = ''
  scopeError.value = ''
  page.value = 1
  void refreshScope()
}
watch([botId, source], resetScope)
async function loadDetail(id: string) {
  const sourceId = source.value
  if (!sourceId || !id || !bindings.value.some((entry) => entry.id === id) || accessEditing.value) return
  const generation = ++detailGeneration,
    key = scopeKey()
  detailRead?.abort()
  const controller = new AbortController()
  detailRead = controller
  try {
    const result = await integrationApi.detail(sourceId, id, controller.signal)
    if (!disposed && generation === detailGeneration && key === scopeKey() && selectedBinding.value === id)
      detail.value = result
  } catch (cause) {
    if (!disposed && generation === detailGeneration && !controller.signal.aborted) scopeError.value = message(cause)
  }
}
async function chooseBot(id: string) {
  if (busy.value || editor.value || id === botId.value) return
  if (
    dirty.value &&
    !(await confirmAction({
      title: tx('discard'),
      confirmText: tx('confirm'),
      cancelText: tx('cancel'),
      variant: 'default',
    }))
  )
    return
  draft.value = null
  fallbackBot.value = id
  emit('selectBot', id)
}
function chooseView(value: TelegramView) {
  localView.value = value
  emit('selectView', value)
  if (value === 'activity') void loadLogs()
}
function allowed(row: TelegramChatRow) {
  return (
    !!draft.value?.enabled && !!draft.value.ai_bot_id && telegramChatAllowed(draft.value.chat_access, row.id, row.kind)
  )
}
function selected(id: string) {
  return draft.value?.chat_access.chat_ids.includes(id) === true
}
function selectChat(id: string, checked: boolean) {
  if (!draft.value || busy.value) return
  const ids = new Set(draft.value.chat_access.chat_ids)
  if (checked) ids.add(id)
  else ids.delete(id)
  draft.value.chat_access.chat_ids = [...ids]
}
function selectVisible(checked: boolean) {
  for (const row of visibleRows.value) selectChat(row.id, checked)
}
function chooseRoom(row: TelegramChatRow) {
  selectedRoom.value = row.id
  selectedBinding.value = row.bindings[0]?.id || ''
  detail.value = null
  if (selectedBinding.value) void loadDetail(selectedBinding.value)
}
function closeRoom() {
  selectedRoom.value = ''
  selectedBinding.value = ''
  detail.value = null
}
function chooseBinding(event: Event) {
  selectedBinding.value = (event.target as HTMLSelectElement).value
  void loadDetail(selectedBinding.value)
}
function openRecentRoom(id: string | null) {
  if (!id) return
  chooseView('chats')
  query.value = id
  const row = rows.value.find((entry) => entry.id === id)
  if (row) chooseRoom(row)
}
function closeBotEditor() {
  editor.value = null
  mutationError.value = ''
}
function closeManual() {
  manual.value = false
  mutationError.value = ''
}
function editBot(target?: TelegramBot) {
  if (busy.value || dirty.value) return
  mutationError.value = ''
  editor.value = {
    initial: target ? botConfig(target) : newBotConfig(),
    existing: target?.managed === true,
    hasToken: target?.credential_kind === 'vault' && target.has_token === true,
    revision: catalog.value.revision,
  }
  void refreshAiBots()
}
async function saveBot(config: TelegramBotConfig, token: string) {
  if (!editor.value || busy.value) return
  busy.value = true
  mutationError.value = ''
  stopScopeReads()
  try {
    await telegramApi.save(editor.value.revision, config, config.credential_kind === 'vault' ? token : undefined)
    if (disposed) return
    editor.value = null
    draft.value = null
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
async function saveAccess() {
  if (!draft.value || !draft.value.ai_bot_id || busy.value || !dirty.value) return
  const config = JSON.parse(JSON.stringify(draft.value)) as TelegramBotConfig
  busy.value = true
  mutationError.value = ''
  stopScopeReads()
  try {
    await telegramApi.save(draftRevision.value, config)
    if (disposed) return
    draft.value = null
    await refreshCatalog()
    toasts.push('success', tx('accessSaved'))
    emit('refresh')
  } catch (cause) {
    if (!disposed) {
      mutationError.value = message(cause)
      await refreshCatalog()
    }
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed) await refreshScope()
}
async function toggleBot(target: TelegramBot) {
  if (busy.value || dirty.value) return
  const config = botConfig(target)
  if (!config.ai_bot_id) {
    editBot(target)
    return
  }
  busy.value = true
  mutationError.value = ''
  stopScopeReads()
  try {
    await telegramApi.save(catalog.value.revision, { ...config, enabled: !config.enabled })
    if (!disposed) await refreshCatalog()
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed) await refreshScope()
}
async function removeBot(target: TelegramBot) {
  if (
    busy.value ||
    dirty.value ||
    !(await confirmAction({
      title: tx('delete'),
      description: tx('deleteHint'),
      confirmText: tx('confirm'),
      cancelText: tx('cancel'),
      variant: 'destructive',
    }))
  )
    return
  if (disposed) return
  busy.value = true
  mutationError.value = ''
  stopScopeReads()
  try {
    await telegramApi.remove(catalog.value.revision, target.id)
    if (disposed) return
    draft.value = null
    await refreshCatalog()
    const next = catalog.value.bots[0]?.id || ''
    fallbackBot.value = next
    emit('selectBot', next)
    emit('refresh')
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed) await refreshScope()
}
async function synchronize() {
  if (!bot.value?.enabled || syncing.value || busy.value) return
  const owner = bot.value.id,
    current = epoch
  syncing.value = true
  scopeError.value = ''
  try {
    const next = await telegramApi.sync(owner)
    if (!disposed && current === epoch && owner === botId.value) inventory.value = next
  } catch (cause) {
    if (!disposed && current === epoch) scopeError.value = message(cause)
  } finally {
    if (!disposed) syncing.value = false
  }
  if (!disposed && current === epoch) {
    await refreshCatalog()
    await refreshScope()
  }
}
async function verify() {
  if (!bot.value || verifying.value || busy.value) return
  const owner = bot.value.id,
    current = epoch
  verifying.value = true
  scopeError.value = ''
  try {
    await telegramApi.verify(owner)
    if (!disposed && current === epoch) await refreshCatalog()
  } catch (cause) {
    if (!disposed && current === epoch) scopeError.value = message(cause)
  } finally {
    if (!disposed) verifying.value = false
  }
}
function openManual() {
  if (busy.value) return
  mutationError.value = ''
  manual.value = true
}
async function addChat(input: { chatId: string; title: string; kind: 'direct' | 'group' | 'broadcast' }) {
  if (!bot.value || busy.value) return
  const owner = bot.value.id
  busy.value = true
  mutationError.value = ''
  stopScopeReads()
  try {
    const result = input.chatId.startsWith('@')
      ? await telegramApi.inspect(owner, input.chatId)
      : await telegramApi.addChat(owner, input.chatId, input.kind, input.title)
    if (disposed || owner !== botId.value) return
    manual.value = false
    selectedRoom.value = result.room.chat_id
    query.value = ''
    kind.value = ''
    accessFilter.value = ''
    if (draft.value?.chat_access.mode === 'whitelist') {
      draft.value.chat_access.chat_ids = [...new Set([...draft.value.chat_access.chat_ids, result.room.chat_id])]
      toasts.push('success', tx('manualAdded'))
    } else toasts.push('success', tx('manualAddedBlacklist'))
  } catch (cause) {
    if (!disposed) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed) await refreshScope()
}
async function control(binding: IntegrationBinding, action: IntegrationControlAction) {
  const sourceId = source.value,
    current = epoch
  if (!sourceId || busy.value || !bindings.value.some((entry) => entry.id === binding.id)) return
  busy.value = true
  mutationError.value = ''
  try {
    await integrationApi.control(sourceId, binding.id, { action, expected_revision: binding.revision })
  } catch (cause) {
    if (!disposed && current === epoch) mutationError.value = message(cause)
  } finally {
    if (!disposed) busy.value = false
  }
  if (!disposed && current === epoch) await refreshScope()
}
function accessChanged(binding: IntegrationBinding) {
  if (detail.value?.binding.id === binding.id) detail.value.binding = binding
  void refreshScope()
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
async function refreshAll() {
  await Promise.allSettled([refreshCatalog(), refreshAiBots()])
  await refreshScope()
}
watch([query, kind, accessFilter], () => {
  page.value = 1
})
watch(pageCount, (count) => {
  page.value = Math.min(page.value, count)
})
onMounted(() => {
  void refreshAll()
})
const timer = setInterval(() => {
  if (visible.value && !busy.value && !editor.value && !manual.value && !dirty.value && !accessEditing.value) {
    void refreshCatalog().then(refreshScope)
  }
}, 15000)
onBeforeUnmount(() => {
  disposed = true
  catalogGeneration++
  aiGeneration++
  stopScopeReads()
  catalogRead?.abort()
  aiRead?.abort()
  clearInterval(timer)
  editor.value = null
})
</script>

<template>
  <main class="mx-auto w-full max-w-[1400px] space-y-5 px-4 py-6 sm:px-7" data-testid="telegram-manager">
    <header class="flex flex-wrap items-center justify-between gap-4">
      <div class="flex min-w-0 items-center gap-3">
        <span class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-sky-500/10 text-sky-500"
          ><RiSendPlane2Line class="h-6 w-6" aria-hidden="true"
        /></span>
        <div class="min-w-0">
          <h2 class="text-xl font-semibold tracking-tight">{{ tx('title') }}</h2>
          <p class="mt-1 text-xs leading-relaxed text-muted-foreground">{{ tx('subtitle') }}</p>
        </div>
      </div>
      <div class="flex gap-2">
        <Button variant="outline" size="sm" :disabled="loading || busy" @click="refreshAll"
          ><RiRefreshLine class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />{{ tx('refresh') }}</Button
        >
        <Button
          size="sm"
          :disabled="busy || dirty || catalog.bots.length >= catalog.max_bots"
          data-testid="telegram-add-bot"
          @click="editBot()"
          ><RiAddLine class="h-4 w-4" />{{ tx('addBot') }}</Button
        >
      </div>
    </header>
    <p v-if="error" role="alert" class="break-words rounded-xl bg-destructive/10 p-4 text-sm text-destructive">
      {{ error }}
    </p>

    <section
      v-if="loaded && !catalog.bots.length"
      class="rounded-2xl border border-dashed border-border py-16 text-center"
    >
      <RiSendPlane2Line class="mx-auto mb-4 h-9 w-9 text-sky-500" />
      <h3 class="font-semibold">{{ tx('emptyBots') }}</h3>
      <p class="mx-auto mt-2 max-w-md px-4 text-sm text-muted-foreground">{{ tx('emptyBotsHint') }}</p>
      <Button class="mt-6" :disabled="busy" @click="editBot()"><RiAddLine class="h-4 w-4" />{{ tx('addBot') }}</Button>
    </section>

    <div v-else class="grid min-w-0 gap-5 xl:grid-cols-[220px_minmax(0,1fr)]">
      <aside class="min-w-0 xl:sticky xl:top-3 xl:self-start">
        <h3 class="mb-3 text-xs font-semibold text-muted-foreground">
          {{ tx('bots') }} <span class="ml-1 tabular-nums">{{ catalog.bots.length }}</span>
        </h3>
        <div class="flex gap-2 overflow-x-auto pb-2 xl:flex-col xl:overflow-visible" data-testid="telegram-bot-list">
          <button
            v-for="entry in catalog.bots"
            :key="entry.id"
            type="button"
            :data-telegram-bot="entry.id"
            :aria-pressed="entry.id === botId"
            class="w-52 shrink-0 rounded-xl border p-3 text-left transition-colors hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring xl:w-full"
            :class="entry.id === botId ? 'border-sky-500/50 bg-sky-500/5' : 'border-border/60 bg-card'"
            :disabled="busy"
            @click="chooseBot(entry.id)"
          >
            <div class="flex items-center justify-between gap-2">
              <span class="truncate text-sm font-semibold">{{ entry.name }}</span>
              <span
                class="h-2 w-2 shrink-0 rounded-full"
                :class="
                  !entry.enabled
                    ? 'bg-muted-foreground/40'
                    : entry.last_error || entry.routing_error
                      ? 'bg-amber-500'
                      : 'bg-emerald-500'
                "
              />
            </div>
            <p class="mt-1 truncate text-xs text-muted-foreground">
              {{ entry.connection.account ? '@' + entry.connection.account : tx('missingToken') }}
            </p>
            <p
              class="mt-3 truncate text-xs"
              :class="entry.ai_bot ? 'text-foreground/80' : 'text-amber-600 dark:text-amber-400'"
            >
              {{ entry.ai_bot?.name || tx('chooseAiBot') }}
            </p>
            <p class="mt-1 text-[11px] text-muted-foreground">
              {{ entry.enabled ? tx('enabled') : tx('disabled') }} · {{ entry.room_count }} {{ tx('knownRooms') }}
            </p>
          </button>
        </div>
      </aside>

      <section v-if="bot && draft" class="min-w-0 space-y-4">
        <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border/70 bg-card p-4">
          <div class="min-w-0">
            <h3 class="truncate text-lg font-semibold">{{ bot.name }}</h3>
            <p class="mt-1 text-xs text-muted-foreground">
              {{ bot.connection.account ? '@' + bot.connection.account : tx('missingToken') }} ·
              {{ bot.enabled ? tx('enabled') : tx('disabled') }}
            </p>
          </div>
          <div class="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              :disabled="busy || dirty"
              data-testid="telegram-edit-bot"
              @click="editBot(bot)"
              ><RiSettings3Line class="h-4 w-4" />{{ tx('edit') }}</Button
            >
            <Button
              :variant="bot.enabled ? 'outline' : 'default'"
              size="sm"
              :disabled="busy || dirty"
              data-testid="telegram-toggle-bot"
              @click="toggleBot(bot)"
              >{{ bot.enabled ? tx('disable') : tx('enable') }}</Button
            >
          </div>
        </div>

        <p
          v-if="mutationError && !editor && !manual"
          role="alert"
          class="break-words rounded-lg bg-destructive/10 p-3 text-sm text-destructive"
        >
          {{ mutationError }}
        </p>
        <p
          v-if="scopeError || bot.routing_error"
          role="alert"
          class="break-words rounded-lg bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-300"
        >
          {{ scopeError || bot.routing_error }}
        </p>
        <nav class="flex gap-1 border-b border-border pb-2" :aria-label="tx('title')">
          <Button
            v-for="tab in ['chats', 'activity', 'settings'] as const"
            :key="tab"
            size="sm"
            :variant="view === tab ? 'secondary' : 'ghost'"
            :data-telegram-tab="tab"
            :aria-pressed="view === tab"
            @click="chooseView(tab)"
            >{{ tx(tab) }}</Button
          >
        </nav>

        <template v-if="view === 'chats'">
          <section class="space-y-3 rounded-xl border border-border/70 bg-card p-4 sm:p-5">
            <div class="flex items-center justify-between gap-3">
              <label for="telegram-ai-bot" class="text-sm font-semibold">{{ tx('aiBot') }}</label>
              <button
                type="button"
                class="text-xs text-primary hover:underline disabled:opacity-50"
                :disabled="aiLoading"
                @click="refreshAiBots"
              >
                {{ tx('refreshAiBots') }}
              </button>
            </div>
            <select
              id="telegram-ai-bot"
              v-model="draft.ai_bot_id"
              :class="field + ' max-w-lg'"
              :disabled="busy"
              data-testid="telegram-ai-bot"
            >
              <option :value="null" disabled>{{ tx('chooseAiBot') }}</option>
              <option
                v-if="draft.ai_bot_id && !aiBots.some((entry) => entry.id === draft.ai_bot_id)"
                :value="draft.ai_bot_id"
                disabled
              >
                {{ bot.ai_bot?.name || tx('unavailableAiBot') }}
              </option>
              <option v-for="entry in aiBots" :key="entry.id" :value="entry.id">{{ entry.name }}</option>
            </select>
            <p class="text-xs leading-relaxed text-muted-foreground">
              {{ aiLoading ? tx('aiBotsLoading') : aiBots.length ? tx('aiBotHint') : tx('noAiBots') }}
            </p>
            <p v-if="aiError" role="alert" class="break-words text-xs text-destructive">{{ aiError }}</p>
          </section>

          <section
            class="space-y-5 rounded-xl border border-border/70 bg-card p-4 sm:p-5"
            data-testid="telegram-chat-policy"
          >
            <div>
              <h4 class="text-sm font-semibold">{{ tx('chatAccess') }}</h4>
              <div class="mt-3 flex flex-wrap gap-x-6 gap-y-3" :aria-label="tx('chatTypes')">
                <label class="flex cursor-pointer items-center gap-2 text-sm"
                  ><input
                    v-model="draft.chat_access.private_enabled"
                    type="checkbox"
                    class="h-4 w-4 accent-sky-600"
                    :disabled="busy"
                    data-testid="telegram-private-enabled"
                  />{{ kindLabel('direct') }}</label
                >
                <label class="flex cursor-pointer items-center gap-2 text-sm"
                  ><input
                    v-model="draft.chat_access.group_enabled"
                    type="checkbox"
                    class="h-4 w-4 accent-sky-600"
                    :disabled="busy"
                    data-testid="telegram-group-enabled"
                  />{{ kindLabel('group') }}</label
                >
                <label class="flex cursor-pointer items-center gap-2 text-sm"
                  ><input
                    v-model="draft.chat_access.channel_enabled"
                    type="checkbox"
                    class="h-4 w-4 accent-sky-600"
                    :disabled="busy"
                    data-testid="telegram-channel-enabled"
                  />{{ kindLabel('broadcast') }}</label
                >
              </div>
            </div>
            <div class="grid gap-3 sm:grid-cols-2">
              <label
                v-for="mode in ['whitelist', 'blacklist'] as const"
                :key="mode"
                class="flex cursor-pointer items-start gap-3 rounded-lg border p-3 transition-colors"
                :class="draft.chat_access.mode === mode ? 'border-sky-500/50 bg-sky-500/5' : 'border-border'"
              >
                <input
                  v-model="draft.chat_access.mode"
                  type="radio"
                  name="telegram-access-mode"
                  :value="mode"
                  :disabled="busy"
                  class="mt-0.5 h-4 w-4 shrink-0 accent-sky-600"
                  :data-testid="'telegram-mode-' + mode"
                />
                <span
                  ><span class="block text-sm font-medium">{{ tx(mode) }}</span
                  ><span class="mt-1 block text-xs leading-relaxed text-muted-foreground">{{
                    tx(mode === 'whitelist' ? 'whitelistHint' : 'blacklistHint')
                  }}</span></span
                >
              </label>
            </div>
          </section>

          <section class="min-w-0 space-y-3">
            <div class="flex flex-wrap items-center justify-between gap-3">
              <h4 class="text-sm font-semibold">
                {{ tx('knownRooms') }} <span class="ml-1 font-normal text-muted-foreground">{{ rows.length }}</span>
              </h4>
              <div class="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  :disabled="syncing || busy || !bot.enabled || !bot.has_token"
                  data-testid="telegram-sync"
                  @click="synchronize"
                  ><RiRefreshLine class="h-4 w-4" :class="syncing ? 'animate-spin' : ''" />{{
                    syncing ? tx('syncing') : tx('sync')
                  }}</Button
                >
                <Button variant="outline" size="sm" :disabled="busy" data-testid="telegram-add-chat" @click="openManual"
                  ><RiAddLine class="h-4 w-4" />{{ tx('addChat') }}</Button
                >
              </div>
            </div>
            <p class="text-xs leading-relaxed text-muted-foreground">{{ tx('discoverHint') }}</p>
            <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_130px_140px]">
              <input
                v-model="query"
                type="search"
                :class="field"
                :placeholder="tx('search')"
                :aria-label="tx('search')"
                data-testid="telegram-chat-search"
              />
              <select v-model="kind" :class="field" :aria-label="tx('allKinds')" data-testid="telegram-kind-filter">
                <option value="">{{ tx('allKinds') }}</option>
                <option v-for="item in ['direct', 'group', 'broadcast']" :key="item" :value="item">
                  {{ kindLabel(item) }}
                </option>
              </select>
              <select
                v-model="accessFilter"
                :class="field"
                :aria-label="tx('allStates')"
                data-testid="telegram-access-filter"
              >
                <option value="">{{ tx('allStates') }}</option>
                <option value="allowed">{{ tx('allowed') }}</option>
                <option value="blocked">{{ tx('blocked') }}</option>
              </select>
            </div>
            <div class="overflow-hidden rounded-xl border border-border/70 bg-card" data-testid="telegram-chat-list">
              <div
                class="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 bg-muted/25 px-4 py-2 text-xs"
              >
                <span>{{ tx('selectedChats') }} {{ draft.chat_access.chat_ids.length }}</span>
                <div class="flex gap-3">
                  <button
                    type="button"
                    class="text-primary hover:underline disabled:opacity-50"
                    :disabled="busy || !visibleRows.length"
                    @click="selectVisible(true)"
                  >
                    {{ tx('selectVisible') }}</button
                  ><button
                    type="button"
                    class="text-muted-foreground hover:text-foreground disabled:opacity-50"
                    :disabled="busy || !visibleRows.length"
                    @click="selectVisible(false)"
                  >
                    {{ tx('clearVisible') }}
                  </button>
                </div>
              </div>
              <div v-if="!visibleRows.length" class="px-5 py-10 text-center">
                <RiChat3Line class="mx-auto mb-3 h-7 w-7 text-muted-foreground/60" />
                <p class="text-sm">{{ tx('noRooms') }}</p>
                <p class="mx-auto mt-2 max-w-md text-xs leading-relaxed text-muted-foreground">
                  {{ tx('noRoomsHint') }}
                </p>
              </div>
              <ul v-else class="divide-y divide-border/60">
                <li
                  v-for="row in visibleRows"
                  :key="row.id"
                  class="flex items-center gap-3 px-4 py-3 hover:bg-muted/20"
                  :data-telegram-room="row.id"
                >
                  <input
                    type="checkbox"
                    :checked="selected(row.id)"
                    :disabled="busy"
                    :aria-label="
                      (draft.chat_access.mode === 'whitelist' ? tx('allowChat') : tx('blockChat')) + ': ' + row.title
                    "
                    class="h-4 w-4 shrink-0 cursor-pointer accent-sky-600"
                    :data-telegram-chat-check="row.id"
                    @change="selectChat(row.id, ($event.target as HTMLInputElement).checked)"
                  />
                  <div class="min-w-0 flex-1">
                    <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
                      <span class="truncate text-sm font-medium">{{ row.title }}</span
                      ><span class="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">{{
                        kindLabel(row.kind)
                      }}</span>
                    </div>
                    <p class="mt-1 truncate text-[11px] text-muted-foreground">
                      <span v-if="row.username">@{{ row.username }} · </span>{{ row.id }}
                    </p>
                  </div>
                  <span
                    class="hidden shrink-0 text-xs sm:inline"
                    :class="allowed(row) ? 'text-emerald-600 dark:text-emerald-400' : 'text-muted-foreground'"
                    >{{ allowed(row) ? tx('allowed') : tx('blocked') }}</span
                  >
                  <Button
                    variant="ghost"
                    size="sm"
                    :aria-label="tx('details') + ': ' + row.title"
                    :data-telegram-room-details="row.id"
                    @click="chooseRoom(row)"
                    ><RiArrowRightUpLine class="h-4 w-4" /><span class="hidden sm:inline">{{
                      tx('details')
                    }}</span></Button
                  >
                </li>
              </ul>
            </div>
            <div v-if="pageCount > 1" class="flex items-center justify-end gap-3 text-xs text-muted-foreground">
              <Button variant="outline" size="sm" :disabled="page === 1" @click="page--">{{ tx('previous') }}</Button
              ><span>{{ page }} / {{ pageCount }}</span
              ><Button variant="outline" size="sm" :disabled="page === pageCount" @click="page++">{{
                tx('next')
              }}</Button>
            </div>
          </section>

          <div
            class="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-background/95 p-3 backdrop-blur"
            data-testid="telegram-access-savebar"
          >
            <p class="text-xs" :class="dirty ? 'text-amber-700 dark:text-amber-400' : 'text-muted-foreground'">
              {{ dirty ? tx('unsaved') : tx('aiBotHint') }}
            </p>
            <div class="ml-auto flex gap-2">
              <Button v-if="dirty" variant="ghost" size="sm" :disabled="busy" @click="resetDraft">{{
                tx('cancel')
              }}</Button
              ><Button
                size="sm"
                :disabled="busy || !dirty || !draft.ai_bot_id"
                data-testid="telegram-save-access"
                @click="saveAccess"
                ><RiCheckLine class="h-4 w-4" />{{ busy ? tx('saving') : tx('saveAccess') }}</Button
              >
            </div>
          </div>

          <section v-if="activeRow" class="space-y-3 rounded-xl border border-border/70 p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0">
                <h4 class="break-words text-sm font-semibold">{{ activeRow.title }}</h4>
                <p class="mt-1 text-xs text-muted-foreground">{{ kindLabel(activeRow.kind) }} · {{ activeRow.id }}</p>
              </div>
              <Button variant="ghost" size="sm" @click="closeRoom">{{ tx('close') }}</Button>
            </div>
            <p v-if="activeRow.room?.description" class="text-xs leading-relaxed text-muted-foreground">
              {{ activeRow.room.description }}
            </p>
            <p
              v-if="activeRow.room?.last_text"
              class="whitespace-pre-wrap break-words rounded-lg bg-muted/40 p-3 text-sm"
            >
              {{ activeRow.room.last_text }}
            </p>
            <p v-if="!activeRow.bindings.length" class="text-xs text-muted-foreground">
              {{ allowed(activeRow) ? tx('waitingBinding') : tx('blocked') }}
            </p>
            <details v-if="activeRow.bindings.length" :open="!!detail">
              <summary class="cursor-pointer text-sm font-medium">{{ tx('advancedBinding') }}</summary>
              <div class="mt-3 space-y-3">
                <select
                  v-if="activeRow.bindings.length > 1"
                  :value="selectedBinding"
                  :class="field"
                  :aria-label="tx('advancedBinding')"
                  @change="chooseBinding"
                >
                  <option v-for="binding in activeRow.bindings" :key="binding.id" :value="binding.id">
                    {{ binding.spec.title }} · {{ stateLabel(binding.state) }}
                  </option>
                </select>
                <BindingDetail
                  v-if="detail && source"
                  :key="detail.binding.id"
                  :detail="detail"
                  :source="source"
                  :busy="busy"
                  :bot-enabled="bot.enabled"
                  @control="control"
                  @changed="accessChanged"
                  @editing="accessEditing = $event"
                  @open-session="emit('openSession', $event)"
                />
              </div>
            </details>
          </section>
        </template>

        <section v-else-if="view === 'activity'" class="space-y-4">
          <input
            v-model="activityQuery"
            type="search"
            :class="field"
            :placeholder="tx('messageFilter')"
            :aria-label="tx('messageFilter')"
          />
          <p v-if="!recent.length" class="py-10 text-center text-sm text-muted-foreground">{{ tx('none') }}</p>
          <div
            v-for="entry in recent"
            :key="entry.sequence"
            class="space-y-2 rounded-xl border border-border/70 bg-card p-4"
          >
            <div class="flex flex-wrap justify-between gap-2 text-[11px] text-muted-foreground">
              <span
                >{{ entry.chat_id }} <span v-if="entry.sender_id">· {{ entry.sender_id }}</span></span
              ><span>{{ date(entry.received_at_ms) }}</span>
            </div>
            <p class="whitespace-pre-wrap break-words text-sm">{{ entry.text }}</p>
            <button
              v-if="entry.chat_id"
              type="button"
              class="text-xs text-primary hover:underline"
              @click="openRecentRoom(entry.chat_id)"
            >
              {{ tx('details') }}
            </button>
          </div>
          <details
            class="rounded-xl border border-border/70 p-4"
            @toggle="($event.target as HTMLDetailsElement).open && loadLogs()"
          >
            <summary class="cursor-pointer text-sm font-medium">{{ tx('logs') }}</summary>
            <p v-if="logsError" role="alert" class="mt-3 break-words text-xs text-destructive">{{ logsError }}</p>
            <pre
              class="mt-3 max-h-80 overflow-auto whitespace-pre-wrap break-words text-[11px] text-muted-foreground"
              >{{
                logs.map((entry) => date(entry.timestamp_ms) + ' ' + entry.level + ' ' + entry.message).join('\n') ||
                tx('none')
              }}</pre
            >
          </details>
        </section>

        <section v-else class="space-y-4 rounded-xl border border-border/70 bg-card p-4 sm:p-5">
          <h4 class="text-sm font-semibold">{{ tx('settings') }}</h4>
          <dl class="grid gap-4 text-sm sm:grid-cols-2">
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('account') }}</dt>
              <dd class="mt-1">@{{ bot.connection.account || '—' }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('botId') }}</dt>
              <dd class="mt-1">{{ bot.verification?.bot_id || '—' }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('lastVerify') }}</dt>
              <dd class="mt-1">{{ date(bot.verification?.checked_at_ms) }}</dd>
            </div>
            <div>
              <dt class="text-xs text-muted-foreground">{{ tx('lastSync') }}</dt>
              <dd class="mt-1">{{ date(inventory?.last_poll_ms || bot.last_poll_ms) }}</dd>
            </div>
            <div class="sm:col-span-2">
              <dt class="text-xs text-muted-foreground">{{ tx('webhook') }}</dt>
              <dd
                class="mt-1 break-words"
                :class="bot.verification?.webhook_active ? 'text-amber-700 dark:text-amber-400' : ''"
              >
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
          <p v-if="bot.last_error" role="alert" class="break-words text-sm text-destructive">{{ bot.last_error }}</p>
          <div class="flex flex-wrap gap-2 border-t border-border pt-4">
            <Button
              variant="outline"
              size="sm"
              :disabled="busy || verifying"
              data-testid="telegram-verify"
              @click="verify"
              >{{ verifying ? tx('verifying') : tx('verify') }}</Button
            >
            <Button variant="outline" size="sm" :disabled="busy || dirty" @click="editBot(bot)">{{
              tx('edit')
            }}</Button>
            <Button variant="ghost" size="sm" @click="emit('manage')">{{ tx('fullSettings') }}</Button>
            <Button
              variant="ghost"
              size="sm"
              class="text-destructive"
              :disabled="busy || dirty"
              data-testid="telegram-remove-bot"
              @click="removeBot(bot)"
              >{{ tx('delete') }}</Button
            >
          </div>
        </section>
      </section>
    </div>

    <BotEditor
      v-if="editor"
      :key="editor.initial.id"
      :initial="editor.initial"
      :existing="editor.existing"
      :has-token="editor.hasToken"
      :busy="busy"
      :error="mutationError || aiError"
      :ai-bots="aiBots"
      :ai-bots-loading="aiLoading"
      @close="closeBotEditor"
      @save="saveBot"
      @refresh-ai-bots="refreshAiBots"
    />
    <ManualChatEditor v-if="manual" :busy="busy" :error="mutationError" @close="closeManual" @add="addChat" />
  </main>
</template>
