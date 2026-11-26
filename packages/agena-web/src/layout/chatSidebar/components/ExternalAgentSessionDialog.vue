<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import Button from '@/components/ui/Button.vue'
import ConfirmPopover from '@/components/ui/ConfirmPopover.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Input from '@/components/ui/Input.vue'
import {
  manageNativeAgentSession,
  type NativeSessionAction,
  type NativeSessionStatus,
} from '@/features/sessions/api/nativeAgentSessions'

const props = defineProps<{ session: string }>()
const emit = defineEmits<{
  (event: 'close'): void
  (event: 'changed'): void
  (event: 'deleted'): void
  (event: 'forked', session: number): void
}>()
const { t } = useI18n()
const state = ref<NativeSessionStatus | null>(null)
const loading = ref(false)
const mutation = ref<NativeSessionAction['action'] | null>(null)
const title = ref('')
const directories = ref('')
const beforeMessageId = ref<number | null>(null)
const rewindMessageId = ref<number | null>(null)
const error = ref('')
let read: AbortController | undefined

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
    if (!response.fork_points?.some((point) => point.message_id === beforeMessageId.value)) {
      beforeMessageId.value = null
    }
    if (!response.rewind_points?.some((point) => point.message_id === rewindMessageId.value)) {
      rewindMessageId.value = null
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
      emit('forked', result.session_id)
      return
    }
    emit('changed')
    if (result.kind === 'deleted') {
      emit('deleted')
      return
    }
    if (result.kind === 'title' && state.value?.session) {
      state.value.session.title = result.title
      title.value = result.title
    } else if (result.kind === 'directories' && state.value) {
      state.value.additional_directories = result.directories
      directories.value = result.directories.join('\n')
    } else {
      await refresh()
    }
  } catch (failure) {
    error.value = failure instanceof Error ? failure.message : String(failure)
  } finally {
    mutation.value = null
  }
}

function saveDirectories() {
  void manage({ action: 'set_directories', directories: directories.value.split('\n').map((path) => path.trim()).filter(Boolean) })
}

function forkBeforeMessage() {
  if (beforeMessageId.value !== null) {
    void manage({ action: 'fork', before_message_id: beforeMessageId.value })
  }
}

function rewindBeforeMessage() {
  if (rewindMessageId.value !== null) {
    void manage({ action: 'rewind', before_message_id: rewindMessageId.value })
  }
}

watch(() => props.session, () => {
  state.value = null
  beforeMessageId.value = null
  rewindMessageId.value = null
  void refresh()
}, { immediate: true })
onBeforeUnmount(() => read?.abort())
</script>

