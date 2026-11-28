<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  RiCheckLine,
  RiFileCopyLine,
  RiGitBranchLine,
  RiHistoryLine,
  RiRefreshLine,
} from '@remixicon/vue'

import AcpInstallGuideCard from '@/components/acp/AcpInstallGuideCard.vue'
import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import Input from '@/components/ui/Input.vue'
import OptionPicker from '@/components/ui/OptionPicker.vue'
import type { PickerOption } from '@/components/ui/pickerOption.types'
import {
  listExternalAgents,
  manageNativeAgentSession,
  useExternalAgentCatalog,
  type NativeSessionAction,
  type NativeSessionStatus,
} from '@/features/sessions/api/nativeAgentSessions'
import { inferAcpAgentIdFromError, isMissingAcpCommandError } from '@/lib/acpInstallGuide'
import { copyTextToClipboard } from '@/lib/clipboard'
import { useChatStoreUnscoped } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import type { ExternalAgent } from '@/types/chat'

const props = defineProps<{ session: string; agent?: ExternalAgent }>()
const emit = defineEmits<{
  (event: 'close'): void
  (event: 'changed'): void
  (event: 'deleted'): void
  (event: 'forked', session: number): void
}>()
const { t } = useI18n()
const chat = useChatStoreUnscoped()
const catalog = useExternalAgentCatalog()
const toasts = useToastsStore()
const state = ref<NativeSessionStatus | null>(null)
const loading = ref(false)
const mutation = ref<NativeSessionAction['action'] | null>(null)
const title = ref('')
const directories = ref('')
const beforeMessageId = ref('')
const rewindMessageId = ref('')
const error = ref('')
const copiedId = ref(false)
let copyTimer: number | undefined
let read: AbortController | undefined

const resolvedAgentId = computed(
  () =>
    props.agent ||
    chat.getSessionById(props.session)?.external_agent ||
    inferAcpAgentIdFromError(error.value) ||
    'custom',
)
const resolvedAgentEntry = computed(() => catalog.value.find((entry) => entry.id === resolvedAgentId.value))
const showInstallGuide = computed(() => Boolean(error.value) && isMissingAcpCommandError(error.value))

async function retryRefresh() {
  void listExternalAgents().catch(() => {})
  await refresh()
}

const forkPointOptions = computed<PickerOption[]>(() =>
  (state.value?.fork_points ?? []).map((point) => ({
    value: String(point.message_id),
    label: point.label,
    description: `#${point.message_id}`,
    keywords: `${point.message_id} ${point.label}`,
  })),
)

const rewindPointOptions = computed<PickerOption[]>(() =>
  (state.value?.rewind_points ?? []).map((point) => ({
    value: String(point.message_id),
    label: point.label,
    description: `#${point.message_id}`,
    keywords: `${point.message_id} ${point.label}`,
  })),
)

async function copyNativeId() {
  const id = state.value?.session?.native_id
  if (!id) return
  const ok = await copyTextToClipboard(id)
  if (!ok) return
  copiedId.value = true
  toasts.push('success', String(t('common.copied')))
  window.clearTimeout(copyTimer)
  copyTimer = window.setTimeout(() => {
    copiedId.value = false
  }, 1800)
}

async function refresh() {
  read?.abort()
  const request = new AbortController()
  read = request
  loading.value = true
  error.value = ''
  try {
    const response = await manageNativeAgentSession(props.session, { action: 'status' }, request.signal)
    if (response.kind !== 'status' || request.signal.aborted) return
    state.value = response
    title.value = response.session?.title ?? ''
    directories.value = response.additional_directories.join('\n')
    if (!response.fork_points?.some((point) => String(point.message_id) === beforeMessageId.value)) {
      beforeMessageId.value = ''
    }
    if (!response.rewind_points?.some((point) => String(point.message_id) === rewindMessageId.value)) {
      rewindMessageId.value = ''
    }
  } catch (failure) {
    if (!request.signal.aborted) error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    if (read === request) loading.value = false
  }
}

async function manage(action: NativeSessionAction) {
  if (mutation.value || loading.value) return
  mutation.value = action.action
  error.value = ''
  try {
    const result = await manageNativeAgentSession(props.session, action)
    if (result.kind === 'forked') {
      toasts.push('success', String(t('chat.nativeAgent.forkedToast')))
      emit('forked', result.session_id)
      return
    }
    emit('changed')
    if (result.kind === 'deleted') {
      toasts.push('success', String(t('chat.nativeAgent.deletedToast')))
      emit('deleted')
      return
    }
    if (result.kind === 'title' && state.value?.session) {
      state.value.session.title = result.title
      title.value = result.title
      toasts.push('success', String(t('chat.nativeAgent.renamedToast')))
    } else if (result.kind === 'directories' && state.value) {
      state.value.additional_directories = result.directories
      directories.value = result.directories.join('\n')
      toasts.push('success', String(t('chat.nativeAgent.directoriesSavedToast')))
    } else {
      if (result.kind === 'closed') toasts.push('info', String(t('chat.nativeAgent.closedToast')))
      if (result.kind === 'rewound') toasts.push('success', String(t('chat.nativeAgent.rewoundToast')))
      await refresh()
    }
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    mutation.value = null
  }
}

