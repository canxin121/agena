<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RiCloseLine, RiFolderAddLine, RiRefreshLine } from '@remixicon/vue'
import SessionChip from './SessionChip.vue'
import Button from '@/components/ui/Button.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { useToastsStore } from '@/stores/toasts'
import {
  fetchSessionWorkingDirectories,
  registerSessionWorkingDirectory,
  removeSessionWorkingDirectory,
  type SessionWorkingDirectoriesView,
  type SessionWorkingDirectoryAccess,
} from '@/lib/workingDirectoriesApi'

const props = defineProps<{
  sessionId: string
  directory: string
  open: boolean
  revision?: number
}>()

const emit = defineEmits<{ (event: 'update:open', value: boolean): void }>()

const { t } = useI18n()
const toasts = useToastsStore()

const view = ref<SessionWorkingDirectoriesView | null>(null)
const loading = ref(false)
const mutating = ref(false)
const errorMessage = ref('')
const draftPath = ref('')
const draftAccess = ref<SessionWorkingDirectoryAccess>('readonly')

let abortController: AbortController | null = null
let requestSerial = 0

const primaryRoot = computed(() => view.value?.primary || props.directory)
const entries = computed(() => view.value?.entries ?? [])
const chipLabel = computed(() => (entries.value.length > 0 ? String(entries.value.length) : ''))
const chipHint = computed(() =>
  entries.value.length > 0
    ? String(t('chat.workingDirectories.countHint', { count: entries.value.length }))
    : String(t('chat.workingDirectories.title')),
)

function accessLabel(access: SessionWorkingDirectoryAccess): string {
  if (access === 'readonly') return String(t('chat.workingDirectories.accessReadonly'))
  if (access === 'readwrite') return String(t('chat.workingDirectories.accessReadwrite'))
  return String(t('chat.workingDirectories.accessInherit'))
}

async function refresh() {
  const sid = String(props.sessionId || '').trim()
  if (!sid) {
    view.value = null
    return
  }
  abortController?.abort()
  const controller = new AbortController()
  abortController = controller
  const serial = ++requestSerial
  loading.value = true
  errorMessage.value = ''
  try {
    const next = await fetchSessionWorkingDirectories(sid, controller.signal)
    if (serial !== requestSerial || props.sessionId !== sid) return
    view.value = next
  } catch (error) {
    if (controller.signal.aborted || serial !== requestSerial) return
    errorMessage.value = error instanceof Error ? error.message : String(t('chat.workingDirectories.loadFailed'))
  } finally {
    if (serial === requestSerial) {
      loading.value = false
      if (abortController === controller) abortController = null
    }
  }
}

watch(
  () => [props.sessionId, props.revision ?? 0] as const,
  ([sid], [prevSid]) => {
    if (sid !== prevSid) {
      view.value = null
      errorMessage.value = ''
      draftPath.value = ''
    }
    void refresh()
  },
  { immediate: true },
)

watch(
  () => props.open,
  (isOpen) => {
    if (isOpen) void refresh()
  },
)

onBeforeUnmount(() => {
  abortController?.abort()
  abortController = null
})

async function handleAdd() {
  const sid = String(props.sessionId || '').trim()
  const targetPath = draftPath.value.trim()
  if (!sid || !targetPath || mutating.value) return
  mutating.value = true
  errorMessage.value = ''
  try {
    const result = await registerSessionWorkingDirectory(sid, targetPath, draftAccess.value)
    if (props.sessionId !== sid) return
    view.value = {
      primary: primaryRoot.value || null,
      directories: result.directories,
      entries: result.entries,
    }
    draftPath.value = ''
    const mode = accessLabel(result.access)
    if (result.added) {
      toasts.push('success', String(t('chat.workingDirectories.added', { path: result.directory, access: mode })))
    } else if (result.updated) {
      toasts.push('success', String(t('chat.workingDirectories.updated', { path: result.directory, access: mode })))
    } else {
      toasts.push('info', String(t('chat.workingDirectories.alreadyCovered', { path: result.directory })))
    }
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error)
  } finally {
    mutating.value = false
  }
}

async function handleAccessChange(path: string, nextAccess: SessionWorkingDirectoryAccess) {
  const sid = String(props.sessionId || '').trim()
  if (!sid || mutating.value) return
  mutating.value = true
  errorMessage.value = ''
  try {
    const result = await registerSessionWorkingDirectory(sid, path, nextAccess)
    if (props.sessionId !== sid) return
    view.value = {
      primary: primaryRoot.value || null,
      directories: result.directories,
      entries: result.entries,
    }
    toasts.push(
      'success',
      String(t('chat.workingDirectories.updated', { path: result.directory, access: accessLabel(result.access) })),
    )
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error)
    void refresh()
  } finally {
    mutating.value = false
  }
}

