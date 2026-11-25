<script setup lang="ts">
import { computed, onScopeDispose, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { refDebounced } from '@vueuse/core'
import {
  RiArrowRightSLine,
  RiFile2Line,
  RiHardDrive2Line,
  RiRefreshLine,
  RiShieldCheckLine,
  RiTimeLine,
} from '@remixicon/vue'

import Button from '@/components/ui/Button.vue'
import Dialog from '@/components/ui/Dialog.vue'
import PaginationControls from '@/components/ui/PaginationControls.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import Skeleton from '@/components/ui/Skeleton.vue'
import SettingsListDetail from './workbench/SettingsListDetail.vue'
import { settingsText as st } from '@/i18n/settingsText'
import { confirmAction } from '@/lib/appConfirm'
import {
  confirmSessionStorage,
  listSessionStorage,
  previewSessionStorage,
  readSessionStorage,
  readStorageOperation,
  reclaimDatabaseStorage,
  totalStorage,
  type SessionStorageDetail,
  type SessionStorageList,
  type SessionStorageSummary,
  type StorageFile,
  type StoragePreview,
  type StorageProtection,
  type StorageSelection,
} from '@/features/sessionStorage/api'

const props = defineProps<{ targetSessionId?: string; targetBotId?: string }>()
const emit = defineEmits<{ select: [id: string] }>()
const { t, locale } = useI18n()
const PAGE_SIZE = 20
const FILE_PAGE_SIZE = 15

const inventory = shallowRef<SessionStorageList>()
const detail = shallowRef<SessionStorageDetail>()
const scope = ref<'' | 'project' | 'bot' | 'deleted'>('')
const bot = ref('')
const search = ref('')
const searchTerm = refDebounced(search, 220)
const page = ref(0)
const filePage = ref(0)
const filesOpen = ref(false)
const selected = ref('')
const detailOpen = ref(false)
const listDetail = ref<InstanceType<typeof SettingsListDetail> | null>(null)
const inventoryLoading = ref(false)
const detailLoading = ref(false)
const detailRefreshing = ref(false)
const inventoryError = ref('')
const detailError = ref('')
const databaseNotice = ref('')
const compacting = ref(false)

const dialog = ref<'cleanup' | 'purge' | 'file' | null>(null)
const chosenFile = shallowRef<StorageFile>()
const cleanCache = ref(true)
const trimRecordings = ref(false)
const preview = shallowRef<StoragePreview>()
const previewError = ref('')
const previewLoading = ref(false)
const submitting = ref(false)

let inventoryRequest: AbortController | undefined
let detailRequest: AbortController | undefined
let operationRequest: AbortController | undefined
let poll: ReturnType<typeof setTimeout> | undefined

const numberFormat = computed(() => new Intl.NumberFormat(locale.value, { maximumFractionDigits: 1 }))
function formatBytes(value: number) {
  if (value < 1024) return `${numberFormat.value.format(value)} B`
  const index = Math.min(4, Math.floor(Math.log(value) / Math.log(1024)))
  return `${numberFormat.value.format(value / 1024 ** index)} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][index]}`
}

const rows = computed(() => {
  const bytes = detail.value?.bytes
  if (!bytes) return []
  return [
    { key: 'conversation', label: st('Conversation and run history'), bytes: bytes.conversation },
    { key: 'recordings', label: st('Tool output and recordings'), bytes: bytes.recordings },
    { key: 'media', label: st('Media stored in the database'), bytes: bytes.media },
    { key: 'files', label: st('Uploads and artifacts'), bytes: bytes.files },
    { key: 'cache', label: st('Session caches'), bytes: bytes.cache },
    { key: 'plugins', label: st('Session plugin data'), bytes: bytes.plugins },
  ]
    .filter((row) => row.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes)
})
const breakdownTotal = computed(() => (detail.value ? totalStorage(detail.value.bytes) : 0))
const selectedBot = computed(() => inventory.value?.bots.find((item) => item.id === bot.value))
const sharedRows = computed(() => {
  const usage = inventory.value?.shared
  if (!usage) return []
  return [
    { label: st('Workspace uploads and artifacts'), bytes: usage.workspace_files },
    { label: st('Workspace and legacy web caches'), bytes: usage.web },
    { label: st('Memory and search indexes'), bytes: usage.memory },
    { label: st('Server and preview logs'), bytes: usage.logs },
    { label: st('Shared plugin data'), bytes: usage.plugins },
  ]
    .filter((row) => row.bytes > 0)
    .sort((a, b) => b.bytes - a.bytes)
})
const operation = computed(() => detail.value?.operation)
const blocked = computed(() => Boolean(detail.value?.busy_reason || operation.value?.state === 'running'))
const canClean = computed(() =>
  Boolean(detail.value && !blocked.value && (detail.value.bytes.cache || detail.value.removable_recording_bytes)),
)
const selection = computed<StorageSelection>(() => {
  if (dialog.value === 'purge') return { purge: true }
  if (dialog.value === 'file') return { files: chosenFile.value ? [chosenFile.value.id] : [] }
  return { cache: cleanCache.value, recordings: trimRecordings.value }
})
const canConfirm = computed(() =>
  Boolean(
    !submitting.value &&
    !previewLoading.value &&
    preview.value &&
    !preview.value.protection &&
    (dialog.value === 'purge' || preview.value.file_count || preview.value.database_bytes),
  ),
)

function reason(protection?: StorageProtection | null) {
  switch (protection) {
    case 'running':
      return st('This session still has work in progress.')
    case 'reading':
      return st('An export or attachment read is in progress.')
    case 'shared':
      return st('Referenced by another conversation.')
    case 'referenced':
      return st('Referenced by saved history or a queued message.')
    case 'unmanaged':
      return st('Keep with the session.')
    default:
      return ''
  }
}

function context(session: SessionStorageSummary) {
  if (session.deleted) return st('Deleted session')
  if (!session.bot_id) return session.workspace_name || st('Global session')
  return session.surface && session.surface !== 'local'
    ? st('{name} · Channel', { name: session.bot_name || 'AI Bot' })
    : st('{name} · Local', { name: session.bot_name || 'AI Bot' })
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}

function pick(id: string, openDetail = true, navigate = true) {
  selected.value = id
  detailOpen.value = openDetail
  if (navigate) emit('select', id)
}

async function loadInventory(refresh = false) {
  inventoryRequest?.abort()
  const request = new AbortController()
  inventoryRequest = request
  inventoryLoading.value = true
  inventoryError.value = ''
  try {
    const result = await listSessionStorage(
      {
        offset: page.value * PAGE_SIZE,
        limit: PAGE_SIZE,
        scope: scope.value || undefined,
        bot: bot.value || undefined,
        search: searchTerm.value.trim() || undefined,
        refresh: refresh || undefined,
      },
      request.signal,
    )
    if (request.signal.aborted) return
    inventory.value = result
    if (page.value > 0 && !result.items.length) {
      page.value -= 1
      return
    }
    if (!selected.value && result.items[0]) pick(String(result.items[0].session_id), false, false)
  } catch (error) {
    if (!request.signal.aborted) inventoryError.value = errorText(error)
  } finally {
    if (inventoryRequest === request) inventoryLoading.value = false
  }
}

function stopPolling() {
  clearTimeout(poll)
  operationRequest?.abort()
}

function pollOperation(id: string, delay = 800) {
  poll = setTimeout(async () => {
    const request = new AbortController()
    operationRequest = request
    try {
      const result = await readStorageOperation(id, request.signal)
      if (request.signal.aborted || !detail.value || selected.value !== id) return
      detail.value = { ...detail.value, operation: result }
      if (result?.state === 'running') {
        pollOperation(id)
      } else {
        void loadDetail(id, true)
        void loadInventory(true)
      }
    } catch {
      if (!request.signal.aborted && selected.value === id) pollOperation(id, 2000)
    }
  }, delay)
}

async function loadDetail(id: string, background = false) {
  detailRequest?.abort()
  stopPolling()
  if (!id) {
    detail.value = undefined
    detailLoading.value = false
    detailRefreshing.value = false
    return
  }
  const request = new AbortController()
  detailRequest = request
  detailLoading.value = !background
  detailRefreshing.value = true
  detailError.value = ''
  try {
    const result = await readSessionStorage(id, filePage.value * FILE_PAGE_SIZE, FILE_PAGE_SIZE, request.signal)
    if (request.signal.aborted || selected.value !== id) return
    if (filePage.value > 0 && !result.files.length) {
      filePage.value = Math.max(0, Math.ceil(result.file_count / FILE_PAGE_SIZE) - 1)
      return
    }
    const wasRunning = detail.value?.operation?.state === 'running'
    detail.value = result
    if (result.operation?.state === 'running') pollOperation(id)
    else if (wasRunning) void loadInventory(true)
  } catch (error) {
    if (!request.signal.aborted) {
      detailError.value = errorText(error)
      if (detail.value?.operation?.state === 'running') pollOperation(id, 2000)
    }
  } finally {
    if (detailRequest === request) {
      detailLoading.value = false
      detailRefreshing.value = false
    }
  }
}

function resetList() {
  page.value = 0
  selected.value = ''
  detailOpen.value = false
}

function selectScope(event: Event) {
  scope.value = (event.target as HTMLSelectElement).value as typeof scope.value
  if (scope.value === 'project') bot.value = ''
  resetList()
}

function selectBot(event: Event) {
  bot.value = (event.target as HTMLSelectElement).value
  resetList()
}

function selectPage(next: number) {
  selected.value = ''
  detailOpen.value = false
  page.value = next - 1
}

watch(
  () => [props.targetSessionId, props.targetBotId] as const,
  ([id, botId], previous) => {
    if (botId !== previous?.[1]) {
      bot.value = botId || ''
      scope.value = ''
      resetList()
    }
    if (id && id !== selected.value) pick(id, true, false)
  },
  { immediate: true },
)
watch(searchTerm, resetList)
watch(
  [scope, bot, searchTerm, page],
  () => {
    listDetail.value?.scrollListToTop()
    void loadInventory()
  },
  { immediate: true },
)
watch(
  selected,
  (id) => {
    dialog.value = null
    detail.value = undefined
    filePage.value = 0
    filesOpen.value = false
    void loadDetail(id)
  },
  { immediate: true },
)
watch(filePage, () => {
  if (detail.value) void loadDetail(selected.value, true)
})

function closeDialog() {
  if (!submitting.value) dialog.value = null
}

function setFilesOpen(event: Event) {
  filesOpen.value = (event.currentTarget as HTMLDetailsElement).open
}

function open(mode: 'cleanup' | 'purge' | 'file', file?: StorageFile) {
  if (!detail.value || blocked.value) return
  chosenFile.value = file
  cleanCache.value = detail.value.bytes.cache > 0
  trimRecordings.value = false
  dialog.value = mode
}

watch([dialog, selection, selected], async ([mode, choices, id], _old, dispose) => {
  preview.value = undefined
  previewError.value = ''
  previewLoading.value = false
  if (!mode || !id) return
  if (mode === 'cleanup' && !choices.cache && !choices.recordings) return
  const controller = new AbortController()
  dispose(() => controller.abort())
  previewLoading.value = true
  try {
    const result = await previewSessionStorage(id, choices, controller.signal)
    if (!controller.signal.aborted) preview.value = result
  } catch (error) {
    if (!controller.signal.aborted) previewError.value = errorText(error)
  } finally {
    if (!controller.signal.aborted) previewLoading.value = false
  }
})

async function runCleanup() {
  if (!canConfirm.value || !preview.value || !detail.value) return
  const id = selected.value
  submitting.value = true
  previewError.value = ''
  try {
    const result = await confirmSessionStorage(id, selection.value, preview.value.token)
    if (selected.value !== id || !detail.value) return
    detail.value = { ...detail.value, operation: result }
    dialog.value = null
    void loadDetail(id, true)
  } catch (error) {
    if (selected.value === id) previewError.value = errorText(error)
  } finally {
    submitting.value = false
  }
}

function refresh() {
  void loadInventory(true)
  if (selected.value) void loadDetail(selected.value, true)
}

async function compactDatabase() {
  if (compacting.value || !inventory.value?.database.can_reclaim) return
  if (
    !(await confirmAction({
      title: st('Reclaim database space'),
      description: st('This compacts the shared database. Finish running sessions and exports first.'),
      confirmText: st('Reclaim space'),
      cancelText: t('common.cancel'),
      variant: 'default',
    }))
  )
    return
  compacting.value = true
  databaseNotice.value = ''
  try {
    const result = await reclaimDatabaseStorage()
    if (inventory.value) inventory.value = { ...inventory.value, database: result.database }
    databaseNotice.value = st('Database files reduced by {size}.', { size: formatBytes(result.released_bytes) })
  } catch (error) {
    databaseNotice.value = errorText(error)
  } finally {
    compacting.value = false
  }
}

onScopeDispose(() => {
  inventoryRequest?.abort()
  detailRequest?.abort()
  stopPolling()
})
</script>

<template>
  <section
    class="storage-workbench mx-auto min-w-0 max-w-6xl space-y-4"
    :aria-label="st('Storage management')"
    data-testid="session-storage"
  >
    <div class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2">
      <div>
        <h2 class="text-lg font-semibold tracking-tight">{{ st('Storage management') }}</h2>
        <p class="mt-1 text-sm text-muted-foreground">
          {{ st('Review stored data and recover space by conversation.') }}
        </p>
      </div>
      <Button variant="outline" size="sm" :disabled="inventoryLoading" @click="refresh">
        <RiRefreshLine class="size-4" :class="{ 'animate-spin': inventoryLoading }" aria-hidden="true" />
        {{ t('common.refresh') }}
      </Button>
    </div>

    <p
      v-if="inventoryError"
      role="alert"
      class="rounded-md border border-destructive/30 px-3 py-2 text-sm text-destructive"
    >
      {{ inventoryError }}
    </p>

    <SettingsListDetail
      ref="listDetail"
      v-model:detail-open="detailOpen"
      :list-label="st('Sessions')"
      :detail-label="st('Data stored by this session')"
      :back-label="st('Sessions')"
      :list-busy="inventoryLoading"
      :detail-loading="detailLoading"
      list-scroll-class="settings-list"
    >
      <template #list-header>
        <div class="flex flex-wrap items-baseline justify-between gap-1.5">
          <h3 data-settings-list-heading tabindex="-1" class="text-sm font-medium outline-none">
            {{ st('Sessions') }}
          </h3>
          <span v-if="inventory" class="text-xs tabular-nums text-muted-foreground">{{
            formatBytes(inventory.bytes)
          }}</span>
        </div>
        <div class="grid min-w-0 gap-1.5">
          <SearchInput
            v-model="search"
            :show-search-button="false"
            :placeholder="st('Find a session')"
            :input-aria-label="st('Find a session')"
            :input-title="st('Find a session')"
            :clear-aria-label="t('common.clear')"
            :clear-title="t('common.clear')"
          />
          <select
            :value="scope"
            @change="selectScope"
            :aria-label="st('Session scope')"
            class="w-full rounded-md border border-input bg-background px-2 py-1.5 text-sm"
          >
            <option value="">{{ st('All sessions') }}</option>
            <option value="project">{{ st('Project sessions') }}</option>
            <option value="bot">{{ st('AI Bot sessions') }}</option>
            <option value="deleted">{{ st('Deleted sessions') }}</option>
          </select>
          <select
            v-if="scope !== 'project' && inventory?.bots.length"
            :value="bot"
            @change="selectBot"
            :aria-label="st('AI Bot')"
            class="w-full rounded-md border border-input bg-background px-2 py-1.5 text-sm"
          >
            <option value="">{{ st('All AI Bots') }}</option>
            <option v-for="item in inventory.bots" :key="item.id" :value="item.id">{{ item.name }}</option>
          </select>
        </div>
        <div v-if="selectedBot" class="rounded-md bg-muted/35 px-2.5 py-2 text-xs">
          <div class="flex items-baseline justify-between gap-2">
            <span class="min-w-0 truncate font-medium">{{ selectedBot.name }}</span>
            <strong class="shrink-0 tabular-nums">{{
              formatBytes(
                selectedBot.session_bytes +
                  selectedBot.profile_bytes +
                  selectedBot.channel_bytes +
                  selectedBot.channel_files,
              )
            }}</strong>
          </div>
          <div class="mt-0.5 text-muted-foreground">
            {{
              st('{size} across {count} sessions', {
                size: formatBytes(selectedBot.session_bytes),
                count: selectedBot.session_count,
              })
            }}
          </div>
          <div class="text-muted-foreground">
            {{
              st('Profile {profile} · Channels {channels}', {
                profile: formatBytes(selectedBot.profile_bytes),
                channels: formatBytes(selectedBot.channel_bytes + selectedBot.channel_files),
              })
            }}
          </div>
        </div>
      </template>
      <template #list>
        <template v-if="inventoryLoading && !inventory">
          <Skeleton v-for="row in 4" :key="row" class="h-11 rounded-md" />
        </template>
        <template v-else-if="inventory?.items.length">
          <button
            v-for="item in inventory.items"
            :key="item.session_id"
            type="button"
            :aria-pressed="selected === String(item.session_id)"
            :data-keyboard-item="item.session_id"
            :data-keyboard-label="item.title"
            :data-keyboard-selected="selected === String(item.session_id)"
            class="flex w-full items-start justify-between gap-2 rounded-md border border-transparent px-2.5 py-2 text-start hover:bg-accent/60 focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring aria-pressed:border-border/70 aria-pressed:bg-accent"
            @click="pick(String(item.session_id))"
          >
            <span class="min-w-0">
              <span class="block truncate text-sm" :title="item.title">{{ item.title }}</span>
              <span class="block truncate text-xs text-muted-foreground">{{ context(item) }}</span>
            </span>
            <span class="shrink-0 whitespace-nowrap pt-0.5 text-xs tabular-nums text-muted-foreground">{{
              formatBytes(totalStorage(item.bytes))
            }}</span>
          </button>
        </template>
        <p v-else-if="!inventoryError && !inventoryLoading" class="px-1 py-4 text-sm text-muted-foreground">
          {{ st('No sessions in this view.') }}
        </p>
      </template>
      <template #list-footer>
        <PaginationControls
          v-if="inventory && inventory.total > PAGE_SIZE"
          :page="page + 1"
          :total-pages="Math.ceil(inventory.total / PAGE_SIZE)"
          :disabled="inventoryLoading"
          :prev-label="t('common.previousPage')"
          :next-label="t('common.nextPage')"
          :page-input-label="t('common.currentPage')"
          size="md"
          class="shrink-0 justify-center border-t border-border/60 py-1.5"
          @update:page="selectPage"
        />
      </template>
      <template #detail>
        <div class="settings-viewport-scroll p-3 sm:p-5">
          <p v-if="detailError" role="alert" class="mb-2 break-words text-sm text-destructive">{{ detailError }}</p>
          <div v-if="detailLoading" class="space-y-3">
            <Skeleton class="h-8 w-2/3" />
            <Skeleton class="h-28 w-full" />
          </div>
          <template v-else-if="detail">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="min-w-0 flex-1">
                <h3 data-settings-detail-heading tabindex="-1" class="break-words text-base font-semibold outline-none">
                  {{ detail.title }}
                </h3>
                <p class="mt-0.5 text-xs text-muted-foreground">{{ context(detail) }}</p>
              </div>
              <Button size="sm" :disabled="!canClean" @click="open('cleanup')">{{ st('Preview cleanup') }}</Button>
            </div>

            <div class="mt-5 flex flex-wrap items-baseline justify-between gap-2">
              <span class="text-sm text-muted-foreground">{{ st('Data stored by this session') }}</span>
              <strong class="text-2xl font-semibold tracking-tight tabular-nums">{{
                formatBytes(breakdownTotal)
              }}</strong>
            </div>
            <div v-if="rows.length" class="mt-2 flex h-2 overflow-hidden rounded-full bg-muted/55" aria-hidden="true">
              <div
                v-for="row in rows"
                :key="row.key"
                :data-bucket="row.key"
                class="storage-segment"
                :style="{ flexGrow: row.bytes }"
              />
            </div>
            <dl class="mt-2 divide-y divide-border/55">
              <div v-for="row in rows" :key="row.key" class="flex items-center justify-between gap-2 py-2.5 text-sm">
                <dt class="flex min-w-0 items-center gap-2">
                  <span
                    class="storage-segment size-[7px] shrink-0 rounded-[2px]"
                    :data-bucket="row.key"
                    aria-hidden="true"
                  />{{ row.label }}
                </dt>
                <dd class="shrink-0 whitespace-nowrap tabular-nums">{{ formatBytes(row.bytes) }}</dd>
              </div>
            </dl>
            <div
              v-if="detail.shared_history || detail.busy_reason"
              class="mt-2 flex items-start gap-2 rounded-md bg-muted/40 px-2.5 py-2 text-xs text-muted-foreground"
            >
              <RiShieldCheckLine class="mt-px size-4 shrink-0" aria-hidden="true" />
              <div>
                <p v-if="detail.shared_history">{{ st('Other conversations still reference this history.') }}</p>
                <p v-if="detail.busy_reason">{{ reason(detail.busy_reason) }}</p>
              </div>
            </div>
            <div v-if="operation" role="status" class="mt-3 rounded-md border border-border/70 px-3 py-2 text-sm">
              <p v-if="operation.state === 'running'" class="flex items-center gap-2">
                <RiTimeLine class="size-4" aria-hidden="true" />{{ st('Cleaning up this session…') }}
              </p>
              <p v-else-if="operation.state === 'failed'" class="text-destructive">
                {{ st('Cleanup stopped. Review the remaining items and try again.') }}
              </p>
              <p v-else class="font-medium">
                {{ operation.purge ? st('Session deletion completed') : st('Cleanup completed') }}
              </p>
              <p class="mt-0.5 text-xs text-muted-foreground">
                {{
                  st('Files removed {files} · Database data removed {database}', {
                    files: formatBytes(operation.file_bytes_removed),
                    database: formatBytes(operation.database_bytes_removed),
                  })
                }}
              </p>
              <p v-if="operation.files_skipped" class="mt-0.5 text-xs text-muted-foreground">
                {{ st('{count} files skipped', { count: operation.files_skipped }) }}
              </p>
            </div>

            <details
              v-if="detail.file_count"
              class="group mt-4 border-t border-border/60 pt-2.5"
              :open="filesOpen"
              @toggle="setFilesOpen"
            >
              <summary
                class="flex cursor-pointer list-none items-center justify-between gap-2 rounded-md py-1.5 text-sm font-medium"
              >
                <span>{{ st('Managed files ({count})', { count: detail.file_count }) }}</span>
                <RiArrowRightSLine class="size-4 text-muted-foreground group-open:rotate-90" aria-hidden="true" />
              </summary>
              <ul class="divide-y divide-border/50">
                <li v-for="file in detail.files" :key="file.id" class="flex flex-wrap items-center gap-2 py-2 text-sm">
                  <RiFile2Line class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <div class="min-w-[5rem] flex-1">
                    <p class="truncate" :title="file.name">{{ file.name }}</p>
                    <p v-if="file.protection" class="text-xs text-muted-foreground">
                      {{
                        file.kind === 'cache' && file.protection === 'unmanaged'
                          ? st('Use cache cleanup to remove this file.')
                          : reason(file.protection)
                      }}
                    </p>
                  </div>
                  <span class="whitespace-nowrap text-xs tabular-nums text-muted-foreground">{{
                    formatBytes(file.bytes)
                  }}</span>
                  <Button
                    v-if="!file.protection && !blocked"
                    size="sm"
                    variant="ghost"
                    :aria-label="st('Remove {name}', { name: file.name })"
                    @click="open('file', file)"
                    >{{ st('Remove file') }}</Button
                  >
                </li>
              </ul>
              <PaginationControls
                v-if="detail.file_count > FILE_PAGE_SIZE"
                :page="filePage + 1"
                :total-pages="Math.ceil(detail.file_count / FILE_PAGE_SIZE)"
                :disabled="detailRefreshing"
                :prev-label="t('common.previousPage')"
                :next-label="t('common.nextPage')"
                :page-input-label="t('common.currentPage')"
                size="md"
                class="mt-2 flex justify-center border-t border-border/60 pt-2"
                @update:page="filePage = $event - 1"
              />
            </details>
            <div class="mt-4 flex justify-end border-t border-border/60 pt-2">
              <Button variant="ghost" size="sm" class="text-destructive" :disabled="blocked" @click="open('purge')">
                {{ detail.deleted ? st('Reclaim deleted session…') : st('Permanently delete session…') }}
              </Button>
            </div>
          </template>
          <p v-else-if="!detailError" class="py-8 text-center text-sm text-muted-foreground">
            {{ st('Choose a session to see its storage.') }}
          </p>
        </div>
      </template>
    </SettingsListDetail>

    <details v-if="inventory" class="rounded-lg border border-border/65 bg-card px-3 py-2">
      <summary class="flex cursor-pointer list-none items-center gap-2 py-0.5 text-sm font-medium">
        <RiHardDrive2Line class="size-4 text-muted-foreground" aria-hidden="true" />{{ st('Shared database') }}
        <span class="ms-auto text-xs font-normal tabular-nums text-muted-foreground">{{
          formatBytes(inventory.database.file_bytes + inventory.database.wal_bytes)
        }}</span>
      </summary>
      <div class="mt-2 flex flex-wrap items-center justify-between gap-2 border-t border-border/60 pt-2">
        <p class="text-xs text-muted-foreground">
          {{ st('{size} available for SQLite to reuse.', { size: formatBytes(inventory.database.reusable_bytes) }) }}
        </p>
        <Button
          size="sm"
          variant="outline"
          :disabled="compacting || !inventory.database.can_reclaim"
          @click="compactDatabase"
        >
          {{ compacting ? st('Reclaiming database space…') : st('Reclaim database space') }}
        </Button>
      </div>
      <p v-if="databaseNotice" role="status" class="mt-1 text-xs">{{ databaseNotice }}</p>
    </details>

    <details v-if="sharedRows.length" class="group rounded-lg border border-border/65 bg-card px-3 py-2">
      <summary class="flex cursor-pointer list-none items-center gap-2 py-0.5 text-sm font-medium">
        <RiArrowRightSLine class="size-4 text-muted-foreground group-open:rotate-90" aria-hidden="true" />
        {{ st('Shared runtime files') }}
        <span class="ms-auto text-xs font-normal tabular-nums text-muted-foreground">{{
          formatBytes(sharedRows.reduce((sum, row) => sum + row.bytes, 0))
        }}</span>
      </summary>
      <dl class="mt-2 divide-y divide-border/60 border-t border-border/60">
        <div v-for="row in sharedRows" :key="row.label" class="flex justify-between gap-2 py-2 text-sm">
          <dt>{{ row.label }}</dt>
          <dd class="shrink-0 tabular-nums">{{ formatBytes(row.bytes) }}</dd>
        </div>
      </dl>
      <p class="mt-0.5 pb-1 text-xs text-muted-foreground">
        {{ st('These files belong to the instance or a workspace.') }}
      </p>
    </details>

    <Dialog
      :open="dialog !== null"
      :title="
        dialog === 'purge'
          ? st('Permanently delete session')
          : dialog === 'file'
            ? st('Remove file')
            : st('Clean session storage')
      "
      :description="detail?.title"
      max-width="max-w-md"
      @close="closeDialog"
    >
      <div v-if="dialog === 'cleanup'" class="divide-y divide-border/60">
        <label v-if="detail?.bytes.cache" class="flex items-start gap-2.5 py-2.5 text-sm">
          <input v-model="cleanCache" type="checkbox" class="mt-0.5 size-4 accent-primary" :disabled="submitting" />
          <span
            >{{ st('Rebuildable caches')
            }}<span class="block text-xs text-muted-foreground">{{
              st('Agena rebuilds them when needed.')
            }}</span></span
          >
          <span class="ms-auto shrink-0 tabular-nums">{{ formatBytes(detail.bytes.cache) }}</span>
        </label>
        <label v-if="detail?.removable_recording_bytes" class="flex items-start gap-2.5 py-2.5 text-sm">
          <input v-model="trimRecordings" type="checkbox" class="mt-0.5 size-4 accent-primary" :disabled="submitting" />
          <span
            >{{ st('Older command recordings')
            }}<span class="block text-xs text-muted-foreground">{{
              st('Keep recent output and tool results.')
            }}</span></span
          >
          <span class="ms-auto shrink-0 tabular-nums">{{ formatBytes(detail.removable_recording_bytes) }}</span>
        </label>
      </div>
      <p v-else-if="dialog === 'purge'" class="text-sm">
        {{
          st(
            'Remove the conversation and reclaim its eligible data. Shared history stays available to other conversations.',
          )
        }}
      </p>
      <template v-else-if="dialog === 'file' && chosenFile">
        <p class="break-all text-sm font-medium">{{ chosenFile.name }}</p>
        <p v-if="chosenFile.kind === 'upload'" class="mt-1 text-xs text-muted-foreground">
          {{ st('Drafts that use this file will need a new upload.') }}
        </p>
      </template>
      <p v-if="previewLoading" role="status" class="mt-3 text-sm text-muted-foreground">
        {{ st('Calculating the cleanup…') }}
      </p>
      <p v-if="previewError" role="alert" class="mt-2 break-words text-sm text-destructive">{{ previewError }}</p>
      <p v-if="preview?.protection" role="status" class="mt-2 text-sm">{{ reason(preview.protection) }}</p>
      <div v-if="preview && !preview.protection" class="mt-3 rounded-md bg-muted/40 p-2.5">
        <dl class="space-y-1.5 text-sm">
          <div class="flex flex-wrap justify-between gap-x-2">
            <dt>{{ st('Files to remove') }}</dt>
            <dd class="tabular-nums">{{ formatBytes(preview.file_bytes) }}</dd>
          </div>
          <div class="flex flex-wrap justify-between gap-x-2">
            <dt>{{ st('Database data to remove') }}</dt>
            <dd class="tabular-nums">{{ formatBytes(preview.database_bytes) }}</dd>
          </div>
        </dl>
        <p v-if="preview.database_bytes" class="mt-1 text-xs text-muted-foreground">
          {{ st('SQLite can reuse freed pages. Reclaim database space below to shrink its files.') }}
        </p>
        <p v-if="preview.keeps_shared_history" class="mt-1 text-xs text-muted-foreground">
          {{ st('Other conversations still reference this history.') }}
        </p>
      </div>
      <template #footer>
        <Button class="ms-auto" variant="outline" :disabled="submitting" @click="dialog = null">{{
          t('common.cancel')
        }}</Button>
        <Button
          :variant="dialog === 'purge' || dialog === 'file' ? 'destructive' : 'default'"
          :disabled="!canConfirm"
          @click="runCleanup"
        >
          {{
            dialog === 'purge'
              ? st('Delete permanently')
              : dialog === 'file'
                ? st('Remove file')
                : st('Clean selected items')
          }}
        </Button>
      </template>
    </Dialog>
  </section>
</template>

<style scoped>
@reference '../../style.css';

.storage-workbench {
  container: settings-workbench / inline-size;
}

.storage-segment {
  background: var(--storage-color);
}

.storage-segment[data-bucket='files'] {
  --storage-color: oklch(var(--primary) / 0.82);
}

.storage-segment[data-bucket='recordings'] {
  --storage-color: light-dark(#5f7982, #90b2bc);
}

.storage-segment[data-bucket='conversation'] {
  --storage-color: light-dark(#806a97, #b4a0c5);
}

.storage-segment[data-bucket='media'] {
  --storage-color: light-dark(#687ca6, #a4b5d9);
}

.storage-segment[data-bucket='plugins'] {
  --storage-color: light-dark(#a17b4c, #d4af7f);
}

.storage-segment[data-bucket='cache'] {
  --storage-color: light-dark(#607a5f, #8ab186);
}
</style>