function saveDirectories() {
  void manage({
    action: 'set_directories',
    directories: directories.value
      .split('\n')
      .map((path) => path.trim())
      .filter(Boolean),
  })
}

function forkBeforeMessage() {
  const id = Number(beforeMessageId.value)
  if (Number.isFinite(id) && id > 0) {
    void manage({ action: 'fork', before_message_id: id })
  }
}

function rewindBeforeMessage() {
  const id = Number(rewindMessageId.value)
  if (Number.isFinite(id) && id > 0) {
    void manage({ action: 'rewind', before_message_id: id })
  }
}

watch(
  () => props.session,
  () => {
    state.value = null
    beforeMessageId.value = ''
    rewindMessageId.value = ''
    void refresh()
  },
  { immediate: true },
)
onBeforeUnmount(() => {
  window.clearTimeout(copyTimer)
  read?.abort()
})
</script>

<template>
  <FormDialog
    :open="true"
    :title="t('chat.nativeAgent.manage')"
    :description="t('chat.nativeAgent.manageDescription')"
    @update:open="(value) => { if (!value && mutation === null) emit('close') }"
  >
    <div class="space-y-4">
      <AcpInstallGuideCard
        v-if="showInstallGuide"
        compact
        :agent-id="resolvedAgentId"
        :agent-name="resolvedAgentEntry?.name"
        :command="resolvedAgentEntry?.command"
        :raw-error="error"
        :show-retry="true"
        :retrying="loading"
        @configure="emit('close')"
        @retry="retryRefresh"
      />
      <p
        v-else-if="error"
        class="break-words rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        role="alert"
      >
        {{ error }}
      </p>
      <div v-else-if="loading && !state" role="status" class="space-y-2 py-2">
        <div class="h-14 animate-pulse rounded-lg bg-muted/40" />
        <div class="h-10 animate-pulse rounded-lg bg-muted/40" />
      </div>

      <template v-if="state">
        <div v-if="state.session" class="space-y-3.5">
          <div class="space-y-2 rounded-lg border border-border/70 bg-muted/20 p-3">
            <div class="flex items-center justify-between gap-2">
              <div class="flex flex-wrap items-center gap-1.5">
                <span
                  v-if="state.native_status === 'open'"
                  class="inline-flex items-center gap-1 rounded-full border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:text-emerald-300"
                >
                  <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" aria-hidden="true" />
                  {{ t('chat.nativeAgent.open') }}
                </span>
                <span
                  v-else-if="state.native_status === 'saved'"
                  class="rounded-full border border-border/70 bg-background px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                >
                  {{ t('chat.nativeAgent.saved') }}
                </span>
                <span
                  v-if="state.capabilities.continue_conversation === false"
                  class="rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300"
                >
                  {{ t('chat.nativeAgent.readOnlyHistory') }}
                </span>
              </div>
              <IconButton
                size="sm"
                variant="ghost"
                class="h-6 w-6 shrink-0"
                :tooltip="t('common.copy')"
                :aria-label="t('common.copy')"
                @click="copyNativeId"
              >
                <RiCheckLine v-if="copiedId" class="h-3.5 w-3.5 text-emerald-500" />
                <RiFileCopyLine v-else class="h-3.5 w-3.5" />
              </IconButton>
            </div>
            <p class="select-all break-all font-mono text-xs text-muted-foreground">{{ state.session.native_id }}</p>
          </div>

          <form
            v-if="state.capabilities.title"
            class="flex gap-2"
            @submit.prevent="manage({ action: 'set_title', title })"
          >
            <Input
              :model-value="title"
              :aria-label="t('chat.sidebar.sessionRow.placeholders.sessionTitle')"
              :placeholder="t('chat.sidebar.sessionRow.placeholders.sessionTitle')"
              :disabled="mutation !== null"
              @update:model-value="(value) => { title = String(value) }"
            />
            <Button type="submit" variant="outline" :disabled="mutation !== null || !title.trim()">
              {{ t('common.rename') }}
            </Button>
          </form>

          <div class="flex flex-wrap gap-2">
            <Button
              v-if="state.capabilities.fork"
              size="sm"
              variant="outline"
              :disabled="mutation !== null"
              @click="manage({ action: 'fork' })"
            >
              <RiGitBranchLine class="mr-1.5 h-3.5 w-3.5" />
              {{ t('chat.nativeAgent.fork') }}
            </Button>
            <Button
              v-if="state.capabilities.close"
              size="sm"
              variant="outline"
              :disabled="mutation !== null"
              @click="manage({ action: 'close' })"
            >
              {{ t('chat.nativeAgent.closeSession') }}
            </Button>
            <ConfirmPopover
              v-if="state.capabilities.delete"
              :title="t('chat.nativeAgent.delete')"
              :description="t('chat.nativeAgent.deleteDescription')"
              :confirm-text="t('common.delete')"
              variant="destructive"
              :busy="mutation !== null"
              @confirm="manage({ action: 'delete' })"
            >
              <Button size="sm" variant="ghost" class="text-destructive hover:bg-destructive/10" :disabled="mutation !== null">
                {{ t('chat.nativeAgent.delete') }}
              </Button>
            </ConfirmPopover>
          </div>

          <form
            v-if="state.capabilities.fork_before_message && forkPointOptions.length"
            class="space-y-1.5 rounded-lg border border-border/60 bg-muted/15 p-3"
            @submit.prevent="forkBeforeMessage"
          >
            <span class="block text-xs font-medium text-muted-foreground">{{ t('chat.nativeAgent.forkBeforeMessage') }}</span>
            <div class="flex flex-wrap items-center gap-2">
              <div class="min-w-0 flex-1">
                <OptionPicker
                  v-model="beforeMessageId"
                  :options="forkPointOptions"
                  :title="t('chat.nativeAgent.chooseForkMessage')"
                  :placeholder="t('chat.nativeAgent.chooseForkMessage')"
                  :include-empty="false"
                  :disabled="loading || mutation !== null"
                  size="sm"
                  trigger-class="w-full"
                />
              </div>
              <Button type="submit" size="sm" variant="outline" :disabled="loading || mutation !== null || !beforeMessageId">
                <RiGitBranchLine class="mr-1 h-3.5 w-3.5" />
                {{ t('chat.nativeAgent.forkBeforeMessage') }}
              </Button>
            </div>
          </form>

          <div
            v-if="state.capabilities.rewind_before_message && rewindPointOptions.length"
            class="space-y-1.5 rounded-lg border border-border/60 bg-muted/15 p-3"
          >
            <span class="block text-xs font-medium text-muted-foreground">{{ t('chat.nativeAgent.rewindBeforeMessage') }}</span>
            <div class="flex flex-wrap items-center gap-2">
              <div class="min-w-0 flex-1">
                <OptionPicker
                  v-model="rewindMessageId"
                  :options="rewindPointOptions"
                  :title="t('chat.nativeAgent.chooseRewindMessage')"
                  :placeholder="t('chat.nativeAgent.chooseRewindMessage')"
                  :include-empty="false"
                  :disabled="loading || mutation !== null"
                  size="sm"
                  trigger-class="w-full"
                />
              </div>
              <ConfirmPopover
                :title="t('chat.nativeAgent.rewindBeforeMessage')"
                :description="t('chat.nativeAgent.nativeRewindDescription')"
                :confirm-text="t('chat.nativeAgent.rewindBeforeMessage')"
                variant="destructive"
                :busy="mutation !== null"
                @confirm="rewindBeforeMessage"
              >
                <Button size="sm" variant="outline" :disabled="loading || mutation !== null || !rewindMessageId">
                  <RiHistoryLine class="mr-1 h-3.5 w-3.5" />
                  {{ t('chat.nativeAgent.rewindBeforeMessage') }}
                </Button>
              </ConfirmPopover>
            </div>
          </div>
          <p v-else-if="state.capabilities.rewind_before_message" class="text-xs text-muted-foreground">
            {{ t('chat.nativeAgent.reopenForRewind') }}
          </p>
        </div>

        <p v-else class="rounded-lg border border-dashed border-border/70 bg-muted/15 px-3 py-4 text-center text-sm text-muted-foreground">
          {{ t('chat.nativeAgent.notStarted') }}
        </p>

        <form
          v-if="state.capabilities.additional_directories || state.additional_directories.length"
          class="space-y-2 rounded-lg border border-border/60 bg-muted/15 p-3"
          @submit.prevent="saveDirectories"
        >
          <label class="block space-y-1.5 text-xs font-medium text-muted-foreground">
            <span>{{ t('chat.nativeAgent.directories') }}</span>
            <textarea
              v-model="directories"
              class="w-full rounded-md border border-input bg-background p-2 font-mono text-xs text-foreground outline-none focus:border-ring disabled:opacity-60"
              :placeholder="t('chat.nativeAgent.directoriesPlaceholder')"
              :disabled="mutation !== null"
              rows="3"
            />
          </label>
          <Button type="submit" variant="outline" size="sm" :disabled="mutation !== null">{{ t('common.save') }}</Button>
        </form>
      </template>

      <div class="flex justify-end gap-2 border-t border-border/50 pt-3">
        <Button
          v-if="state || error"
          size="sm"
          variant="ghost"
          :disabled="loading || mutation !== null"
          @click="refresh"
        >
          <RiRefreshLine class="mr-1.5 h-3.5 w-3.5" :class="loading ? 'animate-spin' : ''" />
          {{ t('common.refresh') }}
        </Button>
        <Button size="sm" variant="ghost" :disabled="mutation !== null" @click="emit('close')">
          {{ t('common.close') }}
        </Button>
      </div>
    </div>
  </FormDialog>
</template>