<template>
  <FormDialog
    :open="true"
    :title="t('chat.nativeAgent.manage')"
    :description="t('chat.nativeAgent.manageDescription')"
    @update:open="(value) => { if (!value && mutation === null) emit('close') }"
  >
    <div class="space-y-4">
      <p v-if="error" class="text-sm text-destructive" role="alert">{{ error }}</p>
      <p v-else-if="loading && !state" class="text-sm text-muted-foreground" role="status">{{ t('common.loading') }}</p>
      <template v-if="state">
        <div v-if="state.session" class="space-y-2">
          <div class="text-xs text-muted-foreground">
            <p class="break-all font-mono">{{ state.session.native_id }}</p>
            <p v-if="state.native_status === 'open'">{{ t('chat.nativeAgent.open') }}</p>
            <p v-else-if="state.native_status === 'saved'">{{ t('chat.nativeAgent.saved') }}</p>
            <p v-if="state.capabilities.continue_conversation === false">{{ t('chat.nativeAgent.readOnlyHistory') }}</p>
          </div>
          <form v-if="state.capabilities.title" class="flex gap-2" @submit.prevent="manage({ action: 'set_title', title })">
            <Input
              :model-value="title"
              :aria-label="t('chat.sidebar.sessionRow.placeholders.sessionTitle')"
              :placeholder="t('chat.sidebar.sessionRow.placeholders.sessionTitle')"
              :disabled="mutation !== null"
              @update:model-value="(value) => { title = String(value) }"
            />
            <Button type="submit" variant="outline" :disabled="mutation !== null || !title.trim()">{{ t('common.rename') }}</Button>
          </form>
          <div class="flex flex-wrap gap-2">
            <Button v-if="state.capabilities.fork" variant="outline" :disabled="mutation !== null" @click="manage({ action: 'fork' })">
              {{ t('chat.nativeAgent.fork') }}
            </Button>
            <Button v-if="state.capabilities.close" variant="outline" :disabled="mutation !== null" @click="manage({ action: 'close' })">
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
              <Button variant="ghost" class="text-destructive" :disabled="mutation !== null">
                {{ t('chat.nativeAgent.delete') }}
              </Button>
            </ConfirmPopover>
          </div>
          <form
            v-if="state.capabilities.fork_before_message && state.fork_points?.length"
            class="flex flex-wrap gap-2"
            @submit.prevent="forkBeforeMessage"
          >
            <select
              v-model="beforeMessageId"
              :aria-label="t('chat.nativeAgent.chooseForkMessage')"
              :disabled="loading || mutation !== null"
              class="min-w-0 flex-1 rounded-md border border-input bg-background px-2 py-1 text-sm"
            >
              <option :value="null" disabled>{{ t('chat.nativeAgent.chooseForkMessage') }}</option>
              <option v-for="point in state.fork_points" :key="point.message_id" :value="point.message_id">
                {{ point.label }}
              </option>
            </select>
            <Button type="submit" variant="outline" :disabled="loading || mutation !== null || beforeMessageId === null">
              {{ t('chat.nativeAgent.forkBeforeMessage') }}
            </Button>
          </form>
          <div v-if="state.capabilities.rewind_before_message && state.rewind_points?.length" class="flex flex-wrap gap-2">
            <select
              v-model="rewindMessageId"
              :aria-label="t('chat.nativeAgent.chooseRewindMessage')"
              :disabled="loading || mutation !== null"
              class="min-w-0 flex-1 rounded-md border border-input bg-background px-2 py-1 text-sm"
            >
              <option :value="null" disabled>{{ t('chat.nativeAgent.chooseRewindMessage') }}</option>
              <option v-for="point in state.rewind_points" :key="point.message_id" :value="point.message_id">
                {{ point.label }}
              </option>
            </select>
            <ConfirmPopover
              :title="t('chat.nativeAgent.rewindBeforeMessage')"
              :description="t('chat.nativeAgent.nativeRewindDescription')"
              :confirm-text="t('chat.nativeAgent.rewindBeforeMessage')"
              variant="destructive"
              :busy="mutation !== null"
              @confirm="rewindBeforeMessage"
            >
              <Button variant="outline" :disabled="loading || mutation !== null || rewindMessageId === null">
                {{ t('chat.nativeAgent.rewindBeforeMessage') }}
              </Button>
            </ConfirmPopover>
          </div>
          <p v-else-if="state.capabilities.rewind_before_message" class="text-sm text-muted-foreground">
            {{ t('chat.nativeAgent.reopenForRewind') }}
          </p>
        </div>
        <p v-else class="text-sm text-muted-foreground">{{ t('chat.nativeAgent.notStarted') }}</p>
        <form
          v-if="state.capabilities.additional_directories || state.additional_directories.length"
          class="space-y-2"
          @submit.prevent="saveDirectories"
        >
          <label class="block space-y-1.5 text-sm">
            <span>{{ t('chat.nativeAgent.directories') }}</span>
            <textarea
              v-model="directories"
              class="w-full rounded-md border border-input bg-background p-2 font-mono text-sm"
              :placeholder="t('chat.nativeAgent.directoriesPlaceholder')"
              :disabled="mutation !== null"
              rows="3"
            />
          </label>
          <Button type="submit" variant="outline" size="sm" :disabled="mutation !== null">{{ t('common.save') }}</Button>
        </form>
      </template>
      <div class="flex justify-end gap-2">
        <Button
          v-if="state || error"
          variant="ghost"
          :disabled="loading || mutation !== null"
          @click="refresh"
        >{{ t('common.refresh') }}</Button>
        <Button variant="ghost" :disabled="mutation !== null" @click="emit('close')">{{ t('common.close') }}</Button>
      </div>
    </div>
  </FormDialog>
</template>