async function handleRemove(path: string) {
  const sid = String(props.sessionId || '').trim()
  if (!sid || mutating.value) return
  mutating.value = true
  errorMessage.value = ''
  try {
    const result = await removeSessionWorkingDirectory(sid, path)
    if (props.sessionId !== sid) return
    view.value = {
      primary: primaryRoot.value || null,
      directories: result.directories,
      entries: result.entries,
    }
    if (result.removed) {
      toasts.push('success', String(t('chat.workingDirectories.removed', { path: result.directory })))
    } else {
      toasts.push('info', String(t('chat.workingDirectories.notMounted', { path: result.directory })))
    }
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error)
  } finally {
    mutating.value = false
  }
}
</script>

<template>
  <SessionChip
    :title="String(t('chat.workingDirectories.title'))"
    :label="chipLabel"
    :hint="chipHint"
    :open="open"
    :busy="loading || mutating"
    panel-class="w-[min(92vw,36rem)]"
    data-testid="working-directories-chip"
    @update:open="emit('update:open', $event)"
  >
    <template #icon>
      <RiFolderAddLine class="h-3 w-3 shrink-0" />
    </template>
    <template #actions>
      <IconButton
        class="h-7 w-7"
        :tooltip="String(t('chat.workingDirectories.refresh'))"
        :aria-label="String(t('chat.workingDirectories.refresh'))"
        :disabled="loading || mutating"
        @click="refresh"
      >
        <RiRefreshLine class="h-3.5 w-3.5" />
      </IconButton>
    </template>
    <div class="flex min-w-0 flex-col gap-2 py-0.5" :aria-busy="loading || mutating">
      <p v-if="errorMessage" role="alert" class="text-xs text-destructive [overflow-wrap:anywhere]">
        {{ errorMessage }}
      </p>

      <div
        v-if="primaryRoot"
        class="flex min-w-0 items-center justify-between gap-2 rounded-md border border-border/60 bg-muted/30 px-2 py-1.5"
      >
        <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="primaryRoot">{{ primaryRoot }}</span>
        <span class="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
          {{ t('chat.workingDirectories.primary') }}
        </span>
      </div>

      <div v-if="entries.length" class="flex flex-col gap-1">
        <div
          v-for="entry in entries"
          :key="entry.path"
          class="flex min-w-0 items-center gap-2 rounded-md border border-border/50 px-2 py-1"
        >
          <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="entry.path">{{ entry.path }}</span>
          <select
            :value="entry.access"
            :disabled="mutating"
            :aria-label="String(t('chat.workingDirectories.accessLabel'))"
            class="h-7 shrink-0 rounded border border-border/70 bg-background px-1.5 text-[11px] text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            @change="
              handleAccessChange(
                entry.path,
                ($event.target as HTMLSelectElement).value as SessionWorkingDirectoryAccess,
              )
            "
          >
            <option value="readonly">{{ t('chat.workingDirectories.accessReadonly') }}</option>
            <option value="readwrite">{{ t('chat.workingDirectories.accessReadwrite') }}</option>
            <option value="inherit">{{ t('chat.workingDirectories.accessInherit') }}</option>
          </select>
          <IconButton
            class="h-7 w-7 shrink-0 text-muted-foreground hover:text-destructive"
            :tooltip="String(t('chat.workingDirectories.remove'))"
            :aria-label="String(t('chat.workingDirectories.remove'))"
            :disabled="mutating"
            @click="handleRemove(entry.path)"
          >
            <RiCloseLine class="h-3.5 w-3.5" />
          </IconButton>
        </div>
      </div>
      <p v-else-if="!loading" class="text-xs text-muted-foreground">
        {{ t('chat.workingDirectories.empty') }}
      </p>

      <form
        class="mt-0.5 flex flex-wrap items-center gap-1.5 border-t border-border/50 pt-2"
        @submit.prevent="handleAdd"
      >
        <input
          v-model="draftPath"
          type="text"
          autocomplete="off"
          spellcheck="false"
          :disabled="mutating"
          :placeholder="String(t('chat.workingDirectories.pathPlaceholder'))"
          :aria-label="String(t('chat.workingDirectories.pathPlaceholder'))"
          class="h-8 min-w-[10rem] flex-1 rounded-md border border-border/70 bg-background px-2 font-mono text-xs text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <select
          v-model="draftAccess"
          :disabled="mutating"
          :aria-label="String(t('chat.workingDirectories.accessLabel'))"
          class="h-8 shrink-0 rounded-md border border-border/70 bg-background px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <option value="readonly">{{ t('chat.workingDirectories.accessReadonly') }}</option>
          <option value="readwrite">{{ t('chat.workingDirectories.accessReadwrite') }}</option>
          <option value="inherit">{{ t('chat.workingDirectories.accessInherit') }}</option>
        </select>
        <Button type="submit" size="sm" :disabled="mutating || !draftPath.trim()">
          {{ t('chat.workingDirectories.add') }}
        </Button>
      </form>
    </div>
  </SessionChip>
</template>
