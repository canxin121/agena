<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import Composer from './Composer.vue'
import FormDialog from '@/components/ui/FormDialog.vue'
import Button from '@/components/ui/Button.vue'
import { useChatStore } from '@/stores/chat'
import { useToastsStore } from '@/stores/toasts'
import { getQueuedMessageForEdit } from '@/stores/chat/api'
import { useChatAttachments, type AttachedFile } from '@/pages/chat/useChatAttachments'
import { getComposerInput, type ComposerExpose, type ComposerSegment } from '@/pages/chat/composerInput'
import { queuedAttachmentIsResource, queuedEditorDocument, queuedEditorState } from '@/pages/chat/queuedEditor'
import type { JsonValue } from '@/types/json'

const props = defineProps<{ sessionId: string; executionId: string }>()
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const chat = useChatStore()
const composer = ref<ComposerExpose | null>(null)
const draft = ref('')
const loading = ref(true)
const loaded = ref(false)
const saving = ref(false)
const replacing = ref(false)
const retainedUrls = ref<string[]>([])
const error = ref('')
const revision = ref(0)
const replaceId = ref('')
const replaceInput = ref<HTMLInputElement | null>(null)
let alive = true
onBeforeUnmount(() => {
  alive = false
})
const attachments = useChatAttachments({
  toasts: useToastsStore(),
  composerRef: composer,
  retainedUrls: () => retainedUrls.value,
})
const { attachedFiles, pendingAttachments, attachmentsBusy, handlePaste, handleDrop, handleFileInputChange } =
  attachments
const canSave = computed(
  () =>
    loaded.value &&
    !saving.value &&
    !replacing.value &&
    !attachmentsBusy.value &&
    (Boolean(draft.value.trim()) || attachedFiles.value.length > 0),
)

onMounted(async () => {
  try {
    const snapshot = await getQueuedMessageForEdit(props.sessionId, props.executionId)
    if (!alive) return
    revision.value = snapshot.revision
    const state = queuedEditorState(snapshot.document)
    attachedFiles.value = state.files
    draft.value = state.text
    loaded.value = true
    loading.value = false
    await nextTick()
    composer.value?.restoreSegments?.(state.segments)
    getComposerInput(composer.value)?.focus()
  } catch (cause) {
    if (alive) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    if (alive) loading.value = false
  }
})
function remove(id: string) {
  attachments.removeAttachment(id)
  composer.value?.removeAttachmentNodes?.([id])
}
function updateFile(id: string, change: Partial<Pick<AttachedFile, 'filename' | 'delivery'>>) {
  attachedFiles.value = attachedFiles.value.map((file) => (file.id === id ? { ...file, ...change } : file))
}
function replace(file: AttachedFile) {
  replaceId.value = file.id
  replaceInput.value?.click()
}
async function replaceSelected(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  const oldId = replaceId.value
  if (!file || !oldId) return
  const before = attachedFiles.value.slice()
  const segments = composer.value?.getSegments?.() || []
  const old = before.find((item) => item.id === oldId)
  if (!old) return
  replacing.value = true
  retainedUrls.value = old.url ? [old.url] : []
  try {
    attachedFiles.value = before.filter((item) => item.id !== oldId)
    await handleFileInputChange(event)
    await attachments.waitForAttachments()
    if (!alive) return
    const replacement = attachedFiles.value.find((item) => !before.some((previous) => previous.id === item.id))
    if (replacement) {
      replacement.delivery = old.delivery
      attachedFiles.value = before.map((item) => (item.id === oldId ? replacement : item))
    } else attachedFiles.value = before
    await nextTick()
    composer.value?.restoreSegments?.(
      segments.map((part) =>
        part.type === 'attachment' && part.id === oldId && replacement
          ? { type: 'attachment', id: replacement.id }
          : part,
      ),
    )
  } finally {
    retainedUrls.value = []
    replacing.value = false
    replaceId.value = ''
    input.value = ''
  }
}
async function prepare(file: AttachedFile): Promise<JsonValue> {
  if (!file.blob) throw new Error(t('chat.page.queue.attachmentUnavailable'))
  const uploaded = await chat.uploadWorkspaceAttachment(props.sessionId, {
    filename: file.filename,
    blob: file.blob,
    mime: file.mime,
  })
  // Retain the successful upload if a later file or queue write fails.
  const kind = file.mime.startsWith('image/')
    ? 'image'
    : file.mime.startsWith('audio/')
      ? 'audio'
      : file.mime.startsWith('video/')
        ? 'video'
        : file.mime === 'application/pdf'
          ? 'pdf'
          : 'file'
  const node = {
    type: 'activity',
    activity: {
      id: crypto.randomUUID(),
      provenance: { content_hash: uploaded.sha256 },
      payload: {
        activity_type: 'resource',
        kind,
        name: file.filename,
        media_type: file.mime,
        size_bytes: uploaded.size_bytes,
        delivery: file.delivery || 'reference',
        reference: { reference_type: 'workspace_path', path: uploaded.path },
      },
    },
  } as JsonValue
  file.composerNode = node
  return node
}
async function save() {
  if (!canSave.value || !revision.value) return
  saving.value = true
  error.value = ''
  try {
    const segments: ComposerSegment[] = composer.value?.getSegments?.() || [{ type: 'text', text: draft.value }]
    const document = await queuedEditorDocument(segments, attachedFiles.value, prepare)
    await chat.editQueuedMessage(props.sessionId, props.executionId, revision.value, document)
    emit('close')
  } catch (cause) {
    if (alive) error.value = cause instanceof Error ? cause.message : String(cause)
  } finally {
    saving.value = false
  }
}
function close() {
  if (!saving.value) emit('close')
}
</script>

<template>
  <FormDialog
    :open="true"
    :title="t('chat.page.queue.edit')"
    :description="t('chat.page.queue.editHint')"
    max-width="max-w-2xl"
    @close="close"
  >
    <p v-if="error" role="alert" class="mb-3 text-sm text-destructive">{{ error }}</p>
    <div v-if="loading" class="py-5 text-sm text-muted-foreground">{{ t('chat.page.queue.loading') }}</div>
    <div v-else-if="loaded" class="space-y-4" :inert="saving || replacing || undefined">
      <Composer
        ref="composer"
        v-model:draft="draft"
        :fullscreen="false"
        hide-expand
        :attached-files="attachedFiles"
        :pending-attachments="pendingAttachments"
        class="min-h-40 max-h-[40vh]"
        @remove-attachment="remove"
        @files-selected="handleFileInputChange"
        @paste="handlePaste"
        @drop="handleDrop"
        @draft-keydown="
          (event: KeyboardEvent) => {
            if (
              !event.isComposing &&
              event.keyCode !== 229 &&
              (event.ctrlKey || event.metaKey) &&
              event.key === 'Enter'
            ) {
              event.preventDefault()
              void save()
            }
          }
        "
      />
      <div v-if="attachedFiles.length" class="space-y-2" :aria-label="t('chat.page.attachments')">
        <div
          v-for="file in attachedFiles"
          :key="file.id"
          class="flex flex-wrap items-center gap-2 rounded border border-border p-2 text-sm"
        >
          <input
            v-if="queuedAttachmentIsResource(file)"
            :value="file.filename"
            @input="updateFile(file.id, { filename: ($event.target as HTMLInputElement).value })"
            :aria-label="t('chat.page.queue.attachmentName')"
            class="min-w-0 flex-1 bg-transparent px-1 py-1"
          />
          <span v-else class="min-w-0 flex-1 truncate">{{ file.filename }}</span>
          <select
            v-if="queuedAttachmentIsResource(file)"
            :value="file.delivery"
            @change="
              updateFile(file.id, {
                delivery: ($event.target as HTMLSelectElement).value as 'reference' | 'model_input',
              })
            "
            :aria-label="t('chat.page.queue.attachmentDelivery')"
            class="rounded border border-border bg-background px-1 py-1"
          >
            <option value="reference">{{ t('chat.page.queue.reference') }}</option>
            <option value="model_input">{{ t('chat.page.queue.modelInput') }}</option>
          </select>
          <button
            v-if="queuedAttachmentIsResource(file)"
            type="button"
            :disabled="attachmentsBusy"
            class="px-1 py-1 hover:text-primary"
            @click="replace(file)"
          >
            {{ t('chat.page.queue.replaceAttachment') }}
          </button>
          <button
            type="button"
            class="px-1 py-1 hover:text-destructive"
            :aria-label="`${t('common.remove')} ${file.filename}`"
            @click="remove(file.id)"
          >
            {{ t('common.remove') }}
          </button>
        </div>
      </div>
      <Button variant="outline" size="sm" @click="composer?.openFilePicker?.()">{{
        t('chat.page.queue.addAttachment')
      }}</Button>
      <input ref="replaceInput" type="file" class="hidden" @change="replaceSelected" />
    </div>
    <template #footer>
      <Button variant="ghost" :disabled="saving" @click="close">{{ t('common.cancel') }}</Button>
      <Button :disabled="!canSave" @click="save">{{ saving ? t('common.saving') : t('common.save') }}</Button>
    </template>
  </FormDialog>
</template>
